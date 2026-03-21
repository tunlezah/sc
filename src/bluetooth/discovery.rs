use tracing::{debug, info, warn};

use crate::bluetooth::constants::{A2DP_SINK_UUID, A2DP_SOURCE_UUID, BLUEZ_NODE_PREFIXES};
use crate::bluetooth::device::{DeviceInfo, DeviceState};
use crate::state::{AppStateHandle, SystemEvent};

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

/// Process a discovered device, updating state and emitting events.
pub async fn handle_device_discovered(
    state: &AppStateHandle,
    address: String,
    name: String,
    rssi: Option<i16>,
    uuids: Vec<String>,
) {
    let has_a2dp = has_a2dp_uuid(&uuids);

    let mut app = state.state.write().await;
    let device = app
        .devices
        .entry(address.clone())
        .or_insert_with(|| DeviceInfo::new(address.clone(), name.clone()));

    device.name = name.clone();
    device.rssi = rssi;
    device.has_a2dp = has_a2dp;
    device.last_seen = chrono::Utc::now();

    drop(app);

    state.publish(SystemEvent::DeviceDiscovered {
        address,
        name,
        rssi,
    });
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
        )
        .await;

        let app = state.state.read().await;
        let dev = app.devices.get("AA:BB:CC:DD:EE:FF").unwrap();
        assert!(dev.has_a2dp);
        assert_eq!(dev.rssi, Some(-50));
        drop(app);

        let event = rx.try_recv().unwrap();
        assert!(matches!(event, SystemEvent::DeviceDiscovered { .. }));
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
        )
        .await;

        let mut rx = state.subscribe();
        update_device_state(&state, "AA:BB:CC:DD:EE:FF", DeviceState::Connected).await;

        let event = rx.try_recv().unwrap();
        assert!(matches!(event, SystemEvent::DeviceStateChanged { .. }));
    }
}
