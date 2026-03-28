use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::audio::capture::AudioCapture;
use crate::audio::filter_chain::FilterChainManager;
use crate::dsp::equalizer::EqBand;
use crate::state::{AppStateHandle, SystemEvent};

/// Commands for the audio pipeline (EQ updates).
#[derive(Debug)]
pub enum PipelineCommand {
    /// Update EQ bands and enabled state.
    UpdateEq { bands: Vec<EqBand>, enabled: bool },
}

const NULL_SINK_NAME: &str = "soundsync-capture";
const NULL_SINK_DESC: &str = "SoundSync-Capture";
/// The EQ filter-chain creates this sink. When EQ is enabled, this must be the
/// default sink so Bluetooth audio flows: BT → EQ → soundsync-capture → capture.
const EQ_SINK_NAME: &str = "effect_input.soundsync-eq";

/// Manages the entire audio pipeline: null sink, filter-chain, and capture.
pub struct AudioPipeline {
    filter_chain: FilterChainManager,
    null_sink_module_id: Option<u32>,
    /// pw-loopback child process (kept alive when using PipeWire-native null sink)
    loopback_child: Option<Child>,
    capture: AudioCapture,
    state: AppStateHandle,
}

impl AudioPipeline {
    pub fn new(state: AppStateHandle) -> Self {
        Self {
            filter_chain: FilterChainManager::new(),
            null_sink_module_id: None,
            loopback_child: None,
            capture: AudioCapture::new(),
            state,
        }
    }

    /// Initialize the audio pipeline: create null sink, start filter-chain, start capture.
    ///
    /// The filter-chain (EQ) is non-fatal: if it fails to start, audio capture
    /// still proceeds without EQ processing.
    ///
    /// Audio routing when EQ is enabled:
    ///   BT → effect_input.soundsync-eq (default sink)
    ///        → EQ filter-chain
    ///        → effect_output.soundsync-eq
    ///        → soundsync-capture (node.target)
    ///        → soundsync-capture.monitor → capture
    ///
    /// Audio routing when EQ is disabled/unavailable:
    ///   BT → soundsync-capture (default sink)
    ///        → soundsync-capture.monitor → capture
    pub async fn initialize(&mut self, bands: &[EqBand]) -> Result<(), String> {
        // Create null sink for monitoring/capture
        self.create_null_sink().await?;

        // Start filter-chain with initial EQ (non-fatal).
        // If successful, set the EQ input as default sink so BT audio flows
        // through the EQ before reaching the null sink.
        match self.filter_chain.apply_eq(bands).await {
            Ok(()) => {
                info!("Filter-chain (EQ) started");
                // Give filter-chain a moment to register its nodes
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                // Set EQ input as default sink so BT audio goes through EQ
                self.set_default_sink_to(EQ_SINK_NAME).await;
            }
            Err(e) => {
                warn!(
                    "Filter-chain (EQ) unavailable: {} — audio will bypass EQ",
                    e
                );
                // Fall back to null sink as default
                self.set_default_sink_to(NULL_SINK_NAME).await;
            }
        }

        // Start audio capture from the null sink (always captures EQ-processed output)
        self.capture.start(NULL_SINK_NAME).await?;

        let mut app = self.state.state.write().await;
        app.pipewire_ready = true;
        drop(app);

        info!("Audio pipeline initialized");
        Ok(())
    }

    /// Create the null sink used for capturing audio.
    /// Tries pactl first (PulseAudio compat), falls back to PipeWire-native pw-loopback.
    async fn create_null_sink(&mut self) -> Result<(), String> {
        if which_exists("pactl") {
            self.create_null_sink_pactl().await
        } else if which_exists("pw-loopback") {
            self.create_null_sink_pw_loopback().await
        } else {
            // No null sink tool available — capture will try to find BT source directly
            warn!(
                "Neither pactl nor pw-loopback found — \
                 null sink not created, will capture from Bluetooth source directly"
            );
            Ok(())
        }
    }

    /// Create null sink via pactl (PulseAudio compatibility layer).
    /// Retries up to 5 times with 1s delays since pipewire-pulse may not be ready.
    async fn create_null_sink_pactl(&mut self) -> Result<(), String> {
        const MAX_ATTEMPTS: u32 = 5;
        let mut last_err = String::new();

        for attempt in 1..=MAX_ATTEMPTS {
            let output = Command::new("pactl")
                .args([
                    "load-module",
                    "module-null-sink",
                    &format!("sink_name={}", NULL_SINK_NAME),
                    &format!("sink_properties=device.description={}", NULL_SINK_DESC),
                ])
                .output()
                .await
                .map_err(|e| format!("Failed to run pactl: {}", e))?;

            if output.status.success() {
                let id_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if let Ok(id) = id_str.parse::<u32>() {
                    self.null_sink_module_id = Some(id);
                    info!("Null sink created via pactl with module ID {}", id);
                }
                break;
            }

            last_err = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if attempt < MAX_ATTEMPTS {
                warn!(
                    "pactl load-module attempt {}/{} failed: {} — retrying in 1s",
                    attempt, MAX_ATTEMPTS, last_err
                );
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }

        // Verify the sink actually exists
        let verify = Command::new("pactl")
            .args(["list", "short", "sinks"])
            .output()
            .await
            .map_err(|e| format!("Failed to run pactl for verification: {}", e))?;

        if verify.status.success() {
            let stdout = String::from_utf8_lossy(&verify.stdout);
            if stdout.contains(NULL_SINK_NAME) {
                info!("Verified null sink {} exists", NULL_SINK_NAME);
                return Ok(());
            }
        }

        Err(format!(
            "Null sink {} not found after {} attempts. Last error: {}",
            NULL_SINK_NAME, MAX_ATTEMPTS, last_err
        ))
    }

    /// Create null sink via pw-loopback (PipeWire native).
    /// pw-loopback runs as a subprocess that creates a virtual sink+source pair.
    async fn create_null_sink_pw_loopback(&mut self) -> Result<(), String> {
        let child = Command::new("pw-loopback")
            .args([
                "--capture-props",
                &format!(
                    "media.class=Audio/Sink node.name={} node.description={}",
                    NULL_SINK_NAME, NULL_SINK_DESC
                ),
                "--playback-props",
                &format!(
                    "media.class=Audio/Source/Virtual node.name={}-source",
                    NULL_SINK_NAME
                ),
            ])
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to start pw-loopback: {}", e))?;

        info!("Null sink created via pw-loopback (PID: {:?})", child.id());
        self.loopback_child = Some(child);

        // Give pw-loopback a moment to register the nodes
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        Ok(())
    }

    /// Set the specified sink as the default so Bluetooth A2DP audio routes there.
    ///
    /// When EQ is enabled, this should be `effect_input.soundsync-eq` so audio
    /// passes through the EQ filter-chain before reaching `soundsync-capture`.
    /// When EQ is disabled, this should be `soundsync-capture` directly.
    async fn set_default_sink_to(&self, sink_name: &str) {
        if which_exists("pactl") {
            let result = Command::new("pactl")
                .args(["set-default-sink", sink_name])
                .output()
                .await;
            match result {
                Ok(out) if out.status.success() => {
                    info!("Default sink set to {} via pactl", sink_name);
                }
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    warn!("pactl set-default-sink {} failed: {}", sink_name, stderr);
                }
                Err(e) => warn!("Failed to run pactl: {}", e),
            }
        } else if which_exists("wpctl") {
            self.set_default_sink_wpctl(sink_name).await;
        } else {
            warn!("Neither pactl nor wpctl available — cannot set default sink");
        }
    }

    /// Set default sink via wpctl (WirePlumber).
    /// Finds the node ID by name, then sets it as default.
    async fn set_default_sink_wpctl(&self, sink_name: &str) {
        let status = Command::new("wpctl").args(["status"]).output().await;

        let node_id = match status {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                find_wpctl_node_id(&stdout, sink_name)
            }
            _ => None,
        };

        if let Some(id) = node_id {
            let result = Command::new("wpctl")
                .args(["set-default", &id.to_string()])
                .output()
                .await;
            match result {
                Ok(out) if out.status.success() => {
                    info!("Default sink set to {} (ID {}) via wpctl", sink_name, id);
                }
                _ => warn!("wpctl set-default failed for ID {}", id),
            }
        } else {
            warn!(
                "Could not find {} in wpctl status — default sink not set",
                sink_name
            );
        }
    }

    /// Update the EQ bands (kills and respawns the filter-chain) and re-route
    /// the default sink accordingly.
    pub async fn update_eq(&mut self, bands: &[EqBand], enabled: bool) -> Result<(), String> {
        if enabled {
            self.filter_chain.apply_eq(bands).await?;
            // Give filter-chain a moment to register its nodes
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            // Route BT audio through EQ
            self.set_default_sink_to(EQ_SINK_NAME).await;
        } else {
            // Bypass: stop filter-chain, route BT audio directly to null sink
            self.filter_chain.stop().await;
            self.set_default_sink_to(NULL_SINK_NAME).await;
        }

        let mut app = self.state.state.write().await;
        app.eq_bands = bands.to_vec();
        app.eq_enabled = enabled;
        drop(app);

        self.state.publish(SystemEvent::EqChanged {
            bands: bands.to_vec(),
            enabled,
        });

        Ok(())
    }

    /// Run the pipeline command loop. Processes EQ update commands from the
    /// web API. Must be spawned as a task. The pipeline is consumed and runs
    /// until the command channel is closed.
    pub async fn run(mut self, mut cmd_rx: mpsc::Receiver<PipelineCommand>) {
        info!("Audio pipeline command loop started");
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                PipelineCommand::UpdateEq { bands, enabled } => {
                    if let Err(e) = self.update_eq(&bands, enabled).await {
                        warn!("EQ update failed: {}", e);
                    }
                }
            }
        }
        info!("Audio pipeline command loop ended, shutting down");
        self.shutdown().await;
    }

    /// Get the audio capture broadcast receiver for spectrum analysis / WebRTC.
    pub fn audio_receiver(&self) -> tokio::sync::broadcast::Receiver<Vec<f32>> {
        self.capture.subscribe()
    }

    /// Get the audio capture broadcast sender (for WebRTC manager to subscribe).
    pub fn audio_sender(&self) -> tokio::sync::broadcast::Sender<Vec<f32>> {
        self.capture.sender()
    }

    /// Shut down the audio pipeline.
    pub async fn shutdown(&mut self) {
        self.capture.stop().await;
        self.filter_chain.stop().await;

        // Unload null sink (pactl method)
        if let Some(id) = self.null_sink_module_id.take() {
            let _ = Command::new("pactl")
                .args(["unload-module", &id.to_string()])
                .output()
                .await;
            info!("Null sink module {} unloaded", id);
        }

        // Kill pw-loopback subprocess
        if let Some(mut child) = self.loopback_child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
            info!("pw-loopback stopped");
        }
    }
}

/// Parse wpctl status output to find a node ID by name.
/// wpctl status output has lines like: " │  42. soundsync-capture [vol: 1.00]"
fn find_wpctl_node_id(status_output: &str, node_name: &str) -> Option<u32> {
    for line in status_output.lines() {
        if line.contains(node_name) {
            // Extract the node ID (number before the dot)
            let trimmed = line
                .trim()
                .trim_start_matches(|c: char| !c.is_ascii_digit());
            if let Some(dot_pos) = trimmed.find('.') {
                if let Ok(id) = trimmed[..dot_pos].trim().parse::<u32>() {
                    return Some(id);
                }
            }
        }
    }
    None
}

fn which_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_wpctl_node_id() {
        let output = r#"
Audio
 ├─ Devices:
 │      40. soundsync-capture        [Audio/Sink]
 │      42. Built-in Audio           [Audio/Sink]
 ├─ Sinks:
 │  *   42. Built-in Audio Analog Stereo [vol: 1.00]
 │      40. soundsync-capture            [vol: 1.00]
"#;
        assert_eq!(find_wpctl_node_id(output, "soundsync-capture"), Some(40));
    }

    #[test]
    fn test_find_wpctl_node_id_with_star() {
        let output = " │  *   40. soundsync-capture [vol: 0.50]\n";
        assert_eq!(find_wpctl_node_id(output, "soundsync-capture"), Some(40));
    }

    #[test]
    fn test_find_wpctl_node_id_not_found() {
        let output = " │      42. Built-in Audio [vol: 1.00]\n";
        assert_eq!(find_wpctl_node_id(output, "soundsync-capture"), None);
    }
}
