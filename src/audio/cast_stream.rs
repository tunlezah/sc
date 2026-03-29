use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::Response;
use futures::stream;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::audio::aac_encoder::AacEncoder;
use crate::audio::mp3_encoder::Mp3Encoder;

/// Creates an HTTP response that streams AAC-LC encoded audio (ADTS) from the
/// PCM broadcast channel.
///
/// AAC-LC at 256 kbps in ADTS container is universally compatible with Safari,
/// Chrome, Chromecast, and AirPlay devices. The stream is served as `audio/aac`
/// with chunked transfer encoding.
///
/// Falls back to MP3 if FFmpeg is not available.
pub async fn stream_audio_aac(
    audio_sender: broadcast::Sender<Vec<f32>>,
) -> Result<Response, StatusCode> {
    match AacEncoder::new() {
        Ok(encoder) => stream_aac_inner(audio_sender, encoder).await,
        Err(e) => {
            warn!("AAC encoder unavailable ({}), falling back to MP3", e);
            stream_audio_mp3(audio_sender).await
        }
    }
}

/// Stream AAC audio via FFmpeg subprocess.
async fn stream_aac_inner(
    audio_sender: broadcast::Sender<Vec<f32>>,
    encoder: AacEncoder,
) -> Result<Response, StatusCode> {
    let mut audio_rx = audio_sender.subscribe();

    // Spawn a task that feeds PCM from the broadcast channel into the encoder
    let enc_sender = encoder.stdin_tx_clone();
    tokio::spawn(async move {
        loop {
            match audio_rx.recv().await {
                Ok(pcm_samples) => {
                    if enc_sender.send(pcm_samples).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    debug!("AAC stream lagged by {} frames, skipping", n);
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
        // Dropping enc_sender closes stdin → ffmpeg flushes and exits
    });

    // Stream the AAC output chunks to the HTTP response
    let stream = stream::unfold(encoder, |mut enc| async move {
        enc.recv_aac()
            .await
            .map(|data| (Ok::<_, std::io::Error>(data), enc))
    });

    let body = Body::from_stream(stream);

    info!("AAC-LC stream started (256 kbps, ADTS)");

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "audio/aac")
        .header(header::TRANSFER_ENCODING, "chunked")
        .header(header::CACHE_CONTROL, "no-cache, no-store")
        .header(header::CONNECTION, "keep-alive")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header("icy-name", "SoundSync")
        .header("icy-br", "256")
        .body(body)
        .unwrap())
}

/// MP3 stream endpoint (used as fallback when FFmpeg is unavailable, 256 kbps).
pub async fn stream_audio_mp3(
    audio_sender: broadcast::Sender<Vec<f32>>,
) -> Result<Response, StatusCode> {
    let audio_rx = audio_sender.subscribe();

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
        .header(header::CONNECTION, "keep-alive")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header("icy-name", "SoundSync")
        .header("icy-br", "256")
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
