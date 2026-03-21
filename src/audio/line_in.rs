use tokio::process::Command;
use tracing::info;

use crate::state::{AppStateHandle, SystemEvent};

/// Manages line-in (analog audio) source detection and activation.
pub struct LineInManager {
    state: AppStateHandle,
}

impl LineInManager {
    pub fn new(state: AppStateHandle) -> Self {
        Self { state }
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
    pub async fn initialize(&self) {
        if let Some(source) = self.detect_sources().await {
            info!("Line-in source detected: {}", source);
            let mut app = self.state.state.write().await;
            app.line_in_source = Some(source);
        } else {
            info!("No line-in source detected");
        }
    }

    /// Activate line-in as the audio source.
    pub async fn activate(&self) -> Result<(), String> {
        // Re-detect source in case it changed
        let source = self
            .detect_sources()
            .await
            .ok_or_else(|| "No line-in source available".to_string())?;

        info!("Activating line-in source: {}", source);

        let mut app = self.state.state.write().await;
        app.line_in_active = true;
        app.line_in_source = Some(source);
        drop(app);

        self.state.publish(SystemEvent::LineInActivated);
        Ok(())
    }

    /// Deactivate line-in (revert to Bluetooth).
    pub async fn deactivate(&self) -> Result<(), String> {
        info!("Deactivating line-in");

        let mut app = self.state.state.write().await;
        app.line_in_active = false;
        drop(app);

        self.state.publish(SystemEvent::LineInDeactivated);
        Ok(())
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
}
