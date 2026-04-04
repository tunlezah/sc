use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::Response;
use futures::stream;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tracing::{info, warn};

/// Resolve the capture device for direct parec streaming.
///
/// Priority:
/// 1. `soundsync-capture.monitor` (EQ-processed audio via null sink)
/// 2. Any `bluez_input.*` / `bluez_source.*` (direct BT capture)
/// 3. `@DEFAULT_MONITOR@` (fallback)
fn resolve_capture_device() -> String {
    // Check if soundsync-capture exists
    let sinks = std::process::Command::new("pactl")
        .args(["list", "short", "sinks"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();

    if sinks.contains("soundsync-capture") {
        return "soundsync-capture.monitor".to_string();
    }

    // Check for direct Bluetooth source
    let sources = std::process::Command::new("pactl")
        .args(["list", "short", "sources"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();

    for line in sources.lines() {
        if let Some(name) = line.split_whitespace().nth(1) {
            if name.starts_with("bluez_input.") || name.starts_with("bluez_source.") {
                return name.to_string();
            }
        }
    }

    "@DEFAULT_MONITOR@".to_string()
}

/// Creates an HTTP response that streams AAC-LC audio using a direct
/// `parec | ffmpeg` pipeline — no intermediate broadcast channel.
///
/// This approach matches BluetoothA2DP's proven architecture: each browser
/// client gets its own dedicated capture→encode pipeline managed entirely
/// by the OS kernel's pipe buffering. This eliminates the timing jitter
/// and allocation overhead that caused stuttering when routing through
/// Tokio's broadcast channel.
pub async fn stream_audio_aac(
    _audio_sender: tokio::sync::broadcast::Sender<Vec<f32>>,
) -> Result<Response, StatusCode> {
    let device = resolve_capture_device();

    // Try direct parec | ffmpeg pipeline (AAC-LC 256k ADTS)
    let cmd = format!(
        "parec --device={} --format=s16le --rate=48000 --channels=2 --latency-msec=50 \
         | ffmpeg -hide_banner -loglevel quiet \
                  -fflags +nobuffer \
                  -f s16le -ar 48000 -ac 2 -i pipe:0 \
                  -c:a aac -b:a 256k -f adts -flush_packets 1 pipe:1",
        device
    );

    if let Some(resp) = try_pipe_stream(&cmd, "audio/aac").await {
        info!(
            "AAC-LC stream started (256 kbps, ADTS, direct pipe from {})",
            device
        );
        return Ok(resp);
    }

    // Fallback to MP3
    warn!("AAC pipeline failed, trying MP3 fallback");
    stream_audio_mp3(_audio_sender).await
}

/// MP3 stream endpoint using direct `parec | ffmpeg` pipeline.
pub async fn stream_audio_mp3(
    _audio_sender: tokio::sync::broadcast::Sender<Vec<f32>>,
) -> Result<Response, StatusCode> {
    let device = resolve_capture_device();

    let cmd = format!(
        "parec --device={} --format=s16le --rate=48000 --channels=2 --latency-msec=50 \
         | ffmpeg -hide_banner -loglevel quiet \
                  -fflags +nobuffer \
                  -f s16le -ar 48000 -ac 2 -i pipe:0 \
                  -c:a libmp3lame -b:a 192k -f mp3 -flush_packets 1 pipe:1",
        device
    );

    if let Some(resp) = try_pipe_stream(&cmd, "audio/mpeg").await {
        info!("MP3 stream started (192 kbps, direct pipe from {})", device);
        return Ok(resp);
    }

    warn!("MP3 pipeline failed — no streaming available");
    Err(StatusCode::SERVICE_UNAVAILABLE)
}

/// Spawn a `parec | ffmpeg` shell pipeline and stream the output as an HTTP response.
///
/// Each stream client gets a dedicated pipeline. When the client disconnects,
/// `kill_on_drop(true)` terminates the shell and all child processes.
async fn try_pipe_stream(cmd: &str, content_type: &'static str) -> Option<Response> {
    let mut child = Command::new("sh")
        .args(["-c", cmd])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;

    let stdout = child.stdout.take()?;

    let stream = stream::unfold(
        (tokio::io::BufReader::new(stdout), child),
        |(mut reader, proc)| async move {
            let mut buf = vec![0u8; 8192];
            match reader.read(&mut buf).await {
                Ok(0) => None, // EOF — process exited
                Ok(n) => {
                    buf.truncate(n);
                    Some((Ok::<_, std::io::Error>(buf), (reader, proc)))
                }
                Err(_) => None,
            }
        },
    );

    let body = Body::from_stream(stream);

    Some(
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::TRANSFER_ENCODING, "chunked")
            .header(header::CACHE_CONTROL, "no-cache, no-store")
            .header(header::CONNECTION, "keep-alive")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header("icy-name", "SoundSync")
            .body(body)
            .unwrap(),
    )
}

/// Detects the local IP address by inspecting the network interfaces.
/// Returns the first non-loopback IPv4 address found, or falls back to "127.0.0.1".
pub fn detect_local_ip() -> String {
    if let Ok(output) = std::process::Command::new("hostname").arg("-I").output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(ip) = stdout.split_whitespace().next() {
                if !ip.is_empty() {
                    return ip.to_string();
                }
            }
        }
    }

    // Fallback: try to parse /proc/net/fib_trie or use socket trick
    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
        // Connect to a public address to determine local interface
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = socket.local_addr() {
                return addr.ip().to_string();
            }
        }
    }

    "127.0.0.1".to_string()
}
