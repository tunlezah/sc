#[allow(dead_code)]
/// AAC capability blob per BlueZ a2dp-codecs.h
/// Object types: MPEG-2 AAC LC (0x80)
/// Sampling frequencies: 44100 (0x01), 48000 (0x02)
/// Channels: 1 (0x08), 2 (0x04)
/// VBR: supported (0x80)
/// Bitrate: 256000 (3 bytes, big-endian)
pub fn capabilities() -> Vec<u8> {
    vec![
        0x80,       // MPEG-2 AAC LC
        0x01 | 0x02, // 44100 + 48000 Hz
        0x04 | 0x08, // Mono + Stereo
        0x80 | 0x03, // VBR + bitrate high byte
        0xE8,       // bitrate mid
        0x00,       // bitrate low (256000 bps)
    ]
}

#[allow(dead_code)]
pub fn select_configuration(remote: &[u8]) -> Vec<u8> {
    if remote.len() < 6 {
        return capabilities();
    }
    // Select AAC LC, 44100 Hz, Stereo, VBR, 256kbps
    vec![
        remote[0] & 0x80,       // AAC LC
        remote[1] & 0x01,       // 44100
        remote[2] & 0x04,       // Stereo
        remote[3] & 0x83,
        remote[4],
        remote[5],
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
    fn test_aac_lc_object_type() {
        let caps = capabilities();
        assert_eq!(caps[0] & 0x80, 0x80);
    }
}
