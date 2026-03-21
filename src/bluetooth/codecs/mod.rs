pub mod aac;
pub mod aptx;
pub mod aptx_hd;
pub mod ldac;
pub mod sbc;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    Sbc,
    Aac,
    Ldac,
    AptX,
    AptXHd,
}

#[allow(dead_code)]
impl AudioCodec {
    pub fn endpoint_path(&self) -> &'static str {
        match self {
            Self::Sbc => "/org/soundsync/a2dp/sbc",
            Self::Aac => "/org/soundsync/a2dp/aac",
            Self::Ldac => "/org/soundsync/a2dp/ldac",
            Self::AptX => "/org/soundsync/a2dp/aptx",
            Self::AptXHd => "/org/soundsync/a2dp/aptx_hd",
        }
    }

    pub fn codec_id(&self) -> u8 {
        match self {
            Self::Sbc => 0x00,
            Self::Aac => 0x02,
            Self::Ldac | Self::AptX | Self::AptXHd => 0xFF, // vendor-specific
        }
    }

    pub fn capabilities(&self) -> Vec<u8> {
        match self {
            Self::Sbc => sbc::capabilities(),
            Self::Aac => aac::capabilities(),
            Self::Ldac => ldac::capabilities(),
            Self::AptX => aptx::capabilities(),
            Self::AptXHd => aptx_hd::capabilities(),
        }
    }

    pub fn max_bitrate_kbps(&self) -> u32 {
        match self {
            Self::Sbc => 345,
            Self::Aac => 256,
            Self::Ldac => 990,
            Self::AptX => 352,
            Self::AptXHd => 576,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Sbc => "SBC",
            Self::Aac => "AAC",
            Self::Ldac => "LDAC",
            Self::AptX => "aptX",
            Self::AptXHd => "aptX HD",
        }
    }

    pub fn all() -> &'static [AudioCodec] {
        &[
            AudioCodec::Sbc,
            AudioCodec::Aac,
            AudioCodec::Ldac,
            AudioCodec::AptX,
            AudioCodec::AptXHd,
        ]
    }

    /// Select the best configuration from remote capabilities.
    pub fn select_configuration(&self, remote_caps: &[u8]) -> Vec<u8> {
        match self {
            Self::Sbc => sbc::select_configuration(remote_caps),
            Self::Aac => aac::select_configuration(remote_caps),
            Self::Ldac => ldac::select_configuration(remote_caps),
            Self::AptX => aptx::select_configuration(remote_caps),
            Self::AptXHd => aptx_hd::select_configuration(remote_caps),
        }
    }
}

impl std::fmt::Display for AudioCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codec_ids() {
        assert_eq!(AudioCodec::Sbc.codec_id(), 0x00);
        assert_eq!(AudioCodec::Aac.codec_id(), 0x02);
        assert_eq!(AudioCodec::Ldac.codec_id(), 0xFF);
        assert_eq!(AudioCodec::AptX.codec_id(), 0xFF);
        assert_eq!(AudioCodec::AptXHd.codec_id(), 0xFF);
    }

    #[test]
    fn test_codec_bitrates() {
        assert_eq!(AudioCodec::Sbc.max_bitrate_kbps(), 345);
        assert_eq!(AudioCodec::Ldac.max_bitrate_kbps(), 990);
    }

    #[test]
    fn test_all_codecs() {
        assert_eq!(AudioCodec::all().len(), 5);
    }

    #[test]
    fn test_capabilities_not_empty() {
        for codec in AudioCodec::all() {
            assert!(
                !codec.capabilities().is_empty(),
                "{} capabilities empty",
                codec
            );
        }
    }

    #[test]
    fn test_codec_serialization() {
        let codec = AudioCodec::AptXHd;
        let json = serde_json::to_string(&codec).unwrap();
        assert_eq!(json, "\"apt_x_hd\"");
    }

    #[test]
    fn test_endpoint_paths_unique() {
        let paths: Vec<_> = AudioCodec::all()
            .iter()
            .map(|c| c.endpoint_path())
            .collect();
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(paths.len(), unique.len());
    }
}
