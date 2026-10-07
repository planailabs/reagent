//! The web API: login, the guard, and driving tasks the way the UI does.

mod common;

use base64::Engine;
use common::*;
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};

async fn logged_in(r: &R) -> (reqwest::Client, String) {
    r.run.app.store.set_setting("password", &reagent_web::auth::hash("secret-pass").unwrap()).await.unwrap();
    let c = reqwest::Client::builder().cookie_store(true).build().unwrap();
    let base = r.run.web_url.clone();
    let res = c.post(format!("{base}/api/login")).json(&json!({"password": "secret-pass"})).send().await.unwrap();
    assert_eq!(res.status(), 200);
    (c, base)
}

async fn get(c: &reqwest::Client, url: String) -> Value {
    let r = c.get(&url).send().await.unwrap();
    assert!(r.status().is_success(), "{url}: {}", r.status());
    r.json().await.unwrap()
}

async fn post(c: &reqwest::Client, url: String, body: Value) -> Value {
    let r = c.post(&url).json(&body).send().await.unwrap();
    let status = r.status();
    let v: Value = r.json().await.unwrap_or_default();
    assert!(status.is_success(), "{url}: {status} {v}");
    v
}

#[tokio::test]
async fn login_guards_the_api() {
    let r = start().await;
    let base = r.run.web_url.clone();
    let anon = reqwest::Client::new();
    assert_eq!(anon.get(format!("{base}/api/projects")).send().await.unwrap().status(), 401);
    let s: Value = anon.get(format!("{base}/api/session")).send().await.unwrap().json().await.unwrap();
    assert_eq!(s, json!({"logged_in": false, "password_set": false}));
    assert_eq!(anon.post(format!("{base}/api/login")).json(&json!({"password": "x"})).send().await.unwrap().status(), 409, "no password yet");
    r.run.app.store.set_setting("password", &reagent_web::auth::hash("secret-pass").unwrap()).await.unwrap();
    for _ in 0..5 {
        assert_eq!(anon.post(format!("{base}/api/login")).json(&json!({"password": "wrong"})).send().await.unwrap().status(), 401);
    }
    assert_eq!(anon.post(format!("{base}/api/login")).json(&json!({"password": "secret-pass"})).send().await.unwrap().status(), 429, "slowed down after five tries");
    let (c, base) = {
        r.run.app.store.set_setting("password", &reagent_web::auth::hash("secret-pass").unwrap()).await.unwrap();
        let c = reqwest::Client::builder().cookie_store(true).local_address(std::net::IpAddr::from([127, 0, 0, 2])).build().unwrap();
        let res = c.post(format!("{base}/api/login")).json(&json!({"password": "secret-pass"})).send().await.unwrap();
        assert_eq!(res.status(), 200);
        assert!(res.headers()["set-cookie"].to_str().unwrap().contains("HttpOnly; SameSite=Strict"));
        (c, base)
    };
    assert_eq!(get(&c, format!("{base}/api/session")).await["logged_in"], true);
    let other_site = c.post(format!("{base}/api/tasks/x/cancel")).header("origin", "https://evil.example").json(&json!({})).send().await.unwrap();
    assert_eq!(other_site.status(), 403, "writes only from this site");
    post(&c, format!("{base}/api/logout"), json!({})).await;
    assert_eq!(c.get(format!("{base}/api/projects")).send().await.unwrap().status(), 401);
}

#[tokio::test]
async fn projects_rules_memory_and_cron_over_the_api() {
    let r = start().await;
    let (c, base) = logged_in(&r).await;
    let other = tempfile::tempdir().unwrap();
    let p = c.put(format!("{base}/api/projects/docs")).json(&json!({"name": "Docs", "path": other.path(), "merge": "auto"})).send().await.unwrap();
    assert_eq!(p.status(), 200);
    let bad = c.put(format!("{base}/api/projects/Bad Name")).json(&json!({"path": other.path()})).send().await.unwrap();
    assert_eq!(bad.status(), 400);
    let list = get(&c, format!("{base}/api/projects")).await;
    assert_eq!(list.as_array().unwrap().iter().map(|p| p["slug"].as_str().unwrap()).collect::<Vec<_>>(), ["docs", "site"]);
    let rules = get(&c, format!("{base}/api/projects/docs/rules")).await;
    assert!(rules.as_array().unwrap().len() > 10, "starter rules");
    let set = c.put(format!("{base}/api/projects/docs/rules")).json(&json!([{"tool": "shell.*", "action": "allow"}, {"tool": "x", "action": "maybe"}])).send().await.unwrap();
    assert_eq!(set.status(), 400, "allow, ask or deny");
    let set: Value = c.put(format!("{base}/api/projects/docs/rules")).json(&json!([{"tool": "shell.*", "command": "make*", "action": "allow"}])).send().await.unwrap().json().await.unwrap();
    assert_eq!(set[0]["command"], "make*");
    let w = c.put(format!("{base}/api/memory")).json(&json!({"scope": "docs", "file": "topics/style.md", "text": "short sentences", "about": "how we write"})).send().await.unwrap();
    assert_eq!(w.status(), 200);
    let m = get(&c, format!("{base}/api/memory?scope=docs")).await;
    assert!(m["index"].as_str().unwrap().contains("how we write") && m["files"] == json!(["topics/style.md"]));
    assert_eq!(get(&c, format!("{base}/api/memory?scope=docs&file=topics/style.md")).await["text"], "short sentences");
    let g = c.put(format!("{base}/api/memory")).json(&json!({"scope": "global", "file": "me.md", "text": "x", "about": "about me"})).send().await.unwrap();
    assert_eq!(g.status(), 200);
    let cron = post(&c, format!("{base}/api/cron"), json!({"project": "docs", "expr": "0 4 * * *", "tz": "Europe/Vienna", "title": "Proofread", "prompt": "check typos"})).await;
    assert!(cron["next_run"].as_i64().unwrap() > 0);
    let bad = c.post(format!("{base}/api/cron")).json(&json!({"project": "docs", "expr": "nope", "title": "x", "prompt": "y"})).send().await.unwrap();
    assert_eq!(bad.status(), 400);
    assert_eq!(get(&c, format!("{base}/api/projects/docs/cron")).await.as_array().unwrap().len(), 1);
    assert!(c.delete(format!("{base}/api/cron/{}", cron["id"])).send().await.unwrap().status().is_success());
    std::fs::create_dir_all(other.path().join(".agents/skills/lint")).unwrap();
    std::fs::write(other.path().join(".agents/skills/lint/SKILL.md"), "---\nname: lint\ndescription: check style\n---\nrun the linter").unwrap();
    let sk = get(&c, format!("{base}/api/projects/docs/skills")).await;
    assert_eq!((sk[0]["name"].as_str(), sk[0]["source"].as_str()), (Some("lint"), Some("project")));
}

#[tokio::test]
async fn a_task_is_driven_from_the_api() {
    let r = start().await;
    let (c, base) = logged_in(&r).await;
    r.push("Api", |_| call("c1", "shell.exec", json!({"cmd": "echo from-the-job; touch x"})));
    r.push("Api", |b| {
        assert!(last_result(b).contains("from-the-job"), "{}", last_result(b));
        text("first done")
    });
    r.push("Api", |b| {
        assert!(all_text(b).contains("[from the person] and now?"), "{}", all_text(b));
        text("second done")
    });
    let t = post(&c, format!("{base}/api/tasks"), json!({"project": "site", "title": "Api", "prompt": "do it"})).await;
    let id = t["id"].as_str().unwrap().to_string();
    // It asks (`touch` isn't a starter rule); the inbox has it.
    let w = r.until(&id, "approval", |t| t.state == "waiting").await;
    let inbox = get(&c, format!("{base}/api/inbox")).await;
    assert_eq!(inbox["waiting"][0]["id"], id.as_str());
    let call_id = w.wait.unwrap().0["call"]["id"].as_str().unwrap().to_string();
    post(&c, format!("{base}/api/tasks/{id}/approve"), json!({"call_id": call_id, "approved": true})).await;
    r.until(&id, "first report", |t| t.report.as_deref() == Some("first done")).await;
    let jobs = get(&c, format!("{base}/api/tasks/{id}/jobs")).await;
    let job = jobs[0]["id"].as_str().unwrap();
    let out = get(&c, format!("{base}/api/jobs/{job}/output")).await;
    assert_eq!(out["text"][0]["text"], "from-the-job");
    let tr = get(&c, format!("{base}/api/tasks/{id}/transcript?full=true")).await;
    assert!(tr["messages"].as_array().unwrap().iter().any(|m| m["content"] == "first done"));
    post(&c, format!("{base}/api/tasks/{id}/message"), json!({"text": "and now?"})).await;
    r.until(&id, "second report", |t| t.report.as_deref() == Some("second done")).await;
    let found = get(&c, format!("{base}/api/search?q=from-the-job")).await;
    assert_eq!(found["tasks"][0]["task"]["id"], id.as_str(), "{found}");
    let t = get(&c, format!("{base}/api/tasks/{id}")).await;
    assert_eq!((t["state"].as_str(), t["subtasks"].as_array().map(Vec::len)), (Some("done"), Some(0)));
    let all = get(&c, format!("{base}/api/tasks?project=site")).await;
    assert_eq!(all.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_terminal_over_the_websocket() {
    let r = start().await;
    let (c, base) = logged_in(&r).await;
    r.push("Term", |_| text("idle"));
    let t = post(&c, format!("{base}/api/tasks"), json!({"project": "site", "title": "Term", "prompt": "x"})).await;
    let id = t["id"].as_str().unwrap();
    let p = post(&c, format!("{base}/api/tasks/{id}/ptys"), json!({"cmd": "cat", "cols": 40, "rows": 10})).await;
    let pty = p["id"].as_str().unwrap();
    // A session the WebSocket client carries as its cookie (as a browser would).
    r.run.app.store.add_session(&reagent_web::auth::token_hash("ws-token"), chrono_like_now() + 60).await.unwrap();
    let token = "ws-token";
    let url = format!("{}/api/ptys/{pty}", base.replace("http", "ws"));
    let mut req = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
    req.headers_mut().insert("cookie", format!("reagent_session={token}").parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let b64 = base64::engine::general_purpose::STANDARD;
    ws.send(tokio_tungstenite::tungstenite::Message::Text(json!({"data": b64.encode("typed here\r")}).to_string().into())).await.unwrap();
    let mut seen = String::new();
    while !seen.contains("typed here") {
        let m = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await.expect("bytes come").unwrap().unwrap();
        if let Ok(v) = serde_json::from_str::<Value>(m.to_text().unwrap())
            && let Some(d) = v["data"].as_str()
        {
            seen.push_str(&String::from_utf8_lossy(&b64.decode(d).unwrap()));
        }
    }
    ws.send(tokio_tungstenite::tungstenite::Message::Text(json!({"cols": 80, "rows": 20}).to_string().into())).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let list = get(&c, format!("{base}/api/tasks/{id}/ptys")).await;
    assert_eq!((list[0]["cols"].as_u64(), list[0]["rows"].as_u64()), (Some(80), Some(20)));
    assert!(c.delete(format!("{base}/api/ptys/{pty}")).send().await.unwrap().status().is_success());
}

fn chrono_like_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}
