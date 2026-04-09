use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::bluetooth::codecs::AudioCodec;
use crate::bluetooth::constants::{A2DP_SINK_UUID, A2DP_SOURCE_UUID};

/// Classification of a Bluetooth device based on its discovery signals.
///
/// `Default::default()` is `Ble`. Per task spec "lacks BR/EDR service UUIDs"
/// is the BLE fallback — we only upgrade to Classic on a positive Classic
/// signal (CoD or known Classic UUID). Once Classic, the classification is
/// sticky (see `handle_device_discovered`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    /// Classic Bluetooth (BR/EDR) — candidate for A2DP audio sources.
    Classic,
    /// Bluetooth Low Energy — mostly irrelevant for audio.
    #[default]
    Ble,
}

/// Classify a device as BLE or Classic based on best-effort signals from BlueZ.
///
/// Logic:
/// * A valid Class of Device (CoD) → Classic (CoD only exists for BR/EDR).
/// * Any known Classic service UUID (A2DP sink/source, HFP, etc.) → Classic.
/// * Otherwise → BLE (per task spec: "lacks BR/EDR service UUIDs" → BLE).
///
/// `address_type` and `has_appearance` aren't used for the decision: in
/// practice BlueZ reports dual-mode / BR-EDR devices as public address too,
/// and appearance is sometimes unavailable on the initial DeviceAdded signal.
/// Falling back to BLE when we have no positive Classic signal matches the
/// task requirements and is corrected on the next property poll if BlueZ
/// later reports CoD or Classic UUIDs.
pub fn classify_device(
    _address_type: Option<&str>,
    class_of_device: Option<u32>,
    uuids: &[String],
    _has_appearance: bool,
) -> DeviceKind {
    // Strongest signal: only BR/EDR devices have a Class of Device.
    if class_of_device.is_some() {
        return DeviceKind::Classic;
    }

    // Any Classic service UUID means Classic.
    if has_classic_uuid(uuids) {
        return DeviceKind::Classic;
    }

    // Fallback: BLE.
    DeviceKind::Ble
}

/// Classic Bluetooth service UUIDs that imply BR/EDR (non-exhaustive).
/// Presence of any of these means the device is definitely Classic.
pub fn has_classic_uuid(uuids: &[String]) -> bool {
    const CLASSIC_UUIDS: &[&str] = &[
        A2DP_SINK_UUID,
        A2DP_SOURCE_UUID,
        // AVRCP target/controller
        "0000110c-0000-1000-8000-00805f9b34fb",
        "0000110e-0000-1000-8000-00805f9b34fb",
        // HFP / HSP (hands-free, headset)
        "0000111e-0000-1000-8000-00805f9b34fb",
        "0000111f-0000-1000-8000-00805f9b34fb",
        "00001108-0000-1000-8000-00805f9b34fb",
        "00001131-0000-1000-8000-00805f9b34fb",
        // Audio Source / Sink generic
        "0000110d-0000-1000-8000-00805f9b34fb",
        // Serial Port Profile
        "00001101-0000-1000-8000-00805f9b34fb",
    ];
    uuids.iter().any(|u| {
        let lower = u.to_lowercase();
        CLASSIC_UUIDS.iter().any(|c| *c == lower)
    })
}

/// Check whether the device UUIDs advertise A2DP source support.
pub fn has_a2dp_source_uuid(uuids: &[String]) -> bool {
    uuids.iter().any(|u| u.to_lowercase() == A2DP_SOURCE_UUID)
}

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
                | (Connected, AudioActive)
                | (Connected, Disconnected)
                | (ProfileNegotiated, PipewireSourceReady)
                | (ProfileNegotiated, AudioActive)
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
    /// Best-effort classification: BLE vs Classic Bluetooth.
    /// Defaults to `Classic` so audio-capable devices stay visible until we
    /// have a positive signal that this is a BLE-only device.
    #[serde(rename = "type")]
    pub device_type: DeviceKind,
    /// Best-effort flag: device advertises the A2DP source UUID.
    pub is_a2dp_source: bool,
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
            device_type: DeviceKind::default(),
            is_a2dp_source: false,
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
        // Default classification is BLE — Classic requires a positive signal.
        assert_eq!(dev.device_type, DeviceKind::Ble);
        assert!(!dev.is_a2dp_source);
    }

    #[test]
    fn test_classify_with_class_of_device() {
        // CoD present → Classic regardless of other signals.
        let kind = classify_device(Some("random"), Some(0x240404), &[], true);
        assert_eq!(kind, DeviceKind::Classic);
    }

    #[test]
    fn test_classify_with_classic_uuid() {
        let uuids = vec![A2DP_SINK_UUID.to_string()];
        let kind = classify_device(Some("public"), None, &uuids, false);
        assert_eq!(kind, DeviceKind::Classic);
    }

    #[test]
    fn test_classify_no_signals_is_ble() {
        // Per task spec: "lacks BR/EDR service UUIDs" → BLE.
        let kind = classify_device(None, None, &[], false);
        assert_eq!(kind, DeviceKind::Ble);
    }

    #[test]
    fn test_classify_random_address_is_ble() {
        let kind = classify_device(Some("random"), None, &[], false);
        assert_eq!(kind, DeviceKind::Ble);
    }

    #[test]
    fn test_classify_ble_only_uuid_is_ble() {
        // Heart Rate Service (BLE) shouldn't match any Classic UUID.
        let uuids = vec!["0000180d-0000-1000-8000-00805f9b34fb".to_string()];
        let kind = classify_device(Some("public"), None, &uuids, true);
        assert_eq!(kind, DeviceKind::Ble);
    }

    #[test]
    fn test_has_a2dp_source_uuid() {
        assert!(has_a2dp_source_uuid(&[A2DP_SOURCE_UUID.to_string()]));
        assert!(has_a2dp_source_uuid(&[A2DP_SOURCE_UUID.to_uppercase()]));
        assert!(!has_a2dp_source_uuid(&[A2DP_SINK_UUID.to_string()]));
        assert!(!has_a2dp_source_uuid(&[]));
    }

    #[test]
    fn test_has_classic_uuid() {
        assert!(has_classic_uuid(&[A2DP_SINK_UUID.to_string()]));
        assert!(has_classic_uuid(&[A2DP_SOURCE_UUID.to_string()]));
        // HFP
        assert!(has_classic_uuid(&[
            "0000111e-0000-1000-8000-00805f9b34fb".to_string()
        ]));
        // Random BLE-only UUID should not match.
        assert!(!has_classic_uuid(&[
            "0000180d-0000-1000-8000-00805f9b34fb".to_string()
        ]));
    }

    #[test]
    fn test_device_kind_serialization() {
        let json = serde_json::to_string(&DeviceKind::Ble).unwrap();
        assert_eq!(json, "\"ble\"");
        let json = serde_json::to_string(&DeviceKind::Classic).unwrap();
        assert_eq!(json, "\"classic\"");
    }

    #[test]
    fn test_device_info_serializes_type_field() {
        // The frontend consumes `type` (renamed from device_type) and
        // `is_a2dp_source`. Guard against accidental renames that would
        // silently break the UI colouring.
        let dev = DeviceInfo::new("AA:BB:CC:DD:EE:FF".into(), "Test".into());
        let json = serde_json::to_value(&dev).unwrap();
        let obj = json.as_object().expect("DeviceInfo serializes as object");
        assert!(
            obj.contains_key("type"),
            "DeviceInfo JSON is missing `type`: {}",
            json
        );
        assert!(
            obj.contains_key("is_a2dp_source"),
            "DeviceInfo JSON is missing `is_a2dp_source`: {}",
            json
        );
        assert_eq!(obj["type"], "ble");
        assert_eq!(obj["is_a2dp_source"], false);
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
