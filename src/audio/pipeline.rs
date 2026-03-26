use tokio::process::Command;
use tracing::{info, warn};

use crate::audio::capture::AudioCapture;
use crate::audio::filter_chain::FilterChainManager;
use crate::dsp::equalizer::EqBand;
use crate::state::{AppStateHandle, SystemEvent};

const NULL_SINK_NAME: &str = "soundsync-capture";
const NULL_SINK_DESC: &str = "SoundSync-Capture";
const MONITOR_SOURCE: &str = "soundsync-capture.monitor";

/// Manages the entire audio pipeline: null sink, filter-chain, and capture.
pub struct AudioPipeline {
    filter_chain: FilterChainManager,
    null_sink_module_id: Option<u32>,
    capture: AudioCapture,
    state: AppStateHandle,
}

impl AudioPipeline {
    pub fn new(state: AppStateHandle) -> Self {
        Self {
            filter_chain: FilterChainManager::new(),
            null_sink_module_id: None,
            capture: AudioCapture::new(),
            state,
        }
    }

    /// Initialize the audio pipeline: create null sink, start filter-chain, start capture.
    ///
    /// The filter-chain (EQ) is non-fatal: if it fails to start, audio capture
    /// still proceeds without EQ processing. This prevents a missing
    /// `pipewire-filter-chain` binary from silently breaking all audio.
    pub async fn initialize(&mut self, bands: &[EqBand]) -> Result<(), String> {
        // Create null sink for monitoring/capture
        self.create_null_sink().await?;

        // Start filter-chain with initial EQ (non-fatal)
        match self.filter_chain.apply_eq(bands).await {
            Ok(()) => info!("Filter-chain (EQ) started"),
            Err(e) => warn!(
                "Filter-chain (EQ) unavailable: {} — audio will bypass EQ",
                e
            ),
        }

        // Start audio capture from monitor source
        self.capture.start(MONITOR_SOURCE).await?;

        let mut app = self.state.state.write().await;
        app.pipewire_ready = true;
        drop(app);

        info!("Audio pipeline initialized");
        Ok(())
    }

    /// Create the null sink used for monitoring and WebRTC capture.
    /// Also sets it as the default PipeWire/PulseAudio sink so that
    /// incoming Bluetooth A2DP audio is routed here automatically.
    async fn create_null_sink(&mut self) -> Result<(), String> {
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

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("pactl load-module failed: {}", stderr));
        }

        let id_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if let Ok(id) = id_str.parse::<u32>() {
            self.null_sink_module_id = Some(id);
            info!("Null sink created with module ID {}", id);
        } else {
            warn!("Could not parse module ID from pactl output: {}", id_str);
        }

        // Set the null sink as the default so Bluetooth A2DP audio is routed
        // here instead of to the system's built-in speakers/HDMI output.
        let set_default = Command::new("pactl")
            .args(["set-default-sink", NULL_SINK_NAME])
            .output()
            .await;

        match set_default {
            Ok(out) if out.status.success() => {
                info!("Default sink set to {}", NULL_SINK_NAME);
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                warn!(
                    "Failed to set default sink to {}: {}",
                    NULL_SINK_NAME, stderr
                );
            }
            Err(e) => {
                warn!("Failed to run pactl set-default-sink: {}", e);
            }
        }

        Ok(())
    }

    /// Update the EQ bands (kills and respawns the filter-chain).
    #[allow(dead_code)]
    pub async fn update_eq(&mut self, bands: &[EqBand], enabled: bool) -> Result<(), String> {
        if enabled {
            self.filter_chain.apply_eq(bands).await?;
        } else {
            // Bypass: stop filter-chain, audio goes directly to output
            self.filter_chain.stop().await;
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

        // Unload null sink
        if let Some(id) = self.null_sink_module_id.take() {
            let _ = Command::new("pactl")
                .args(["unload-module", &id.to_string()])
                .output()
                .await;
            info!("Null sink module {} unloaded", id);
        }
    }
}
