//! Triggers from outside: webhooks (signed), the web API, a task's tools and
//! the MCP API.

mod common;

use std::collections::HashMap;
use std::time::Duration;

use common::*;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};

async fn until<T>(what: &str, mut f: impl AsyncFnMut() -> Option<T>) -> T {
    for _ in 0..400 {
        if let Some(v) = f().await {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn login(r: &R) -> reqwest::Client {
    r.run.app.store.set_setting("password", &reagent_web::auth::hash("secret-pass").unwrap()).await.unwrap();
    let c = reqwest::Client::builder().cookie_store(true).build().unwrap();
    c.post(format!("{}/api/login", r.run.web_url)).json(&json!({"password": "secret-pass"})).send().await.unwrap();
    c
}

async fn tasks_from(r: &R, origin: &str) -> Vec<reagent_store::Task> {
    r.run.app.store.tasks(Some("site"), None, false, 100).await.unwrap().into_iter().filter(|t| t.origin == origin).collect()
}

fn hmac_hex(secret: &str, body: &[u8]) -> String {
    use hmac::Mac;
    let mut m = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    m.update(body);
    m.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

#[tokio::test]
async fn webhooks_are_signed_and_their_deliveries_start_tasks_once() {
    let r = start().await;
    let c = login(&r).await;
    let base = r.run.web_url.clone();
    r.run.app.store.set_secret(Some("site"), "HOOK", "s3cret-hook").await.unwrap();
    // No script: the body is the event.
    let put = c.put(format!("{base}/api/triggers/site/push")).json(&json!({"mode": "webhook", "secret": "HOOK", "title": "Pushed {{vars.ref}}", "prompt": "Look at {{vars.after}}"})).send().await.unwrap();
    assert!(put.status().is_success(), "{}", put.text().await.unwrap());
    r.push("Pushed main", |b| {
        assert!(all_text(b).contains("Look at abc123"));
        text("looked")
    });
    let hook = format!("{base}/hook/site/push");
    let body = json!({"ref": "main", "after": "abc123"}).to_string();
    let send = |auth: &'static str, delivery: &'static str| {
        let (hook, body) = (hook.clone(), body.clone());
        async move { reqwest::Client::new().post(hook).header("authorization", auth).header("x-github-delivery", delivery).body(body).send().await.unwrap().status().as_u16() }
    };
    assert_eq!(send("Bearer wrong", "d1").await, 401);
    assert_eq!(send("Bearer s3cret-hook", "d1").await, 202);
    let t = until("its task", async || tasks_from(&r, "trigger:site/push").await.pop()).await;
    r.done(&t.id).await;
    assert_eq!(send("Bearer s3cret-hook", "d1").await, 202, "the same delivery again");
    until("the second run", async || Some(r.run.app.store.trigger_runs("site", "push").await.unwrap()).filter(|x| x.len() == 2)).await;
    assert_eq!(tasks_from(&r, "trigger:site/push").await.len(), 1, "seen: dropped");
    assert_eq!(reqwest::Client::new().post(format!("{base}/hook/site/nope")).body("{}").send().await.unwrap().status(), 404);
    // GitHub's signature, and a script that filters: only failed runs.
    let script = "jq -c 'select(.body.workflow_run.conclusion == \"failure\") | {key: (.body.workflow_run.id|tostring), vars: .body.workflow_run}'";
    let put = c.put(format!("{base}/api/triggers/site/gh")).json(&json!({"mode": "webhook", "secret": "HOOK", "script": script, "title": "CI {{vars.id}} failed", "prompt": "Fix {{vars.html_url}}"})).send().await.unwrap();
    assert!(put.status().is_success());
    r.push("CI 7 failed", |b| {
        assert!(all_text(b).contains("Fix https://ci/7"));
        text("fixed")
    });
    for (id, conclusion) in [(6, "success"), (7, "failure")] {
        let body = json!({"workflow_run": {"id": id, "conclusion": conclusion, "html_url": format!("https://ci/{id}")}}).to_string();
        let st = reqwest::Client::new().post(format!("{base}/hook/site/gh")).header("x-hub-signature-256", format!("sha256={}", hmac_hex("s3cret-hook", body.as_bytes()))).body(body).send().await.unwrap().status();
        assert_eq!(st, 202);
    }
    let t = until("the failure's task", async || tasks_from(&r, "trigger:site/gh").await.pop()).await;
    assert_eq!(t.title, "CI 7 failed");
    let runs = until("both runs", async || Some(r.run.app.store.trigger_runs("site", "gh").await.unwrap()).filter(|x| x.len() == 2)).await;
    assert!(runs.iter().all(|x| x.ok), "{runs:?}");
    assert_eq!(tasks_from(&r, "trigger:site/gh").await.len(), 1, "the success started nothing");
    let bad = reqwest::Client::new().post(format!("{base}/hook/site/gh")).header("x-hub-signature-256", "sha256=00").body("{}").send().await.unwrap();
    assert_eq!(bad.status(), 401);
}

#[tokio::test]
async fn the_person_manages_triggers_over_the_api() {
    let r = start().await;
    let c = login(&r).await;
    let base = r.run.web_url.clone();
    let bad = c.put(format!("{base}/api/triggers/site/x")).json(&json!({"mode": "poll", "title": "t", "prompt": "p", "script": "true"})).send().await.unwrap();
    assert_eq!(bad.status(), 400, "no schedule");
    let ok = c.put(format!("{base}/api/triggers/site/x")).json(&json!({"mode": "poll", "every": 3600, "title": "t", "prompt": "p", "script": "echo checked"})).send().await.unwrap();
    assert!(ok.status().is_success());
    let list: Value = c.get(format!("{base}/api/projects/site/triggers")).send().await.unwrap().json().await.unwrap();
    assert_eq!((list["triggers"][0]["name"].as_str(), list["triggers"][0]["made_by"].as_str()), (Some("x"), Some("person")));
    // The person's script runs without asking.
    let runs = until("a run", async || Some(c.get(format!("{base}/api/triggers/site/x/runs")).send().await.unwrap().json::<Value>().await.unwrap()).filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))).await;
    assert!(runs[0]["output"].as_str().unwrap().contains("checked"));
    assert!(c.post(format!("{base}/api/triggers/site/x/run")).send().await.unwrap().status().is_success());
    until("another run", async || Some(r.run.app.store.trigger_runs("site", "x").await.unwrap()).filter(|x| x.len() == 2)).await;
    assert!(c.post(format!("{base}/api/triggers/site/x/enabled")).json(&json!({"enabled": false})).send().await.unwrap().status().is_success());
    assert!(!r.run.app.store.trigger("site", "x").await.unwrap().unwrap().enabled);
    let moved: Value = c.post(format!("{base}/api/triggers/site/x/move")).json(&json!({"to": "repo"})).send().await.unwrap().json().await.unwrap();
    assert_eq!(moved["source"], "repo");
    assert!(r.project.path().join(".agents/triggers/x/TRIGGER.md").is_file());
    assert!(c.delete(format!("{base}/api/triggers/site/x")).send().await.unwrap().status().is_success());
    assert!(!r.project.path().join(".agents/triggers/x").exists(), "a repo trigger's files go with it");
    assert!(r.run.app.store.trigger("site", "x").await.unwrap().is_none());
    assert_eq!(reqwest::get(format!("{base}/api/projects/site/triggers")).await.unwrap().status(), 401);
}

#[tokio::test]
async fn a_task_adds_a_trigger_the_person_approves_its_script() {
    let r = start().await;
    let c = login(&r).await;
    let base = r.run.web_url.clone();
    // The task may add triggers (a rule); its script is the policy's call: ask.
    let mut rules = r.run.app.store.rules("site").await.unwrap();
    rules.insert(0, reagent_store::Rule { id: 0, project: "site".into(), pos: 0, tool: "triggers.trigger_add".into(), command: None, target: None, action: "allow".into() });
    r.run.app.store.set_rules("site", &rules).await.unwrap();
    r.push("Watch CI", |_| call("c1", "triggers.trigger_add", json!({"name": "ci", "mode": "poll", "every": "1h", "script": "echo '{\"key\": \"1\"}'", "title": "CI broke", "prompt": "fix it"})));
    r.push("Watch CI", |b| {
        assert!(last_result(b).starts_with("added: ci (poll, every 3600s, in reagent, by task:"), "{}", last_result(b));
        call("c2", "triggers.trigger_list", json!({}))
    });
    r.push("Watch CI", |b| {
        assert!(last_result(b).contains("waits for approval") || last_result(b).contains("ci (poll"), "{}", last_result(b));
        text("watching")
    });
    let t = r.start_task("Watch CI", "x").await;
    r.done(&t.id).await;
    let inbox: Value = until("the inbox to show it", async || Some(c.get(format!("{base}/api/inbox")).send().await.unwrap().json::<Value>().await.unwrap()).filter(|v| v["triggers"].as_array().is_some_and(|a| !a.is_empty()))).await;
    assert_eq!(inbox["triggers"][0]["name"], "ci");
    r.push("CI broke", |_| text("on it"));
    assert!(c.post(format!("{base}/api/triggers/site/ci/approve")).json(&json!({"approved": true})).send().await.unwrap().status().is_success());
    let started = until("its task", async || tasks_from(&r, "trigger:site/ci").await.pop()).await;
    r.done(&started.id).await;
}

#[tokio::test]
async fn outside_agents_manage_triggers() {
    let r = start().await;
    let token = "rgt_trigger-token";
    r.run.app.store.add_api_token("outside", &reagent_web::auth::token_hash(token)).await.unwrap();
    let mut h = HashMap::new();
    h.insert(axum::http::HeaderName::from_static("authorization"), axum::http::HeaderValue::from_str(&format!("Bearer {token}")).unwrap());
    let c = ().serve(StreamableHttpClientTransport::from_config(StreamableHttpClientTransportConfig::with_uri(format!("{}/mcp", r.run.web_url)).custom_headers(h))).await.unwrap();
    let tool = async |name: &str, args: Value| {
        let mut p = CallToolRequestParams::new(name.to_string());
        p.arguments = args.as_object().cloned();
        let res = c.call_tool(p).await.unwrap();
        (res.is_error == Some(true), res.content.iter().filter_map(|x| x.as_text().map(|t| t.text.clone())).collect::<Vec<_>>().join("\n"))
    };
    let (err, out) = tool("trigger_add", json!({"project": "site", "name": "hook", "mode": "webhook", "secret": "HOOK", "title": "t", "prompt": "p"})).await;
    assert!(!err && out.contains("/hook/site/hook"), "{out}");
    let (_, list) = tool("trigger_list", json!({"project": "site"})).await;
    assert!(list.contains("hook (webhook, webhook, in reagent, by mcp)"), "{list}");
    let (err, out) = tool("trigger_run", json!({"project": "site", "name": "hook"})).await;
    assert!(err && out.contains("when it's called"));
    let (err, _) = tool("trigger_move", json!({"project": "site", "name": "hook", "to": "repo"})).await;
    assert!(!err);
    assert!(r.project.path().join(".agents/triggers/hook/TRIGGER.md").is_file());
    let (err, _) = tool("trigger_remove", json!({"project": "site", "name": "hook"})).await;
    assert!(!err);
    assert!(r.run.app.store.triggers(None).await.unwrap().is_empty());
}
