use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{info, warn};

use crate::dsp::equalizer::{default_bands, EqBand, NUM_BANDS};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub bands: Vec<f32>, // 10 gain values
}

impl Preset {
    /// Apply this preset's gains to the default band configuration.
    pub fn apply(&self) -> Vec<EqBand> {
        let mut bands = default_bands().to_vec();
        for (i, gain) in self.bands.iter().enumerate().take(NUM_BANDS) {
            bands[i].gain_db = gain.clamp(-12.0, 12.0);
        }
        bands
    }
}

/// Return all built-in presets.
pub fn builtin_presets() -> HashMap<String, Preset> {
    let mut presets = HashMap::new();

    presets.insert(
        "Flat".to_string(),
        Preset {
            name: "Flat".to_string(),
            bands: vec![0.0; NUM_BANDS],
        },
    );

    presets.insert(
        "Bass Boost".to_string(),
        Preset {
            name: "Bass Boost".to_string(),
            bands: vec![6.0, 5.0, 3.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        },
    );

    presets.insert(
        "Vocal".to_string(),
        Preset {
            name: "Vocal".to_string(),
            bands: vec![-2.0, -1.0, 0.0, 3.0, 4.0, 4.0, 3.0, 1.0, 0.0, -1.0],
        },
    );

    presets.insert(
        "Classical".to_string(),
        Preset {
            name: "Classical".to_string(),
            bands: vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -2.0, -2.0, -2.0, -4.0],
        },
    );

    presets.insert(
        "Rock".to_string(),
        Preset {
            name: "Rock".to_string(),
            bands: vec![4.0, 3.0, 1.0, 0.0, -1.0, 0.0, 2.0, 3.0, 4.0, 4.0],
        },
    );

    presets.insert(
        "Electronic".to_string(),
        Preset {
            name: "Electronic".to_string(),
            bands: vec![5.0, 4.0, 2.0, 0.0, -1.0, 0.0, 1.0, 3.0, 4.0, 5.0],
        },
    );

    presets.insert(
        "Podcast".to_string(),
        Preset {
            name: "Podcast".to_string(),
            bands: vec![-3.0, -1.0, 2.0, 4.0, 5.0, 5.0, 4.0, 2.0, 0.0, -2.0],
        },
    );

    presets
}

/// Get the preset directory path.
pub fn preset_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("soundsync/presets")
}

/// Load custom presets from disk.
pub fn load_custom_presets() -> HashMap<String, Preset> {
    let dir = preset_dir();
    let mut presets = HashMap::new();

    if !dir.exists() {
        return presets;
    }

    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            warn!("Failed to read preset directory: {}", e);
            return presets;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml") {
            match std::fs::read_to_string(&path) {
                Ok(contents) => match toml::from_str::<Preset>(&contents) {
                    Ok(preset) => {
                        info!("Loaded custom preset: {}", preset.name);
                        presets.insert(preset.name.clone(), preset);
                    }
                    Err(e) => warn!("Failed to parse preset {}: {}", path.display(), e),
                },
                Err(e) => warn!("Failed to read preset {}: {}", path.display(), e),
            }
        }
    }

    presets
}

/// Save a preset to disk.
pub fn save_preset(name: &str, bands: &[EqBand]) -> Result<(), String> {
    let dir = preset_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create preset dir: {}", e))?;

    let preset = Preset {
        name: name.to_string(),
        bands: bands.iter().map(|b| b.gain_db).collect(),
    };

    let toml_str = toml::to_string_pretty(&preset)
        .map_err(|e| format!("Failed to serialize preset: {}", e))?;

    let path = dir.join(format!("{}.toml", sanitize_filename(name)));
    std::fs::write(&path, toml_str)
        .map_err(|e| format!("Failed to write preset {}: {}", path.display(), e))?;

    info!("Saved preset '{}' to {}", name, path.display());
    Ok(())
}

/// Delete a custom preset from disk.
pub fn delete_preset(name: &str) -> Result<(), String> {
    let path = preset_dir().join(format!("{}.toml", sanitize_filename(name)));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("Failed to delete preset: {}", e))?;
        info!("Deleted preset '{}'", name);
        Ok(())
    } else {
        Err(format!("Preset '{}' not found", name))
    }
}

/// Get all available presets (builtin + custom).
pub fn all_preset_names() -> Vec<String> {
    let mut names: Vec<String> = builtin_presets().keys().cloned().collect();
    let custom = load_custom_presets();
    names.extend(custom.keys().cloned());
    names.sort();
    names
}

/// Look up a preset by name (builtin first, then custom).
pub fn get_preset(name: &str) -> Option<Preset> {
    builtin_presets()
        .get(name)
        .cloned()
        .or_else(|| load_custom_presets().get(name).cloned())
}

/// Sanitize a filename by removing non-alphanumeric characters (except dash/underscore).
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_presets_count() {
        assert_eq!(builtin_presets().len(), 7);
    }

    #[test]
    fn test_builtin_preset_names() {
        let presets = builtin_presets();
        assert!(presets.contains_key("Flat"));
        assert!(presets.contains_key("Bass Boost"));
        assert!(presets.contains_key("Vocal"));
        assert!(presets.contains_key("Classical"));
        assert!(presets.contains_key("Rock"));
        assert!(presets.contains_key("Electronic"));
        assert!(presets.contains_key("Podcast"));
    }

    #[test]
    fn test_preset_band_counts() {
        for (name, preset) in &builtin_presets() {
            assert_eq!(
                preset.bands.len(),
                NUM_BANDS,
                "Preset {} has wrong band count",
                name
            );
        }
    }

    #[test]
    fn test_preset_gains_in_range() {
        for (name, preset) in &builtin_presets() {
            for (i, gain) in preset.bands.iter().enumerate() {
                assert!(
                    *gain >= -12.0 && *gain <= 12.0,
                    "Preset {} band {} gain {} out of range",
                    name,
                    i,
                    gain
                );
            }
        }
    }

    #[test]
    fn test_flat_preset_all_zero() {
        let flat = builtin_presets().get("Flat").unwrap().clone();
        assert!(flat.bands.iter().all(|g| *g == 0.0));
    }

    #[test]
    fn test_preset_apply() {
        let preset = Preset {
            name: "Test".to_string(),
            bands: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0],
        };
        let bands = preset.apply();
        assert_eq!(bands.len(), NUM_BANDS);
        assert_eq!(bands[0].gain_db, 1.0);
        assert_eq!(bands[9].gain_db, 10.0);
        // Frequencies should remain default
        assert_eq!(bands[0].freq, 60.0);
    }

    #[test]
    fn test_preset_apply_clamps_gain() {
        let preset = Preset {
            name: "Extreme".to_string(),
            bands: vec![20.0; NUM_BANDS],
        };
        let bands = preset.apply();
        for band in &bands {
            assert_eq!(band.gain_db, 12.0);
        }
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("My Preset"), "My_Preset");
        assert_eq!(sanitize_filename("rock-n-roll"), "rock-n-roll");
        assert_eq!(sanitize_filename("test/bad"), "test_bad");
    }

    #[test]
    fn test_preset_serialization() {
        let preset = Preset {
            name: "Test".to_string(),
            bands: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0],
        };
        let toml_str = toml::to_string_pretty(&preset).unwrap();
        let back: Preset = toml::from_str(&toml_str).unwrap();
        assert_eq!(back.name, "Test");
        assert_eq!(back.bands.len(), 10);
    }
}
