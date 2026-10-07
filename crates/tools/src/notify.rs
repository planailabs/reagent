//! Telling the person: Web Push to the browsers that subscribed (VAPID keys
//! made once, kept in the settings) and the apprise CLI with the URLs from
//! reagent.hcl. apprise is Python, CLI only: it's run, not linked.

use base64::Engine;
use serde_json::json;
use web_push_native::jwt_simple::algorithms::ES256KeyPair;
use web_push_native::{Auth, WebPushBuilder};

use crate::app::App;
use crate::config::Notify;

pub struct Notifier {
    cfg: Notify,
    http: reqwest::Client,
}

fn b64url() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
}

/// The VAPID key pair, made the first time.
pub async fn vapid(store: &reagent_store::Store) -> Result<ES256KeyPair, String> {
    if let Some(k) = store.setting("vapid_key").await.map_err(|e| e.to_string())? {
        return ES256KeyPair::from_bytes(&b64url().decode(k).map_err(|e| e.to_string())?).map_err(|e| e.to_string());
    }
    let kp = ES256KeyPair::generate();
    store.set_setting("vapid_key", &b64url().encode(kp.to_bytes())).await.map_err(|e| e.to_string())?;
    Ok(kp)
}

/// The public key browsers subscribe with (`applicationServerKey`).
pub async fn vapid_public(store: &reagent_store::Store) -> Result<String, String> {
    use web_push_native::p256::elliptic_curve::sec1::ToEncodedPoint;
    // Browsers want the uncompressed point; jwt-simple gives the compressed one.
    let compressed = vapid(store).await?.public_key().to_bytes();
    let pk = web_push_native::p256::PublicKey::from_sec1_bytes(&compressed).map_err(|e| e.to_string())?;
    Ok(b64url().encode(pk.to_encoded_point(false).as_bytes()))
}

impl Notifier {
    pub fn new(cfg: Notify) -> Self {
        Notifier { cfg, http: reqwest::Client::new() }
    }

    pub async fn send(&self, app: &App, kind: &str, task: Option<&str>, title: &str, body: &str) {
        if !self.cfg.wants(kind) {
            return;
        }
        let link = match (&self.cfg.url, task) {
            (Some(u), Some(t)) => Some(format!("{}/#/task/{t}", u.trim_end_matches('/'))),
            (Some(u), None) => Some(u.clone()),
            _ => None,
        };
        let urls = self.cfg.apprise_urls();
        if !urls.is_empty() {
            let text = match &link {
                Some(l) => format!("{body}\n\n{l}"),
                None => body.to_string(),
            };
            let mut cmd = tokio::process::Command::new("apprise");
            cmd.arg("-t").arg(title).arg("-b").arg(text).args(&urls).stdin(std::process::Stdio::null());
            tokio::spawn(async move {
                match cmd.output().await {
                    Ok(o) if o.status.success() => {}
                    Ok(o) => tracing::warn!(error = %String::from_utf8_lossy(&o.stderr).trim(), "apprise failed"),
                    Err(e) => tracing::warn!(error = %e, "apprise can't run (is it installed?)"),
                }
            });
        }
        let subs = app.store.push_subscriptions().await.unwrap_or_default();
        if subs.is_empty() {
            return;
        }
        let Ok(kp) = vapid(&app.store).await else { return };
        let payload = json!({"title": title, "body": body, "task": task, "kind": kind}).to_string();
        for s in subs {
            if let Err(e) = self.push(&kp, &s, &payload).await {
                tracing::warn!(endpoint = %s["endpoint"], error = %e, "web push failed");
                // Gone (the browser unsubscribed): forget it.
                if e.contains("410") || e.contains("404") {
                    let _ = app.store.remove_push_subscription(s["endpoint"].as_str().unwrap_or_default()).await;
                }
            }
        }
    }

    async fn push(&self, kp: &ES256KeyPair, sub: &serde_json::Value, payload: &str) -> Result<(), String> {
        let endpoint = sub["endpoint"].as_str().ok_or("no endpoint")?;
        let p256dh = b64url().decode(sub["keys"]["p256dh"].as_str().ok_or("no p256dh")?.trim_end_matches('=')).map_err(|e| e.to_string())?;
        let auth = b64url().decode(sub["keys"]["auth"].as_str().ok_or("no auth")?.trim_end_matches('=')).map_err(|e| e.to_string())?;
        if auth.len() != 16 {
            return Err("auth isn't 16 bytes".into());
        }
        let builder = WebPushBuilder::new(
            endpoint.parse().map_err(|e| format!("{e}"))?,
            web_push_native::p256::PublicKey::from_sec1_bytes(&p256dh).map_err(|e| e.to_string())?,
            #[allow(deprecated)]
            Auth::clone_from_slice(&auth),
        )
        .with_vapid(kp, "mailto:reagent@localhost");
        let req = builder.build(payload.as_bytes().to_vec()).map_err(|e| e.to_string())?;
        let mut r = self.http.post(endpoint).body(req.body().clone()).timeout(std::time::Duration::from_secs(20));
        for (k, v) in req.headers() {
            r = r.header(k.as_str(), v.as_bytes());
        }
        let resp = r.send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("{}", resp.status()));
        }
        Ok(())
    }
}
