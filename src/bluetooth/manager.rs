use futures::StreamExt;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};

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
    Connect {
        address: String,
    },
    Disconnect {
        address: String,
    },
    Remove {
        address: String,
    },
    SetName {
        name: String,
    },
    /// Fire an active HCI Remote-Name-Request against every device in state
    /// whose friendly name hasn't been resolved yet. Works even on BR/EDR
    /// devices that never sent their name in the advertising / EIR payload.
    ResolveNames,
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
            // Promoted to error! because a silent set_alias failure in the
            // past caused an A2DP source (AT-SB727 turntable) to reject us —
            // the adapter was advertising the hostname instead of the
            // configured alias, with no visible diagnostic.
            error!(
                "Failed to set adapter alias '{}' on {}: {}",
                device_name, adapter_name, e
            );
        }

        // Force the Class of Device to "Audio/Video → HiFi Audio Device"
        // (0x240414). BlueZ 5.x no longer honours `Class = ...` in
        // /etc/bluetooth/main.conf and bluer 0.17 exposes no setter on
        // Adapter1, so the adapter starts out advertising "Computer/Laptop".
        // A2DP source devices filter candidates by CoD and skip anything
        // that doesn't look like a sink — which is exactly why an
        // Audio-Technica AT-SB727 could see the adapter during inquiry
        // yet never try to pair with it. hciconfig is the canonical
        // user-space wrapper for the HCI Write_Class_of_Device command
        // and ships with the same bluez package as the daemon.
        set_adapter_class_of_device(&adapter_name).await;

        // Belt-and-braces: also write the HCI-level local name to match
        // the Alias. On every BlueZ I've seen Alias takes precedence in
        // EIR, but there are reports of edge-case firmwares reading the
        // raw HCI name instead — hciconfig `name` uses HCI
        // Write_Local_Name, cheap insurance.
        set_adapter_hci_name(&adapter_name, &device_name).await;

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
                                discover = None;
                                stop_discovery_direct(&connection, &adapter_name).await;
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
                            Some(evt) => self.handle_adapter_event(&adapter, &session, evt).await,
                            None => {
                                warn!("Discovery stream ended");
                                discover = None;
                                // The stream ended but the BlueZ D-Bus discovery
                                // session may still be open — close it explicitly.
                                stop_discovery_direct(&connection, &adapter_name).await;
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
                            stop_discovery_direct(&connection, &adapter_name).await;
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
                                match start_discovery_direct(&adapter, &connection, &adapter_name).await {
                                    Ok(stream) => {
                                        discover = Some(stream);
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

    async fn handle_adapter_event(
        &self,
        adapter: &bluer::Adapter,
        session: &bluer::Session,
        event: bluer::AdapterEvent,
    ) {
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

                    let signals = collect_discovery_signals(&device).await;

                    let address_str = addr.to_string();
                    let needs_name_resolution = name.is_empty();

                    discovery::handle_device_discovered(
                        &self.state,
                        address_str.clone(),
                        name,
                        rssi,
                        uuids,
                        signals,
                    )
                    .await;

                    // BlueZ resolves the GAP friendly name asynchronously;
                    // the regular 500ms property poll eventually picks it up,
                    // but users can stare at a bare MAC for several seconds.
                    // Kick off a per-device fast poll that retires once the
                    // name resolves or the attempt budget runs out.
                    if needs_name_resolution {
                        spawn_name_resolution(self.state.clone(), session.clone(), address_str);
                    }
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
            BluetoothCommand::ResolveNames => {
                let targets: Vec<String> = {
                    let app = self.state.state.read().await;
                    app.devices
                        .values()
                        .filter(|d| d.name.is_empty() || discovery::is_mac_address(&d.name))
                        .map(|d| d.address.clone())
                        .collect()
                };
                let adapter_name = self.state.state.read().await.config.adapter.clone();
                info!(
                    "Active name resolution requested: {} unnamed device(s) on {}",
                    targets.len(),
                    adapter_name
                );
                for address in targets {
                    spawn_active_name_request(self.state.clone(), adapter_name.clone(), address);
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
                    // update_device_name itself rejects empty / MAC-shaped
                    // aliases, but skipping the call upfront avoids a
                    // pointless write-lock on every poll tick.
                    if let Some(ref name) = current_name {
                        if let Ok(alias) = device.alias().await {
                            if !alias.is_empty()
                                && !discovery::is_mac_address(&alias)
                                && alias != *name
                            {
                                discovery::update_device_name(&self.state, &address, alias).await;
                            }
                        }
                    }

                    // Refresh classification: BlueZ may resolve UUIDs / class
                    // of device after the initial DeviceAdded signal. Only
                    // upgrades BLE → Classic (handle_device_discovered guards
                    // against the reverse on stale data).
                    let uuids: Vec<String> = device
                        .uuids()
                        .await
                        .ok()
                        .flatten()
                        .map(|set| set.into_iter().map(|u| u.to_string()).collect())
                        .unwrap_or_default();
                    let signals = collect_discovery_signals(&device).await;
                    discovery::refresh_classification(&self.state, &address, &uuids, &signals)
                        .await;

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

/// Read best-effort BLE/Classic classification signals from a bluer Device.
/// All fields are optional — missing values fall through to the default
/// classification rules.
async fn collect_discovery_signals(device: &bluer::Device) -> discovery::DiscoverySignals {
    let address_type = device.address_type().await.ok().map(|t| match t {
        bluer::AddressType::BrEdr => "br_edr".to_string(),
        bluer::AddressType::LePublic => "public".to_string(),
        bluer::AddressType::LeRandom => "random".to_string(),
    });
    let class_of_device = device.class().await.ok().flatten();
    let has_appearance = device.appearance().await.ok().flatten().is_some();
    discovery::DiscoverySignals {
        address_type,
        class_of_device,
        has_appearance,
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

/// Start Bluetooth discovery using a fresh `bluer::Session`.
///
/// The `bluer` crate tracks discovery sessions internally via a
/// `SingleSessionToken`. When a discovery stream is dropped (e.g. during
/// auto-stop or user stop), the token's async Drop may not complete before
/// the next scan request. This leaves bluer's client-side state thinking a
/// session is active, causing "Operation already in progress" errors even
/// when BlueZ itself has no active discovery.
///
/// The fix:
/// 1. Stop any active BlueZ discovery via direct D-Bus call (bypasses bluer
///    tracking, always works).
/// 2. Wait until the `Discovering` property is actually false.
/// 3. Create a **fresh** `bluer::Session` + `Adapter` with clean session
///    tracking — no stale state from previous scans.
/// 4. Call `discover_devices()` on the fresh adapter, which properly
///    subscribes to ObjectManager signals for newly discovered devices.
///    (`adapter.events()` does NOT emit `DeviceAdded` for discovery results.)
/// 5. Return the stream wrapped so the fresh session stays alive.
///
/// Cleanup (StopScan / auto-stop) uses `stop_discovery_direct()` which
/// calls `StopDiscovery` via D-Bus regardless of bluer's tracking state.
async fn start_discovery_direct(
    _adapter: &bluer::Adapter,
    connection: &zbus::Connection,
    adapter_name: &str,
) -> bluer::Result<std::pin::Pin<Box<dyn futures::Stream<Item = bluer::AdapterEvent> + Send>>> {
    // Stop any existing BlueZ discovery via D-Bus (ignore errors — fine if
    // nothing was running). This ensures BlueZ-level state is clean.
    stop_discovery_direct(connection, adapter_name).await;

    // Wait until Discovering is actually false (up to 2s).
    // Use a temporary adapter to poll the property.
    let poll_session = bluer::Session::new().await?;
    let poll_adapter = poll_session.adapter(adapter_name)?;
    for _ in 0..20 {
        if !poll_adapter.is_discovering().await.unwrap_or(true) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    drop(poll_adapter);
    drop(poll_session);

    // Create a fresh session with clean SingleSessionToken tracking.
    let fresh_session = bluer::Session::new().await?;
    let fresh_adapter = fresh_session.adapter(adapter_name)?;

    // discover_devices() calls StartDiscovery internally and subscribes to
    // ObjectManager InterfacesAdded signals — this is the only reliable way
    // to receive DeviceAdded events for newly discovered devices.
    let stream = fresh_adapter.discover_devices().await?;

    info!("Bluetooth discovery started via fresh bluer session");

    // Wrap the stream so the fresh session + adapter stay alive.
    Ok(Box::pin(FreshDiscoveryStream {
        inner: Box::pin(stream),
        _session: fresh_session,
        _adapter: fresh_adapter,
    }))
}

/// Wrapper stream that keeps a fresh bluer Session + Adapter alive for
/// the lifetime of the discovery stream.
struct FreshDiscoveryStream {
    inner: std::pin::Pin<Box<dyn futures::Stream<Item = bluer::AdapterEvent> + Send>>,
    _session: bluer::Session,
    _adapter: bluer::Adapter,
}

impl futures::Stream for FreshDiscoveryStream {
    type Item = bluer::AdapterEvent;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

/// Cap on concurrent active HCI Remote-Name-Requests. Each request holds a
/// BT baseband slot; a handful at a time is a safe middle ground between
/// "one at a time takes forever" and "flood the radio and stall scanning".
const ACTIVE_NAME_REQUEST_CONCURRENCY: usize = 3;
/// Per-request timeout for `hcitool name`. The HCI Remote-Name-Request
/// procedure itself is bounded (~5.1s default page timeout); we give it a
/// bit of headroom for process startup.
const ACTIVE_NAME_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(7);

/// Global semaphore for active name requests. Created on first use and
/// shared across all spawned tasks so concurrency is capped process-wide
/// even if the user clicks "Identify" multiple times.
fn active_name_request_semaphore() -> std::sync::Arc<tokio::sync::Semaphore> {
    use std::sync::OnceLock;
    static SEM: OnceLock<std::sync::Arc<tokio::sync::Semaphore>> = OnceLock::new();
    SEM.get_or_init(|| {
        std::sync::Arc::new(tokio::sync::Semaphore::new(ACTIVE_NAME_REQUEST_CONCURRENCY))
    })
    .clone()
}

/// Spawn a one-shot task that fires an active HCI Remote-Name-Request via
/// `hcitool name <MAC>` and updates the device name on success.
///
/// Why hcitool: BlueZ's D-Bus `Device1` interface only exposes a passive
/// `Alias` property — there is no "fetch the name now" method. At the HCI
/// layer the Remote-Name-Request procedure fetches the name without
/// pairing or a full connection; `hcitool name` is the canonical
/// user-space wrapper around that command and is shipped with `bluez`
/// itself. Access only requires membership in the `bluetooth` group,
/// which the installer already grants to the service user.
///
/// The task self-terminates after the timeout. On success it calls
/// `discovery::update_device_name` (which refuses empty / MAC-shaped
/// results, so a spurious response can't regress a previously-resolved
/// name).
fn spawn_active_name_request(state: AppStateHandle, adapter_name: String, address: String) {
    let sem = active_name_request_semaphore();
    tokio::spawn(async move {
        // Bail early if the name already got resolved by the passive poller
        // between the click and this task starting.
        {
            let app = state.state.read().await;
            if let Some(d) = app.devices.get(&address) {
                if !d.name.is_empty() && !discovery::is_mac_address(&d.name) {
                    return;
                }
            } else {
                return;
            }
        }

        let _permit = match sem.acquire_owned().await {
            Ok(p) => p,
            Err(_) => return,
        };

        // Re-check after acquiring the permit — another task may have
        // resolved the name while we were queued behind the semaphore.
        {
            let app = state.state.read().await;
            if let Some(d) = app.devices.get(&address) {
                if !d.name.is_empty() && !discovery::is_mac_address(&d.name) {
                    return;
                }
            } else {
                return;
            }
        }

        let cmd = tokio::process::Command::new("hcitool")
            .args(["-i", &adapter_name, "name", &address])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output();

        let output = match tokio::time::timeout(ACTIVE_NAME_REQUEST_TIMEOUT, cmd).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                // hcitool missing or unexecutable — log once per invocation
                // at debug so the passive poller still drives resolution.
                debug!(
                    "hcitool not runnable ({}); active name request for {} skipped",
                    e, address
                );
                return;
            }
            Err(_) => {
                debug!(
                    "hcitool name {} timed out after {:?}",
                    address, ACTIVE_NAME_REQUEST_TIMEOUT
                );
                return;
            }
        };

        if !output.status.success() {
            debug!(
                "hcitool name {} exited {:?}: {}",
                address,
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
            return;
        }

        let resolved = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if resolved.is_empty() || discovery::is_mac_address(&resolved) {
            debug!("hcitool name {} returned no friendly name", address);
            return;
        }

        info!(
            "Active name resolution for {}: '{}' (via hcitool)",
            address, resolved
        );
        discovery::update_device_name(&state, &address, resolved).await;
    });
}

/// Class of Device value advertised by SoundSync: Audio/Video major class
/// (0x04), HiFi Audio Device minor class (0x05), with Rendering + Audio
/// service bits. Matches the CoD most commercial Bluetooth speakers emit,
/// which is what A2DP source devices look for during inquiry selection.
const ADAPTER_COD: &str = "0x240414";
/// Bounded timeout for the hciconfig calls. The HCI Write_Class_of_Device
/// and Write_Local_Name commands are near-instant; 5 s is generous.
const HCICONFIG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Force the adapter's Class of Device to [`ADAPTER_COD`] by shelling out
/// to `hciconfig`. Logs and swallows errors — if hciconfig is missing or
/// exits non-zero we still want SoundSync to keep running, but the user
/// needs to know why A2DP sources might refuse to pair.
async fn set_adapter_class_of_device(adapter_name: &str) {
    let adapter = adapter_name.to_string();
    let result = tokio::time::timeout(
        HCICONFIG_TIMEOUT,
        tokio::process::Command::new("hciconfig")
            .args([&adapter, "class", ADAPTER_COD])
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) if output.status.success() => {
            info!(
                "Adapter {} Class of Device set to {} (Audio/Video → HiFi)",
                adapter_name, ADAPTER_COD
            );
        }
        Ok(Ok(output)) => {
            warn!(
                "hciconfig {} class {} exited {:?}: {}. \
                 A2DP sources may refuse to pair because the adapter \
                 advertises a non-audio CoD. Is the service user in the \
                 `bluetooth` group?",
                adapter_name,
                ADAPTER_COD,
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(Err(e)) => {
            warn!(
                "Could not run hciconfig to set Class of Device: {} \
                 (install `bluez` or add the service user to the `bluetooth` \
                 group). A2DP sources may not find this adapter.",
                e
            );
        }
        Err(_) => {
            warn!(
                "hciconfig {} class {} timed out after {:?}",
                adapter_name, ADAPTER_COD, HCICONFIG_TIMEOUT
            );
        }
    }
}

/// Force the HCI-level local name via `hciconfig <adapter> name <name>`.
/// `adapter.set_alias(...)` handles the Adapter1.Alias D-Bus property which
/// BlueZ uses in EIR responses, but some BT source firmwares read the raw
/// HCI name instead. Keeping both in sync is cheap insurance.
async fn set_adapter_hci_name(adapter_name: &str, device_name: &str) {
    let adapter = adapter_name.to_string();
    let name = device_name.to_string();
    let result = tokio::time::timeout(
        HCICONFIG_TIMEOUT,
        tokio::process::Command::new("hciconfig")
            .args([&adapter, "name", &name])
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) if output.status.success() => {
            info!(
                "Adapter {} HCI-level name set to '{}'",
                adapter_name, device_name
            );
        }
        Ok(Ok(output)) => {
            debug!(
                "hciconfig {} name '{}' exited {:?}: {} (Alias path still in effect)",
                adapter_name,
                device_name,
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(Err(e)) => {
            debug!(
                "Could not run hciconfig to set HCI name: {} (Alias path still in effect)",
                e
            );
        }
        Err(_) => {
            debug!(
                "hciconfig {} name timed out after {:?}",
                adapter_name, HCICONFIG_TIMEOUT
            );
        }
    }
}

/// Per-device fast name-resolution task.
///
/// BlueZ populates the `Alias` property asynchronously after `DeviceAdded`
/// fires. The main 500ms property poll picks names up eventually, but on
/// busy radio environments it can take 5–10s — long enough that users see
/// a wall of MAC addresses and can't tell their device apart. This task
/// polls just `alias()` for a single device on a faster cadence so the
/// friendly name appears within ~1s of becoming available.
///
/// Bails early when:
/// * the device disappeared from app state (Remove / scan reset),
/// * the name was already resolved by another path (main poll, re-scan),
/// * BlueZ returns a non-empty, non-MAC alias (success — call
///   `update_device_name` and exit).
///
/// `update_device_name` itself rejects empty / MAC-shaped aliases, so a
/// transient bad read can't regress a previously-resolved name.
fn spawn_name_resolution(state: AppStateHandle, session: bluer::Session, address: String) {
    tokio::spawn(async move {
        let adapter_name = state.state.read().await.config.adapter.clone();
        let adapter = match session.adapter(&adapter_name) {
            Ok(a) => a,
            Err(e) => {
                debug!(
                    "Name resolution: adapter {} unavailable for {}: {}",
                    adapter_name, address, e
                );
                return;
            }
        };
        let parsed_addr: bluer::Address = match address.parse() {
            Ok(a) => a,
            Err(e) => {
                debug!("Name resolution: bad address {}: {}", address, e);
                return;
            }
        };
        let device = match adapter.device(parsed_addr) {
            Ok(d) => d,
            Err(e) => {
                debug!("Name resolution: device {} unavailable: {}", address, e);
                return;
            }
        };

        for _ in 0..constants::NAME_RESOLUTION_MAX_ATTEMPTS {
            tokio::time::sleep(constants::NAME_RESOLUTION_POLL).await;

            // Check device presence + current name in a single read.
            let current_name = {
                let app = state.state.read().await;
                match app.devices.get(&address) {
                    Some(d) => d.name.clone(),
                    None => return,
                }
            };
            if !current_name.is_empty() {
                return;
            }

            if let Ok(alias) = device.alias().await {
                if !alias.is_empty() && !discovery::is_mac_address(&alias) {
                    discovery::update_device_name(&state, &address, alias).await;
                    return;
                }
            }
        }
    });
}

/// Stop Bluetooth discovery via direct D-Bus call.
async fn stop_discovery_direct(connection: &zbus::Connection, adapter_name: &str) {
    let path: zbus::zvariant::OwnedObjectPath =
        format!("/org/bluez/{}", adapter_name).try_into().unwrap();
    let result: Result<zbus::Message, _> = connection
        .call_method(
            Some("org.bluez"),
            &path,
            Some("org.bluez.Adapter1"),
            "StopDiscovery",
            &(),
        )
        .await;
    match result {
        Ok(_) => info!("Bluetooth discovery stopped"),
        Err(e) => warn!("StopDiscovery D-Bus call failed: {}", e),
    }
}
