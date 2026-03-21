#[allow(dead_code)]
/// SBC capability blob per BlueZ a2dp-codecs.h
///
/// Byte 0: Sampling Frequency + Channel Mode
///     - 48000 Hz (0x10) | 44100 Hz (0x20) | Joint Stereo (0x01) | Stereo (0x02) | Dual (0x04) | Mono (0x08)
///
/// Byte 1: Block Length + Subbands + Allocation Method
///     - 16 blocks (0x10) | 12 blocks (0x20) | 8 blocks (0x40) | 4 blocks (0x80) | 8 subbands (0x04) | 4 subbands (0x08) | Loudness (0x01) | SNR (0x02)
///
/// Byte 2: Minimum Bitpool (2)
///
/// Byte 3: Maximum Bitpool (53 for high quality)
pub fn capabilities() -> Vec<u8> {
    vec![
        0x3F, // All sample rates (44.1k, 48k) + All channel modes
        0xFF, // All block lengths + all subbands + all allocation methods
        2,    // Minimum bitpool
        53,   // Maximum bitpool (HQ)
    ]
}

/// Select optimal SBC configuration from remote capabilities.
#[allow(dead_code)]
pub fn select_configuration(remote: &[u8]) -> Vec<u8> {
    if remote.len() < 4 {
        return capabilities();
    }

    let freq_channel = remote[0];
    let blocks_subbands = remote[1];
    let min_bitpool = remote[2];
    let max_bitpool = remote[3];

    // Prefer 44100 Hz Joint Stereo
    let selected_freq_channel = if freq_channel & 0x20 != 0 && freq_channel & 0x01 != 0 {
        0x21 // 44100 Hz + Joint Stereo
    } else if freq_channel & 0x10 != 0 && freq_channel & 0x01 != 0 {
        0x11 // 48000 Hz + Joint Stereo
    } else {
        freq_channel & 0x31 // Best available
    };

    // Prefer 16 blocks, 8 subbands, Loudness
    let selected_blocks = if blocks_subbands & 0x10 != 0 {
        0x10 // 16 blocks
    } else {
        blocks_subbands & 0xF0
    };

    let selected_subbands = if blocks_subbands & 0x04 != 0 {
        0x04 // 8 subbands
    } else {
        blocks_subbands & 0x0C
    };

    let selected_alloc = if blocks_subbands & 0x01 != 0 {
        0x01 // Loudness
    } else {
        blocks_subbands & 0x03
    };

    vec![
        selected_freq_channel,
        selected_blocks | selected_subbands | selected_alloc,
        min_bitpool,
        max_bitpool.min(53),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capabilities_length() {
        assert_eq!(capabilities().len(), 4);
    }

    #[test]
    fn test_bitpool_range() {
        let caps = capabilities();
        assert_eq!(caps[2], 2);
        assert_eq!(caps[3], 53);
    }

    #[test]
    fn test_select_configuration_prefers_joint_stereo() {
        let remote = vec![0x3F, 0xFF, 2, 53];
        let selected = select_configuration(&remote);
        assert_eq!(selected[0] & 0x01, 0x01); // Joint stereo
    }
}
