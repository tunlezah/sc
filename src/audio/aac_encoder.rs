use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tracing::{info, warn};

/// Encodes PCM f32 audio to AAC-LC in ADTS container using FFmpeg.
///
/// AAC-LC at 256 kbps in ADTS is the most universally compatible streaming
/// format across Safari, Chrome, Chromecast, and AirPlay devices.
pub struct AacEncoder {
    child: Child,
    stdin_tx: mpsc::Sender<Vec<f32>>,
    output_rx: mpsc::Receiver<Vec<u8>>,
}

impl AacEncoder {
    /// Create a new AAC-LC encoder at 256 kbps, 48 kHz stereo via FFmpeg.
    pub fn new() -> Result<Self, String> {
        // Verify ffmpeg exists
        let check = std::process::Command::new("which")
            .arg("ffmpeg")
            .output()
            .map_err(|e| format!("Failed to check for ffmpeg: {}", e))?;
        if !check.status.success() {
            return Err("ffmpeg not found in PATH".to_string());
        }

        let mut child = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel", "error",
                // Input: raw PCM f32le stereo 48kHz from stdin
                "-f", "f32le",
                "-ar", "48000",
                "-ac", "2",
                "-i", "pipe:0",
                // Output: AAC-LC 256kbps in ADTS container to stdout
                "-c:a", "aac",
                "-b:a", "256k",
                "-profile:a", "aac_low",
                "-cutoff", "20000",
                "-f", "adts",
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to start ffmpeg AAC encoder: {}", e))?;

        let stdin = child.stdin.take().ok_or("No stdin on ffmpeg process")?;
        let stdout = child.stdout.take().ok_or("No stdout on ffmpeg process")?;
        let stderr = child.stderr.take().ok_or("No stderr on ffmpeg process")?;

        // Channel for sending PCM data to the writer task
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<f32>>(32);
        // Channel for receiving encoded AAC data from the reader task
        let (output_tx, output_rx) = mpsc::channel::<Vec<u8>>(32);

        // Spawn task to write PCM samples to ffmpeg stdin
        tokio::spawn(async move {
            let mut writer = stdin;
            while let Some(pcm) = stdin_rx.recv().await {
                let bytes: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
                if writer.write_all(&bytes).await.is_err() {
                    break;
                }
            }
            // Closing stdin signals EOF to ffmpeg
            drop(writer);
        });

        // Spawn task to read AAC output from ffmpeg stdout
        tokio::spawn(async move {
            let mut reader = stdout;
            let mut buf = vec![0u8; 8192];
            loop {
                match reader.read(&mut buf).await {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        if output_tx.send(buf[..n].to_vec()).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        warn!("AAC encoder read error: {}", e);
                        break;
                    }
                }
            }
        });

        // Spawn task to log any ffmpeg errors
        tokio::spawn(async move {
            let mut reader = stderr;
            let mut buf = vec![0u8; 1024];
            loop {
                match reader.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        let msg = String::from_utf8_lossy(&buf[..n]);
                        if !msg.trim().is_empty() {
                            warn!("ffmpeg AAC encoder: {}", msg.trim());
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        info!("AAC-LC encoder started (256 kbps, 48 kHz stereo, ADTS)");

        Ok(Self {
            child,
            stdin_tx,
            output_rx,
        })
    }

    /// Get a clone of the PCM input sender (for feeding audio from another task).
    pub fn stdin_tx_clone(&self) -> mpsc::Sender<Vec<f32>> {
        self.stdin_tx.clone()
    }

    /// Receive the next chunk of encoded AAC (ADTS) data.
    /// Returns None when the encoder has finished.
    pub async fn recv_aac(&mut self) -> Option<Vec<u8>> {
        self.output_rx.recv().await
    }
}

impl Drop for AacEncoder {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_encoder() {
        // Skip if ffmpeg not available
        if std::process::Command::new("which")
            .arg("ffmpeg")
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            return;
        }
        let encoder = AacEncoder::new();
        assert!(encoder.is_ok());
    }
}
