//! MCP servers the person adds: every task gets their tools, lazily or not;
//! one that doesn't run is reported and doesn't stop tasks.

mod common;

use common::*;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager};
use rmcp::{schemars, tool, tool_router};
use serde_json::{Value, json};

#[derive(Clone)]
struct Extra;

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct Text {
    text: String,
}

#[tool_router(server_handler)]
impl Extra {
    #[tool(description = "Echo text back")]
    fn echo(&self, Parameters(Text { text }): Parameters<Text>) -> String {
        format!("echo: {text}")
    }
}

async fn extra_server() -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", l.local_addr().unwrap());
    let svc = StreamableHttpService::new(|| Ok(Extra), LocalSessionManager::default().into(), StreamableHttpServerConfig::default());
    let r = axum::Router::new().nest_service("/mcp", svc);
    tokio::spawn(async move { axum::serve(l, r).await.unwrap() });
    url
}

fn server(v: Value) -> reagent_store::McpServer {
    serde_json::from_value(v).unwrap()
}

fn tool_names(body: &Value) -> Vec<String> {
    body["tools"].as_array().map(|t| t.iter().map(|t| t["function"]["name"].as_str().unwrap_or("").to_string()).collect()).unwrap_or_default()
}

#[tokio::test]
async fn an_added_server_gives_every_task_its_tools() {
    let r = start().await;
    r.allow_all().await;
    let url = extra_server().await;
    r.run.app.store.put_mcp_server(&server(json!({"name": "extra", "url": url}))).await.unwrap();
    r.run.app.store.put_mcp_server(&server(json!({"name": "broken", "url": "http://127.0.0.1:9/mcp"}))).await.unwrap();
    r.run.app.apply_cluster().await.unwrap();
    let status = r.run.app.mcp_status.lock().unwrap().clone();
    assert_eq!(status["extra"]["ok"], true, "{status:?}");
    assert_eq!(status["broken"]["ok"], false, "{status:?}");

    // Lazy: the task sees its name, not its schema; a first call loads it.
    r.push("Lazy", |b| {
        let names = tool_names(b);
        assert!(names.contains(&"load_tools".into()) && !names.contains(&"extra__echo".into()), "{names:?}");
        assert!(!names.iter().any(|n| n.starts_with("broken")), "a server that doesn't run isn't given");
        call("c1", "extra.echo", json!({"text": "hi"}))
    });
    r.push("Lazy", |_| call("c2", "extra.echo", json!({"text": "hi"})));
    r.push("Lazy", |b| {
        assert_eq!(last_result(b), "echo: hi");
        text("echoed")
    });
    let t = r.start_task("Lazy", "x").await;
    r.done(&t.id).await;

    // Eager: offered from the start (and through the web API, which applies at once).
    let (c, base) = {
        r.run.app.store.set_setting("password", &reagent_web::auth::hash("secret-pass").unwrap()).await.unwrap();
        let c = reqwest::Client::builder().cookie_store(true).build().unwrap();
        c.post(format!("{}/api/login", r.run.web_url)).json(&json!({"password": "secret-pass"})).send().await.unwrap();
        (c, r.run.web_url.clone())
    };
    let res: Value = c.put(format!("{base}/api/mcp/extra")).json(&json!({"url": url, "lazy": false})).send().await.unwrap().json().await.unwrap();
    assert_eq!(res["status"]["ok"], true, "{res}");
    let bad = c.put(format!("{base}/api/mcp/fs")).json(&json!({"url": url})).send().await.unwrap();
    assert_eq!(bad.status(), 400, "reagent's own names are taken");
    let list: Value = c.get(format!("{base}/api/mcp")).send().await.unwrap().json().await.unwrap();
    assert_eq!(list.as_array().unwrap().len(), 2);
    r.push("Eager", |b| {
        assert!(tool_names(b).contains(&"extra__echo".into()), "{:?}", tool_names(b));
        call("c1", "extra.echo", json!({"text": "now"}))
    });
    r.push("Eager", |b| {
        assert_eq!(last_result(b), "echo: now");
        text("ok")
    });
    let t = r.start_task("Eager", "x").await;
    r.done(&t.id).await;
    assert!(c.delete(format!("{base}/api/mcp/broken")).send().await.unwrap().status().is_success());
    assert!(!r.run.app.mcp_status.lock().unwrap().contains_key("broken"));
}

#[tokio::test]
async fn a_change_written_by_the_cli_is_applied_by_the_running_reagent() {
    let r = start().await;
    let url = extra_server().await;
    // As `reagent mcp add` does: only the database.
    r.run.app.store.put_mcp_server(&server(json!({"name": "later", "url": url}))).await.unwrap();
    let mut ok = false;
    for _ in 0..100 {
        if r.run.app.mcp_status.lock().unwrap().get("later").is_some_and(|s| s["ok"] == true) {
            ok = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(ok, "applied within seconds");
}

#[tokio::test]
async fn a_projects_own_server_reaches_only_its_tasks() {
    let r = start().await;
    let other = tempfile::tempdir().unwrap();
    r.run.app.put_project(reagent_store::Project::new("other", "Other", &other.path().display().to_string())).await.unwrap();
    let url = extra_server().await;
    r.run.app.store.put_mcp_server(&server(json!({"name": "siteonly", "url": url, "lazy": false, "project": "site"}))).await.unwrap();
    r.run.app.apply_cluster().await.unwrap();
    r.push("Here", |b| {
        assert!(tool_names(b).contains(&"siteonly__echo".into()), "{:?}", tool_names(b));
        text("seen")
    });
    r.push("Elsewhere", |b| {
        assert!(!tool_names(b).iter().any(|n| n.starts_with("siteonly")), "{:?}", tool_names(b));
        assert!(tool_names(b).contains(&"fs__read".into()));
        text("not seen")
    });
    let here = r.start_task("Here", "x").await;
    let elsewhere = r.run.app.start_task(reagent_tools::app::StartTask { project: "other".into(), title: "Elsewhere".into(), prompt: "x".into(), ..Default::default() }).await.unwrap();
    r.done(&here.id).await;
    r.done(&elsewhere.id).await;
    // The project goes, its server with it; tasks start from the shared mixture again.
    r.run.app.store.remove_mcp_server("siteonly").await.unwrap();
    r.run.app.apply_cluster().await.unwrap();
    assert!(!r.run.app.hub().unwrap().cluster().spec.mixtures.contains_key("task-default--site"));
}
