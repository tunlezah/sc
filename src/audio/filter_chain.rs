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

        info!(
            "Filter-chain config written to {}",
            self.config_path.display()
        );

        // Spawn the filter-chain process.
        // Some distributions provide `pipewire-filter-chain` as a separate binary,
        // others require `pipewire -c <config>` (Debian/Ubuntu/Raspberry Pi OS).
        let child = Self::spawn_filter_chain(&self.config_path)?;

        info!(
            "Filter-chain process started (PID: {:?}), config: {}",
            child.id(),
            self.config_path.display()
        );
        self.child = Some(child);

        Ok(())
    }

    /// Spawn the filter-chain process using the best available method.
    ///
    /// Tries in order:
    /// 1. `pipewire-filter-chain --config <path>` — standalone binary (some distros)
    /// 2. `pipewire -c <path>` — embedded mode (Debian/Ubuntu/Raspberry Pi OS)
    fn spawn_filter_chain(config_path: &std::path::Path) -> Result<Child, String> {
        // Method 1: try pipewire-filter-chain
        if which_exists("pipewire-filter-chain") {
            return Command::new("pipewire-filter-chain")
                .arg("--config")
                .arg(config_path)
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| format!("Failed to spawn pipewire-filter-chain: {}", e));
        }

        // Method 2: try pipewire -c (Debian/Ubuntu style)
        if which_exists("pipewire") {
            return Command::new("pipewire")
                .arg("-c")
                .arg(config_path)
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| format!("Failed to spawn pipewire -c: {}", e));
        }

        Err("Neither pipewire-filter-chain nor pipewire found in PATH".to_string())
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
            // Spawn a task to reap the child process and prevent zombies.
            // Drop is synchronous so we cannot await here directly.
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
        }
        // Clean up config file
        let _ = std::fs::remove_file(&self.config_path);
    }
}

fn which_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
