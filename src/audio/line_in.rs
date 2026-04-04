use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::state::{AppStateHandle, SystemEvent};

const NULL_SINK_NAME: &str = "soundsync-capture";

/// Manages line-in (analog audio) source detection and activation.
///
/// When activated, a PulseAudio loopback module routes the line-in source
/// into the `soundsync-capture` null sink so the audio flows through the
/// standard pipeline (EQ → capture → WebRTC / HTTP stream / spectrum).
pub struct LineInManager {
    state: AppStateHandle,
    /// Module ID of the loopback module (set when active).
    loopback_module_id: Arc<Mutex<Option<u32>>>,
}

impl LineInManager {
    pub fn new(state: AppStateHandle) -> Self {
        Self {
            state,
            loopback_module_id: Arc::new(Mutex::new(None)),
        }
    }

    /// Detect available line-in sources via pactl.
    pub async fn detect_sources(&self) -> Option<String> {
        let output = Command::new("pactl")
            .args(["list", "short", "sources"])
            .output()
            .await
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_line_in_source(&stdout)
    }

    /// Update state with detected line-in source.
    /// Also cleans up any orphaned loopback modules from previous runs.
    pub async fn initialize(&self) {
        // Clean up orphaned loopback modules targeting our sink from previous runs
        cleanup_orphaned_loopbacks().await;

        if let Some(source) = self.detect_sources().await {
            info!("Line-in source detected: {}", source);
            let mut app = self.state.state.write().await;
            app.line_in_source = Some(source);
        } else {
            info!("No line-in source detected");
        }
    }

    /// Activate line-in as the audio source.
    ///
    /// Loads a PulseAudio `module-loopback` that routes the line-in source
    /// into the capture null sink. This ensures line-in audio flows through
    /// the same pipeline as Bluetooth audio (EQ, spectrum, streams).
    pub async fn activate(&self) -> Result<(), String> {
        // Re-detect source in case it changed
        let source = self
            .detect_sources()
            .await
            .ok_or_else(|| "No line-in source available".to_string())?;

        info!("Activating line-in source: {}", source);

        // Deactivate first to clean up any existing loopback module
        self.deactivate_inner().await;

        // Load loopback module: routes line-in source → soundsync-capture sink
        let output = Command::new("pactl")
            .args([
                "load-module",
                "module-loopback",
                &format!("source={}", source),
                &format!("sink={}", NULL_SINK_NAME),
                "latency_msec=50",
                "source_dont_move=true",
                "sink_dont_move=true",
            ])
            .output()
            .await
            .map_err(|e| format!("Failed to run pactl: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Failed to load loopback module: {}", stderr));
        }

        let module_id_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let module_id = module_id_str
            .parse::<u32>()
            .map_err(|_| "Failed to parse module ID".to_string())?;

        info!(
            "Line-in loopback module loaded (ID: {}, source: {} → sink: {})",
            module_id, source, NULL_SINK_NAME
        );

        {
            let mut lock = self.loopback_module_id.lock().await;
            *lock = Some(module_id);
        }

        let mut app = self.state.state.write().await;
        app.line_in_active = true;
        app.line_in_source = Some(source);
        drop(app);

        self.state.publish(SystemEvent::LineInActivated);
        Ok(())
    }

    /// Deactivate line-in (revert to Bluetooth).
    ///
    /// Unloads the loopback module so line-in audio stops flowing into the
    /// capture pipeline.
    pub async fn deactivate(&self) -> Result<(), String> {
        info!("Deactivating line-in");

        self.deactivate_inner().await;

        let mut app = self.state.state.write().await;
        app.line_in_active = false;
        drop(app);

        self.state.publish(SystemEvent::LineInDeactivated);
        Ok(())
    }

    /// Unload the tracked loopback module if one is active.
    async fn deactivate_inner(&self) {
        let module_id = {
            let mut lock = self.loopback_module_id.lock().await;
            lock.take()
        };

        if let Some(id) = module_id {
            let output = Command::new("pactl")
                .args(["unload-module", &id.to_string()])
                .output()
                .await;

            match output {
                Ok(out) if out.status.success() => {
                    info!("Line-in loopback module {} unloaded", id);
                }
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    warn!("Failed to unload loopback module {}: {}", id, stderr);
                }
                Err(e) => {
                    warn!("Failed to run pactl to unload module: {}", e);
                }
            }
        }
    }

    /// Get current line-in status.
    pub async fn status(&self) -> LineInStatus {
        let app = self.state.state.read().await;
        LineInStatus {
            available: app.line_in_source.is_some(),
            active: app.line_in_active,
            source_name: app.line_in_source.clone(),
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct LineInStatus {
    pub available: bool,
    pub active: bool,
    pub source_name: Option<String>,
}

/// Clean up orphaned module-loopback instances that target soundsync-capture.
///
/// Queries `pactl list modules` and unloads any module-loopback whose arguments
/// contain our sink name. This prevents accumulation across restarts.
async fn cleanup_orphaned_loopbacks() {
    let output = match Command::new("pactl")
        .args(["list", "modules"])
        .output()
        .await
    {
        Ok(out) if out.status.success() => out,
        _ => return,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let module_ids = parse_loopback_module_ids(&stdout, NULL_SINK_NAME);

    for id in &module_ids {
        info!(
            "Cleaning up orphaned loopback module {} (targets {})",
            id, NULL_SINK_NAME
        );
        let result = Command::new("pactl")
            .args(["unload-module", &id.to_string()])
            .output()
            .await;
        match result {
            Ok(out) if out.status.success() => {
                info!("Unloaded orphaned loopback module {}", id);
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                warn!(
                    "Failed to unload orphaned loopback module {}: {}",
                    id, stderr
                );
            }
            Err(e) => {
                warn!("Failed to run pactl to unload module {}: {}", id, e);
            }
        }
    }

    if !module_ids.is_empty() {
        info!(
            "Cleaned up {} orphaned loopback module(s)",
            module_ids.len()
        );
    }
}

/// Parse `pactl list modules` output and return module IDs for module-loopback
/// instances whose arguments reference the given sink name.
///
/// The output format has blocks like:
/// ```text
/// Module #42
///     Name: module-loopback
///     Argument: source=alsa_input... sink=soundsync-capture ...
///     ...
/// ```
fn parse_loopback_module_ids(pactl_output: &str, sink_name: &str) -> Vec<u32> {
    let mut ids = Vec::new();
    let mut current_id: Option<u32> = None;
    let mut is_loopback = false;

    for line in pactl_output.lines() {
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("Module #") {
            // Save previous module if it matched
            current_id = rest.parse::<u32>().ok();
            is_loopback = false;
        } else if trimmed.starts_with("Name:") {
            is_loopback = trimmed.contains("module-loopback");
        } else if trimmed.starts_with("Argument:") && is_loopback && trimmed.contains(sink_name) {
            if let Some(id) = current_id {
                ids.push(id);
            }
        }
    }

    ids
}

/// Parse pactl output to find an alsa_input source (line-in).
fn parse_line_in_source(pactl_output: &str) -> Option<String> {
    for line in pactl_output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 && parts[1].starts_with("alsa_input.") {
            return Some(parts[1].to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_line_in_source_found() {
        let output = "1\talsa_input.pci-0000_00_1f.3.analog-stereo\tPipeWire\tfloat32le 2ch 48000Hz\tIDLE\n\
                       2\talsa_output.pci-0000_00_1f.3.analog-stereo.monitor\tPipeWire\tfloat32le 2ch 48000Hz\tRUNNING\n";
        let source = parse_line_in_source(output);
        assert_eq!(
            source,
            Some("alsa_input.pci-0000_00_1f.3.analog-stereo".to_string())
        );
    }

    #[test]
    fn test_parse_line_in_source_not_found() {
        let output = "1\tsoundsync-capture.monitor\tPipeWire\tfloat32le 2ch 48000Hz\tIDLE\n";
        assert!(parse_line_in_source(output).is_none());
    }

    #[test]
    fn test_parse_line_in_empty() {
        assert!(parse_line_in_source("").is_none());
    }

    #[test]
    fn test_parse_line_in_usb() {
        let output = "3\talsa_input.usb-Generic_USB_Audio-00.analog-stereo\tPipeWire\tfloat32le 2ch 48000Hz\tIDLE\n";
        let source = parse_line_in_source(output);
        assert!(source.is_some());
        assert!(source.unwrap().contains("usb"));
    }

    #[test]
    fn test_parse_loopback_module_ids_finds_matching() {
        let output = "\
Module #10
\tName: module-null-sink
\tArgument: sink_name=soundsync-capture
Module #42
\tName: module-loopback
\tArgument: source=alsa_input.pci sink=soundsync-capture latency_msec=20
Module #43
\tName: module-loopback
\tArgument: source=alsa_input.usb sink=some-other-sink latency_msec=20
Module #44
\tName: module-loopback
\tArgument: source=alsa_input.hdmi sink=soundsync-capture latency_msec=20
";
        let ids = parse_loopback_module_ids(output, "soundsync-capture");
        assert_eq!(ids, vec![42, 44]);
    }

    #[test]
    fn test_parse_loopback_module_ids_empty() {
        let output = "\
Module #10
\tName: module-null-sink
\tArgument: sink_name=soundsync-capture
";
        let ids = parse_loopback_module_ids(output, "soundsync-capture");
        assert!(ids.is_empty());
    }

    #[test]
    fn test_parse_loopback_module_ids_no_match() {
        let output = "\
Module #42
\tName: module-loopback
\tArgument: source=alsa_input.pci sink=other-sink latency_msec=20
";
        let ids = parse_loopback_module_ids(output, "soundsync-capture");
        assert!(ids.is_empty());
    }
}
