use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::bluetooth::codecs::AudioCodec;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    Disconnected,
    Discovered,
    Pairing,
    Paired,
    Connected,
    ProfileNegotiated,
    PipewireSourceReady,
    AudioActive,
}

impl DeviceState {
    /// Validate that a transition from the current state to `to` is legal.
    pub fn can_transition_to(&self, to: &DeviceState) -> bool {
        use DeviceState::*;
        matches!(
            (self, to),
            (Disconnected, Discovered)
                | (Discovered, Pairing)
                | (Discovered, Paired)
                | (Discovered, Connected)
                | (Discovered, Disconnected)
                | (Pairing, Paired)
                | (Pairing, Disconnected)
                | (Paired, Connected)
                | (Paired, Disconnected)
                | (Connected, ProfileNegotiated)
                | (Connected, Disconnected)
                | (ProfileNegotiated, PipewireSourceReady)
                | (ProfileNegotiated, Disconnected)
                | (PipewireSourceReady, AudioActive)
                | (PipewireSourceReady, Disconnected)
                | (AudioActive, Connected)
                | (AudioActive, Disconnected)
        )
    }

    #[allow(dead_code)]
    pub fn transition(&mut self, to: DeviceState) -> Result<(), String> {
        if self.can_transition_to(&to) {
            *self = to;
            Ok(())
        } else {
            Err(format!(
                "Invalid device state transition: {:?} → {:?}",
                self, to
            ))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub address: String,
    pub name: String,
    pub state: DeviceState,
    pub rssi: Option<i16>,
    pub trusted: bool,
    pub has_a2dp: bool,
    pub codec: Option<AudioCodec>,
    pub last_seen: DateTime<Utc>,
    pub pipewire_node: Option<String>,
}

impl DeviceInfo {
    pub fn new(address: String, name: String) -> Self {
        Self {
            address,
            name,
            state: DeviceState::Discovered,
            rssi: None,
            trusted: false,
            has_a2dp: false,
            codec: None,
            last_seen: Utc::now(),
            pipewire_node: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_transitions() {
        let mut state = DeviceState::Disconnected;
        assert!(state.transition(DeviceState::Discovered).is_ok());
        assert!(state.transition(DeviceState::Pairing).is_ok());
        assert!(state.transition(DeviceState::Paired).is_ok());
        assert!(state.transition(DeviceState::Connected).is_ok());
        assert!(state.transition(DeviceState::ProfileNegotiated).is_ok());
        assert!(state.transition(DeviceState::PipewireSourceReady).is_ok());
        assert!(state.transition(DeviceState::AudioActive).is_ok());
    }

    #[test]
    fn test_invalid_transitions() {
        let mut state = DeviceState::Disconnected;
        assert!(state.transition(DeviceState::AudioActive).is_err());
        assert!(state.transition(DeviceState::Connected).is_err());
    }

    #[test]
    fn test_disconnect_from_any_active() {
        let mut state = DeviceState::AudioActive;
        assert!(state.transition(DeviceState::Disconnected).is_ok());
        assert_eq!(state, DeviceState::Disconnected);
    }

    #[test]
    fn test_discovered_to_connected_shortcut() {
        let mut state = DeviceState::Discovered;
        assert!(state.transition(DeviceState::Connected).is_ok());
    }

    #[test]
    fn test_device_info_new() {
        let dev = DeviceInfo::new("AA:BB:CC:DD:EE:FF".into(), "TestPhone".into());
        assert_eq!(dev.state, DeviceState::Discovered);
        assert!(!dev.trusted);
        assert!(!dev.has_a2dp);
        assert!(dev.codec.is_none());
    }

    #[test]
    fn test_device_state_serialization() {
        let state = DeviceState::AudioActive;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"audio_active\"");
        let back: DeviceState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, DeviceState::AudioActive);
    }
}
