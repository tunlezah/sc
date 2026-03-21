#[allow(dead_code)]
/// LDAC capability blob (vendor-specific, Sony)
/// Vendor ID: 0x012D (Sony) - little endian
/// Codec ID: 0x00AA - little endian
/// Sampling frequencies: 44100 | 48000 | 88200 | 96000
/// Channel mode: Stereo | Dual | Mono
pub fn capabilities() -> Vec<u8> {
    vec![
        0x2D, 0x01, // Vendor ID: Sony (little-endian)
        0xAA, 0x00, // Codec ID: LDAC (little-endian)
        0x3C, // Sampling freqs: 44100 | 48000 | 88200 | 96000
        0x07, // Channel modes: Stereo | Dual | Mono
    ]
}

#[allow(dead_code)]
pub fn select_configuration(remote: &[u8]) -> Vec<u8> {
    if remote.len() < 6 {
        return capabilities();
    }
    // Select 96kHz stereo for maximum quality
    let freq = if remote[4] & 0x04 != 0 {
        0x04 // 96000
    } else if remote[4] & 0x08 != 0 {
        0x08 // 88200
    } else if remote[4] & 0x10 != 0 {
        0x10 // 48000
    } else {
        remote[4] & 0x3C
    };

    vec![
        remote[0], remote[1], // Vendor ID
        remote[2], remote[3], // Codec ID
        freq, 0x01, // Stereo
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
    fn test_vendor_id_sony() {
        let caps = capabilities();
        assert_eq!(caps[0], 0x2D);
        assert_eq!(caps[1], 0x01);
    }
}
