use futures::StreamExt;
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info, warn};

use crate::bluetooth::agent;
use crate::bluetooth::constants;
use crate::bluetooth::device::DeviceState;
use crate::bluetooth::discovery;
use crate::state::{AppStateHandle, BluetoothStatus, SystemEvent};

/// Commands that can be sent to the Bluetooth manager from the web API.
#[derive(Debug)]
pub enum BluetoothCommand {
    StartScan,
    StopScan,
    Connect { address: String },
    Disconnect { address: String },
    Remove { address: String },
    SetName { name: String },
}

pub struct BluetoothManager {
    state: AppStateHandle,
    cmd_rx: mpsc::Receiver<BluetoothCommand>,
    /// Sends the D-Bus connection back once adapter setup is complete,
    /// so that endpoints can be registered on the same connection.
    conn_tx: Option<oneshot::Sender<zbus::Connection>>,
}

impl BluetoothManager {
    pub fn new(
        state: AppStateHandle,
        cmd_rx: mpsc::Receiver<BluetoothCommand>,
        conn_tx: oneshot::Sender<zbus::Connection>,
    ) -> Self {
        Self {
            state,
            cmd_rx,
            conn_tx: Some(conn_tx),
        }
    }

    /// Main run loop for the Bluetooth manager.
    pub async fn run(mut self) {
        info!("Bluetooth manager starting...");

        // Connect to system D-Bus for BlueZ
        let connection = match zbus::Connection::system().await {
            Ok(conn) => conn,
            Err(e) => {
                error!("Failed to connect to system D-Bus: {}", e);
                self.state.publish(SystemEvent::BluetoothStatusChanged {
                    status: BluetoothStatus::Error(format!("D-Bus connection failed: {}", e)),
                });
                return;
            }
        };

        // Register pairing agent
        let auto_pair = self.state.state.read().await.config.auto_pair;
        if let Err(e) = agent::register_agent(&connection, auto_pair).await {
            error!("Failed to register agent: {}", e);
        }

        // Initialize bluer session
        let session = match bluer::Session::new().await {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to create BlueZ session: {}", e);
                self.state.publish(SystemEvent::BluetoothStatusChanged {
                    status: BluetoothStatus::Error(format!("BlueZ session failed: {}", e)),
                });
                return;
            }
        };

        let adapter_name = self.state.state.read().await.config.adapter.clone();
        let adapter = match session.adapter(&adapter_name) {
            Ok(a) => a,
            Err(e) => {
                error!("Failed to get adapter {}: {}", adapter_name, e);
                self.state.publish(SystemEvent::BluetoothStatusChanged {
                    status: BluetoothStatus::Error(format!(
                        "Adapter {} not found: {}",
                        adapter_name, e
                    )),
                });
                return;
            }
        };

        // Power on, set alias, set discoverable, set pairable
        if let Err(e) = adapter.set_powered(true).await {
            warn!("Failed to power on adapter: {}", e);
        }

        let device_name = self.state.state.read().await.config.device_name.clone();
        if let Err(e) = adapter.set_alias(device_name.clone()).await {
            warn!("Failed to set alias: {}", e);
        }

        if let Err(e) = adapter.set_discoverable_timeout(0).await {
            warn!("Failed to set discoverable timeout: {}", e);
        }

        if let Err(e) = adapter.set_discoverable(true).await {
            warn!("Failed to set discoverable: {}", e);
        }

        if let Err(e) = adapter.set_pairable(true).await {
            warn!("Failed to set pairable: {}", e);
        }

        info!(
            "Bluetooth adapter {} powered on as '{}'",
            adapter_name, device_name
        );

        {
            let mut app = self.state.state.write().await;
            app.bluetooth_status = BluetoothStatus::Ready;
        }
        self.state.publish(SystemEvent::BluetoothStatusChanged {
            status: BluetoothStatus::Ready,
        });

        // Send the D-Bus connection to main so endpoints are registered
        // on the same connection (same D-Bus unique name as the agent).
        if let Some(tx) = self.conn_tx.take() {
            let _ = tx.send(connection.clone());
        }

        // Do NOT start discovery automatically - wait for user to click Scan
        let mut discover: Option<
            std::pin::Pin<Box<dyn futures::Stream<Item = bluer::AdapterEvent> + Send>>,
        > = None;

        // Main event loop
        let poll_interval = constants::DEVICE_PROPS_POLL;

        loop {
            // Build the select based on whether discovery is active
            if let Some(ref mut stream) = discover {
                tokio::select! {
                    cmd = self.cmd_rx.recv() => {
                        match cmd {
                            Some(BluetoothCommand::StartScan) => {
                                info!("Discovery already active, refreshing status");
                                let mut app = self.state.state.write().await;
                                app.bluetooth_status = BluetoothStatus::Scanning;
                                drop(app);
                                self.state.publish(SystemEvent::BluetoothStatusChanged {
                                    status: BluetoothStatus::Scanning,
                                });
                            }
                            Some(BluetoothCommand::StopScan) => {
                                info!("Stopping Bluetooth discovery");
                                // Drop the discovery stream to stop BlueZ discovery
                                discover = None;
                                let mut app = self.state.state.write().await;
                                app.bluetooth_status = BluetoothStatus::Ready;
                                drop(app);
                                self.state.publish(SystemEvent::BluetoothStatusChanged {
                                    status: BluetoothStatus::Ready,
                                });
                            }
                            Some(cmd) => self.handle_command(&adapter, cmd).await,
                            None => {
                                info!("Command channel closed, shutting down BT manager");
                                break;
                            }
                        }
                    }

                    event = stream.next() => {
                        match event {
                            Some(evt) => self.handle_adapter_event(&adapter, evt).await,
                            None => {
                                warn!("Discovery stream ended");
                                discover = None;
                                let mut app = self.state.state.write().await;
                                app.bluetooth_status = BluetoothStatus::Ready;
                                drop(app);
                                self.state.publish(SystemEvent::BluetoothStatusChanged {
                                    status: BluetoothStatus::Ready,
                                });
                            }
                        }
                    }

                    _ = tokio::time::sleep(poll_interval) => {
                        self.poll_device_properties(&adapter).await;
                        // Auto-stop scanning when a device is actively streaming audio.
                        // BT scanning and A2DP share the same radio — concurrent
                        // scanning causes audible stuttering in the audio stream.
                        if discover.is_some() && self.has_active_audio_stream().await {
                            info!("Auto-stopping Bluetooth scan — audio stream active");
                            discover = None;
                            let mut app = self.state.state.write().await;
                            app.bluetooth_status = BluetoothStatus::Ready;
                            drop(app);
                            self.state.publish(SystemEvent::BluetoothStatusChanged {
                                status: BluetoothStatus::Ready,
                            });
                        }
                    }
                }
            } else {
                tokio::select! {
                    cmd = self.cmd_rx.recv() => {
                        match cmd {
                            Some(BluetoothCommand::StartScan) => {
                                info!("Starting Bluetooth discovery");
                                match start_discovery(&adapter, &adapter_name).await {
                                    Ok(stream) => {
                                        discover = Some(Box::pin(stream));
                                        let mut app = self.state.state.write().await;
                                        app.bluetooth_status = BluetoothStatus::Scanning;
                                        drop(app);
                                        self.state.publish(SystemEvent::BluetoothStatusChanged {
                                            status: BluetoothStatus::Scanning,
                                        });
                                    }
                                    Err(e) => {
                                        error!("Failed to start discovery: {}", e);
                                        let mut app = self.state.state.write().await;
                                        app.bluetooth_status = BluetoothStatus::Error(
                                            format!("Discovery failed: {}", e),
                                        );
                                        drop(app);
                                        self.state.publish(SystemEvent::BluetoothStatusChanged {
                                            status: BluetoothStatus::Error(
                                                format!("Discovery failed: {}", e),
                                            ),
                                        });
                                    }
                                }
                            }
                            Some(BluetoothCommand::StopScan) => {
                                // Already stopped, just confirm status
                                let mut app = self.state.state.write().await;
                                app.bluetooth_status = BluetoothStatus::Ready;
                                drop(app);
                                self.state.publish(SystemEvent::BluetoothStatusChanged {
                                    status: BluetoothStatus::Ready,
                                });
                            }
                            Some(cmd) => self.handle_command(&adapter, cmd).await,
                            None => {
                                info!("Command channel closed, shutting down BT manager");
                                break;
                            }
                        }
                    }

                    _ = tokio::time::sleep(poll_interval) => {
                        self.poll_device_properties(&adapter).await;
                    }
                }
            }
        }
    }

    async fn handle_adapter_event(&self, adapter: &bluer::Adapter, event: bluer::AdapterEvent) {
        match event {
            bluer::AdapterEvent::DeviceAdded(addr) => match adapter.device(addr) {
                Ok(device) => {
                    let alias = device.alias().await.unwrap_or_default();
                    let name = if alias.is_empty() || discovery::is_mac_address(&alias) {
                        String::new()
                    } else {
                        alias
                    };
                    let rssi = device.rssi().await.ok().flatten();
                    let uuids: Vec<String> = device
                        .uuids()
                        .await
                        .ok()
                        .flatten()
                        .map(|set| set.into_iter().map(|u| u.to_string()).collect())
                        .unwrap_or_default();

                    discovery::handle_device_discovered(
                        &self.state,
                        addr.to_string(),
                        name,
                        rssi,
                        uuids,
                    )
                    .await;
                }
                Err(e) => {
                    warn!("Failed to get discovered device {}: {}", addr, e);
                }
            },
            bluer::AdapterEvent::DeviceRemoved(addr) => {
                discovery::remove_device(&self.state, &addr.to_string()).await;
            }
            _ => {}
        }
    }

    async fn handle_command(&self, adapter: &bluer::Adapter, cmd: BluetoothCommand) {
        match cmd {
            BluetoothCommand::StartScan | BluetoothCommand::StopScan => {
                // Handled in main loop
            }
            BluetoothCommand::Connect { address } => {
                info!("Connecting to {}", address);
                if let Ok(addr) = address.parse() {
                    match adapter.device(addr) {
                        Ok(device) => {
                            // Trust the device to auto-connect profiles
                            if let Err(e) = device.set_trusted(true).await {
                                warn!("Failed to trust {}: {}", address, e);
                            }
                            if let Err(e) = device.connect().await {
                                error!("Failed to connect to {}: {}", address, e);
                                self.state.publish(SystemEvent::Error {
                                    message: format!("Connect failed: {}", e),
                                });
                            } else {
                                // Check if A2DP profile is available
                                let uuids: Vec<String> = device
                                    .uuids()
                                    .await
                                    .ok()
                                    .flatten()
                                    .map(|set| set.into_iter().map(|u| u.to_string()).collect())
                                    .unwrap_or_default();

                                let has_a2dp = uuids.iter().any(|u| {
                                    u.contains("110b") || u.contains("110a") || u.contains("110d")
                                });

                                if has_a2dp {
                                    discovery::update_device_state(
                                        &self.state,
                                        &address,
                                        DeviceState::Connected,
                                    )
                                    .await;
                                    info!("Device {} connected with A2DP support", address);
                                } else {
                                    discovery::update_device_state(
                                        &self.state,
                                        &address,
                                        DeviceState::Connected,
                                    )
                                    .await;
                                    info!("Device {} connected (no A2DP detected)", address);
                                }
                            }
                        }
                        Err(e) => {
                            error!("Device {} not found: {}", address, e);
                        }
                    }
                }
            }
            BluetoothCommand::Disconnect { address } => {
                info!("Disconnecting {}", address);
                if let Ok(addr) = address.parse() {
                    match adapter.device(addr) {
                        Ok(device) => {
                            if let Err(e) = device.disconnect().await {
                                warn!("Failed to disconnect {}: {}", address, e);
                            }
                            discovery::update_device_state(
                                &self.state,
                                &address,
                                DeviceState::Disconnected,
                            )
                            .await;
                        }
                        Err(e) => {
                            warn!("Device {} not found for disconnect: {}", address, e);
                        }
                    }
                }
            }
            BluetoothCommand::Remove { address } => {
                info!("Removing device {}", address);
                if let Ok(addr) = address.parse() {
                    if let Ok(_device) = adapter.device(addr) {
                        if let Err(e) = adapter.remove_device(addr).await {
                            warn!("Failed to remove {}: {}", address, e);
                        }
                    }
                }
                discovery::remove_device(&self.state, &address).await;
            }
            BluetoothCommand::SetName { name } => {
                info!("Setting adapter name to '{}'", name);
                if let Err(e) = adapter.set_alias(name.clone()).await {
                    warn!("Failed to set name: {}", e);
                }
                let mut app = self.state.state.write().await;
                app.config.device_name = name;
                if let Err(e) = app.config.save_to_user_config() {
                    warn!("Failed to persist device name to config: {}", e);
                }
            }
        }
    }

    /// Check if any device is actively streaming audio.
    /// Used to auto-stop scanning — BT scanning and A2DP share the same
    /// radio, so concurrent scanning causes audible stuttering.
    /// Only triggers on AudioActive (actual streaming), not merely Connected,
    /// since pre-existing connections shouldn't prevent the user from scanning.
    async fn has_active_audio_stream(&self) -> bool {
        let app = self.state.state.read().await;
        app.devices
            .values()
            .any(|d| d.state == DeviceState::AudioActive)
    }

    async fn poll_device_properties(&self, adapter: &bluer::Adapter) {
        let addresses: Vec<String> = {
            let app = self.state.state.read().await;
            app.devices.keys().cloned().collect()
        };

        for address in addresses {
            if let Ok(addr) = address.parse() {
                if let Ok(device) = adapter.device(addr) {
                    let connected = device.is_connected().await.unwrap_or(false);
                    let (current_state, current_name) = {
                        let app = self.state.state.read().await;
                        app.devices
                            .get(&address)
                            .map(|d| (d.state.clone(), d.name.clone()))
                            .unzip()
                    };

                    // Re-read device alias to pick up names that BlueZ
                    // resolves after the initial DeviceAdded signal.
                    if let Some(ref name) = current_name {
                        if let Ok(alias) = device.alias().await {
                            if !discovery::is_mac_address(&alias) && alias != *name {
                                discovery::update_device_name(&self.state, &address, alias).await;
                            }
                        }
                    }

                    if let Some(state) = current_state {
                        if connected && state == DeviceState::Discovered {
                            discovery::update_device_state(
                                &self.state,
                                &address,
                                DeviceState::Connected,
                            )
                            .await;
                        } else if !connected
                            && !matches!(state, DeviceState::Disconnected | DeviceState::Discovered)
                        {
                            discovery::update_device_state(
                                &self.state,
                                &address,
                                DeviceState::Disconnected,
                            )
                            .await;
                        }
                    }
                }
            }
        }

        // Check if any connected device now has a PipeWire audio source
        // (meaning WirePlumber acquired the transport)
        if let Some(bt_source) = detect_bt_audio_source().await {
            // Extract address from source name (e.g. "bluez_input.44_4A_DB_B4_E7_0D" -> "44:4A:DB:B4:E7:0D")
            if let Some(addr) = extract_address_from_bt_source(&bt_source) {
                let current_state = {
                    let app = self.state.state.read().await;
                    app.devices.get(&addr).map(|d| d.state.clone())
                };
                if let Some(state) = current_state {
                    if state == DeviceState::Connected {
                        discovery::update_device_state(
                            &self.state,
                            &addr,
                            DeviceState::AudioActive,
                        )
                        .await;
                    }
                }
            }
        }
    }
}

/// Detect if a Bluetooth audio source exists in PipeWire/PulseAudio.
async fn detect_bt_audio_source() -> Option<String> {
    let output = tokio::process::Command::new("pactl")
        .args(["list", "short", "sources"])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        for word in line.split_whitespace() {
            if word.starts_with("bluez_input.") || word.starts_with("bluez_source.") {
                return Some(word.to_string());
            }
        }
    }
    None
}

/// Extract MAC address from a bluez source name.
/// e.g. "bluez_input.44_4A_DB_B4_E7_0D" -> "44:4A:DB:B4:E7:0D"
fn extract_address_from_bt_source(source: &str) -> Option<String> {
    let name = source
        .strip_prefix("bluez_input.")
        .or_else(|| source.strip_prefix("bluez_source."))?;
    // Take only the MAC part (first 17 chars of underscore-separated hex)
    let mac_part: String = name.chars().take(17).collect();
    if mac_part.len() == 17 {
        Some(mac_part.replace('_', ":"))
    } else {
        None
    }
}

/// Start Bluetooth discovery, clearing stale BlueZ state if necessary.
///
/// BlueZ returns "Operation already in progress" if a previous discovery
/// session wasn't cleanly stopped (crash, unclean shutdown, or another
/// process). This function detects that case and recovers by:
/// 1. Calling `StopDiscovery` via busctl (separate D-Bus client)
/// 2. If that fails, power-cycling the adapter as a last resort
async fn start_discovery(
    adapter: &bluer::Adapter,
    adapter_name: &str,
) -> bluer::Result<impl futures::Stream<Item = bluer::AdapterEvent>> {
    match adapter.discover_devices().await {
        Ok(stream) => return Ok(stream),
        Err(e) => {
            let msg = e.to_string();
            if !msg.contains("already in progress") && !msg.contains("InProgress") {
                return Err(e);
            }
            warn!("Discovery already in progress — clearing stale BlueZ session");
        }
    }

    // Try busctl StopDiscovery (runs as a separate D-Bus client, so
    // BlueZ accepts it even though we aren't the session that started it)
    let dbus_path = format!("/org/bluez/{}", adapter_name);
    let stop_result = tokio::process::Command::new("busctl")
        .args([
            "call",
            "org.bluez",
            &dbus_path,
            "org.bluez.Adapter1",
            "StopDiscovery",
        ])
        .output()
        .await;
    match &stop_result {
        Ok(out) if out.status.success() => {
            info!("Stopped stale discovery via busctl");
        }
        Ok(out) => {
            warn!(
                "busctl StopDiscovery failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Err(e) => warn!("Failed to run busctl: {}", e),
    }

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // Retry after stopping
    match adapter.discover_devices().await {
        Ok(stream) => return Ok(stream),
        Err(e) => {
            warn!(
                "Discovery still failing after StopDiscovery — power-cycling adapter: {}",
                e
            );
        }
    }

    // Last resort: power-cycle the adapter to reset all BlueZ state
    let _ = adapter.set_powered(false).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let _ = adapter.set_powered(true).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    adapter.discover_devices().await
}
