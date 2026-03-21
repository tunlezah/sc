#[allow(dead_code)]
/// aptX HD capability blob (vendor-specific, Qualcomm)
/// Vendor ID: 0x00D7 (Qualcomm for HD) - little endian
/// Codec ID: 0x0024 - little endian
/// Sampling frequencies: 44100 | 48000
/// Channel mode: Stereo
pub fn capabilities() -> Vec<u8> {
    vec![
        0xD7, 0x00, // Vendor ID: Qualcomm HD (little-endian)
        0x24, 0x00, // Codec ID: aptX HD (little-endian)
        0x30, // Sampling freqs: 44100 (0x20) | 48000 (0x10)
        0x02, // Channel mode: Stereo
        0x00, 0x00, 0x00, 0x00, // Reserved/padding
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

    let mut config = vec![
        remote[0], remote[1], remote[2], remote[3], freq, 0x02, // Stereo
    ];
    // Pad to match remote length
    while config.len() < remote.len() {
        config.push(0x00);
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capabilities_length() {
        let caps = capabilities();
        assert!(caps.len() >= 6);
    }

    #[test]
    fn test_vendor_id() {
        let caps = capabilities();
        assert_eq!(caps[0], 0xD7);
        assert_eq!(caps[1], 0x00);
    }
}
