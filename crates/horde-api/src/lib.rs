//! Localhost HTTP API: admin collective controls + studio job surface + operator UI.
//!
//! Admin/control plane binds to loopback (or collective-only). Mesh overlay uses a
//! separate bind (`HORDE_MESH_BIND`) and never carries admin traffic.
//! Static operator UI assets live under `/ui` (see repo `ui/`).

mod auth;
mod error;
mod routes;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::response::Redirect;
use axum::routing::get;
use axum::Router;
use horde_collective::CollectiveHead;
use horde_protocol::{BootstrapPeer, DEFAULT_API_BIND};
use serde::Serialize;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

pub use routes::api_router;

/// Redacted runtime config for the operator UI (never includes passphrase).
#[derive(Clone, Debug, Serialize)]
pub struct PublicRuntimeConfig {
    pub bind: String,
    pub data_dir: String,
    pub hostname: String,
    pub mesh_bind: Option<String>,
    pub mesh_enabled: bool,
    pub ui_path: String,
}

#[derive(Clone)]
pub struct AppState {
    pub head: Arc<CollectiveHead>,
    /// Mutable so runtime mesh rebind can refresh redacted config for the UI.
    pub public_config: Arc<RwLock<PublicRuntimeConfig>>,
}

pub struct ApiConfig {
    pub data_dir: PathBuf,
    pub passphrase: String,
    pub hostname: String,
    pub bind: SocketAddr,
    /// When set, enable invite-bounded Noise mesh on this address.
    pub mesh_bind: Option<SocketAddr>,
    /// Publish-only bootstrap peer list (not a job authority).
    pub bootstrap_peers: Vec<BootstrapPeer>,
    /// Directory of static operator UI assets (`index.html`, CSS, JS).
    pub ui_dir: PathBuf,
}

impl ApiConfig {
    /// Load config from environment. `HORDE_PASSPHRASE` is required (no baked-in default).
    pub fn from_env() -> anyhow::Result<Self> {
        let data_dir = std::env::var("HORDE_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./horde-data"));
        let passphrase = std::env::var("HORDE_PASSPHRASE").map_err(|_| {
            anyhow::anyhow!(
                "HORDE_PASSPHRASE is required (set a strong passphrase; no default secret is embedded)"
            )
        })?;
        if passphrase.trim().is_empty() {
            anyhow::bail!("HORDE_PASSPHRASE must not be empty");
        }
        let hostname = std::env::var("HORDE_HOSTNAME").unwrap_or_else(|_| {
            hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "localhost".into())
        });
        let bind: SocketAddr = std::env::var("HORDE_BIND")
            .unwrap_or_else(|_| DEFAULT_API_BIND.into())
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid HORDE_BIND: {e}"))?;
        let mesh_bind = match std::env::var("HORDE_MESH_BIND") {
            Ok(s) => Some(
                s.parse::<SocketAddr>()
                    .map_err(|e| anyhow::anyhow!("invalid HORDE_MESH_BIND: {e}"))?,
            ),
            Err(_) => None,
        };
        let bootstrap_peers = std::env::var("HORDE_BOOTSTRAP_PEERS")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(|addr| BootstrapPeer {
                        addr: addr.to_string(),
                        note: Some("env bootstrap".into()),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let ui_dir = resolve_ui_dir();
        Ok(Self {
            data_dir,
            passphrase,
            hostname,
            bind,
            mesh_bind,
            bootstrap_peers,
            ui_dir,
        })
    }
}

/// Locate static operator UI assets.
///
/// Order: `HORDE_UI_DIR`, then common relative paths from the process CWD / crate.
pub fn resolve_ui_dir() -> PathBuf {
    if let Ok(p) = std::env::var("HORDE_UI_DIR") {
        return PathBuf::from(p);
    }
    let manifest_ui = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ui");
    let candidates = [
        PathBuf::from("ui"),
        PathBuf::from("./ui"),
        PathBuf::from("../ui"),
        PathBuf::from("../../ui"),
        manifest_ui,
    ];
    for c in candidates {
        if c.join("index.html").is_file() {
            return c;
        }
    }
    PathBuf::from("ui")
}

pub async fn build_app(config: ApiConfig) -> anyhow::Result<(Router, AppState, String)> {
    let ui_dir = config.ui_dir.clone();
    let public_config = PublicRuntimeConfig {
        bind: config.bind.to_string(),
        data_dir: config.data_dir.display().to_string(),
        hostname: config.hostname.clone(),
        mesh_bind: config.mesh_bind.map(|a| a.to_string()),
        mesh_enabled: config.mesh_bind.is_some(),
        ui_path: ui_dir.display().to_string(),
    };
    let head = CollectiveHead::bootstrap(config.data_dir, &config.passphrase, config.hostname)?;
    if let Some(mesh_bind) = config.mesh_bind {
        head.enable_mesh(mesh_bind).await?;
        if !config.bootstrap_peers.is_empty() {
            head.set_bootstrap_peers(config.bootstrap_peers).await?;
        }
    }
    let api_key = head.ensure_dev_api_key().await;
    // Prefer the OS-assigned mesh address when env used an ephemeral port.
    let mut public_config = public_config;
    if let Some(actual) = head.mesh_bind_addr().await {
        public_config.mesh_bind = Some(actual.to_string());
        public_config.mesh_enabled = true;
    }
    let state = AppState {
        head,
        public_config: Arc::new(RwLock::new(public_config)),
    };
    let ui_index = ui_dir.join("index.html");
    if !ui_index.is_file() {
        warn!(
            ui_dir = %ui_dir.display(),
            "operator UI assets not found; /ui will 404 until ui/ is present or HORDE_UI_DIR is set"
        );
    } else {
        info!(ui_dir = %ui_dir.display(), "serving operator UI at /ui");
    }
    let static_ui = ServeDir::new(&ui_dir).append_index_html_on_directories(true);
    let app = api_router(state.clone())
        .route("/", get(|| async { Redirect::temporary("/ui/") }))
        .nest_service("/ui", static_ui)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());
    Ok((app, state, api_key))
}

pub async fn serve(config: ApiConfig) -> anyhow::Result<()> {
    let bind = config.bind;
    let mesh_bind = config.mesh_bind;
    let (app, state, api_key) = build_app(config).await?;
    let mode = if mesh_bind.is_some() {
        "network"
    } else {
        "local"
    };
    info!(
        node_id = %state.head.node_id(),
        %bind,
        ?mesh_bind,
        mode,
        "horde API listening (admin localhost; mesh optional)"
    );
    info!(%api_key, "studio API key (Authorization: Bearer <key>)");
    info!(ui = %format!("http://{bind}/ui/"), "operator UI");
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// Optional hostname crate — avoid extra dep by inlining fallback above.
mod hostname {
    pub fn get() -> std::io::Result<std::ffi::OsString> {
        Ok(std::env::var_os("HOSTNAME")
            .or_else(|| std::env::var_os("COMPUTERNAME"))
            .unwrap_or_else(|| std::ffi::OsString::from("localhost")))
    }
}
