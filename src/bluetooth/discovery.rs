use tracing::{debug, info, warn};

use crate::bluetooth::constants::{A2DP_SINK_UUID, A2DP_SOURCE_UUID, BLUEZ_NODE_PREFIXES};
use crate::bluetooth::device::{
    classify_device, has_a2dp_source_uuid, DeviceInfo, DeviceKind, DeviceState,
};
use crate::state::{AppStateHandle, SystemEvent};

/// Check if a string looks like a MAC address (colon or dash separated).
/// BlueZ returns `XX-XX-XX-XX-XX-XX` as the alias when no friendly name is known,
/// while our internal addresses use `XX:XX:XX:XX:XX:XX`.
pub fn is_mac_address(s: &str) -> bool {
    // Must be exactly 17 chars: 6 hex pairs separated by colons or dashes
    if s.len() != 17 {
        return false;
    }
    let sep = if s.contains(':') {
        ':'
    } else if s.contains('-') {
        '-'
    } else {
        return false;
    };
    s.split(sep).count() == 6
        && s.split(sep)
            .all(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Check if a set of UUIDs contains A2DP-related profiles.
pub fn has_a2dp_uuid(uuids: &[String]) -> bool {
    uuids.iter().any(|u| {
        let lower = u.to_lowercase();
        lower == A2DP_SINK_UUID || lower == A2DP_SOURCE_UUID
    })
}

/// Check if a PipeWire node name matches a known Bluetooth audio prefix.
#[allow(dead_code)]
pub fn is_bluetooth_audio_node(node_name: &str) -> bool {
    BLUEZ_NODE_PREFIXES
        .iter()
        .any(|prefix| node_name.starts_with(prefix))
}

/// Extra signals collected at discovery time used to classify the device
/// as BLE vs Classic Bluetooth. All fields are best-effort.
#[derive(Debug, Default, Clone)]
pub struct DiscoverySignals {
    /// BlueZ AddressType ("public" or "random").
    pub address_type: Option<String>,
    /// Class of Device — only present for BR/EDR devices.
    pub class_of_device: Option<u32>,
    /// GAP Appearance — typically only present for LE devices.
    pub has_appearance: bool,
}

/// Process a discovered device, updating state and emitting events.
pub async fn handle_device_discovered(
    state: &AppStateHandle,
    address: String,
    name: String,
    rssi: Option<i16>,
    uuids: Vec<String>,
    signals: DiscoverySignals,
) {
    let has_a2dp = has_a2dp_uuid(&uuids);
    let is_a2dp_source = has_a2dp_source_uuid(&uuids);
    let new_kind = classify_device(
        signals.address_type.as_deref(),
        signals.class_of_device,
        &uuids,
        signals.has_appearance,
    );

    let mut app = state.state.write().await;
    let device = app
        .devices
        .entry(address.clone())
        .or_insert_with(|| DeviceInfo::new(address.clone(), name.clone()));

    // Only update name if the new value is non-empty — an empty name means
    // BlueZ hasn't resolved the friendly name yet and we don't want to
    // overwrite a previously known name.
    if !name.is_empty() {
        device.name = name.clone();
    }
    device.rssi = rssi;
    device.has_a2dp = has_a2dp;
    // Keep is_a2dp_source sticky once observed — UUID lists can flip empty
    // between scans before BlueZ has finished resolving them.
    if is_a2dp_source {
        device.is_a2dp_source = true;
    }
    // Sticky Classic rule: default is BLE, so a fresh device is BLE and the
    // first positive Classic signal upgrades it; subsequent sparse scans
    // cannot downgrade it back to BLE.
    if device.device_type == DeviceKind::Ble {
        device.device_type = new_kind;
    }
    device.last_seen = chrono::Utc::now();

    let resolved_name = device.name.clone();

    drop(app);

    state.publish(SystemEvent::DeviceDiscovered {
        address,
        name: resolved_name,
        rssi,
    });
}

/// Re-classify an existing device based on a fresh property read.
/// Used by the property poller to upgrade BLE → Classic when BlueZ resolves
/// CoD or service UUIDs after the initial DeviceAdded signal. Never downgrades
/// Classic → BLE — that direction would lose audio-capable candidates if a
/// later poll returns sparse data.
pub async fn refresh_classification(
    state: &AppStateHandle,
    address: &str,
    uuids: &[String],
    signals: &DiscoverySignals,
) {
    let new_kind = classify_device(
        signals.address_type.as_deref(),
        signals.class_of_device,
        uuids,
        signals.has_appearance,
    );
    let new_a2dp_source = has_a2dp_source_uuid(uuids);

    let mut app = state.state.write().await;
    if let Some(device) = app.devices.get_mut(address) {
        // Once Classic, stay Classic. BLE → Classic upgrades are allowed.
        if device.device_type == DeviceKind::Ble && new_kind == DeviceKind::Classic {
            device.device_type = DeviceKind::Classic;
        }
        if new_a2dp_source {
            device.is_a2dp_source = true;
        }
    }
}

/// Update a device's display name and notify the frontend.
///
/// Refuses empty strings or MAC-shaped aliases — both indicate that BlueZ
/// hasn't resolved the friendly name yet and overwriting a previously
/// resolved name with one of those would regress the UI from "MyPhone"
/// back to a raw MAC address.
pub async fn update_device_name(state: &AppStateHandle, address: &str, name: String) {
    if name.is_empty() || is_mac_address(&name) {
        return;
    }
    let mut app = state.state.write().await;
    if let Some(device) = app.devices.get_mut(address) {
        if device.name != name {
            info!(
                "Device {} name resolved: '{}' → '{}'",
                address, device.name, name
            );
            device.name = name.clone();
            let device_state = device.state.clone();
            drop(app);

            state.publish(SystemEvent::DeviceStateChanged {
                address: address.to_string(),
                name,
                state: device_state,
            });
        }
    }
}

/// Update device state and emit appropriate events.
pub async fn update_device_state(state: &AppStateHandle, address: &str, new_state: DeviceState) {
    let mut app = state.state.write().await;
    if let Some(device) = app.devices.get_mut(address) {
        let name = device.name.clone();
        let old_state = device.state.clone();

        if old_state != new_state {
            if device.state.can_transition_to(&new_state) {
                device.state = new_state.clone();
                info!(
                    "Device {} ({}) state: {:?} → {:?}",
                    address, name, old_state, new_state
                );
            } else {
                // Force transition for robustness (BlueZ can skip states)
                warn!(
                    "Forced device {} state: {:?} → {:?} (invalid transition)",
                    address, old_state, new_state
                );
                device.state = new_state.clone();
            }

            if new_state == DeviceState::AudioActive {
                app.active_device = Some(address.to_string());
            } else if new_state == DeviceState::Disconnected
                && app.active_device.as_deref() == Some(address)
            {
                app.active_device = None;
            }

            drop(app);

            state.publish(SystemEvent::DeviceStateChanged {
                address: address.to_string(),
                name,
                state: new_state,
            });
        }
    } else {
        debug!("State update for unknown device {}", address);
    }
}

/// Remove a device from the state.
pub async fn remove_device(state: &AppStateHandle, address: &str) {
    let mut app = state.state.write().await;
    if app.devices.remove(address).is_some() {
        if app.active_device.as_deref() == Some(address) {
            app.active_device = None;
        }
        drop(app);
        state.publish(SystemEvent::DeviceRemoved {
            address: address.to_string(),
        });
        info!("Removed device {}", address);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_a2dp_uuid() {
        assert!(has_a2dp_uuid(&[A2DP_SINK_UUID.to_string()]));
        assert!(has_a2dp_uuid(&[A2DP_SOURCE_UUID.to_string()]));
        assert!(has_a2dp_uuid(&[A2DP_SINK_UUID.to_uppercase()]));
        assert!(!has_a2dp_uuid(&[
            "00001101-0000-1000-8000-00805f9b34fb".to_string()
        ]));
        assert!(!has_a2dp_uuid(&[]));
    }

    #[test]
    fn test_is_mac_address() {
        // Colon-separated (our internal format)
        assert!(is_mac_address("AA:BB:CC:DD:EE:FF"));
        assert!(is_mac_address("7C:24:37:AF:9E:DF"));
        // Dash-separated (BlueZ alias fallback format)
        assert!(is_mac_address("AA-BB-CC-DD-EE-FF"));
        assert!(is_mac_address("7C-24-37-AF-9E-DF"));
        // Not MAC addresses
        assert!(!is_mac_address("iPhone"));
        assert!(!is_mac_address("Living Room Speaker"));
        assert!(!is_mac_address(""));
        assert!(!is_mac_address("ZZ:ZZ:ZZ:ZZ:ZZ:ZZ"));
    }

    #[test]
    fn test_is_bluetooth_audio_node() {
        assert!(is_bluetooth_audio_node(
            "bluez_input.AA_BB_CC_DD_EE_FF.a2dp_sink"
        ));
        assert!(is_bluetooth_audio_node("bluez_source.something"));
        assert!(is_bluetooth_audio_node("api.bluez5.something"));
        assert!(!is_bluetooth_audio_node("alsa_input.something"));
        assert!(!is_bluetooth_audio_node(""));
    }

    #[tokio::test]
    async fn test_handle_device_discovered() {
        let state = AppStateHandle::new(crate::state::config::Config::default());
        let mut rx = state.subscribe();

        handle_device_discovered(
            &state,
            "AA:BB:CC:DD:EE:FF".into(),
            "TestPhone".into(),
            Some(-50),
            vec![A2DP_SINK_UUID.to_string()],
            DiscoverySignals::default(),
        )
        .await;

        let app = state.state.read().await;
        let dev = app.devices.get("AA:BB:CC:DD:EE:FF").unwrap();
        assert!(dev.has_a2dp);
        assert_eq!(dev.rssi, Some(-50));
        // A2DP sink UUID → Classic
        assert_eq!(dev.device_type, DeviceKind::Classic);
        drop(app);

        let event = rx.try_recv().unwrap();
        assert!(matches!(event, SystemEvent::DeviceDiscovered { .. }));
    }

    #[tokio::test]
    async fn test_handle_device_discovered_ble() {
        let state = AppStateHandle::new(crate::state::config::Config::default());

        handle_device_discovered(
            &state,
            "BB:BB:BB:BB:BB:BB".into(),
            "FitnessTracker".into(),
            Some(-70),
            vec![],
            DiscoverySignals {
                address_type: Some("random".into()),
                class_of_device: None,
                has_appearance: true,
            },
        )
        .await;

        let app = state.state.read().await;
        let dev = app.devices.get("BB:BB:BB:BB:BB:BB").unwrap();
        assert_eq!(dev.device_type, DeviceKind::Ble);
        assert!(!dev.is_a2dp_source);
    }

    #[tokio::test]
    async fn test_handle_device_discovered_a2dp_source_flag() {
        let state = AppStateHandle::new(crate::state::config::Config::default());

        handle_device_discovered(
            &state,
            "CC:CC:CC:CC:CC:CC".into(),
            "Phone".into(),
            None,
            vec![A2DP_SOURCE_UUID.to_string()],
            DiscoverySignals::default(),
        )
        .await;

        let app = state.state.read().await;
        let dev = app.devices.get("CC:CC:CC:CC:CC:CC").unwrap();
        assert!(dev.is_a2dp_source);
        assert_eq!(dev.device_type, DeviceKind::Classic);
    }

    #[tokio::test]
    async fn test_ble_upgrades_to_classic_on_rescan() {
        // Simulate the real-world case where the initial DeviceAdded signal
        // has no UUIDs resolved yet (BlueZ populates them asynchronously).
        let state = AppStateHandle::new(crate::state::config::Config::default());

        // First scan: empty — classified as BLE (default).
        handle_device_discovered(
            &state,
            "EE:EE:EE:EE:EE:EE".into(),
            "Phone".into(),
            None,
            vec![],
            DiscoverySignals::default(),
        )
        .await;
        {
            let app = state.state.read().await;
            assert_eq!(
                app.devices.get("EE:EE:EE:EE:EE:EE").unwrap().device_type,
                DeviceKind::Ble
            );
        }

        // Second scan: UUIDs now resolved — upgrade to Classic.
        handle_device_discovered(
            &state,
            "EE:EE:EE:EE:EE:EE".into(),
            "Phone".into(),
            None,
            vec![A2DP_SOURCE_UUID.to_string()],
            DiscoverySignals::default(),
        )
        .await;

        let app = state.state.read().await;
        let dev = app.devices.get("EE:EE:EE:EE:EE:EE").unwrap();
        assert_eq!(dev.device_type, DeviceKind::Classic);
        assert!(dev.is_a2dp_source);
    }

    #[tokio::test]
    async fn test_classification_does_not_downgrade_classic_to_ble() {
        let state = AppStateHandle::new(crate::state::config::Config::default());

        // First scan: Classic UUID seen.
        handle_device_discovered(
            &state,
            "DD:DD:DD:DD:DD:DD".into(),
            "Speaker".into(),
            None,
            vec![A2DP_SINK_UUID.to_string()],
            DiscoverySignals::default(),
        )
        .await;

        // Second scan: empty UUIDs and a "random" address — would normally classify as BLE.
        handle_device_discovered(
            &state,
            "DD:DD:DD:DD:DD:DD".into(),
            "Speaker".into(),
            None,
            vec![],
            DiscoverySignals {
                address_type: Some("random".into()),
                class_of_device: None,
                has_appearance: false,
            },
        )
        .await;

        let app = state.state.read().await;
        let dev = app.devices.get("DD:DD:DD:DD:DD:DD").unwrap();
        // Must remain Classic — don't lose A2DP candidates on a stale rescan.
        assert_eq!(dev.device_type, DeviceKind::Classic);
    }

    #[tokio::test]
    async fn test_update_device_state() {
        let state = AppStateHandle::new(crate::state::config::Config::default());

        // First add a device
        handle_device_discovered(
            &state,
            "AA:BB:CC:DD:EE:FF".into(),
            "Test".into(),
            None,
            vec![],
            DiscoverySignals::default(),
        )
        .await;

        let mut rx = state.subscribe();
        update_device_state(&state, "AA:BB:CC:DD:EE:FF", DeviceState::Connected).await;

        let event = rx.try_recv().unwrap();
        assert!(matches!(event, SystemEvent::DeviceStateChanged { .. }));
    }

    #[tokio::test]
    async fn test_empty_name_does_not_overwrite_existing() {
        let state = AppStateHandle::new(crate::state::config::Config::default());

        // First discovery with a real name
        handle_device_discovered(
            &state,
            "AA:BB:CC:DD:EE:FF".into(),
            "MyPhone".into(),
            Some(-40),
            vec![],
            DiscoverySignals::default(),
        )
        .await;

        // Second discovery with empty name (MAC alias detected)
        handle_device_discovered(
            &state,
            "AA:BB:CC:DD:EE:FF".into(),
            "".into(),
            Some(-50),
            vec![],
            DiscoverySignals::default(),
        )
        .await;

        let app = state.state.read().await;
        let dev = app.devices.get("AA:BB:CC:DD:EE:FF").unwrap();
        // Name must be preserved, not overwritten with empty
        assert_eq!(dev.name, "MyPhone");
        // RSSI should still update
        assert_eq!(dev.rssi, Some(-50));
    }

    #[tokio::test]
    async fn test_discovered_event_sends_resolved_name() {
        let state = AppStateHandle::new(crate::state::config::Config::default());

        // First: device known with a name
        handle_device_discovered(
            &state,
            "AA:BB:CC:DD:EE:FF".into(),
            "Speaker".into(),
            None,
            vec![],
            DiscoverySignals::default(),
        )
        .await;

        let mut rx = state.subscribe();

        // Re-discovered with empty name
        handle_device_discovered(
            &state,
            "AA:BB:CC:DD:EE:FF".into(),
            "".into(),
            None,
            vec![],
            DiscoverySignals::default(),
        )
        .await;

        let event = rx.try_recv().unwrap();
        // The event should carry the resolved name "Speaker", not ""
        match event {
            SystemEvent::DeviceDiscovered { name, .. } => {
                assert_eq!(name, "Speaker");
            }
            _ => panic!("Expected DeviceDiscovered event"),
        }
    }

    #[tokio::test]
    async fn test_update_device_name() {
        let state = AppStateHandle::new(crate::state::config::Config::default());

        // Add a device with empty name
        handle_device_discovered(
            &state,
            "AA:BB:CC:DD:EE:FF".into(),
            "".into(),
            None,
            vec![],
            DiscoverySignals::default(),
        )
        .await;

        let mut rx = state.subscribe();

        // Name resolves later
        update_device_name(&state, "AA:BB:CC:DD:EE:FF", "Living Room Speaker".into()).await;

        let app = state.state.read().await;
        let dev = app.devices.get("AA:BB:CC:DD:EE:FF").unwrap();
        assert_eq!(dev.name, "Living Room Speaker");
        drop(app);

        let event = rx.try_recv().unwrap();
        match event {
            SystemEvent::DeviceStateChanged { name, .. } => {
                assert_eq!(name, "Living Room Speaker");
            }
            _ => panic!("Expected DeviceStateChanged event"),
        }
    }

    #[tokio::test]
    async fn test_update_device_name_rejects_empty_and_mac() {
        // Guard against the fast-poll race where alias() can momentarily
        // return "" or a MAC-shaped fallback after the device already has
        // a real name. Overwriting with either would regress the UI.
        let state = AppStateHandle::new(crate::state::config::Config::default());

        handle_device_discovered(
            &state,
            "AA:BB:CC:DD:EE:FF".into(),
            "MyPhone".into(),
            None,
            vec![],
            DiscoverySignals::default(),
        )
        .await;

        update_device_name(&state, "AA:BB:CC:DD:EE:FF", "".into()).await;
        update_device_name(&state, "AA:BB:CC:DD:EE:FF", "AA-BB-CC-DD-EE-FF".into()).await;
        update_device_name(&state, "AA:BB:CC:DD:EE:FF", "AA:BB:CC:DD:EE:FF".into()).await;

        let app = state.state.read().await;
        assert_eq!(
            app.devices.get("AA:BB:CC:DD:EE:FF").unwrap().name,
            "MyPhone"
        );
    }
}
