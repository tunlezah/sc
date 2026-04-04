use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

/// Captures PCM audio from PipeWire/PulseAudio sources.
/// Audio is broadcast as Vec<f32> frames (960 stereo samples = 1920 f32 values per 20ms).
///
/// Uses a 3-tier source resolution strategy:
/// 1. Direct Bluetooth source (`bluez_input.*` / `bluez_source.*`) — highest fidelity
/// 2. Named null sink (e.g. `soundsync-capture`) — captures routed audio
/// 3. No target (default source) — fallback
///
/// Works with both PulseAudio tools (parec) and PipeWire-native tools (pw-cat).
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
        // 256 frames × 20ms = 5.12 seconds of buffer. This must be large
        // enough to absorb temporary slowdowns from ANY subscriber (WebRTC
        // encoder, spectrum FFT, HTTP stream encoder). A capacity of 64
        // (1.28s) was too small — when any consumer falls behind by ~1s, ALL
        // consumers see Lagged errors, causing audible stuttering in WebRTC.
        let (tx, _) = broadcast::channel(256);
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
    /// 2. The named null sink (for routed audio)
    /// 3. Default source (no target specified)
    pub async fn start(&mut self, null_sink_name: &str) -> Result<(), String> {
        self.stop().await;

        let source = resolve_capture_source(null_sink_name).await;
        info!("Resolved capture source: {:?}", source);

        let (cmd, args) = build_capture_command(&source)?;

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
            // Spawn a task to reap the child process and prevent zombies.
            // Drop is synchronous so we cannot await here directly.
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
        }
    }
}

/// Resolved audio source for capture.
#[derive(Debug)]
enum CaptureSource {
    /// Direct Bluetooth source node (e.g. "bluez_input.44_4A_DB_B4_E7_0D")
    BluetoothDirect(String),
    /// Named null sink to capture monitor from (e.g. "soundsync-capture")
    NullSink(String),
    /// No specific source — use default
    Default,
}

/// Resolve the audio capture source.
///
/// Always captures from the null sink monitor so that audio passes through the
/// EQ filter-chain before reaching consumers (WebRTC, spectrum, HTTP streams).
/// Direct Bluetooth capture is only used as a last resort when the null sink
/// doesn't exist (e.g. pactl/pw-loopback both failed).
async fn resolve_capture_source(null_sink_name: &str) -> CaptureSource {
    // Primary: capture from null sink monitor (receives EQ-processed audio)
    if node_exists(null_sink_name).await {
        debug!("Using null sink monitor: {}", null_sink_name);
        return CaptureSource::NullSink(null_sink_name.to_string());
    }

    // Fallback: direct Bluetooth source (no EQ processing)
    if let Some(bt_source) = find_bluetooth_source().await {
        warn!(
            "Null sink not available — falling back to direct Bluetooth capture: {} \
             (EQ will not be applied)",
            bt_source
        );
        return CaptureSource::BluetoothDirect(bt_source);
    }

    info!("No specific source found — using default");
    CaptureSource::Default
}

/// Build the capture command and arguments based on available tools and source.
fn build_capture_command(source: &CaptureSource) -> Result<(&'static str, Vec<String>), String> {
    if which_exists("parec") {
        // parec uses PulseAudio source names
        let device = match source {
            CaptureSource::BluetoothDirect(name) => name.clone(),
            CaptureSource::NullSink(name) => format!("{}.monitor", name),
            CaptureSource::Default => "@DEFAULT_MONITOR@".to_string(),
        };

        Ok((
            "parec",
            vec![
                "--raw".to_string(),
                "--format=float32".to_string(),
                format!("--channels={}", CHANNELS),
                format!("--rate={}", SAMPLE_RATE),
                format!("--device={}", device),
                // 50ms gives the OS scheduler enough headroom to handle
                // scheduling jitter without causing buffer underruns. The
                // previous value of 20ms matched the Opus frame size but left
                // zero margin — any kernel scheduling delay (even 1-2ms) caused
                // underruns. PipeWire's quantum can be 21.3ms (1024 samples)
                // which is already larger than 20ms, guaranteeing underruns.
                "--latency-msec=50".to_string(),
            ],
        ))
    } else if which_exists("pw-cat") {
        // pw-cat uses PipeWire node names
        let mut args = vec![
            "--format".to_string(),
            "f32".to_string(),
            "--channels".to_string(),
            CHANNELS.to_string(),
            "--rate".to_string(),
            SAMPLE_RATE.to_string(),
            "-r".to_string(), // record mode
        ];

        match source {
            CaptureSource::BluetoothDirect(name) | CaptureSource::NullSink(name) => {
                args.push("--target".to_string());
                args.push(name.clone());
            }
            CaptureSource::Default => {
                // No --target: pw-cat records from default source
            }
        }

        args.push("-".to_string()); // output to stdout

        Ok(("pw-cat", args))
    } else {
        Err("Neither parec nor pw-cat found".to_string())
    }
}

/// Search for active Bluetooth audio sources.
/// Tries pactl first, then PipeWire-native pw-cli/pw-dump.
async fn find_bluetooth_source() -> Option<String> {
    // Method 1: pactl (PulseAudio compat)
    if which_exists("pactl") {
        if let Some(src) = find_bt_source_pactl().await {
            return Some(src);
        }
    }

    // Method 2: pw-cli (PipeWire native)
    if which_exists("pw-cli") {
        if let Some(src) = find_bt_source_pwcli().await {
            return Some(src);
        }
    }

    // Method 3: pw-dump (PipeWire JSON dump)
    if which_exists("pw-dump") {
        if let Some(src) = find_bt_source_pwdump().await {
            return Some(src);
        }
    }

    None
}

/// Find Bluetooth source via pactl.
async fn find_bt_source_pactl() -> Option<String> {
    let output = Command::new("pactl")
        .args(["list", "short", "sources"])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    find_bt_name_in_listing(&stdout)
}

/// Find Bluetooth source via pw-cli.
async fn find_bt_source_pwcli() -> Option<String> {
    let output = Command::new("pw-cli")
        .args(["list-objects"])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // pw-cli list-objects shows node names in its output
    find_bt_name_in_listing(&stdout)
}

/// Find Bluetooth source via pw-dump (JSON).
async fn find_bt_source_pwdump() -> Option<String> {
    let output = Command::new("pw-dump").output().await.ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Search for bluez node names in the JSON dump
    for line in stdout.lines() {
        let trimmed = line.trim().trim_end_matches(',').trim_matches('"');
        if is_bt_source_name(trimmed) {
            return Some(trimmed.to_string());
        }
        // Also check for "node.name" = "bluez_input.*" patterns
        if line.contains("node.name") {
            if let Some(name) = extract_json_string_value(line) {
                if is_bt_source_name(&name) {
                    return Some(name);
                }
            }
        }
    }

    None
}

/// Check if a given node exists in PipeWire.
async fn node_exists(name: &str) -> bool {
    // Try pactl first
    if which_exists("pactl") {
        let output = Command::new("pactl")
            .args(["list", "short", "sinks"])
            .output()
            .await;
        if let Ok(out) = output {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                if stdout.contains(name) {
                    return true;
                }
            }
        }
    }

    // Try pw-cli
    if which_exists("pw-cli") {
        let output = Command::new("pw-cli").args(["list-objects"]).output().await;
        if let Ok(out) = output {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout);
                if stdout.contains(name) {
                    return true;
                }
            }
        }
    }

    false
}

/// Check if a name looks like a Bluetooth audio source.
fn is_bt_source_name(name: &str) -> bool {
    name.starts_with("bluez_input.") || name.starts_with("bluez_source.")
}

/// Find a Bluetooth source name in a text listing (works for both pactl and pw-cli output).
fn find_bt_name_in_listing(text: &str) -> Option<String> {
    for line in text.lines() {
        for word in line.split_whitespace() {
            if is_bt_source_name(word) {
                return Some(word.to_string());
            }
        }
    }
    None
}

/// Extract a string value from a JSON-like line: `"key": "value",` → `value`
fn extract_json_string_value(line: &str) -> Option<String> {
    let parts: Vec<&str> = line.splitn(2, ':').collect();
    if parts.len() == 2 {
        // Strip whitespace, then commas, then quotes (order matters for `"value",` patterns)
        let val = parts[1]
            .trim()
            .trim_end_matches(',')
            .trim_matches('"')
            .trim();
        if !val.is_empty() {
            return Some(val.to_string());
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
