//! Tasks running at once: over the global or the project's limit a task
//! waits (queued) and starts when a place frees; subtasks never wait.

mod common;

use common::*;
use serde_json::json;

#[tokio::test]
async fn over_the_limit_tasks_wait_their_turn() {
    let r = start().await;
    r.allow_all().await;
    r.run.app.store.set_setting("max_tasks", "1").await.unwrap();
    // The first works a while, with a subtask (which doesn't wait for a place).
    r.push("First", |_| call("c1", "tasks.task_spawn", json!({"title": "Sub", "prompt": "x"})));
    r.push("First", |_| call("c2", "shell.exec", json!({"cmd": "sleep 2"})));
    r.push("First", |_| text("first done"));
    r.push("Sub", |_| text("sub done"));
    r.push("Second", |b| {
        assert!(all_text(b).contains("[a message that came while it was queued] also this"), "{}", all_text(b));
        text("second done")
    });
    let a = r.start_task("First", "x").await;
    assert_eq!(a.state, "running");
    let b = r.start_task("Second", "x").await;
    assert_eq!((b.state.as_str(), b.agent.as_deref(), b.started), ("queued", None, None));
    // A message for it waits with it.
    r.run.app.message(&b.id, "also this").await.unwrap();
    let kids = loop {
        let k = r.run.app.store.tasks(None, Some(&a.id), false, 10).await.unwrap();
        if !k.is_empty() {
            break k;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert_ne!(kids[0].state, "queued", "a subtask doesn't wait");
    assert_eq!(r.task(&b.id).await.state, "queued", "still waiting while the first runs");
    r.done(&a.id).await;
    let b = r.done(&b.id).await;
    assert!(b.started.is_some_and(|s| s >= b.created));
}

#[tokio::test]
async fn a_projects_own_limit_and_starting_now() {
    let r = start().await;
    r.allow_all().await;
    let mut p = r.run.app.project("site").await.unwrap();
    p.max_tasks = Some(1);
    r.run.app.put_project(p).await.unwrap();
    r.push("Long", |_| call("c1", "shell.exec", json!({"cmd": "sleep 3"})));
    r.push("Long", |_| text("long done"));
    r.push("Urgent", |_| text("urgent done"));
    let a = r.start_task("Long", "x").await;
    let b = r.start_task("Urgent", "x").await;
    assert_eq!(b.state, "queued");
    // The person starts it anyway.
    let b = r.run.app.start_now(&b.id).await.unwrap();
    assert_eq!(b.state, "running");
    assert!(r.run.app.start_now(&b.id).await.unwrap_err().contains("isn't queued"));
    r.done(&b.id).await;
    r.done(&a.id).await;
    // A queued task can be cancelled before it starts.
    r.push("Long2", |_| call("c1", "shell.exec", json!({"cmd": "sleep 2"})));
    r.push("Long2", |_| text("ok"));
    let c = r.start_task("Long2", "x").await;
    let d = r.start_task("Never", "x").await;
    assert_eq!(d.state, "queued");
    r.run.app.cancel(&d.id).await.unwrap();
    r.done(&c.id).await;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    assert_eq!(r.task(&d.id).await.state, "cancelled");
}

#[tokio::test]
async fn the_person_sets_the_limit() {
    let r = start().await;
    r.run.app.store.set_setting("password", &reagent_web::auth::hash("secret-pass").unwrap()).await.unwrap();
    let c = reqwest::Client::builder().cookie_store(true).build().unwrap();
    let base = r.run.web_url.clone();
    c.post(format!("{base}/api/login")).json(&json!({"password": "secret-pass"})).send().await.unwrap();
    let v: serde_json::Value = c.put(format!("{base}/api/settings")).json(&json!({"max_tasks": 2})).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["max_tasks"], 2);
    let v: serde_json::Value = c.put(format!("{base}/api/settings")).json(&json!({"max_tasks": null})).send().await.unwrap().json().await.unwrap();
    assert!(v["max_tasks"].is_null());
    let p: serde_json::Value = c.put(format!("{base}/api/projects/site")).json(&json!({"max_tasks": 1})).send().await.unwrap().json().await.unwrap();
    assert_eq!(p["max_tasks"], 1);
}
