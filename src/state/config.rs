use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_adapter")]
    pub adapter: String,
    #[serde(default = "default_device_name")]
    pub device_name: String,
    #[serde(default = "default_true")]
    pub auto_pair: bool,
    #[serde(default = "default_max_devices")]
    pub max_devices: usize,
}

fn default_port() -> u16 {
    8080
}
fn default_adapter() -> String {
    "hci0".to_string()
}
fn default_device_name() -> String {
    "SoundSync".to_string()
}
fn default_true() -> bool {
    true
}
fn default_max_devices() -> usize {
    1
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: default_port(),
            adapter: default_adapter(),
            device_name: default_device_name(),
            auto_pair: default_true(),
            max_devices: default_max_devices(),
        }
    }
}

impl Config {
    /// Load configuration with layered precedence:
    /// /etc/soundsync/config.toml → ~/.config/soundsync/config.toml → ./config.toml
    pub fn load() -> Self {
        let mut config = Config::default();

        let paths = [
            PathBuf::from("/etc/soundsync/config.toml"),
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("~/.config"))
                .join("soundsync/config.toml"),
            PathBuf::from("config.toml"),
        ];

        for path in &paths {
            if path.exists() {
                match std::fs::read_to_string(path) {
                    Ok(contents) => match toml::from_str::<Config>(&contents) {
                        Ok(file_config) => {
                            tracing::info!("Loaded config from {}", path.display());
                            config = file_config;
                        }
                        Err(e) => {
                            tracing::warn!("Failed to parse config {}: {}", path.display(), e);
                        }
                    },
                    Err(e) => {
                        tracing::warn!("Failed to read config {}: {}", path.display(), e);
                    }
                }
            }
        }

        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.port, 8080);
        assert_eq!(config.adapter, "hci0");
        assert_eq!(config.device_name, "SoundSync");
        assert!(config.auto_pair);
        assert_eq!(config.max_devices, 1);
    }

    #[test]
    fn test_config_deserialize() {
        let toml_str = r#"
port = 9090
adapter = "hci1"
device_name = "MySync"
auto_pair = false
max_devices = 3
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.port, 9090);
        assert_eq!(config.adapter, "hci1");
        assert_eq!(config.device_name, "MySync");
        assert!(!config.auto_pair);
        assert_eq!(config.max_devices, 3);
    }

    #[test]
    fn test_config_partial_deserialize() {
        let toml_str = r#"
port = 3000
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.port, 3000);
        assert_eq!(config.adapter, "hci0");
        assert_eq!(config.device_name, "SoundSync");
    }
}
