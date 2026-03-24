use std::time::Duration;

pub const A2DP_SINK_UUID: &str = "0000110b-0000-1000-8000-00805f9b34fb";
pub const A2DP_SOURCE_UUID: &str = "0000110a-0000-1000-8000-00805f9b34fb";
#[allow(dead_code)]
pub const AVRCP_TARGET: &str = "0000110c-0000-1000-8000-00805f9b34fb";
#[allow(dead_code)]
pub const AVRCP_CONTROLLER: &str = "0000110e-0000-1000-8000-00805f9b34fb";

pub const AGENT_PATH: &str = "/org/soundsync/agent";

#[allow(dead_code)]
pub const BLUEZ_NODE_PREFIXES: &[&str] = &["bluez_input.", "bluez_source.", "api.bluez5."];

pub const DEVICE_PROPS_POLL: Duration = Duration::from_millis(500);
pub const AVRCP_POLL_ACTIVE: Duration = Duration::from_millis(250);
pub const AVRCP_POLL_IDLE: Duration = Duration::from_millis(2000);

/// Convert a D-Bus device path to a MAC address.
/// e.g. `/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF` → `AA:BB:CC:DD:EE:FF`
#[allow(dead_code)]
pub fn address_from_path(path: &str) -> Option<String> {
    path.split('/')
        .find(|seg| seg.starts_with("dev_"))
        .and_then(|seg| seg.strip_prefix("dev_"))
        .map(|s| s.replace('_', ":"))
}

/// Convert a MAC address to a D-Bus device path.
/// e.g. `("AA:BB:CC:DD:EE:FF")` → `/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF`
#[allow(dead_code)]
pub fn path_from_address(adapter_path: &str, address: &str) -> String {
    format!("{}/dev_{}", adapter_path, address.replace(':', "_"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_address_from_path() {
        assert_eq!(
            address_from_path("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF"),
            Some("AA:BB:CC:DD:EE:FF".to_string())
        );
        assert_eq!(
            address_from_path("/org/bluez/hci0/dev_11_22_33_44_55_66"),
            Some("11:22:33:44:55:66".to_string())
        );
    }

    #[test]
    fn test_address_from_transport_path() {
        // Transport paths have extra segments after the device address
        assert_eq!(
            address_from_path("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF/sep1/fd0"),
            Some("AA:BB:CC:DD:EE:FF".to_string())
        );
        assert_eq!(
            address_from_path("/org/bluez/hci0/dev_11_22_33_44_55_66/sep2/fd1"),
            Some("11:22:33:44:55:66".to_string())
        );
    }

    #[test]
    fn test_address_from_path_invalid() {
        assert_eq!(address_from_path("/org/bluez/hci0"), None);
        assert_eq!(address_from_path("/org/bluez/hci0/player0"), None);
    }

    #[test]
    fn test_path_from_address() {
        assert_eq!(
            path_from_address("/org/bluez/hci0", "AA:BB:CC:DD:EE:FF"),
            "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF"
        );
    }

    #[test]
    fn test_roundtrip() {
        let addr = "12:34:56:78:9A:BC";
        let path = path_from_address("/org/bluez/hci0", addr);
        let recovered = address_from_path(&path).unwrap();
        assert_eq!(recovered, addr);
    }
}
