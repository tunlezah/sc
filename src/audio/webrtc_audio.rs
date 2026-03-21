use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::info;

use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_OPUS};
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::TrackLocal;

/// Manages WebRTC sessions for browser audio streaming.
#[allow(dead_code)]
pub struct WebRtcManager {
    sessions: Arc<RwLock<HashMap<String, WebRtcSession>>>,
    audio_rx: broadcast::Sender<Vec<f32>>,
}

#[allow(dead_code)]
struct WebRtcSession {
    peer_connection: Arc<RTCPeerConnection>,
}

#[allow(dead_code)]
impl WebRtcManager {
    pub fn new(audio_tx: broadcast::Sender<Vec<f32>>) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            audio_rx: audio_tx,
        }
    }

    /// Handle a WebRTC offer from a browser client.
    /// Returns an SDP answer string.
    pub async fn handle_offer(
        &self,
        session_id: String,
        offer_sdp: String,
    ) -> Result<String, String> {
        // Create media engine with Opus codec
        let mut media_engine = MediaEngine::default();
        media_engine
            .register_default_codecs()
            .map_err(|e| format!("Failed to register codecs: {}", e))?;

        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut media_engine)
            .map_err(|e| format!("Failed to register interceptors: {}", e))?;

        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .build();

        let config = RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        };

        let peer_connection = Arc::new(
            api.new_peer_connection(config)
                .await
                .map_err(|e| format!("Failed to create PeerConnection: {}", e))?,
        );

        // Create audio track
        let audio_track = Arc::new(TrackLocalStaticRTP::new(
            RTCRtpCodecCapability {
                mime_type: MIME_TYPE_OPUS.to_string(),
                clock_rate: 48000,
                channels: 2,
                ..Default::default()
            },
            "audio".to_string(),
            "soundsync".to_string(),
        ));

        // Add track to peer connection
        peer_connection
            .add_track(Arc::clone(&audio_track) as Arc<dyn TrackLocal + Send + Sync>)
            .await
            .map_err(|e| format!("Failed to add track: {}", e))?;

        // Set remote description (the offer)
        let offer = RTCSessionDescription::offer(offer_sdp)
            .map_err(|e| format!("Invalid offer SDP: {}", e))?;
        peer_connection
            .set_remote_description(offer)
            .await
            .map_err(|e| format!("Failed to set remote description: {}", e))?;

        // Create answer
        let answer = peer_connection
            .create_answer(None)
            .await
            .map_err(|e| format!("Failed to create answer: {}", e))?;

        // Set local description
        peer_connection
            .set_local_description(answer.clone())
            .await
            .map_err(|e| format!("Failed to set local description: {}", e))?;

        // Store session
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(
                session_id.clone(),
                WebRtcSession {
                    peer_connection: Arc::clone(&peer_connection),
                },
            );
        }

        info!("WebRTC session created: {}", session_id);

        Ok(answer.sdp)
    }

    /// Handle an ICE candidate from a browser client.
    pub async fn handle_ice_candidate(
        &self,
        session_id: &str,
        candidate: &str,
        sdp_mid: Option<&str>,
        sdp_mline_index: Option<u16>,
    ) -> Result<(), String> {
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| format!("Session not found: {}", session_id))?;

        let candidate_init = webrtc::ice_transport::ice_candidate::RTCIceCandidateInit {
            candidate: candidate.to_string(),
            sdp_mid: sdp_mid.map(|s| s.to_string()),
            sdp_mline_index,
            username_fragment: None,
        };

        session
            .peer_connection
            .add_ice_candidate(candidate_init)
            .await
            .map_err(|e| format!("Failed to add ICE candidate: {}", e))?;

        Ok(())
    }

    /// Remove a WebRTC session.
    pub async fn remove_session(&self, session_id: &str) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.remove(session_id) {
            let _ = session.peer_connection.close().await;
            info!("WebRTC session removed: {}", session_id);
        }
    }

    /// Get the number of active sessions.
    pub async fn session_count(&self) -> usize {
        self.sessions.read().await.len()
    }
}
