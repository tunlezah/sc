use serde::{Deserialize, Serialize};

pub const NUM_BANDS: usize = 10;
#[allow(dead_code)]
pub const SAMPLE_RATE: f64 = 48000.0;
#[allow(dead_code)]
pub const MIN_GAIN_DB: f32 = -12.0;
#[allow(dead_code)]
pub const MAX_GAIN_DB: f32 = 12.0;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FilterType {
    LowShelf,
    Peaking,
    HighShelf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqBand {
    pub freq: f64,
    pub gain_db: f32,
    pub q: f64,
    pub filter_type: FilterType,
}

impl EqBand {
    /// Clamp gain to [-12.0, 12.0] dB range.
    #[allow(dead_code)]
    pub fn clamp_gain(&mut self) {
        self.gain_db = self.gain_db.clamp(MIN_GAIN_DB, MAX_GAIN_DB);
    }
}

/// Default 10-band EQ configuration.
pub fn default_bands() -> [EqBand; NUM_BANDS] {
    [
        EqBand {
            freq: 60.0,
            gain_db: 0.0,
            q: 0.707,
            filter_type: FilterType::LowShelf,
        },
        EqBand {
            freq: 120.0,
            gain_db: 0.0,
            q: 1.414,
            filter_type: FilterType::Peaking,
        },
        EqBand {
            freq: 250.0,
            gain_db: 0.0,
            q: 1.414,
            filter_type: FilterType::Peaking,
        },
        EqBand {
            freq: 500.0,
            gain_db: 0.0,
            q: 1.414,
            filter_type: FilterType::Peaking,
        },
        EqBand {
            freq: 1000.0,
            gain_db: 0.0,
            q: 1.414,
            filter_type: FilterType::Peaking,
        },
        EqBand {
            freq: 2000.0,
            gain_db: 0.0,
            q: 1.414,
            filter_type: FilterType::Peaking,
        },
        EqBand {
            freq: 4000.0,
            gain_db: 0.0,
            q: 1.414,
            filter_type: FilterType::Peaking,
        },
        EqBand {
            freq: 8000.0,
            gain_db: 0.0,
            q: 1.820,
            filter_type: FilterType::Peaking,
        },
        EqBand {
            freq: 12000.0,
            gain_db: 0.0,
            q: 2.870,
            filter_type: FilterType::Peaking,
        },
        EqBand {
            freq: 16000.0,
            gain_db: 0.0,
            q: 0.707,
            filter_type: FilterType::HighShelf,
        },
    ]
}

/// Generate a PipeWire filter-chain label for a given filter type.
pub fn filter_label(filter_type: FilterType) -> &'static str {
    match filter_type {
        FilterType::LowShelf => "bq_lowshelf",
        FilterType::Peaking => "bq_peaking",
        FilterType::HighShelf => "bq_highshelf",
    }
}

/// Generate PipeWire filter-chain configuration string from EQ bands.
pub fn generate_filter_chain_config(bands: &[EqBand]) -> String {
    let mut nodes = String::new();
    let mut links = String::new();

    for (i, band) in bands.iter().enumerate() {
        let label = filter_label(band.filter_type);
        nodes.push_str(&format!(
            "                    {{ type = builtin  label = {}  name = eq_band_{}\n\
             \x20                     control = {{ \"Freq\" = {}  \"Q\" = {:.3}  \"Gain\" = {:.1} }} }}\n",
            label, i, band.freq, band.q, band.gain_db
        ));

        if i > 0 {
            links.push_str(&format!(
                "                    {{ output = \"eq_band_{}:Out\"  input = \"eq_band_{}:In\" }}\n",
                i - 1, i
            ));
        }
    }

    format!(
        r#"# PipeWire filter-chain for 10-band parametric EQ (auto-generated)
context.modules = [
    {{ name = libpipewire-module-filter-chain
        args = {{
            node.name = "soundsync-eq"
            node.description = "SoundSync Equalizer"
            capture.props = {{
                node.name = "effect_input.soundsync-eq"
                media.class = "Audio/Sink"
                audio.rate = 48000
                audio.channels = 2
                audio.position = "FL,FR"
            }}
            playback.props = {{
                node.name = "effect_output.soundsync-eq"
                node.target = "soundsync-capture"
            }}
            filter.graph = {{
                nodes = [
{nodes}                ]
                links = [
{links}                ]
            }}
        }}
    }}
]
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_bands_count() {
        assert_eq!(default_bands().len(), NUM_BANDS);
    }

    #[test]
    fn test_default_bands_zero_gain() {
        for band in &default_bands() {
            assert_eq!(band.gain_db, 0.0);
        }
    }

    #[test]
    fn test_default_bands_frequencies() {
        let bands = default_bands();
        assert_eq!(bands[0].freq, 60.0);
        assert_eq!(bands[4].freq, 1000.0);
        assert_eq!(bands[9].freq, 16000.0);
    }

    #[test]
    fn test_default_bands_filter_types() {
        let bands = default_bands();
        assert_eq!(bands[0].filter_type, FilterType::LowShelf);
        for band in &bands[1..9] {
            assert_eq!(band.filter_type, FilterType::Peaking);
        }
        assert_eq!(bands[9].filter_type, FilterType::HighShelf);
    }

    #[test]
    fn test_gain_clamping() {
        let mut band = EqBand {
            freq: 1000.0,
            gain_db: 20.0,
            q: 1.0,
            filter_type: FilterType::Peaking,
        };
        band.clamp_gain();
        assert_eq!(band.gain_db, 12.0);

        band.gain_db = -20.0;
        band.clamp_gain();
        assert_eq!(band.gain_db, -12.0);

        band.gain_db = 5.0;
        band.clamp_gain();
        assert_eq!(band.gain_db, 5.0);
    }

    #[test]
    fn test_filter_label() {
        assert_eq!(filter_label(FilterType::LowShelf), "bq_lowshelf");
        assert_eq!(filter_label(FilterType::Peaking), "bq_peaking");
        assert_eq!(filter_label(FilterType::HighShelf), "bq_highshelf");
    }

    #[test]
    fn test_generate_filter_chain_config() {
        let bands = default_bands();
        let config = generate_filter_chain_config(&bands);

        assert!(config.contains("soundsync-eq"));
        assert!(config.contains("effect_input.soundsync-eq"));
        assert!(config.contains("effect_output.soundsync-eq"));
        assert!(config.contains("soundsync-capture"));
        assert!(config.contains("bq_lowshelf"));
        assert!(config.contains("bq_highshelf"));
        assert!(config.contains("eq_band_0"));
        assert!(config.contains("eq_band_9"));
        // Should have 9 links (bands 0→1, 1→2, ... 8→9)
        assert_eq!(config.matches("eq_band_0:Out").count(), 1);
        assert_eq!(config.matches("eq_band_9:In").count(), 1);
    }

    #[test]
    fn test_generate_config_with_gains() {
        let mut bands = default_bands().to_vec();
        bands[0].gain_db = 6.0;
        bands[9].gain_db = -3.0;
        let config = generate_filter_chain_config(&bands);
        assert!(config.contains("\"Gain\" = 6.0"));
        assert!(config.contains("\"Gain\" = -3.0"));
    }

    #[test]
    fn test_eq_band_serialization() {
        let band = EqBand {
            freq: 1000.0,
            gain_db: 3.5,
            q: 1.414,
            filter_type: FilterType::Peaking,
        };
        let json = serde_json::to_string(&band).unwrap();
        assert!(json.contains("\"freq\":1000.0"));
        assert!(json.contains("\"peaking\""));
        let back: EqBand = serde_json::from_str(&json).unwrap();
        assert_eq!(back.freq, 1000.0);
        assert_eq!(back.gain_db, 3.5);
    }
}
