//! The web interface's server: the built UI, its JSON API under `/api`,
//! live events (SSE), job output and terminals (WebSocket), and login (one
//! password, a session cookie).

pub mod api;
pub mod auth;
pub mod mcp;

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
    // A notification's button carries its own one-time token.
    if !path.starts_with("/api/") || path == "/api/login" || path == "/api/session" || path == "/api/action" {
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

/// The built web UI (`webui/dist`), in the binary (a debug build reads the folder).
#[derive(rust_embed::Embed)]
#[folder = "../../webui/dist"]
#[allow_missing = true]
struct Ui;

/// A file of the UI; anything else is the app (it routes by the hash).
async fn ui(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let (file, name) = match Ui::get(path).filter(|_| !path.is_empty()) {
        Some(f) => (f, path),
        None => match Ui::get("index.html") {
            Some(f) => (f, "index.html"),
            None => return (StatusCode::NOT_FOUND, "the web UI isn't built (cd webui && npm run build)").into_response(),
        },
    };
    // Parcel's file names carry their hash: kept; the page and the service worker aren't.
    let cache = if name == "index.html" || name.starts_with("sw") { "no-cache" } else { "public, max-age=31536000, immutable" };
    ([(header::CONTENT_TYPE, file.metadata.mimetype().to_string()), (header::CACHE_CONTROL, cache.to_string())], file.data).into_response()
}

pub fn router(w: Arc<Web>) -> Router {
    let s: S = Arc::new(St { w, attempts: Default::default() });
    let mcp = axum::Router::new().nest_service("/mcp", mcp::service(s.w.app.clone())).layer(axum::middleware::from_fn_with_state(s.clone(), mcp::auth));
    api::routes()
        .merge(mcp.with_state(s.clone()))
        .fallback(ui)
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
