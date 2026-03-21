use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tokio::time::Instant;

use crate::bluetooth::codecs::AudioCodec;
use crate::bluetooth::device::{DeviceInfo, DeviceState};
use crate::dsp::equalizer::EqBand;

/// Central event bus for all subsystem communication.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum SystemEvent {
    BluetoothStatusChanged {
        status: BluetoothStatus,
    },
    DeviceDiscovered {
        address: String,
        name: String,
        rssi: Option<i16>,
    },
    DeviceStateChanged {
        address: String,
        name: String,
        state: DeviceState,
    },
    DeviceRemoved {
        address: String,
    },
    StreamStarted {
        address: String,
        codec: AudioCodec,
    },
    StreamStopped {
        address: String,
    },
    EqChanged {
        bands: Vec<EqBand>,
        enabled: bool,
    },
    SpectrumData {
        bands: Vec<f32>,
    },
    TrackChanged {
        track: Option<TrackInfo>,
    },
    PlaybackStatusChanged {
        status: PlaybackStatus,
    },
    LineInActivated,
    LineInDeactivated,
    WebRtcAnswer {
        session_id: String,
        sdp: String,
    },
    WebRtcIceCandidate {
        session_id: String,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },
    Error {
        message: String,
    },
    ServiceStopping,
    StateSnapshot {
        state: Box<AppStateSnapshot>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BluetoothStatus {
    Ready,
    Scanning,
    #[default]
    Unavailable,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub track_number: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
    #[default]
    Unknown,
}

impl PlaybackStatus {
    pub fn from_bluez(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "playing" => Self::Playing,
            "paused" => Self::Paused,
            "stopped" => Self::Stopped,
            _ => Self::Unknown,
        }
    }
}

/// Serializable snapshot of the full application state for WebSocket clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStateSnapshot {
    pub status: BluetoothStatus,
    pub devices: Vec<DeviceInfo>,
    pub eq: EqSnapshot,
    pub active_device: Option<String>,
    pub track_info: Option<TrackInfo>,
    pub playback_status: PlaybackStatus,
    pub line_in_active: bool,
    pub line_in_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqSnapshot {
    pub bands: Vec<EqBand>,
    pub enabled: bool,
}

/// Full application state (runtime, not serializable directly).
pub struct AppState {
    pub bluetooth_status: BluetoothStatus,
    pub devices: HashMap<String, DeviceInfo>,
    pub active_device: Option<String>,
    pub eq_bands: Vec<EqBand>,
    pub eq_enabled: bool,
    pub config: crate::state::config::Config,
    pub track_info: Option<TrackInfo>,
    pub playback_status: PlaybackStatus,
    pub line_in_active: bool,
    pub line_in_source: Option<String>,
    pub pipewire_ready: bool,
    pub started_at: Instant,
}

impl AppState {
    pub fn new(config: crate::state::config::Config) -> Self {
        Self {
            bluetooth_status: BluetoothStatus::default(),
            devices: HashMap::new(),
            active_device: None,
            eq_bands: crate::dsp::equalizer::default_bands().to_vec(),
            eq_enabled: true,
            config,
            track_info: None,
            playback_status: PlaybackStatus::default(),
            line_in_active: false,
            line_in_source: None,
            pipewire_ready: false,
            started_at: Instant::now(),
        }
    }

    pub fn snapshot(&self) -> AppStateSnapshot {
        AppStateSnapshot {
            status: self.bluetooth_status.clone(),
            devices: self.devices.values().cloned().collect(),
            eq: EqSnapshot {
                bands: self.eq_bands.clone(),
                enabled: self.eq_enabled,
            },
            active_device: self.active_device.clone(),
            track_info: self.track_info.clone(),
            playback_status: self.playback_status,
            line_in_active: self.line_in_active,
            line_in_available: self.line_in_source.is_some(),
        }
    }
}

/// Thread-safe handle to the application state and event bus.
#[derive(Clone)]
pub struct AppStateHandle {
    pub state: Arc<RwLock<AppState>>,
    pub events: broadcast::Sender<SystemEvent>,
}

impl std::fmt::Debug for AppStateHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppStateHandle").finish()
    }
}

impl AppStateHandle {
    pub fn new(config: crate::state::config::Config) -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            state: Arc::new(RwLock::new(AppState::new(config))),
            events: tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SystemEvent> {
        self.events.subscribe()
    }

    pub fn publish(&self, event: SystemEvent) {
        let _ = self.events.send(event);
    }
}

pub mod config;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_playback_status_from_bluez() {
        assert_eq!(
            PlaybackStatus::from_bluez("playing"),
            PlaybackStatus::Playing
        );
        assert_eq!(PlaybackStatus::from_bluez("paused"), PlaybackStatus::Paused);
        assert_eq!(
            PlaybackStatus::from_bluez("stopped"),
            PlaybackStatus::Stopped
        );
        assert_eq!(
            PlaybackStatus::from_bluez("Playing"),
            PlaybackStatus::Playing
        );
        assert_eq!(
            PlaybackStatus::from_bluez("unknown"),
            PlaybackStatus::Unknown
        );
        assert_eq!(
            PlaybackStatus::from_bluez("garbage"),
            PlaybackStatus::Unknown
        );
    }

    #[test]
    fn test_bluetooth_status_default() {
        assert_eq!(BluetoothStatus::default(), BluetoothStatus::Unavailable);
    }

    #[test]
    fn test_app_state_snapshot() {
        let config = crate::state::config::Config::default();
        let state = AppState::new(config);
        let snap = state.snapshot();
        assert_eq!(snap.status, BluetoothStatus::Unavailable);
        assert!(snap.devices.is_empty());
        assert!(snap.eq.enabled);
        assert_eq!(snap.eq.bands.len(), 10);
        assert!(snap.active_device.is_none());
        assert!(snap.track_info.is_none());
        assert_eq!(snap.playback_status, PlaybackStatus::Unknown);
        assert!(!snap.line_in_active);
        assert!(!snap.line_in_available);
    }

    #[test]
    fn test_app_state_handle_pubsub() {
        let handle = AppStateHandle::new(crate::state::config::Config::default());
        let mut rx = handle.subscribe();
        handle.publish(SystemEvent::LineInActivated);
        let event = rx.try_recv().unwrap();
        assert!(matches!(event, SystemEvent::LineInActivated));
    }
}
