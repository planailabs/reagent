//! The web interface's server: the built UI, its JSON API under `/api`,
//! live events (SSE), job output and terminals (WebSocket), and login (one
//! password, a session cookie).

pub mod api;
pub mod auth;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use reagent_tools::app::App;

/// What the web server needs.
pub struct Web {
    pub app: Arc<App>,
    /// The hub's own API, for reference (its admin token stays here).
    pub hub_url: String,
    pub hub_token: String,
    /// The built UI (`webui/dist`).
    pub dist: std::path::PathBuf,
}

pub struct St {
    pub w: Arc<Web>,
    /// Failed logins per address: (count, first in this minute).
    pub attempts: Mutex<HashMap<std::net::IpAddr, (u32, std::time::Instant)>>,
}

pub type S = Arc<St>;

pub const COOKIE: &str = "reagent_session";

/// The session token from the cookie.
pub fn session_of(headers: &axum::http::HeaderMap) -> Option<String> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|c| c.trim().strip_prefix(&format!("{COOKIE}=")).map(String::from))
}

/// Logged in for everything under /api but login; writes must come from this site.
async fn guard(State(s): State<S>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if !path.starts_with("/api/") || path == "/api/login" || path == "/api/session" {
        return next.run(req).await;
    }
    if req.method() != Method::GET && req.method() != Method::HEAD {
        let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or_default();
        if let Some(origin) = req.headers().get(header::ORIGIN).and_then(|o| o.to_str().ok())
            && origin.split("://").nth(1) != Some(host)
        {
            return (StatusCode::FORBIDDEN, "a request from another site").into_response();
        }
    }
    let ok = match session_of(req.headers()) {
        Some(tok) => s.w.app.store.session_valid(&auth::token_hash(&tok)).await.unwrap_or(false),
        None => false,
    };
    if !ok {
        return (StatusCode::UNAUTHORIZED, "log in first").into_response();
    }
    next.run(req).await
}

pub fn router(w: Arc<Web>) -> Router {
    let dist = w.dist.clone();
    let s: S = Arc::new(St { w, attempts: Default::default() });
    let index = dist.join("index.html");
    let files = tower_http::services::ServeDir::new(&dist).fallback(tower_http::services::ServeFile::new(index));
    api::routes()
        .fallback_service(files)
        .layer(axum::middleware::from_fn_with_state(s.clone(), guard))
        .with_state(s)
}

/// The caller's address (behind a reverse proxy: the proxy's, unless it
/// says `X-Forwarded-For` and is local).
pub fn client_ip(ci: &ConnectInfo<SocketAddr>, headers: &axum::http::HeaderMap) -> std::net::IpAddr {
    let peer = ci.0.ip();
    if peer.is_loopback()
        && let Some(f) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.split(',').next()).and_then(|v| v.trim().parse().ok())
    {
        return f;
    }
    peer
}
