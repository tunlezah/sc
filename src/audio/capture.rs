use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

/// Captures PCM audio from PipeWire/PulseAudio sources.
/// Audio is broadcast as Vec<f32> frames (960 stereo samples = 1920 f32 values per 20ms).
///
/// Uses a 3-tier source resolution strategy (matching the reference BluetoothA2DP project):
/// 1. Direct Bluetooth source (`bluez_input.*` / `bluez_source.*`) — highest fidelity
/// 2. Null sink monitor (`soundsync-capture.monitor`) — captures routed audio
/// 3. Default monitor (`@DEFAULT_MONITOR@`) — fallback
///
/// Prefers `parec` over `pw-cat` because `parec` uses PulseAudio source names
/// which are stable and predictable, while `pw-cat --target` expects PipeWire
/// node names/serials which may differ.
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
        let (tx, _) = broadcast::channel(64);
        Self {
            sender: tx,
            child: None,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Vec<f32>> {
        self.sender.subscribe()
    }

    /// Get a clone of the broadcast sender (for WebRTC manager).
    pub fn sender(&self) -> broadcast::Sender<Vec<f32>> {
        self.sender.clone()
    }

    /// Start capturing audio using smart source resolution.
    ///
    /// Tries sources in priority order:
    /// 1. Any active `bluez_input.*` or `bluez_source.*` source (direct BT capture)
    /// 2. The named monitor source (e.g. `soundsync-capture.monitor`)
    /// 3. `@DEFAULT_MONITOR@` (PulseAudio's default monitor)
    pub async fn start(&mut self, fallback_source: &str) -> Result<(), String> {
        self.stop().await;

        let source = resolve_capture_source(fallback_source).await;
        info!("Resolved capture source: {}", source);

        // Prefer parec (PulseAudio client) — uses PulseAudio source names which
        // are stable. pw-cat --target expects PipeWire node names which may differ.
        let (cmd, args) = if which_exists("parec") {
            (
                "parec",
                vec![
                    "--raw".to_string(),
                    "--format=float32".to_string(),
                    format!("--channels={}", CHANNELS),
                    format!("--rate={}", SAMPLE_RATE),
                    format!("--device={}", source),
                ],
            )
        } else if which_exists("pw-cat") {
            (
                "pw-cat",
                vec![
                    "--target".to_string(),
                    source.clone(),
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
        } else {
            return Err("Neither parec nor pw-cat found".to_string());
        };

        info!("Starting audio capture: {} {:?}", cmd, args);

        let mut child = Command::new(cmd)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to start capture: {}", e))?;

        let stdout = child.stdout.take().ok_or("No stdout")?;
        let stderr = child.stderr.take();
        let sender = self.sender.clone();

        // Spawn reader task for PCM data
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

        // Spawn stderr logger so capture errors are visible
        if let Some(stderr_stream) = stderr {
            tokio::spawn(async move {
                let mut reader = stderr_stream;
                let mut buf = vec![0u8; 1024];
                loop {
                    match reader.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            let msg = String::from_utf8_lossy(&buf[..n]);
                            if !msg.trim().is_empty() {
                                warn!("Audio capture stderr: {}", msg.trim());
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        self.child = Some(child);
        info!("Audio capture started from source: {}", source);
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

/// Resolve the best audio capture source using a 3-tier priority:
/// 1. Active Bluetooth source (`bluez_input.*` or `bluez_source.*`)
/// 2. Named fallback source (typically `soundsync-capture.monitor`)
/// 3. `@DEFAULT_MONITOR@` (PulseAudio default monitor)
async fn resolve_capture_source(fallback: &str) -> String {
    // Try to find an active Bluetooth audio source
    if let Some(bt_source) = find_bluetooth_source().await {
        info!("Found Bluetooth audio source: {}", bt_source);
        return bt_source;
    }

    // Check if the fallback source exists
    if source_exists(fallback).await {
        debug!("Using fallback source: {}", fallback);
        return fallback.to_string();
    }

    // Last resort: PulseAudio's built-in default monitor
    info!("Using @DEFAULT_MONITOR@ as capture source");
    "@DEFAULT_MONITOR@".to_string()
}

/// Search for active Bluetooth audio sources in PulseAudio/PipeWire.
/// Returns the first `bluez_input.*` or `bluez_source.*` source found.
async fn find_bluetooth_source() -> Option<String> {
    let output = Command::new("pactl")
        .args(["list", "short", "sources"])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 2 {
            let name = fields[1];
            if name.starts_with("bluez_input.") || name.starts_with("bluez_source.") {
                return Some(name.to_string());
            }
        }
    }

    None
}

/// Check if a PulseAudio source exists.
async fn source_exists(name: &str) -> bool {
    let output = Command::new("pactl")
        .args(["list", "short", "sources"])
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout.lines().any(|line| line.contains(name))
        }
        _ => false,
    }
}

fn which_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
