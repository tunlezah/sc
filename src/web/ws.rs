use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::audio::webrtc_audio::WebRtcCommand;
use crate::state::SystemEvent;
use crate::web::routes::AppRouter;

/// WebSocket handler for the `/ws/status` endpoint.
pub async fn ws_handler(ws: WebSocketUpgrade, State(app): State<AppRouter>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, app))
}

/// Handle an individual WebSocket connection.
async fn handle_socket(socket: WebSocket, app: AppRouter) {
    let (mut sender, mut receiver) = socket.split();
    let session_id = uuid::Uuid::new_v4().to_string();
    info!("WebSocket connected: {}", session_id);

    // Send initial state snapshot
    {
        let state = app.state.state.read().await;
        let snapshot = state.snapshot();
        let msg = WsOutMessage::StateSnapshot {
            data: Box::new(snapshot),
        };
        if let Ok(json) = serde_json::to_string(&msg) {
            let _ = sender.send(Message::Text(json)).await;
        }
    }

    // Subscribe to events
    let mut event_rx = app.state.subscribe();

    // Spawn task to forward events to the WebSocket
    let send_session_id = session_id.clone();
    let ws_session_id = session_id.clone();
    let send_task = tokio::spawn(async move {
        loop {
            match event_rx.recv().await {
                Ok(event) => {
                    if let Some(msg) = event_to_ws_message(&event, &ws_session_id) {
                        if let Ok(json) = serde_json::to_string(&msg) {
                            if sender.send(Message::Text(json)).await.is_err() {
                                break;
                            }
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!("WebSocket {} lagged, dropped {} events", send_session_id, n);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Handle incoming messages from browser
    let recv_app = app.clone();
    let recv_session_id = session_id.clone();
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    handle_client_message(&recv_app, &recv_session_id, &text).await;
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    // Wait for either task to finish
    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    // Clean up WebRTC session if one was started
    if let Some(ref tx) = app.webrtc_cmd_tx {
        let _ = tx
            .send(WebRtcCommand::Stop {
                session_id: session_id.clone(),
            })
            .await;
    }

    info!("WebSocket disconnected: {}", session_id);
}

/// Handle incoming WebSocket messages from the browser (WebRTC signaling).
async fn handle_client_message(app: &AppRouter, session_id: &str, text: &str) {
    let msg = match serde_json::from_str::<WsInMessage>(text) {
        Ok(m) => m,
        Err(_) => return,
    };

    let webrtc_tx = match &app.webrtc_cmd_tx {
        Some(tx) => tx,
        None => {
            warn!("WebRTC command received but no WebRTC manager available");
            return;
        }
    };

    match msg {
        WsInMessage::Offer { data } => {
            info!(
                "WebRTC offer from session {} ({} bytes)",
                session_id,
                data.sdp.len()
            );
            let _ = webrtc_tx
                .send(WebRtcCommand::Offer {
                    session_id: session_id.to_string(),
                    sdp: data.sdp,
                })
                .await;
        }
        WsInMessage::IceCandidate { data } => {
            let _ = webrtc_tx
                .send(WebRtcCommand::IceCandidate {
                    session_id: session_id.to_string(),
                    candidate: data.candidate,
                    sdp_mid: data.sdp_mid,
                    sdp_mline_index: data.sdp_mline_index,
                })
                .await;
        }
        WsInMessage::Stop { .. } => {
            info!("WebRTC stop from session {}", session_id);
            let _ = webrtc_tx
                .send(WebRtcCommand::Stop {
                    session_id: session_id.to_string(),
                })
                .await;
        }
    }
}

/// Convert a SystemEvent to a WebSocket message.
/// Only forwards events relevant to the specific WebSocket client.
/// WebRTC events are filtered by session_id to send only to the right client.
fn event_to_ws_message(event: &SystemEvent, ws_session_id: &str) -> Option<WsOutMessage> {
    match event {
        SystemEvent::DeviceStateChanged {
            address,
            name,
            state,
        } => Some(WsOutMessage::DeviceStateChanged {
            data: DeviceStateData {
                address: address.clone(),
                name: name.clone(),
                state: state.clone(),
            },
        }),
        SystemEvent::EqChanged { bands, enabled } => Some(WsOutMessage::EqChanged {
            data: crate::state::EqSnapshot {
                bands: bands.clone(),
                enabled: *enabled,
            },
        }),
        SystemEvent::TrackChanged { track } => Some(WsOutMessage::TrackChanged {
            data: track.clone(),
        }),
        SystemEvent::PlaybackStatusChanged { status } => {
            Some(WsOutMessage::PlaybackStatusChanged {
                data: PlaybackStatusData { status: *status },
            })
        }
        SystemEvent::SpectrumData { bands } => Some(WsOutMessage::SpectrumData {
            data: SpectrumBands {
                bands: bands.clone(),
            },
        }),
        SystemEvent::BluetoothStatusChanged { status } => {
            Some(WsOutMessage::BluetoothStatusChanged {
                data: BtStatusData {
                    status: status.clone(),
                },
            })
        }
        SystemEvent::DeviceDiscovered {
            address,
            name,
            rssi: _,
        } => Some(WsOutMessage::DeviceStateChanged {
            data: DeviceStateData {
                address: address.clone(),
                name: name.clone(),
                state: crate::bluetooth::device::DeviceState::Discovered,
            },
        }),
        // WebRTC events: only send to the matching session
        SystemEvent::WebRtcAnswer { session_id, sdp } if session_id == ws_session_id => {
            Some(WsOutMessage::WebrtcAnswer {
                data: SdpData { sdp: sdp.clone() },
            })
        }
        SystemEvent::WebRtcIceCandidate {
            session_id,
            candidate,
            sdp_mid,
            sdp_mline_index,
        } if session_id == ws_session_id => Some(WsOutMessage::WebrtcIceCandidate {
            data: IceCandidateData {
                candidate: candidate.clone(),
                sdp_mid: sdp_mid.clone(),
                sdp_mline_index: *sdp_mline_index,
            },
        }),
        SystemEvent::StreamStarted { address, codec: _ } => {
            // Don't include name here - the frontend will preserve the existing name
            Some(WsOutMessage::DeviceStateChanged {
                data: DeviceStateData {
                    address: address.clone(),
                    name: String::new(), // Frontend should not overwrite with empty
                    state: crate::bluetooth::device::DeviceState::AudioActive,
                },
            })
        }
        SystemEvent::CastDeviceDiscovered { device } => Some(WsOutMessage::CastDeviceDiscovered {
            data: device.clone(),
        }),
        SystemEvent::CastDeviceRemoved { device_id } => Some(WsOutMessage::CastDeviceRemoved {
            data: CastDeviceRemovedData {
                device_id: device_id.clone(),
            },
        }),
        SystemEvent::CastSessionStarted { device } => Some(WsOutMessage::CastSessionStarted {
            data: device.clone(),
        }),
        SystemEvent::CastSessionStopped { device_id } => Some(WsOutMessage::CastSessionStopped {
            data: CastDeviceRemovedData {
                device_id: device_id.clone(),
            },
        }),
        SystemEvent::CastError { message } => Some(WsOutMessage::CastError {
            data: ErrorData {
                message: message.clone(),
            },
        }),
        SystemEvent::AirPlayDeviceDiscovered { device } => {
            Some(WsOutMessage::AirPlayDeviceDiscovered {
                data: device.clone(),
            })
        }
        SystemEvent::AirPlayDeviceRemoved { device_name } => {
            Some(WsOutMessage::AirPlayDeviceRemoved {
                data: AirPlayDeviceRemovedData {
                    device_name: device_name.clone(),
                },
            })
        }
        SystemEvent::AirPlaySessionStarted { device } => {
            Some(WsOutMessage::AirPlaySessionStarted {
                data: device.clone(),
            })
        }
        SystemEvent::AirPlaySessionStopped { device_name } => {
            Some(WsOutMessage::AirPlaySessionStopped {
                data: AirPlayDeviceRemovedData {
                    device_name: device_name.clone(),
                },
            })
        }
        SystemEvent::AirPlayError { message } => Some(WsOutMessage::AirPlayError {
            data: ErrorData {
                message: message.clone(),
            },
        }),
        SystemEvent::LineInActivated => Some(WsOutMessage::LineInChanged {
            data: LineInData { active: true },
        }),
        SystemEvent::LineInDeactivated => Some(WsOutMessage::LineInChanged {
            data: LineInData { active: false },
        }),
        _ => None,
    }
}

// -- WebSocket outgoing message types --

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WsOutMessage {
    StateSnapshot {
        data: Box<crate::state::AppStateSnapshot>,
    },
    DeviceStateChanged {
        data: DeviceStateData,
    },
    EqChanged {
        data: crate::state::EqSnapshot,
    },
    TrackChanged {
        data: Option<crate::state::TrackInfo>,
    },
    PlaybackStatusChanged {
        data: PlaybackStatusData,
    },
    SpectrumData {
        data: SpectrumBands,
    },
    BluetoothStatusChanged {
        data: BtStatusData,
    },
    WebrtcAnswer {
        data: SdpData,
    },
    WebrtcIceCandidate {
        data: IceCandidateData,
    },
    CastDeviceDiscovered {
        data: crate::audio::chromecast::CastDeviceInfo,
    },
    CastDeviceRemoved {
        data: CastDeviceRemovedData,
    },
    CastSessionStarted {
        data: crate::audio::chromecast::CastDeviceInfo,
    },
    CastSessionStopped {
        data: CastDeviceRemovedData,
    },
    CastError {
        data: ErrorData,
    },
    AirPlayDeviceDiscovered {
        data: crate::audio::airplay::AirPlayDeviceInfo,
    },
    AirPlayDeviceRemoved {
        data: AirPlayDeviceRemovedData,
    },
    AirPlaySessionStarted {
        data: crate::audio::airplay::AirPlayDeviceInfo,
    },
    AirPlaySessionStopped {
        data: AirPlayDeviceRemovedData,
    },
    AirPlayError {
        data: ErrorData,
    },
    LineInChanged {
        data: LineInData,
    },
}

#[derive(Serialize)]
struct LineInData {
    active: bool,
}

#[derive(Serialize)]
struct DeviceStateData {
    address: String,
    name: String,
    state: crate::bluetooth::device::DeviceState,
}

#[derive(Serialize)]
struct PlaybackStatusData {
    status: crate::state::PlaybackStatus,
}

#[derive(Serialize)]
struct SpectrumBands {
    bands: Vec<f32>,
}

#[derive(Serialize)]
struct BtStatusData {
    status: crate::state::BluetoothStatus,
}

#[derive(Serialize)]
struct SdpData {
    sdp: String,
}

#[derive(Serialize)]
struct IceCandidateData {
    candidate: String,
    /// Must be "sdpMid" (not "sdp_mid") to match the WebRTC spec.
    #[serde(rename = "sdpMid")]
    sdp_mid: Option<String>,
    /// Must be "sdpMLineIndex" — serde's camelCase produces "sdpMlineIndex"
    /// (lowercase 'l'), which Safari rejects because it requires at least one
    /// of sdpMid or sdpMLineIndex to be non-null on every ICE candidate.
    #[serde(rename = "sdpMLineIndex")]
    sdp_mline_index: Option<u16>,
}

#[derive(Serialize)]
struct CastDeviceRemovedData {
    device_id: String,
}

#[derive(Serialize)]
struct AirPlayDeviceRemovedData {
    device_name: String,
}

#[derive(Serialize)]
struct ErrorData {
    message: String,
}

// -- Incoming message types --

#[derive(Deserialize)]
#[serde(tag = "type")]
enum WsInMessage {
    #[serde(rename = "webrtc_offer")]
    Offer { data: WsOfferData },
    #[serde(rename = "webrtc_ice_candidate")]
    IceCandidate { data: WsIceCandidateData },
    #[serde(rename = "webrtc_stop")]
    Stop {},
}

#[derive(Deserialize)]
struct WsOfferData {
    sdp: String,
}

#[derive(Deserialize)]
struct WsIceCandidateData {
    candidate: String,
    #[serde(rename = "sdpMid")]
    sdp_mid: Option<String>,
    #[serde(rename = "sdpMLineIndex")]
    sdp_mline_index: Option<u16>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::SystemEvent;

    #[test]
    fn test_line_in_activated_produces_ws_message() {
        let msg = event_to_ws_message(&SystemEvent::LineInActivated, "test-session");
        assert!(msg.is_some());
        let json = serde_json::to_string(&msg.unwrap()).unwrap();
        assert!(json.contains("\"type\":\"line_in_changed\""));
        assert!(json.contains("\"active\":true"));
    }

    #[test]
    fn test_line_in_deactivated_produces_ws_message() {
        let msg = event_to_ws_message(&SystemEvent::LineInDeactivated, "test-session");
        assert!(msg.is_some());
        let json = serde_json::to_string(&msg.unwrap()).unwrap();
        assert!(json.contains("\"type\":\"line_in_changed\""));
        assert!(json.contains("\"active\":false"));
    }

    #[test]
    fn test_bluetooth_scanning_status_produces_ws_message() {
        let msg = event_to_ws_message(
            &SystemEvent::BluetoothStatusChanged {
                status: crate::state::BluetoothStatus::Scanning,
            },
            "test-session",
        );
        assert!(msg.is_some());
        let json = serde_json::to_string(&msg.unwrap()).unwrap();
        assert!(json.contains("\"type\":\"bluetooth_status_changed\""));
        assert!(json.contains("\"scanning\""));
    }

    #[test]
    fn test_device_discovered_produces_ws_message() {
        let msg = event_to_ws_message(
            &SystemEvent::DeviceDiscovered {
                address: "AA:BB:CC:DD:EE:FF".to_string(),
                name: "TestPhone".to_string(),
                rssi: Some(-50),
            },
            "test-session",
        );
        assert!(msg.is_some());
        let json = serde_json::to_string(&msg.unwrap()).unwrap();
        assert!(json.contains("\"type\":\"device_state_changed\""));
        assert!(json.contains("TestPhone"));
        assert!(json.contains("discovered"));
    }
}
