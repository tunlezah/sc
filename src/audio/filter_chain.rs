use std::path::PathBuf;
use tokio::process::{Child, Command};
use tracing::{info, warn};

use crate::dsp::equalizer::{generate_filter_chain_config, EqBand};

/// Manages the PipeWire filter-chain subprocess for EQ processing.
pub struct FilterChainManager {
    config_path: PathBuf,
    child: Option<Child>,
}

impl FilterChainManager {
    pub fn new() -> Self {
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
            .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() as u32 }));
        let config_dir = PathBuf::from(runtime_dir).join("soundsync");

        Self {
            config_path: config_dir.join("filter-chain.conf"),
            child: None,
        }
    }

    /// Start or restart the filter-chain with the given EQ bands.
    pub async fn apply_eq(&mut self, bands: &[EqBand]) -> Result<(), String> {
        // Kill existing process
        self.stop().await;

        // Ensure config directory exists
        if let Some(parent) = self.config_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config dir: {}", e))?;
        }

        // Write config atomically
        let config = generate_filter_chain_config(bands);
        let tmp_path = self.config_path.with_extension("tmp");
        std::fs::write(&tmp_path, &config)
            .map_err(|e| format!("Failed to write filter-chain config: {}", e))?;
        std::fs::rename(&tmp_path, &self.config_path)
            .map_err(|e| format!("Failed to rename config: {}", e))?;

        info!("Filter-chain config written to {}", self.config_path.display());

        // Spawn the filter-chain process
        let child = Command::new("pipewire-filter-chain")
            .arg("--config")
            .arg(&self.config_path)
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to spawn filter-chain: {}", e))?;

        info!("Filter-chain process started (PID: {:?})", child.id());
        self.child = Some(child);

        Ok(())
    }

    /// Stop the filter-chain process.
    pub async fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            info!("Stopping filter-chain process");
            if let Err(e) = child.kill().await {
                warn!("Failed to kill filter-chain: {}", e);
            }
            let _ = child.wait().await;
        }
    }

    /// Check if the filter-chain is running.
    #[allow(dead_code)]
    pub fn is_running(&mut self) -> bool {
        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(Some(_)) => {
                    self.child = None;
                    false
                }
                Ok(None) => true,
                Err(_) => false,
            }
        } else {
            false
        }
    }
}

impl Drop for FilterChainManager {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
        // Clean up config file
        let _ = std::fs::remove_file(&self.config_path);
    }
}
