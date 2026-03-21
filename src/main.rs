mod audio;
mod bluetooth;
mod dsp;
mod state;
mod web;

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tracing::{error, info, warn};

use crate::audio::line_in::LineInManager;
use crate::audio::pipeline::AudioPipeline;
use crate::audio::spectrum::SpectrumAnalyzer;
use crate::audio::webrtc_audio::{WebRtcCommand, WebRtcManager};
use crate::bluetooth::avrcp::{AvrcpCommand, AvrcpMonitor};
use crate::bluetooth::manager::{BluetoothCommand, BluetoothManager};
use crate::state::config::Config;
use crate::state::AppStateHandle;
use crate::web::routes::{create_router, AppRouter};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("soundsync=info".parse().unwrap()),
        )
        .init();

    info!("SoundSync v{} starting...", VERSION);

    // Load configuration
    let config = Config::load();
    let port = config.port;

    // Set XDG_RUNTIME_DIR if unset
    ensure_xdg_runtime_dir();

    // Create shared state
    let state = AppStateHandle::new(config);

    // Create command channels
    let (bt_cmd_tx, bt_cmd_rx) = mpsc::channel::<BluetoothCommand>(32);
    let (avrcp_cmd_tx, avrcp_cmd_rx) = mpsc::channel::<AvrcpCommand>(32);
    let (webrtc_cmd_tx, webrtc_cmd_rx) = mpsc::channel::<WebRtcCommand>(32);

    // Initialize line-in manager
    let line_in = Arc::new(LineInManager::new(state.clone()));
    line_in.initialize().await;

    // Initialize audio pipeline
    let mut pipeline = AudioPipeline::new(state.clone());
    let eq_bands = {
        let app = state.state.read().await;
        app.eq_bands.clone()
    };

    if let Err(e) = pipeline.initialize(&eq_bands).await {
        error!("Failed to initialize audio pipeline: {}", e);
        info!("Continuing without audio pipeline (audio features will be unavailable)");
    }

    // Start spectrum analyzer
    let audio_rx = pipeline.audio_receiver();
    let spectrum = SpectrumAnalyzer::new(state.clone());
    tokio::spawn(async move {
        spectrum.run(audio_rx).await;
    });

    // Start WebRTC manager with audio capture subscription
    let audio_sender = pipeline.audio_sender();
    let webrtc_state = state.clone();
    tokio::spawn(async move {
        match WebRtcManager::new(audio_sender, webrtc_state) {
            Ok(manager) => {
                manager.run(webrtc_cmd_rx).await;
            }
            Err(e) => {
                error!("Failed to initialize WebRTC manager: {}", e);
            }
        }
    });

    // Start Bluetooth manager
    let bt_state = state.clone();
    let bt_manager = BluetoothManager::new(bt_state, bt_cmd_rx);
    tokio::spawn(async move {
        bt_manager.run().await;
    });

    // Register A2DP endpoints for codec negotiation
    let endpoint_state = state.clone();
    tokio::spawn(async move {
        match zbus::Connection::system().await {
            Ok(connection) => {
                if let Err(e) =
                    bluetooth::endpoint::register_endpoints(&connection, endpoint_state).await
                {
                    warn!("Failed to register A2DP endpoints: {}", e);
                    info!("A2DP codec negotiation will not be available");
                }
            }
            Err(e) => {
                warn!("Failed to connect to D-Bus for A2DP endpoints: {}", e);
            }
        }
    });

    // Start AVRCP monitor
    let avrcp_monitor = AvrcpMonitor::new(state.clone(), avrcp_cmd_rx);
    tokio::spawn(async move {
        avrcp_monitor.run().await;
    });

    // Create web router
    let app_router = AppRouter {
        state: state.clone(),
        bt_cmd_tx,
        avrcp_cmd_tx,
        line_in,
        webrtc_cmd_tx: Some(webrtc_cmd_tx),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Determine webui path (check dist/ first, then webui/dist/)
    let webui_path = if std::path::Path::new("webui/dist").exists() {
        "webui/dist"
    } else if std::path::Path::new("dist").exists() {
        "dist"
    } else {
        "webui/dist"
    };

    let app = create_router(app_router)
        .nest_service(
            "/",
            ServeDir::new(webui_path).append_index_html_on_directories(true),
        )
        .layer(cors)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("Web server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();

    // Graceful shutdown
    let shutdown_state = state.clone();
    let shutdown = async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Shutting down...");
        shutdown_state.publish(crate::state::SystemEvent::ServiceStopping);
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .unwrap();

    // Cleanup
    pipeline.shutdown().await;
    info!("SoundSync stopped");
}

/// Ensure XDG_RUNTIME_DIR is set (required by PipeWire).
fn ensure_xdg_runtime_dir() {
    if std::env::var("XDG_RUNTIME_DIR").is_err() {
        let uid = unsafe { libc::getuid() };
        let dir = format!("/run/user/{}", uid);
        if std::path::Path::new(&dir).exists() {
            std::env::set_var("XDG_RUNTIME_DIR", &dir);
            info!("Set XDG_RUNTIME_DIR={}", dir);
        } else {
            tracing::warn!(
                "XDG_RUNTIME_DIR not set and /run/user/{} doesn't exist",
                uid
            );
        }
    }
}
