use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::broadcast;
use tracing::{info, warn};

/// Captures PCM audio from the PipeWire monitor source.
/// Audio is broadcast as Vec<f32> frames (960 stereo samples = 1920 f32 values per 20ms).
pub struct AudioCapture {
    sender: broadcast::Sender<Vec<f32>>,
    child: Option<Child>,
}

const SAMPLE_RATE: u32 = 48000;
const CHANNELS: u32 = 2;
const FRAME_SIZE: usize = 960; // 20ms at 48kHz
const SAMPLES_PER_FRAME: usize = FRAME_SIZE * CHANNELS as usize; // 1920
const BYTES_PER_FRAME: usize = SAMPLES_PER_FRAME * 4; // f32 = 4 bytes

impl AudioCapture {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(16);
        Self {
            sender: tx,
            child: None,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Vec<f32>> {
        self.sender.subscribe()
    }

    /// Start capturing audio from the monitor source.
    pub async fn start(&mut self, source_name: &str) -> Result<(), String> {
        self.stop().await;

        // Try pw-cat first, fall back to parec
        let (cmd, args) = if which_exists("pw-cat") {
            (
                "pw-cat",
                vec![
                    "--target".to_string(),
                    source_name.to_string(),
                    "--format".to_string(),
                    "f32".to_string(),
                    "--channels".to_string(),
                    CHANNELS.to_string(),
                    "--rate".to_string(),
                    SAMPLE_RATE.to_string(),
                    "-r".to_string(),
                    "-".to_string(),
                ],
            )
        } else if which_exists("parec") {
            (
                "parec",
                vec![
                    "--raw".to_string(),
                    "--format=float32".to_string(),
                    format!("--channels={}", CHANNELS),
                    format!("--rate={}", SAMPLE_RATE),
                    format!("--monitor-stream={}", source_name),
                ],
            )
        } else {
            return Err("Neither pw-cat nor parec found".to_string());
        };

        info!("Starting audio capture: {} {:?}", cmd, args);

        let mut child = Command::new(cmd)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to start capture: {}", e))?;

        let stdout = child.stdout.take().ok_or("No stdout")?;
        let sender = self.sender.clone();

        // Spawn reader task
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stdout);
            let mut buf = vec![0u8; BYTES_PER_FRAME];

            loop {
                match reader.read_exact(&mut buf).await {
                    Ok(_) => {
                        let samples: Vec<f32> = buf
                            .chunks_exact(4)
                            .map(|chunk| {
                                f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
                            })
                            .collect();
                        let _ = sender.send(samples);
                    }
                    Err(e) => {
                        warn!("Audio capture read error: {}", e);
                        break;
                    }
                }
            }
        });

        self.child = Some(child);
        info!("Audio capture started");
        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
            info!("Audio capture stopped");
        }
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
    }
}

fn which_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
