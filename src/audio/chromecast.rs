use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use mdns_sd::{ServiceDaemon, ServiceEvent};
use rust_cast::channels::heartbeat::HeartbeatResponse;
use rust_cast::channels::media::{Media, StreamType};
use rust_cast::channels::receiver::CastDeviceApp;
use rust_cast::{CastDevice, ChannelMessage};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::audio::cast_stream::detect_local_ip;
use crate::state::{AppStateHandle, SystemEvent};

const MDNS_SERVICE_TYPE: &str = "_googlecast._tcp.local.";
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_MEDIA_RECEIVER_APP_ID: &str = "CC1AD845";

/// Commands sent from the web API to the Chromecast manager.
#[derive(Debug)]
pub enum ChromecastCommand {
    Discover,
    Connect { device_id: String },
    Disconnect,
    SetVolume { level: f32 },
}

/// Information about a discovered Chromecast device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastDeviceInfo {
    pub id: String,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub model: String,
}

/// Active Chromecast streaming session state.
struct CastSession {
    device_info: CastDeviceInfo,
    heartbeat_task: JoinHandle<()>,
    stream_task: JoinHandle<()>,
}

/// Manages Chromecast device discovery, connections, and streaming sessions.
///
/// Follows the same command/manager pattern as `WebRtcManager`: a command enum
/// is sent via an mpsc channel, and the manager processes commands in a `run()` loop.
///
/// Discovery uses mDNS to find `_googlecast._tcp.local.` services on the network.
/// Streaming works by telling the Chromecast to load the HTTP MP3 stream URL
/// served by `cast_stream::stream_audio_mp3`, which pulls audio from the existing
/// PCM broadcast channel. This ensures all audio flows through the core pipeline.
pub struct ChromecastManager {
    state: AppStateHandle,
    server_port: u16,
    discovered_devices: Arc<Mutex<HashMap<String, CastDeviceInfo>>>,
    active_session: Option<CastSession>,
}

impl ChromecastManager {
    pub fn new(state: AppStateHandle, server_port: u16) -> Self {
        Self {
            state,
            server_port,
            discovered_devices: Arc::new(Mutex::new(HashMap::new())),
            active_session: None,
        }
    }

    /// Run the Chromecast manager, processing commands from the channel.
    pub async fn run(mut self, mut cmd_rx: mpsc::Receiver<ChromecastCommand>) {
        info!("Chromecast manager started");
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                ChromecastCommand::Discover => {
                    self.handle_discover().await;
                }
                ChromecastCommand::Connect { device_id } => {
                    self.handle_connect(&device_id).await;
                }
                ChromecastCommand::Disconnect => {
                    self.handle_disconnect().await;
                }
                ChromecastCommand::SetVolume { level } => {
                    self.handle_set_volume(level).await;
                }
            }
        }
        info!("Chromecast manager shutting down");

        // Cleanup active session
        self.handle_disconnect().await;
    }

    /// Discover Chromecast devices on the network via mDNS.
    async fn handle_discover(&self) {
        info!("Starting Chromecast device discovery");
        let devices = self.discovered_devices.clone();
        let state = self.state.clone();

        // Run discovery in a blocking task since mdns-sd uses synchronous APIs
        tokio::task::spawn(async move {
            let mdns = match ServiceDaemon::new() {
                Ok(d) => d,
                Err(e) => {
                    error!("Failed to create mDNS daemon: {}", e);
                    state.publish(SystemEvent::CastError {
                        message: format!("mDNS initialization failed: {}", e),
                    });
                    return;
                }
            };

            let receiver = match mdns.browse(MDNS_SERVICE_TYPE) {
                Ok(r) => r,
                Err(e) => {
                    error!("Failed to browse mDNS: {}", e);
                    state.publish(SystemEvent::CastError {
                        message: format!("mDNS browse failed: {}", e),
                    });
                    return;
                }
            };

            let deadline = tokio::time::Instant::now() + DISCOVERY_TIMEOUT;

            loop {
                let timeout = deadline.saturating_duration_since(tokio::time::Instant::now());
                if timeout.is_zero() {
                    break;
                }

                match tokio::time::timeout(
                    timeout,
                    tokio::task::spawn_blocking({
                        let receiver = receiver.clone();
                        move || receiver.recv_timeout(Duration::from_secs(2))
                    }),
                )
                .await
                {
                    Ok(Ok(Ok(event))) => match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let addresses_v4 = info.get_addresses_v4();
                            if let Some(addr) = addresses_v4.into_iter().next() {
                                let device_name = info
                                    .get_property_val_str("fn")
                                    .unwrap_or_else(|| info.get_fullname())
                                    .to_string();
                                let model = info
                                    .get_property_val_str("md")
                                    .unwrap_or("Chromecast")
                                    .to_string();
                                let device_id = info
                                    .get_property_val_str("id")
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| format!("{}:{}", addr, info.get_port()));

                                let device_info = CastDeviceInfo {
                                    id: device_id.clone(),
                                    name: device_name.clone(),
                                    address: addr.to_string(),
                                    port: info.get_port(),
                                    model: model.clone(),
                                };

                                info!(
                                    "Discovered Chromecast: {} ({}) at {}:{}",
                                    device_name,
                                    model,
                                    addr,
                                    info.get_port()
                                );

                                let mut devs = devices.blocking_lock();
                                devs.insert(device_id.clone(), device_info.clone());
                                drop(devs);

                                state.publish(SystemEvent::CastDeviceDiscovered {
                                    device: device_info,
                                });
                            }
                        }
                        ServiceEvent::ServiceRemoved(_type, fullname) => {
                            let mut devs = devices.blocking_lock();
                            // Find and remove device by matching fullname
                            let removed_id: Option<String> = devs
                                .iter()
                                .find(|(_, d)| {
                                    fullname.contains(&d.name) || fullname.contains(&d.id)
                                })
                                .map(|(id, _)| id.clone());
                            if let Some(id) = removed_id {
                                devs.remove(&id);
                                drop(devs);
                                state.publish(SystemEvent::CastDeviceRemoved { device_id: id });
                            }
                        }
                        _ => {}
                    },
                    Ok(Ok(Err(_))) => {
                        // Timeout on recv, continue
                    }
                    Ok(Err(e)) => {
                        warn!("mDNS discovery task error: {}", e);
                        break;
                    }
                    Err(_) => {
                        // Overall timeout reached
                        break;
                    }
                }
            }

            let _ = mdns.shutdown();
            info!("Chromecast discovery completed");
        });
    }

    /// Connect to a Chromecast device and start streaming the HTTP MP3 stream.
    async fn handle_connect(&mut self, device_id: &str) {
        // Disconnect any existing session first
        self.handle_disconnect().await;

        let device_info = {
            let devs = self.discovered_devices.lock().await;
            match devs.get(device_id) {
                Some(d) => d.clone(),
                None => {
                    warn!("Chromecast device not found: {}", device_id);
                    self.state.publish(SystemEvent::CastError {
                        message: format!("Device not found: {}", device_id),
                    });
                    return;
                }
            }
        };

        info!(
            "Connecting to Chromecast: {} at {}:{}",
            device_info.name, device_info.address, device_info.port
        );

        let local_ip = detect_local_ip();
        let stream_url = format!(
            "http://{}:{}/api/stream/audio.mp3",
            local_ip, self.server_port
        );

        let address = device_info.address.clone();
        let port = device_info.port;

        // Connect and start media in a blocking task (rust_cast uses synchronous I/O)
        let connect_result =
            tokio::task::spawn_blocking(move || connect_and_play(&address, port, &stream_url))
                .await;

        match connect_result {
            Ok(Ok(transport_id)) => {
                info!(
                    "Chromecast connected: {} (transport: {})",
                    device_info.name, transport_id
                );

                // Start heartbeat task to keep connection alive
                let hb_address = device_info.address.clone();
                let hb_port = device_info.port;
                let hb_state = self.state.clone();
                let hb_device_id = device_info.id.clone();
                let heartbeat_task = tokio::spawn(async move {
                    run_heartbeat_loop(hb_address, hb_port, hb_state, hb_device_id).await;
                });

                // Spawn a monitoring task that keeps the session alive
                let monitor_address = device_info.address.clone();
                let monitor_port = device_info.port;
                let monitor_state = self.state.clone();
                let monitor_device_id = device_info.id.clone();
                let stream_task = tokio::spawn(async move {
                    run_session_monitor(
                        monitor_address,
                        monitor_port,
                        monitor_state,
                        monitor_device_id,
                    )
                    .await;
                });

                self.active_session = Some(CastSession {
                    device_info: device_info.clone(),
                    heartbeat_task,
                    stream_task,
                });

                // Update state
                {
                    let mut app = self.state.state.write().await;
                    app.cast_active = Some(device_info.id.clone());
                }

                self.state.publish(SystemEvent::CastSessionStarted {
                    device: device_info,
                });
            }
            Ok(Err(e)) => {
                error!("Failed to connect to Chromecast: {}", e);
                self.state.publish(SystemEvent::CastError {
                    message: format!("Connection failed: {}", e),
                });
            }
            Err(e) => {
                error!("Chromecast connect task panicked: {}", e);
                self.state.publish(SystemEvent::CastError {
                    message: "Connection task failed".to_string(),
                });
            }
        }
    }

    /// Disconnect from the active Chromecast session.
    async fn handle_disconnect(&mut self) {
        if let Some(session) = self.active_session.take() {
            info!(
                "Disconnecting from Chromecast: {}",
                session.device_info.name
            );

            session.heartbeat_task.abort();
            session.stream_task.abort();

            // Send stop command to the Chromecast
            let address = session.device_info.address.clone();
            let port = session.device_info.port;
            let _ = tokio::task::spawn_blocking(move || {
                disconnect_device(&address, port);
            })
            .await;

            let device_id = session.device_info.id.clone();

            // Update state
            {
                let mut app = self.state.state.write().await;
                app.cast_active = None;
            }

            self.state
                .publish(SystemEvent::CastSessionStopped { device_id });
        }
    }

    /// Set volume on the active Chromecast device.
    async fn handle_set_volume(&self, level: f32) {
        if let Some(ref session) = self.active_session {
            let address = session.device_info.address.clone();
            let port = session.device_info.port;
            let volume = level.clamp(0.0, 1.0);

            let result =
                tokio::task::spawn_blocking(move || set_device_volume(&address, port, volume))
                    .await;

            match result {
                Ok(Ok(())) => {
                    debug!("Chromecast volume set to {:.0}%", level * 100.0);
                }
                Ok(Err(e)) => {
                    warn!("Failed to set Chromecast volume: {}", e);
                }
                Err(e) => {
                    warn!("Volume task panicked: {}", e);
                }
            }
        }
    }
}

/// Connect to a Chromecast device and tell it to play the MP3 stream URL.
/// Returns the transport ID for the media session.
fn connect_and_play(address: &str, port: u16, stream_url: &str) -> Result<String, String> {
    let device = CastDevice::connect_without_host_verification(address, port)
        .map_err(|e| format!("TCP connect failed: {}", e))?;

    // Connect to the receiver
    device
        .connection
        .connect("receiver-0")
        .map_err(|e| format!("Connection channel connect failed: {}", e))?;

    // Launch or get the default media receiver app
    let app = device
        .receiver
        .launch_app(&CastDeviceApp::DefaultMediaReceiver)
        .map_err(|e| format!("Failed to launch media receiver: {}", e))?;

    let transport_id = app.transport_id.clone();
    let session_id = app.session_id.clone();

    // Connect to the media receiver transport
    device
        .connection
        .connect(&transport_id)
        .map_err(|e| format!("Failed to connect to transport: {}", e))?;

    // Load the MP3 stream
    let media = Media {
        content_id: stream_url.to_string(),
        content_type: "audio/mpeg".to_string(),
        stream_type: StreamType::Live,
        duration: None,
        metadata: None,
    };

    device
        .media
        .load(&transport_id, &session_id, &media)
        .map_err(|e| format!("Failed to load media: {}", e))?;

    info!(
        "Chromecast playing: {} (transport: {})",
        stream_url, transport_id
    );

    Ok(transport_id)
}

/// Periodically sends heartbeat pings to keep the Chromecast connection alive.
async fn run_heartbeat_loop(address: String, port: u16, state: AppStateHandle, device_id: String) {
    let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
    let mut consecutive_failures: u32 = 0;
    const MAX_FAILURES: u32 = 3;

    loop {
        interval.tick().await;

        let addr = address.clone();
        let result = tokio::task::spawn_blocking(move || send_heartbeat(&addr, port)).await;

        match result {
            Ok(Ok(())) => {
                consecutive_failures = 0;
            }
            Ok(Err(e)) => {
                consecutive_failures += 1;
                warn!(
                    "Chromecast heartbeat failed ({}/{}): {}",
                    consecutive_failures, MAX_FAILURES, e
                );
                if consecutive_failures >= MAX_FAILURES {
                    error!("Chromecast heartbeat lost after {} failures", MAX_FAILURES);
                    state.publish(SystemEvent::CastError {
                        message: "Connection lost (heartbeat timeout)".to_string(),
                    });
                    state.publish(SystemEvent::CastSessionStopped {
                        device_id: device_id.clone(),
                    });
                    {
                        let mut app = state.state.write().await;
                        app.cast_active = None;
                    }
                    break;
                }
            }
            Err(_) => {
                // Task panicked, stop heartbeat
                break;
            }
        }
    }
}

/// Monitor the Chromecast session for status changes and errors.
async fn run_session_monitor(address: String, port: u16, state: AppStateHandle, device_id: String) {
    let mut interval = tokio::time::interval(Duration::from_secs(10));

    loop {
        interval.tick().await;

        let addr = address.clone();
        let result = tokio::task::spawn_blocking(move || check_session_status(&addr, port)).await;

        match result {
            Ok(Ok(active)) => {
                if !active {
                    info!("Chromecast session ended (device idle)");
                    state.publish(SystemEvent::CastSessionStopped {
                        device_id: device_id.clone(),
                    });
                    {
                        let mut app = state.state.write().await;
                        app.cast_active = None;
                    }
                    break;
                }
            }
            Ok(Err(e)) => {
                debug!("Session monitor check failed: {}", e);
                // Don't break on individual check failures; the heartbeat
                // loop handles connection loss.
            }
            Err(_) => break,
        }
    }
}

/// Send a heartbeat ping to the Chromecast.
fn send_heartbeat(address: &str, port: u16) -> Result<(), String> {
    let device = CastDevice::connect_without_host_verification(address, port)
        .map_err(|e| format!("Heartbeat connect failed: {}", e))?;

    device
        .connection
        .connect("receiver-0")
        .map_err(|e| format!("Heartbeat connection failed: {}", e))?;

    device
        .heartbeat
        .ping()
        .map_err(|e| format!("Heartbeat ping failed: {}", e))?;

    // Wait briefly for pong
    match device.receive() {
        Ok(ChannelMessage::Heartbeat(HeartbeatResponse::Pong)) => Ok(()),
        Ok(_) => Ok(()), // Any response means device is alive
        Err(e) => Err(format!("Heartbeat response error: {}", e)),
    }
}

/// Check if the Chromecast session is still active.
fn check_session_status(address: &str, port: u16) -> Result<bool, String> {
    let device = CastDevice::connect_without_host_verification(address, port)
        .map_err(|e| format!("Status check connect failed: {}", e))?;

    device
        .connection
        .connect("receiver-0")
        .map_err(|e| format!("Status connection failed: {}", e))?;

    let status = device
        .receiver
        .get_status()
        .map_err(|e| format!("Get status failed: {}", e))?;

    // Check if the default media receiver is still running
    let has_active_app = status
        .applications
        .iter()
        .any(|app| app.app_id == DEFAULT_MEDIA_RECEIVER_APP_ID);

    Ok(has_active_app)
}

/// Disconnect from a Chromecast device by stopping the receiver.
fn disconnect_device(address: &str, port: u16) {
    match CastDevice::connect_without_host_verification(address, port) {
        Ok(device) => {
            let _ = device.connection.connect("receiver-0");
            // Get the running app's session_id to stop it
            if let Ok(status) = device.receiver.get_status() {
                for app in &status.applications {
                    if app.app_id == DEFAULT_MEDIA_RECEIVER_APP_ID {
                        let _ = device.receiver.stop_app(&app.session_id);
                        info!(
                            "Chromecast stop command sent for session {}",
                            app.session_id
                        );
                        return;
                    }
                }
            }
            info!("No active media receiver app found to stop");
        }
        Err(e) => {
            warn!("Could not connect to send stop command: {}", e);
        }
    }
}

/// Set volume on the Chromecast device.
fn set_device_volume(address: &str, port: u16, level: f32) -> Result<(), String> {
    let device = CastDevice::connect_without_host_verification(address, port)
        .map_err(|e| format!("Volume connect failed: {}", e))?;

    device
        .connection
        .connect("receiver-0")
        .map_err(|e| format!("Volume connection failed: {}", e))?;

    device
        .receiver
        .set_volume(level)
        .map_err(|e| format!("Set volume failed: {}", e))?;

    Ok(())
}
