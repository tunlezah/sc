mod audio;
mod bluetooth;
mod dsp;
mod state;
mod web;

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tracing::{error, info, warn};

use crate::audio::airplay::{AirPlayCommand, AirPlayManager};
use crate::audio::chromecast::{ChromecastCommand, ChromecastManager};
use crate::audio::line_in::LineInManager;
use crate::audio::pipeline::{AudioPipeline, PipelineCommand};
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
    // Install rustls CryptoProvider before any WebRTC/DTLS code runs.
    // The webrtc crate uses rustls for DTLS, which requires a crypto backend.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls CryptoProvider");

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
    let (cast_cmd_tx, cast_cmd_rx) = mpsc::channel::<ChromecastCommand>(32);
    let (airplay_cmd_tx, airplay_cmd_rx) = mpsc::channel::<AirPlayCommand>(32);
    let (pipeline_cmd_tx, pipeline_cmd_rx) = mpsc::channel::<PipelineCommand>(16);

    // Initialize line-in manager
    let line_in = Arc::new(LineInManager::new(state.clone()));
    line_in.initialize().await;

    // Initialize audio pipeline
    let mut pipeline = AudioPipeline::new(state.clone());
    let eq_bands = {
        let app = state.state.read().await;
        app.eq_bands.clone()
    };

    // Subscribe to audio broadcast BEFORE initializing the pipeline so the
    // receiver is already registered when the capture task starts sending data.
    let audio_rx = pipeline.audio_receiver();

    if let Err(e) = pipeline.initialize(&eq_bands).await {
        error!("Failed to initialize audio pipeline: {}", e);
        info!("Continuing without audio pipeline (audio features will be unavailable)");
    }

    // Get audio senders BEFORE moving pipeline into the command loop task.
    let audio_sender = pipeline.audio_sender();
    let stream_audio_sender = pipeline.audio_sender();

    // Spawn pipeline command loop (handles EQ updates from the web API).
    // The pipeline is moved into this task; shutdown happens when the channel
    // closes (i.e. when the server shuts down).
    tokio::spawn(async move {
        pipeline.run(pipeline_cmd_rx).await;
    });

    // Start spectrum analyzer
    let spectrum = SpectrumAnalyzer::new(state.clone());
    tokio::spawn(async move {
        spectrum.run(audio_rx).await;
    });

    // Start WebRTC manager with audio capture subscription
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

    // Start Chromecast manager
    let cast_state = state.clone();
    tokio::spawn(async move {
        let manager = ChromecastManager::new(cast_state, port);
        manager.run(cast_cmd_rx).await;
    });

    // Start AirPlay manager
    let airplay_state = state.clone();
    tokio::spawn(async move {
        let manager = AirPlayManager::new(airplay_state).await;
        manager.run(airplay_cmd_rx).await;
    });

    // Start Bluetooth manager. A oneshot channel passes the D-Bus connection
    // back after adapter setup completes so we can keep it alive for the agent.
    let (conn_tx, conn_rx) = tokio::sync::oneshot::channel::<zbus::Connection>();
    let bt_state = state.clone();
    let adapter_name = state.state.read().await.config.adapter.clone();
    let bt_manager = BluetoothManager::new(bt_state, bt_cmd_rx, conn_tx);
    tokio::spawn(async move {
        bt_manager.run().await;
    });

    // Keep the D-Bus connection alive (for the BlueZ agent) but do NOT register
    // custom A2DP endpoints — they conflict with WirePlumber's BlueZ plugin.
    // WirePlumber handles codec negotiation, transport acquisition, and creates
    // bluez_input.* PipeWire audio nodes. Custom endpoints steal the transport,
    // causing WirePlumber to log "unknown transport" errors and never create
    // audio nodes.
    tokio::spawn(async move {
        match conn_rx.await {
            Ok(_connection) => {
                // Keep connection alive for agent lifetime
                futures::future::pending::<()>().await;
            }
            Err(_) => {
                warn!("Bluetooth manager shut down before sending D-Bus connection");
            }
        }
    });

    // Start AVRCP monitor
    let avrcp_monitor = AvrcpMonitor::new(state.clone(), avrcp_cmd_rx, adapter_name.clone());
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
        cast_cmd_tx: Some(cast_cmd_tx),
        airplay_cmd_tx: Some(airplay_cmd_tx),
        pipeline_cmd_tx: Some(pipeline_cmd_tx),
        audio_sender: Some(stream_audio_sender),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Determine webui path (check webui/dist/ first, then dist/)
    let webui_path = if std::path::Path::new("webui/dist").exists() {
        "webui/dist"
    } else if std::path::Path::new("dist").exists() {
        "dist"
    } else {
        "webui/dist"
    };

    // Warn if the webui directory is missing or has no index.html
    let index_path = std::path::Path::new(webui_path).join("index.html");
    if !index_path.exists() {
        warn!(
            "Web UI not found at {}/index.html — the page will be blank. \
             Run 'npm run build' in webui/ or re-run install.sh.",
            webui_path
        );
    }

    // Serve static files with SPA fallback: unmatched routes get index.html
    // so that client-side routing works correctly
    let serve_dir = ServeDir::new(webui_path)
        .append_index_html_on_directories(true)
        .fallback(ServeFile::new(index_path));

    let app = create_router(app_router)
        .fallback_service(serve_dir)
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

    // Pipeline shutdown happens automatically when the command channel is
    // dropped (the pipeline task calls shutdown() in its run() method).
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
