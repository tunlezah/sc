use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use zbus::zvariant::OwnedValue;

use crate::bluetooth::constants::{AVRCP_POLL_ACTIVE, AVRCP_POLL_IDLE};
use crate::state::{AppStateHandle, PlaybackStatus, SystemEvent, TrackInfo};

/// Commands for AVRCP playback control.
#[derive(Debug)]
pub enum AvrcpCommand {
    Play,
    Pause,
    #[allow(dead_code)]
    Stop,
    Next,
    Previous,
}

/// Number of consecutive D-Bus failures before triggering a reconnect.
const DBUS_RECONNECT_THRESHOLD: u32 = 5;

pub struct AvrcpMonitor {
    state: AppStateHandle,
    cmd_rx: mpsc::Receiver<AvrcpCommand>,
    adapter_name: String,
    last_track: Option<TrackInfo>,
    last_status: PlaybackStatus,
    /// Consecutive poll failures (used to detect stale D-Bus connection).
    consecutive_failures: u32,
}

impl AvrcpMonitor {
    pub fn new(
        state: AppStateHandle,
        cmd_rx: mpsc::Receiver<AvrcpCommand>,
        adapter_name: String,
    ) -> Self {
        Self {
            state,
            cmd_rx,
            adapter_name,
            last_track: None,
            last_status: PlaybackStatus::Unknown,
            consecutive_failures: 0,
        }
    }

    /// Establish (or re-establish) a D-Bus system connection.
    async fn connect_dbus() -> Option<zbus::Connection> {
        match zbus::Connection::system().await {
            Ok(conn) => {
                info!("AVRCP: D-Bus connection established");
                Some(conn)
            }
            Err(e) => {
                error!("AVRCP: Failed to connect to D-Bus: {}", e);
                None
            }
        }
    }

    pub async fn run(mut self) {
        info!("AVRCP monitor starting...");

        let mut connection = match Self::connect_dbus().await {
            Some(conn) => conn,
            None => return,
        };

        loop {
            // Reconnect D-Bus if too many consecutive failures
            if self.consecutive_failures >= DBUS_RECONNECT_THRESHOLD {
                warn!(
                    "AVRCP: {} consecutive D-Bus failures — reconnecting",
                    self.consecutive_failures
                );
                self.consecutive_failures = 0;
                match Self::connect_dbus().await {
                    Some(conn) => {
                        connection = conn;
                        info!("AVRCP: D-Bus reconnected successfully");
                    }
                    None => {
                        // Wait before retrying
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                }
            }

            let poll_interval = self.current_poll_interval().await;

            tokio::select! {
                cmd = self.cmd_rx.recv() => {
                    match cmd {
                        Some(cmd) => self.handle_command(&connection, cmd).await,
                        None => {
                            info!("AVRCP command channel closed");
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep(poll_interval) => {
                    self.poll_media_player(&connection).await;
                }
            }
        }
    }

    async fn current_poll_interval(&self) -> Duration {
        let app = self.state.state.read().await;
        // Poll fast if any device is connected (not just AudioActive).
        // The reference BluetoothA2DP project polls at 250ms for Connected,
        // ProfileNegotiated, PipewireSourceReady, and AudioActive states.
        let has_connected = app.active_device.is_some()
            || app.devices.values().any(|d| {
                matches!(
                    d.state,
                    crate::bluetooth::device::DeviceState::Connected
                        | crate::bluetooth::device::DeviceState::AudioActive
                )
            });
        if has_connected {
            AVRCP_POLL_ACTIVE
        } else {
            AVRCP_POLL_IDLE
        }
    }

    async fn handle_command(&mut self, connection: &zbus::Connection, cmd: AvrcpCommand) {
        let player_path = match self.get_player_path().await {
            Some(p) => p,
            None => {
                warn!("AVRCP: No active device for command {:?}", cmd);
                return;
            }
        };

        let proxy = match zbus::Proxy::new(
            connection,
            "org.bluez",
            player_path.as_str(),
            "org.bluez.MediaPlayer1",
        )
        .await
        {
            Ok(p) => p,
            Err(e) => {
                error!("AVRCP: Failed to create proxy for command: {}", e);
                self.consecutive_failures += 1;
                return;
            }
        };

        let method = match cmd {
            AvrcpCommand::Play => "Play",
            AvrcpCommand::Pause => "Pause",
            AvrcpCommand::Stop => "Stop",
            AvrcpCommand::Next => "Next",
            AvrcpCommand::Previous => "Previous",
        };

        if let Err(e) = proxy.call_method(method, &()).await {
            warn!(
                "AVRCP: {} failed: {} (will trigger reconnect after {} more failures)",
                method,
                e,
                DBUS_RECONNECT_THRESHOLD.saturating_sub(self.consecutive_failures + 1)
            );
            self.consecutive_failures += 1;
        } else {
            info!("AVRCP: {} executed", method);
            self.consecutive_failures = 0;
        }
    }

    async fn get_player_path(&self) -> Option<String> {
        let app = self.state.state.read().await;
        // Try active_device first, then fall back to any connected device.
        // Without custom A2DP endpoints, active_device may not be set until
        // WirePlumber creates a bluez_input.* node, but AVRCP should still
        // work for any connected device.
        let addr = app.active_device.clone().or_else(|| {
            app.devices
                .iter()
                .find(|(_, d)| {
                    matches!(
                        d.state,
                        crate::bluetooth::device::DeviceState::Connected
                            | crate::bluetooth::device::DeviceState::AudioActive
                    )
                })
                .map(|(addr, _)| addr.clone())
        })?;
        Some(format!(
            "/org/bluez/{}/dev_{}/player0",
            self.adapter_name,
            addr.replace(':', "_")
        ))
    }

    /// Try to extract album artwork from AVRCP cover art.
    ///
    /// BlueZ may expose artwork via:
    /// - The `Track` dict's `Image` key (some implementations)
    /// - `org.bluez.MediaItem1` interface
    /// - `org.bluez.MediaPlayer1` Browsing interface
    ///
    /// Returns true if artwork was successfully extracted and stored.
    async fn try_extract_artwork(
        connection: &zbus::Connection,
        player_path: &str,
        state: &AppStateHandle,
    ) -> bool {
        // Method 1: Check for Image/CoverArt in Track properties
        let proxy = match zbus::Proxy::new(
            connection,
            "org.bluez",
            player_path,
            "org.bluez.MediaPlayer1",
        )
        .await
        {
            Ok(p) => p,
            Err(_) => return false,
        };

        // Try to get cover art properties that some BlueZ builds expose
        if let Ok(track_map) = proxy
            .get_property::<HashMap<String, OwnedValue>>("Track")
            .await
        {
            // Some BT implementations provide an Image URL or data
            if let Some(image_val) = track_map.get("Image") {
                if let Ok(image_path) = <&str as TryFrom<&OwnedValue>>::try_from(image_val) {
                    if !image_path.is_empty() {
                        // If it's a file path, try to read it
                        if let Ok(data) = tokio::fs::read(image_path).await {
                            let mime = if image_path.ends_with(".png") {
                                "image/png"
                            } else {
                                "image/jpeg"
                            };
                            let mut app = state.state.write().await;
                            app.artwork_data = Some(data);
                            app.artwork_mime = Some(mime.to_string());
                            info!("Album artwork extracted from AVRCP ({})", image_path);
                            return true;
                        }
                    }
                }
            }
        }

        // No artwork available — clear any stale artwork
        {
            let mut app = state.state.write().await;
            app.artwork_data = None;
            app.artwork_mime = None;
        }
        false
    }

    async fn poll_media_player(&mut self, connection: &zbus::Connection) {
        let player_path = match self.get_player_path().await {
            Some(p) => p,
            None => return,
        };

        let proxy = match zbus::Proxy::new(
            connection,
            "org.bluez",
            player_path.as_str(),
            "org.bluez.MediaPlayer1",
        )
        .await
        {
            Ok(p) => p,
            Err(e) => {
                self.consecutive_failures += 1;
                if self.consecutive_failures == 1 || self.consecutive_failures.is_multiple_of(10) {
                    warn!(
                        "AVRCP: poll proxy creation failed (attempt {}): {}",
                        self.consecutive_failures, e
                    );
                }
                return;
            }
        };

        // Poll status — track D-Bus health via success/failure
        let mut poll_succeeded = false;
        if let Ok(status_val) = proxy.get_property::<String>("Status").await {
            poll_succeeded = true;
            let status = PlaybackStatus::from_bluez(&status_val);
            if status != self.last_status {
                self.last_status = status;
                {
                    let mut app = self.state.state.write().await;
                    app.playback_status = status;
                }
                self.state
                    .publish(SystemEvent::PlaybackStatusChanged { status });
            }
        }

        // Poll track info
        if let Ok(track_map) = proxy
            .get_property::<HashMap<String, OwnedValue>>("Track")
            .await
        {
            poll_succeeded = true;
            let mut track = parse_track_info(&track_map);
            let changed = match (&self.last_track, &track) {
                (None, None) => false,
                (Some(_), None) | (None, Some(_)) => true,
                (Some(a), Some(b)) => a.title != b.title || a.artist != b.artist,
            };

            if changed {
                // Try to extract album artwork from AVRCP cover art
                let has_artwork =
                    Self::try_extract_artwork(connection, &player_path, &self.state).await;
                if let Some(ref mut t) = track {
                    if has_artwork {
                        t.artwork_url = Some("/api/artwork".to_string());
                    }
                }

                self.last_track.clone_from(&track);
                {
                    let mut app = self.state.state.write().await;
                    app.track_info.clone_from(&track);
                }
                self.state.publish(SystemEvent::TrackChanged { track });
            }
        }

        // Reset failure counter on successful poll, increment otherwise
        if poll_succeeded {
            if self.consecutive_failures > 0 {
                info!(
                    "AVRCP: D-Bus poll recovered after {} failures",
                    self.consecutive_failures
                );
            }
            self.consecutive_failures = 0;
        } else {
            self.consecutive_failures += 1;
            if self.consecutive_failures == 1 || self.consecutive_failures.is_multiple_of(10) {
                warn!(
                    "AVRCP: D-Bus poll failed ({} consecutive) — metadata may be stale",
                    self.consecutive_failures
                );
            }
        }
    }
}

/// Parse track metadata from BlueZ MediaPlayer1 Track property.
pub fn parse_track_info(map: &HashMap<String, OwnedValue>) -> Option<TrackInfo> {
    let title = get_string_value(map, "Title")?;
    if title.is_empty() {
        return None;
    }

    Some(TrackInfo {
        title,
        artist: get_string_value(map, "Artist").unwrap_or_default(),
        album: get_string_value(map, "Album").unwrap_or_default(),
        duration_ms: get_u64_value(map, "Duration").unwrap_or(0),
        track_number: get_u32_value(map, "TrackNumber"),
        artwork_url: None, // Set later if artwork is available
    })
}

fn get_string_value(map: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    map.get(key).and_then(|v| {
        <&str as TryFrom<&OwnedValue>>::try_from(v)
            .ok()
            .map(|s| s.to_string())
    })
}

fn get_u64_value(map: &HashMap<String, OwnedValue>, key: &str) -> Option<u64> {
    map.get(key).and_then(|v| {
        if let Ok(n) = <u32 as TryFrom<&OwnedValue>>::try_from(v) {
            Some(n as u64)
        } else {
            <u64 as TryFrom<&OwnedValue>>::try_from(v).ok()
        }
    })
}

fn get_u32_value(map: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    map.get(key)
        .and_then(|v| <u32 as TryFrom<&OwnedValue>>::try_from(v).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_track_info_empty() {
        let map = HashMap::new();
        assert!(parse_track_info(&map).is_none());
    }
}
