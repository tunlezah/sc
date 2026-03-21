#[allow(dead_code)]
/// aptX capability blob (vendor-specific, Qualcomm)
/// Vendor ID: 0x004F (Qualcomm) - little endian
/// Codec ID: 0x0001 - little endian
/// Sampling frequencies: 44100 | 48000
/// Channel mode: Stereo
pub fn capabilities() -> Vec<u8> {
    vec![
        0x4F, 0x00, // Vendor ID: Qualcomm (little-endian)
        0x01, 0x00, // Codec ID: aptX (little-endian)
        0x30, // Sampling freqs: 44100 (0x20) | 48000 (0x10)
        0x02, // Channel mode: Stereo
    ]
}

#[allow(dead_code)]
pub fn select_configuration(remote: &[u8]) -> Vec<u8> {
    if remote.len() < 6 {
        return capabilities();
    }
    let freq = if remote[4] & 0x20 != 0 {
        0x20 // 44100
    } else {
        remote[4] & 0x30
    };

    vec![
        remote[0], remote[1], remote[2], remote[3], freq, 0x02, // Stereo
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capabilities_length() {
        assert_eq!(capabilities().len(), 6);
    }

    #[test]
    fn test_vendor_id_qualcomm() {
        let caps = capabilities();
        assert_eq!(caps[0], 0x4F);
        assert_eq!(caps[1], 0x00);
    }
}
