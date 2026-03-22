use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{delete, get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

use crate::audio::airplay::{AirPlayCommand, AirPlayDeviceInfo};
use crate::audio::chromecast::{CastDeviceInfo, ChromecastCommand};
use crate::audio::line_in::LineInManager;
use crate::audio::webrtc_audio::WebRtcCommand;
use crate::bluetooth::avrcp::AvrcpCommand;
use crate::bluetooth::manager::BluetoothCommand;
use crate::dsp::presets;
use crate::state::AppStateHandle;
use crate::web::ws;

#[derive(Clone)]
pub struct AppRouter {
    pub state: AppStateHandle,
    pub bt_cmd_tx: mpsc::Sender<BluetoothCommand>,
    pub avrcp_cmd_tx: mpsc::Sender<AvrcpCommand>,
    pub line_in: Arc<LineInManager>,
    pub webrtc_cmd_tx: Option<mpsc::Sender<WebRtcCommand>>,
    pub cast_cmd_tx: Option<mpsc::Sender<ChromecastCommand>>,
    pub airplay_cmd_tx: Option<mpsc::Sender<AirPlayCommand>>,
    pub audio_sender: Option<broadcast::Sender<Vec<f32>>>,
}

#[derive(Serialize)]
struct OkResponse {
    ok: bool,
}

#[derive(Serialize)]
struct StatusResponse {
    status: String,
    device_count: usize,
    uptime_secs: u64,
}

#[derive(Deserialize)]
struct ScanRequest {
    scanning: bool,
}

#[derive(Deserialize)]
struct AddressRequest {
    address: String,
}

#[derive(Deserialize)]
struct NameRequest {
    name: String,
}

#[derive(Deserialize)]
struct EqRequest {
    bands: Option<Vec<EqBandUpdate>>,
    enabled: Option<bool>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct EqBandUpdate {
    freq: Option<f64>,
    gain_db: f32,
}

#[derive(Deserialize)]
struct PresetRequest {
    name: String,
}

#[derive(Deserialize)]
struct CastConnectRequest {
    device_id: String,
}

#[derive(Deserialize)]
struct AirPlayConnectRequest {
    name: String,
}

#[derive(Deserialize)]
struct VolumeRequest {
    level: f32,
}

fn ok_response() -> Json<OkResponse> {
    Json(OkResponse { ok: true })
}

pub fn create_router(app: AppRouter) -> Router {
    Router::new()
        // System
        .route("/api/status", get(get_status))
        .route("/api/devices", get(get_devices))
        // Bluetooth
        .route("/api/bluetooth/scan", post(post_scan))
        .route("/api/bluetooth/connect", post(post_connect))
        .route("/api/bluetooth/disconnect", post(post_disconnect))
        .route("/api/bluetooth/device", delete(delete_device))
        .route("/api/bluetooth/name", post(post_name))
        // EQ
        .route("/api/eq", get(get_eq))
        .route("/api/eq", post(post_eq))
        .route("/api/eq/presets", get(get_presets))
        .route("/api/eq/preset", post(post_apply_preset))
        .route("/api/eq/preset/save", post(post_save_preset))
        .route("/api/eq/preset/{name}", delete(delete_preset))
        // Line-in
        .route("/api/line-in/status", get(get_line_in_status))
        .route("/api/line-in/activate", post(post_line_in_activate))
        .route("/api/line-in/deactivate", post(post_line_in_deactivate))
        // AVRCP
        .route("/api/avrcp/play", post(post_avrcp_play))
        .route("/api/avrcp/pause", post(post_avrcp_pause))
        .route("/api/avrcp/next", post(post_avrcp_next))
        .route("/api/avrcp/previous", post(post_avrcp_previous))
        // Chromecast
        .route("/api/cast/devices", get(get_cast_devices))
        .route("/api/cast/discover", post(post_cast_discover))
        .route("/api/cast/connect", post(post_cast_connect))
        .route("/api/cast/disconnect", post(post_cast_disconnect))
        .route("/api/cast/volume", post(post_cast_volume))
        // AirPlay
        .route("/api/airplay/devices", get(get_airplay_devices))
        .route("/api/airplay/discover", post(post_airplay_discover))
        .route("/api/airplay/connect", post(post_airplay_connect))
        .route("/api/airplay/disconnect", post(post_airplay_disconnect))
        .route("/api/airplay/volume", post(post_airplay_volume))
        // HTTP Audio Stream
        .route("/api/stream/audio.mp3", get(get_audio_stream))
        // WebSocket
        .route("/ws/status", get(ws::ws_handler))
        .with_state(app)
}

// -- System endpoints --

async fn get_status(State(app): State<AppRouter>) -> Json<StatusResponse> {
    let state = app.state.state.read().await;
    Json(StatusResponse {
        status: serde_json::to_value(&state.bluetooth_status)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "unavailable".to_string()),
        device_count: state.devices.len(),
        uptime_secs: state.started_at.elapsed().as_secs(),
    })
}

async fn get_devices(
    State(app): State<AppRouter>,
) -> Json<Vec<crate::bluetooth::device::DeviceInfo>> {
    let state = app.state.state.read().await;
    Json(state.devices.values().cloned().collect())
}

// -- Bluetooth endpoints --

async fn post_scan(
    State(app): State<AppRouter>,
    Json(body): Json<ScanRequest>,
) -> Json<OkResponse> {
    let cmd = if body.scanning {
        BluetoothCommand::StartScan
    } else {
        BluetoothCommand::StopScan
    };
    let _ = app.bt_cmd_tx.send(cmd).await;
    ok_response()
}

async fn post_connect(
    State(app): State<AppRouter>,
    Json(body): Json<AddressRequest>,
) -> Json<OkResponse> {
    let _ = app
        .bt_cmd_tx
        .send(BluetoothCommand::Connect {
            address: body.address,
        })
        .await;
    ok_response()
}

async fn post_disconnect(
    State(app): State<AppRouter>,
    Json(body): Json<AddressRequest>,
) -> Json<OkResponse> {
    let _ = app
        .bt_cmd_tx
        .send(BluetoothCommand::Disconnect {
            address: body.address,
        })
        .await;
    ok_response()
}

async fn delete_device(
    State(app): State<AppRouter>,
    Json(body): Json<AddressRequest>,
) -> Json<OkResponse> {
    let _ = app
        .bt_cmd_tx
        .send(BluetoothCommand::Remove {
            address: body.address,
        })
        .await;
    ok_response()
}

async fn post_name(
    State(app): State<AppRouter>,
    Json(body): Json<NameRequest>,
) -> Json<OkResponse> {
    let _ = app
        .bt_cmd_tx
        .send(BluetoothCommand::SetName { name: body.name })
        .await;
    ok_response()
}

// -- EQ endpoints --

async fn get_eq(State(app): State<AppRouter>) -> Json<crate::state::EqSnapshot> {
    let state = app.state.state.read().await;
    Json(crate::state::EqSnapshot {
        bands: state.eq_bands.clone(),
        enabled: state.eq_enabled,
    })
}

async fn post_eq(State(app): State<AppRouter>, Json(body): Json<EqRequest>) -> Json<OkResponse> {
    let mut state = app.state.state.write().await;

    if let Some(band_updates) = body.bands {
        for (i, update) in band_updates.iter().enumerate() {
            if i < state.eq_bands.len() {
                state.eq_bands[i].gain_db = update.gain_db.clamp(-12.0, 12.0);
            }
        }
    }

    if let Some(enabled) = body.enabled {
        state.eq_enabled = enabled;
    }

    let bands = state.eq_bands.clone();
    let enabled = state.eq_enabled;
    drop(state);

    app.state
        .publish(crate::state::SystemEvent::EqChanged { bands, enabled });

    ok_response()
}

async fn get_presets() -> Json<Vec<String>> {
    Json(presets::all_preset_names())
}

async fn post_apply_preset(
    State(app): State<AppRouter>,
    Json(body): Json<PresetRequest>,
) -> Result<Json<OkResponse>, StatusCode> {
    let preset = presets::get_preset(&body.name).ok_or(StatusCode::NOT_FOUND)?;
    let bands = preset.apply();

    let mut state = app.state.state.write().await;
    state.eq_bands = bands.clone();
    let enabled = state.eq_enabled;
    drop(state);

    app.state
        .publish(crate::state::SystemEvent::EqChanged { bands, enabled });

    Ok(ok_response())
}

async fn post_save_preset(
    State(app): State<AppRouter>,
    Json(body): Json<PresetRequest>,
) -> Result<Json<OkResponse>, StatusCode> {
    let state = app.state.state.read().await;
    let bands = state.eq_bands.clone();
    drop(state);

    presets::save_preset(&body.name, &bands).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(ok_response())
}

async fn delete_preset(Path(name): Path<String>) -> Result<Json<OkResponse>, StatusCode> {
    presets::delete_preset(&name).map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(ok_response())
}

// -- Line-in endpoints --

async fn get_line_in_status(
    State(app): State<AppRouter>,
) -> Json<crate::audio::line_in::LineInStatus> {
    Json(app.line_in.status().await)
}

async fn post_line_in_activate(
    State(app): State<AppRouter>,
) -> Result<Json<OkResponse>, StatusCode> {
    app.line_in
        .activate()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(ok_response())
}

async fn post_line_in_deactivate(
    State(app): State<AppRouter>,
) -> Result<Json<OkResponse>, StatusCode> {
    app.line_in
        .deactivate()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(ok_response())
}

// -- AVRCP endpoints --

async fn post_avrcp_play(State(app): State<AppRouter>) -> Json<OkResponse> {
    let _ = app.avrcp_cmd_tx.send(AvrcpCommand::Play).await;
    ok_response()
}

async fn post_avrcp_pause(State(app): State<AppRouter>) -> Json<OkResponse> {
    let _ = app.avrcp_cmd_tx.send(AvrcpCommand::Pause).await;
    ok_response()
}

async fn post_avrcp_next(State(app): State<AppRouter>) -> Json<OkResponse> {
    let _ = app.avrcp_cmd_tx.send(AvrcpCommand::Next).await;
    ok_response()
}

async fn post_avrcp_previous(State(app): State<AppRouter>) -> Json<OkResponse> {
    let _ = app.avrcp_cmd_tx.send(AvrcpCommand::Previous).await;
    ok_response()
}

// -- Chromecast endpoints --

async fn get_cast_devices(State(app): State<AppRouter>) -> Json<Vec<CastDeviceInfo>> {
    let state = app.state.state.read().await;
    Json(state.cast_devices.values().cloned().collect())
}

async fn post_cast_discover(State(app): State<AppRouter>) -> Json<OkResponse> {
    if let Some(ref tx) = app.cast_cmd_tx {
        let _ = tx.send(ChromecastCommand::Discover).await;
    }
    ok_response()
}

async fn post_cast_connect(
    State(app): State<AppRouter>,
    Json(body): Json<CastConnectRequest>,
) -> Json<OkResponse> {
    if let Some(ref tx) = app.cast_cmd_tx {
        let _ = tx
            .send(ChromecastCommand::Connect {
                device_id: body.device_id,
            })
            .await;
    }
    ok_response()
}

async fn post_cast_disconnect(State(app): State<AppRouter>) -> Json<OkResponse> {
    if let Some(ref tx) = app.cast_cmd_tx {
        let _ = tx.send(ChromecastCommand::Disconnect).await;
    }
    ok_response()
}

async fn post_cast_volume(
    State(app): State<AppRouter>,
    Json(body): Json<VolumeRequest>,
) -> Json<OkResponse> {
    if let Some(ref tx) = app.cast_cmd_tx {
        let _ = tx
            .send(ChromecastCommand::SetVolume {
                level: body.level,
            })
            .await;
    }
    ok_response()
}

// -- AirPlay endpoints --

async fn get_airplay_devices(State(app): State<AppRouter>) -> Json<Vec<AirPlayDeviceInfo>> {
    let state = app.state.state.read().await;
    Json(state.airplay_devices.values().cloned().collect())
}

async fn post_airplay_discover(State(app): State<AppRouter>) -> Json<OkResponse> {
    if let Some(ref tx) = app.airplay_cmd_tx {
        let _ = tx.send(AirPlayCommand::Discover).await;
    }
    ok_response()
}

async fn post_airplay_connect(
    State(app): State<AppRouter>,
    Json(body): Json<AirPlayConnectRequest>,
) -> Json<OkResponse> {
    if let Some(ref tx) = app.airplay_cmd_tx {
        let _ = tx
            .send(AirPlayCommand::Connect { name: body.name })
            .await;
    }
    ok_response()
}

async fn post_airplay_disconnect(State(app): State<AppRouter>) -> Json<OkResponse> {
    if let Some(ref tx) = app.airplay_cmd_tx {
        let _ = tx.send(AirPlayCommand::Disconnect).await;
    }
    ok_response()
}

async fn post_airplay_volume(
    State(app): State<AppRouter>,
    Json(body): Json<VolumeRequest>,
) -> Json<OkResponse> {
    if let Some(ref tx) = app.airplay_cmd_tx {
        let _ = tx
            .send(AirPlayCommand::SetVolume { level: body.level })
            .await;
    }
    ok_response()
}

// -- HTTP Audio Stream endpoint --

async fn get_audio_stream(
    State(app): State<AppRouter>,
) -> Result<axum::response::Response, StatusCode> {
    let audio_sender = app
        .audio_sender
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .clone();

    crate::audio::cast_stream::stream_audio_mp3(audio_sender).await
}
