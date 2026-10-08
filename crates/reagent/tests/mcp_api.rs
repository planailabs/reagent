//! reagent's MCP API: another agent signs in with a token and starts and
//! follows tasks.

mod common;

use std::collections::HashMap;

use common::*;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};

async fn client(url: &str, token: &str) -> Result<rmcp::service::RunningService<rmcp::RoleClient, ()>, String> {
    let mut h = HashMap::new();
    h.insert(axum::http::HeaderName::from_static("authorization"), axum::http::HeaderValue::from_str(&format!("Bearer {token}")).unwrap());
    let cfg = StreamableHttpClientTransportConfig::with_uri(format!("{url}/mcp")).custom_headers(h);
    ().serve(StreamableHttpClientTransport::from_config(cfg)).await.map_err(|e| e.to_string())
}

async fn call(c: &rmcp::service::RunningService<rmcp::RoleClient, ()>, tool: &str, args: Value) -> (bool, String) {
    let mut p = CallToolRequestParams::new(tool.to_string());
    p.arguments = args.as_object().cloned();
    let r = c.call_tool(p).await.unwrap();
    (r.is_error == Some(true), r.content.iter().filter_map(|c| c.as_text().map(|t| t.text.clone())).collect::<Vec<_>>().join("\n"))
}

#[tokio::test]
async fn another_agent_starts_and_follows_a_task_with_a_token() {
    let r = start().await;
    let token = "rgt_test-token";
    r.run.app.store.add_api_token("outside", &reagent_web::auth::token_hash(token)).await.unwrap();
    assert!(client(&r.run.web_url, "wrong").await.is_err(), "no token, no session");

    let c = client(&r.run.web_url, token).await.unwrap();
    let mut tools: Vec<String> = c.list_all_tools().await.unwrap().into_iter().map(|t| t.name.to_string()).collect();
    tools.sort();
    assert!(["projects", "search", "prompt_design", "design_proposal", "design_create", "docs_list", "docs_read", "task_answer", "task_approve", "task_cancel", "task_get", "task_list", "task_merge", "task_message", "task_pause", "task_raise_budget", "task_resume", "task_retry", "task_start", "task_transcript", "task_wait"].iter().all(|t| tools.contains(&t.to_string())), "{tools:?}");
    let (_, p) = call(&c, "projects", json!({})).await;
    assert!(p.contains("\"id\": \"site\""), "{p}");

    r.push("Outside", |_| call_tool("c1", "ask.ask", json!({"question": "go on?"})));
    r.push("Outside", |b| {
        assert_eq!(last_result(b), "yes");
        text("done for the outside")
    });
    let (err, started) = call(&c, "task_start", json!({"project": "site", "title": "Outside", "prompt": "x"})).await;
    assert!(!err, "{started}");
    let id = serde_json::from_str::<Value>(&started).unwrap()["task"].as_str().unwrap().to_string();
    // It waits for an answer; the outside agent sees why and answers.
    let (_, w) = call(&c, "task_wait", json!({"task": id, "timeout": 20})).await;
    let w: Value = serde_json::from_str(&w).unwrap();
    assert_eq!((w["state"].as_str(), w["waits_for"]["question"].as_str()), (Some("waiting"), Some("go on?")), "{w}");
    assert!(!call(&c, "task_answer", json!({"task": id, "text": "yes"})).await.0);
    let mut done = Value::Null;
    for _ in 0..5 {
        let (_, w) = call(&c, "task_wait", json!({"task": id, "timeout": 20})).await;
        done = serde_json::from_str(&w).unwrap();
        if done["state"] == "done" {
            break;
        }
    }
    assert_eq!(done["report"], "done for the outside", "{done}");
    let (_, tr) = call(&c, "task_transcript", json!({"task": id, "tail": 2})).await;
    assert!(tr.contains("done for the outside"));
    let (_, list) = call(&c, "task_list", json!({"all": true})).await;
    assert!(list.contains(&id));
    let t = r.run.app.task(&id).await.unwrap();
    assert_eq!(t.origin, "mcp");
    let (err, msg) = call(&c, "task_retry", json!({"task": id})).await;
    assert!(err && msg.contains("isn't failed"), "{msg}");
    // Revoked: no more.
    r.run.app.store.revoke_api_token("outside").await.unwrap();
    assert!(client(&r.run.web_url, token).await.is_err());
}

fn call_tool(id: &str, tool: &str, args: Value) -> Vec<String> {
    common::call(id, tool, args)
}
