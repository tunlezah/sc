use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::state::{AppStateHandle, SystemEvent};
use crate::web::routes::AppRouter;

/// WebSocket handler for the `/ws/status` endpoint.
pub async fn ws_handler(ws: WebSocketUpgrade, State(app): State<AppRouter>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, app.state))
}

/// Handle an individual WebSocket connection.
async fn handle_socket(socket: WebSocket, state: AppStateHandle) {
    let (mut sender, mut receiver) = socket.split();
    let session_id = uuid::Uuid::new_v4().to_string();
    info!("WebSocket connected: {}", session_id);

    // Send initial state snapshot
    {
        let app = state.state.read().await;
        let snapshot = app.snapshot();
        let msg = WsOutMessage::StateSnapshot { data: snapshot };
        if let Ok(json) = serde_json::to_string(&msg) {
            let _ = sender.send(Message::Text(json)).await;
        }
    }

    // Subscribe to events
    let mut event_rx = state.subscribe();

    // Spawn task to forward events to the WebSocket
    let send_session_id = session_id.clone();
    let send_task = tokio::spawn(async move {
        loop {
            match event_rx.recv().await {
                Ok(event) => {
                    if let Some(msg) = event_to_ws_message(&event) {
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
    let recv_state = state.clone();
    let recv_session_id = session_id.clone();
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    handle_client_message(&recv_state, &recv_session_id, &text).await;
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

    info!("WebSocket disconnected: {}", session_id);
}

/// Handle incoming WebSocket messages from the browser (e.g., WebRTC signaling).
async fn handle_client_message(_state: &AppStateHandle, session_id: &str, text: &str) {
    if let Ok(msg) = serde_json::from_str::<WsInMessage>(text) {
        match msg {
            WsInMessage::WebrtcOffer { data: _ } => {
                info!("WebRTC offer received from {}", session_id);
                // WebRTC offer handling would be wired here
            }
            WsInMessage::WebrtcIceCandidate { data: _ } => {
                // ICE candidate handling
            }
            WsInMessage::WebrtcStart { .. } => {
                info!("WebRTC start requested by {}", session_id);
            }
            WsInMessage::WebrtcStop { .. } => {
                info!("WebRTC stop requested by {}", session_id);
            }
        }
    }
}

/// Convert a SystemEvent to a WebSocket message (if applicable).
fn event_to_ws_message(event: &SystemEvent) -> Option<WsOutMessage> {
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
        _ => None,
    }
}

// -- WebSocket message types --

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WsOutMessage {
    StateSnapshot {
        data: crate::state::AppStateSnapshot,
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
    #[allow(dead_code)]
    WebrtcAnswer {
        data: SdpData,
    },
    #[allow(dead_code)]
    WebrtcIceCandidate {
        data: IceCandidateData,
    },
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
    sdp_mid: Option<String>,
    sdp_mline_index: Option<u16>,
}

// -- Incoming message types --

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::enum_variant_names)]
enum WsInMessage {
    WebrtcOffer {
        #[allow(dead_code)]
        data: WsOfferData,
    },
    WebrtcIceCandidate {
        #[allow(dead_code)]
        data: WsIceCandidateData,
    },
    WebrtcStart {
        #[allow(dead_code)]
        data: serde_json::Value,
    },
    WebrtcStop {
        #[allow(dead_code)]
        data: serde_json::Value,
    },
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct WsOfferData {
    sdp: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct WsIceCandidateData {
    candidate: String,
    #[serde(rename = "sdpMid")]
    sdp_mid: Option<String>,
    #[serde(rename = "sdpMLineIndex")]
    sdp_mline_index: Option<u16>,
}
