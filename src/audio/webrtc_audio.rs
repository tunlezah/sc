use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_local::TrackLocalWriter;

use crate::audio::opus_encoder::OpusEncoder;
use crate::state::{AppStateHandle, SystemEvent};

/// Commands sent from the WebSocket handler to the WebRTC manager task.
#[derive(Debug)]
pub enum WebRtcCommand {
    /// Browser sent an SDP offer to start streaming.
    Offer { session_id: String, sdp: String },
    /// Browser sent an ICE candidate.
    IceCandidate {
        session_id: String,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },
    /// Browser requested session teardown.
    Stop { session_id: String },
}

/// Manages WebRTC sessions for audio streaming to browsers.
///
/// Uses the pure-Rust `webrtc` crate. Each session gets its own peer connection
/// and an audio pump task that reads PCM from the AudioCapture broadcast channel,
/// encodes to Opus, and writes RTP packets to the track.
pub struct WebRtcManager {
    api: webrtc::api::API,
    sessions: HashMap<String, WebRtcSession>,
    audio_sender: broadcast::Sender<Vec<f32>>,
    state: AppStateHandle,
}

/// A single WebRTC session with a browser client.
struct WebRtcSession {
    peer_connection: Arc<webrtc::peer_connection::RTCPeerConnection>,
    #[allow(dead_code)]
    audio_track: Arc<TrackLocalStaticRTP>,
    pump_task: Option<JoinHandle<()>>,
}

impl WebRtcManager {
    /// Create a new WebRTC manager.
    ///
    /// `audio_sender` is the broadcast channel from AudioCapture that provides
    /// PCM audio frames (1920 interleaved f32 samples per 20ms frame).
    pub fn new(
        audio_sender: broadcast::Sender<Vec<f32>>,
        state: AppStateHandle,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs()?;

        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut media_engine)?;

        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .build();

        Ok(Self {
            api,
            sessions: HashMap::new(),
            audio_sender,
            state,
        })
    }

    /// Run the WebRTC manager, processing commands from the channel.
    pub async fn run(mut self, mut cmd_rx: mpsc::Receiver<WebRtcCommand>) {
        info!("WebRTC manager started");
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                WebRtcCommand::Offer { session_id, sdp } => {
                    info!(
                        "WebRTC offer for session {} ({} bytes)",
                        session_id,
                        sdp.len()
                    );
                    match self.handle_offer(session_id.clone(), &sdp).await {
                        Ok(answer_sdp) => {
                            self.state.publish(SystemEvent::WebRtcAnswer {
                                session_id: session_id.clone(),
                                sdp: answer_sdp,
                            });
                        }
                        Err(e) => {
                            error!("WebRTC offer failed for {}: {}", session_id, e);
                        }
                    }
                }
                WebRtcCommand::IceCandidate {
                    session_id,
                    candidate,
                    sdp_mid,
                    sdp_mline_index,
                } => {
                    debug!("ICE candidate for session {}", session_id);
                    if let Err(e) = self
                        .handle_ice_candidate(
                            &session_id,
                            &candidate,
                            sdp_mid.as_deref(),
                            sdp_mline_index,
                        )
                        .await
                    {
                        error!("ICE candidate failed for {}: {}", session_id, e);
                    }
                }
                WebRtcCommand::Stop { session_id } => {
                    info!("Removing WebRTC session: {}", session_id);
                    self.remove_session(&session_id).await;
                }
            }
        }
        info!("WebRTC manager shutting down");
    }

    /// Handle an SDP offer: create peer connection, audio track, and start the
    /// Opus encoding pump that streams audio to the browser.
    async fn handle_offer(
        &mut self,
        session_id: String,
        sdp: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let config = RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        };

        let peer_connection = Arc::new(self.api.new_peer_connection(config).await?);

        // Create an audio track for Opus at 48kHz stereo
        let audio_track = Arc::new(TrackLocalStaticRTP::new(
            webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability {
                mime_type: "audio/opus".to_string(),
                clock_rate: 48000,
                channels: 2,
                ..Default::default()
            },
            format!("audio-{}", session_id),
            format!("soundsync-{}", session_id),
        ));

        peer_connection
            .add_track(Arc::clone(&audio_track) as Arc<dyn TrackLocal + Send + Sync>)
            .await?;

        // Set up ICE candidate callback to forward candidates to the browser
        let state_handle = self.state.clone();
        let ice_session_id = session_id.clone();
        peer_connection.on_ice_candidate(Box::new(move |candidate| {
            let state_handle = state_handle.clone();
            let session_id = ice_session_id.clone();
            Box::pin(async move {
                if let Some(candidate) = candidate {
                    if let Ok(json) = candidate.to_json() {
                        state_handle.publish(SystemEvent::WebRtcIceCandidate {
                            session_id,
                            candidate: json.candidate,
                            sdp_mid: json.sdp_mid,
                            sdp_mline_index: json.sdp_mline_index,
                        });
                    }
                }
            })
        }));

        // Process the SDP offer
        let offer = RTCSessionDescription::offer(sdp.to_string())?;
        peer_connection.set_remote_description(offer).await?;

        let answer = peer_connection.create_answer(None).await?;
        peer_connection
            .set_local_description(answer.clone())
            .await?;

        // Start the audio pump task: subscribe to PCM, encode Opus, write RTP
        let track = Arc::clone(&audio_track);
        let mut audio_rx = self.audio_sender.subscribe();

        let pump_task = tokio::spawn(async move {
            let mut encoder = match OpusEncoder::new() {
                Ok(enc) => enc,
                Err(e) => {
                    error!("Failed to create Opus encoder: {}", e);
                    return;
                }
            };

            info!("WebRTC audio pump started, waiting for PCM data...");
            let mut timestamp: u32 = 0;
            let mut sequence_number: u16 = 0;
            let mut frames_sent: u64 = 0;

            loop {
                match audio_rx.recv().await {
                    Ok(pcm_samples) => {
                        if frames_sent == 0 {
                            info!(
                                "WebRTC audio pump: first PCM frame ({} samples)",
                                pcm_samples.len()
                            );
                        }
                        let opus_data = match encoder.encode_frame(&pcm_samples) {
                            Some(data) => data,
                            None => continue,
                        };

                        let packet = rtp::packet::Packet {
                            header: rtp::header::Header {
                                version: 2,
                                payload_type: 111,
                                sequence_number,
                                timestamp,
                                ..Default::default()
                            },
                            payload: bytes::Bytes::from(opus_data),
                        };

                        if track.write_rtp(&packet).await.is_err() {
                            info!(
                                "WebRTC audio pump: write_rtp failed after {} frames",
                                frames_sent
                            );
                            break;
                        }

                        frames_sent += 1;
                        if frames_sent % 500 == 0 {
                            info!("WebRTC audio pump: sent {} frames", frames_sent);
                        }

                        timestamp = timestamp.wrapping_add(960);
                        sequence_number = sequence_number.wrapping_add(1);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("WebRTC audio pump: broadcast channel closed");
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        info!("WebRTC audio pump: lagged by {} frames", n);
                    }
                }
            }
            info!("WebRTC audio pump exited after {} frames", frames_sent);
        });

        self.sessions.insert(
            session_id,
            WebRtcSession {
                peer_connection,
                audio_track,
                pump_task: Some(pump_task),
            },
        );

        Ok(answer.sdp)
    }

    /// Handle an ICE candidate from a browser client.
    async fn handle_ice_candidate(
        &self,
        session_id: &str,
        candidate: &str,
        sdp_mid: Option<&str>,
        sdp_mline_index: Option<u16>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| format!("Session not found: {}", session_id))?;

        let ice_candidate = RTCIceCandidateInit {
            candidate: candidate.to_string(),
            sdp_mid: sdp_mid.map(|s| s.to_string()),
            sdp_mline_index,
            ..Default::default()
        };

        session
            .peer_connection
            .add_ice_candidate(ice_candidate)
            .await?;

        Ok(())
    }

    /// Remove a session and clean up resources.
    async fn remove_session(&mut self, session_id: &str) {
        if let Some(mut session) = self.sessions.remove(session_id) {
            if let Some(task) = session.pump_task.take() {
                task.abort();
            }
            let _ = session.peer_connection.close().await;
            info!("WebRTC session {} cleaned up", session_id);
        }
    }
}
