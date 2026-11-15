//! Localhost HTTP API: admin collective controls + studio job surface.
//!
//! Admin/control plane binds to loopback (or collective-only). Mesh overlay uses a
//! separate bind (`HORDE_MESH_BIND`) and never carries admin traffic.

mod auth;
mod error;
mod routes;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use horde_collective::CollectiveHead;
use horde_protocol::{BootstrapPeer, DEFAULT_API_BIND, DEFAULT_MESH_BIND};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

pub use routes::api_router;

#[derive(Clone)]
pub struct AppState {
    pub head: Arc<CollectiveHead>,
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
}

impl Default for ApiConfig {
    fn default() -> Self {
        let data_dir = std::env::var("HORDE_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./horde-data"));
        let passphrase =
            std::env::var("HORDE_PASSPHRASE").unwrap_or_else(|_| "horde-dev-passphrase".into());
        let hostname = std::env::var("HORDE_HOSTNAME").unwrap_or_else(|_| {
            hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "localhost".into())
        });
        let bind: SocketAddr = std::env::var("HORDE_BIND")
            .unwrap_or_else(|_| DEFAULT_API_BIND.into())
            .parse()
            .expect("invalid HORDE_BIND");
        let mesh_bind = std::env::var("HORDE_MESH_BIND").ok().map(|s| {
            s.parse::<SocketAddr>()
                .unwrap_or_else(|_| DEFAULT_MESH_BIND.parse().expect("default mesh bind"))
        });
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
        Self {
            data_dir,
            passphrase,
            hostname,
            bind,
            mesh_bind,
            bootstrap_peers,
        }
    }
}

pub async fn build_app(config: ApiConfig) -> anyhow::Result<(Router, AppState, String)> {
    let head = CollectiveHead::bootstrap(config.data_dir, &config.passphrase, config.hostname)?;
    if let Some(mesh_bind) = config.mesh_bind {
        head.enable_mesh(mesh_bind).await?;
        if !config.bootstrap_peers.is_empty() {
            head.set_bootstrap_peers(config.bootstrap_peers).await?;
        }
    }
    let api_key = head.ensure_dev_api_key().await;
    let state = AppState { head };
    let app = api_router(state.clone())
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
