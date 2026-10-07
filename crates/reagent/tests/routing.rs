//! Kinds pick profiles; tasks move onto another profile (the person, the
//! task itself, after tool errors) and fall back when a model fails.

mod common;

use common::*;
use serde_json::json;

const EXTRA: &str = r#"
provider "down" {
  base_url = "http://127.0.0.1:9/v1"
}
profile "big" {
  provider = "mock"
  model = "big-model"
}
profile "flaky" {
  provider = "down"
  model = "m"
  fallback = "default"
}
kind "research" {
  profile = "big"
}
kind "chore" {
  profile = "default"
  escalate = "big"
  escalate_after = 2
}
kind "shaky" {
  profile = "flaky"
}
routing {
  subtask = "chore"
}
"#;

async fn routed() -> R {
    start_in(tempfile::tempdir().unwrap(), EXTRA).await
}

fn model(b: &serde_json::Value) -> String {
    b["model"].as_str().unwrap_or("").to_string()
}

async fn start_kind(r: &R, title: &str, kind: &str) -> reagent_store::Task {
    r.run.app.start_task(reagent_tools::app::StartTask { project: "site".into(), title: title.into(), prompt: "x".into(), kind: Some(kind.into()), ..Default::default() }).await.unwrap()
}

#[tokio::test]
async fn a_kind_picks_the_profile_and_subtasks_route_by_origin() {
    let r = routed().await;
    r.push("Research", |b| {
        assert_eq!(model(b), "big-model");
        call("c1", "tasks.task_spawn", json!({"title": "Sub", "prompt": "y"}))
    });
    r.push("Sub", |b| {
        assert_eq!(model(b), "m", "a subtask is a chore: the default profile");
        text("sub done")
    });
    r.push("Research", |_| text("started it"));
    r.push("Research", |_| text("all done"));
    let t = start_kind(&r, "Research", "research").await;
    assert_eq!((t.kind.as_deref(), t.profile.as_str()), (Some("research"), "big"));
    r.until(&t.id, "the parent's last report", |t| t.report.as_deref() == Some("all done")).await;
    let kids = r.run.app.store.tasks(None, Some(&t.id), false, 10).await.unwrap();
    assert_eq!(kids[0].kind.as_deref(), Some("chore"));
}

#[tokio::test]
async fn the_person_moves_a_task_onto_another_model_with_its_history() {
    let r = routed().await;
    r.push("Move", |b| {
        assert_eq!(model(b), "m");
        text("on the small one")
    });
    r.push("Move", |b| {
        assert_eq!(model(b), "big-model", "the next step runs on the new model");
        assert!(all_text(b).contains("on the small one"), "with the history");
        text("on the big one")
    });
    let t = r.start_task("Move", "x").await;
    r.done(&t.id).await;
    let moved = r.run.app.switch_profile(&t.id, "big", "test").await.unwrap();
    assert_eq!(moved.profile, "big");
    assert_ne!(moved.agent, t.agent, "a new agent");
    assert!(r.run.app.switch_profile(&t.id, "big", "again").await.unwrap_err().contains("already"));
    r.run.app.message(&t.id, "go on").await.unwrap();
    r.until(&t.id, "the next report", |t| t.report.as_deref() == Some("on the big one")).await;
    let n = r.run.app.store.notifications(20).await.unwrap();
    assert!(n.iter().any(|n| n.kind == "model" && n.title.contains("now runs on big")));
}

#[tokio::test]
async fn a_task_escalates_itself_and_tool_errors_escalate_it() {
    let r = routed().await;
    r.allow_all().await;
    r.push("Stuck", |_| call("c1", "tasks.task_escalate", json!({"why": "this is hard"})));
    r.push("Stuck", |b| {
        assert!(last_result(b).contains("moving onto big"), "{}", last_result(b));
        text("thinking")
    });
    r.push("Stuck", |b| {
        assert_eq!(model(b), "big-model");
        text("solved")
    });
    let t = start_kind(&r, "Stuck", "chore").await;
    r.until(&t.id, "moved", |t| t.profile == "big").await;
    r.run.app.message(&t.id, "and now?").await.unwrap();
    r.until(&t.id, "solved", |t| t.report.as_deref() == Some("solved")).await;

    // Two tool errors in a row (escalate_after = 2): moved by itself.
    r.push("Errs", |_| call("c1", "fs.read", json!({"path": "missing-1"})));
    r.push("Errs", |_| call("c2", "fs.read", json!({"path": "missing-2"})));
    r.push("Errs", |b| {
        assert_eq!(model(b), "big-model", "escalated after the second error");
        text("found another way")
    });
    let e = start_kind(&r, "Errs", "chore").await;
    let done = r.done(&e.id).await;
    assert_eq!((done.profile.as_str(), done.report.as_deref()), ("big", Some("found another way")));
}

#[tokio::test]
async fn a_failing_model_falls_back() {
    let r = routed().await;
    r.push("Shaky", |b| {
        assert_eq!(model(b), "m", "on the fallback");
        text("made it")
    });
    let t = start_kind(&r, "Shaky", "shaky").await;
    assert_eq!(t.profile, "flaky");
    let d = r.done(&t.id).await;
    assert_eq!((d.profile.as_str(), d.report.as_deref()), ("default", Some("made it")));
}
