use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::stream;
use tokio::sync::broadcast;
use tracing::{debug, warn};

use crate::audio::mp3_encoder::Mp3Encoder;

/// Creates an HTTP response that streams MP3-encoded audio from the PCM broadcast channel.
///
/// This endpoint subscribes to the same broadcast channel used by WebRTC and the
/// spectrum analyzer, encoding PCM f32 frames to MP3 in real-time. The stream is
/// served as `audio/mpeg` with chunked transfer encoding, suitable for Chromecast
/// media playback or any HTTP media player.
///
/// The stream continues until the client disconnects or the broadcast channel closes.
pub async fn stream_audio_mp3(
    audio_sender: broadcast::Sender<Vec<f32>>,
) -> Result<Response, StatusCode> {
    let mut audio_rx = audio_sender.subscribe();

    let encoder = Mp3Encoder::new().map_err(|e| {
        warn!("Failed to create MP3 encoder for stream: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let stream = stream::unfold((audio_rx, encoder), |(mut rx, mut enc)| async move {
        loop {
            match rx.recv().await {
                Ok(pcm_samples) => {
                    if let Some(mp3_data) = enc.encode_frame(&pcm_samples) {
                        return Some((Ok::<_, std::io::Error>(mp3_data), (rx, enc)));
                    }
                    // Encoder buffered the frame, continue to next
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    debug!("MP3 stream lagged by {} frames, skipping", n);
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    // Channel closed, flush encoder and end stream
                    if let Some(final_data) = enc.flush() {
                        return Some((Ok(final_data), (rx, enc)));
                    }
                    return None;
                }
            }
        }
    });

    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "audio/mpeg")
        .header(header::TRANSFER_ENCODING, "chunked")
        .header(header::CACHE_CONTROL, "no-cache, no-store")
        .header("icy-name", "SoundSync")
        .header("icy-br", "192")
        .body(body)
        .unwrap())
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
