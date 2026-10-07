//! The web interface's server: the UI, its API, live events, login.

pub mod auth;

use std::sync::Arc;

use reagent_tools::app::App;

/// What the web server needs.
pub struct Web {
    pub app: Arc<App>,
    /// The hub's own API (proxied for the agent panel).
    pub hub_url: String,
    pub hub_token: String,
    /// The built UI (`webui/dist`).
    pub dist: std::path::PathBuf,
}

pub fn router(w: Arc<Web>) -> axum::Router {
    axum::Router::new().route("/api/health", axum::routing::get(|| async { "ok" })).with_state(w)
}
