//! Notifications: apprise is run with the configured URLs, and Web Push
//! reaches a browser's endpoint (encrypted, VAPID-signed).

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use serde_json::json;

#[tokio::test]
async fn a_finished_task_goes_out_by_apprise_and_web_push() {
    // A stand-in apprise: it writes what it was given.
    let bin = tempfile::tempdir().unwrap();
    let out = bin.path().join("apprise.out");
    std::fs::write(bin.path().join("apprise"), format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\n", out.display())).unwrap();
    std::fs::set_permissions(bin.path().join("apprise"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    unsafe { std::env::set_var("PATH", format!("{}:{}", bin.path().display(), std::env::var("PATH").unwrap_or_default())) };

    // A stand-in push service.
    let got: Arc<Mutex<Vec<(axum::http::HeaderMap, usize)>>> = Default::default();
    let g = got.clone();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/push/abc", l.local_addr().unwrap());
    let app = axum::Router::new().route(
        "/push/abc",
        axum::routing::post(move |h: axum::http::HeaderMap, body: axum::body::Bytes| {
            let g = g.clone();
            async move {
                g.lock().unwrap().push((h, body.len()));
                axum::http::StatusCode::CREATED
            }
        }),
    );
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

    let r = start_in(tempfile::tempdir().unwrap(), "notify {\n  apprise = [\"json://localhost/hook\"]\n  url = \"https://reagent.example\"\n  events = [\"done\"]\n}\n").await;
    // A browser's subscription (the keys from RFC 8291's example).
    let sub = json!({"endpoint": endpoint, "keys": {"p256dh": "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4", "auth": "BTBZMqHH6r4Tts7J_aSIgg"}});
    r.run.app.store.add_push_subscription(&endpoint, &sub).await.unwrap();
    assert_eq!(reagent_tools::notify::vapid_public(&r.run.app.store).await.unwrap().len(), 87, "an uncompressed P-256 key, base64url");

    r.push("Ping", |_| text("all good"));
    let t = r.start_task("Ping", "x").await;
    r.done(&t.id).await;
    for _ in 0..100 {
        if out.exists() && !got.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let args = std::fs::read_to_string(&out).expect("apprise ran");
    assert!(args.contains("-t\nPing — done\n-b\nall good"), "{args}");
    assert!(args.contains(&format!("https://reagent.example/#/task/{}", t.id)) && args.trim_end().ends_with("json://localhost/hook"), "{args}");
    let pushes = got.lock().unwrap().clone();
    let (h, len) = &pushes[0];
    assert_eq!(h["content-encoding"], "aes128gcm");
    assert!(h["authorization"].to_str().unwrap().starts_with("vapid t="), "{h:?}");
    assert!(*len > 86, "an encrypted body");
    // Only the events asked for: a failure isn't sent.
    r.run.app.notify("failed", None, "x", "y").await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(got.lock().unwrap().len(), 1);
}
