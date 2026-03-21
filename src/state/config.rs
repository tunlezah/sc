use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub port: u16,
    pub adapter: String,
    pub device_name: String,
    pub auto_pair: bool,
    pub max_devices: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: 8080,
            adapter: "hci0".to_string(),
            device_name: "SoundSync".to_string(),
            auto_pair: true,
            max_devices: 1,
        }
    }
}

/// Partial configuration for true field-level layered merging.
/// Each field is optional so that individual config files only override
/// the fields they explicitly set, leaving the rest untouched.
#[derive(Debug, Clone, Deserialize)]
struct PartialConfig {
    port: Option<u16>,
    adapter: Option<String>,
    device_name: Option<String>,
    auto_pair: Option<bool>,
    max_devices: Option<usize>,
}

impl PartialConfig {
    /// Merge this partial config into a full config, overriding only
    /// the fields that are explicitly set.
    fn merge_into(self, config: &mut Config) {
        if let Some(port) = self.port {
            config.port = port;
        }
        if let Some(adapter) = self.adapter {
            config.adapter = adapter;
        }
        if let Some(device_name) = self.device_name {
            config.device_name = device_name;
        }
        if let Some(auto_pair) = self.auto_pair {
            config.auto_pair = auto_pair;
        }
        if let Some(max_devices) = self.max_devices {
            config.max_devices = max_devices;
        }
    }
}

impl Config {
    /// Load configuration with layered precedence (field-level merging):
    /// defaults → /etc/soundsync/config.toml → ~/.config/soundsync/config.toml → ./config.toml
    ///
    /// Each file only overrides the fields it explicitly sets, so a user
    /// config with just `device_name = "MySink"` won't reset port or adapter.
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
                    Ok(contents) => match toml::from_str::<PartialConfig>(&contents) {
                        Ok(partial) => {
                            tracing::info!("Loaded config from {}", path.display());
                            partial.merge_into(&mut config);
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
    fn test_config_deserialize_full() {
        let toml_str = r#"
port = 9090
adapter = "hci1"
device_name = "MySync"
auto_pair = false
max_devices = 3
"#;
        let partial: PartialConfig = toml::from_str(toml_str).unwrap();
        let mut config = Config::default();
        partial.merge_into(&mut config);
        assert_eq!(config.port, 9090);
        assert_eq!(config.adapter, "hci1");
        assert_eq!(config.device_name, "MySync");
        assert!(!config.auto_pair);
        assert_eq!(config.max_devices, 3);
    }

    #[test]
    fn test_config_partial_merge_preserves_defaults() {
        let toml_str = r#"
port = 3000
"#;
        let partial: PartialConfig = toml::from_str(toml_str).unwrap();
        let mut config = Config::default();
        partial.merge_into(&mut config);
        assert_eq!(config.port, 3000);
        // Other fields should retain defaults
        assert_eq!(config.adapter, "hci0");
        assert_eq!(config.device_name, "SoundSync");
        assert!(config.auto_pair);
        assert_eq!(config.max_devices, 1);
    }

    #[test]
    fn test_layered_merge_across_files() {
        // Simulate system config setting port and adapter
        let system_toml = r#"
port = 9090
adapter = "hci1"
"#;
        let mut config = Config::default();
        let partial: PartialConfig = toml::from_str(system_toml).unwrap();
        partial.merge_into(&mut config);

        // Simulate user config overriding just device_name
        let user_toml = r#"
device_name = "Living Room Speaker"
"#;
        let partial: PartialConfig = toml::from_str(user_toml).unwrap();
        partial.merge_into(&mut config);

        assert_eq!(config.port, 9090);
        assert_eq!(config.adapter, "hci1");
        assert_eq!(config.device_name, "Living Room Speaker");
        assert!(config.auto_pair); // untouched default
        assert_eq!(config.max_devices, 1); // untouched default
    }
}
