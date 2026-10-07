//! A notification's buttons: one-time tokens that approve, deny, answer or
//! merge without a login, only while the task still waits for that.

mod common;

use common::*;
use serde_json::{Value, json};

/// The buttons of the next notification for a task.
async fn buttons(rx: &mut tokio::sync::broadcast::Receiver<Value>, task: &str) -> Vec<(String, String)> {
    loop {
        let e = tokio::time::timeout(std::time::Duration::from_secs(20), rx.recv()).await.expect("a notification").unwrap();
        if e["kind"] == "notification" && e["task"] == task && e["actions"].as_array().is_some_and(|a| !a.is_empty()) {
            return e["actions"].as_array().unwrap().iter().map(|a| (a["title"].as_str().unwrap().to_string(), a["token"].as_str().unwrap().to_string())).collect();
        }
    }
}

async fn press(base: &str, token: &str) -> (u16, Value) {
    let r = reqwest::Client::new().post(format!("{base}/api/action")).json(&json!({"token": token})).send().await.unwrap();
    (r.status().as_u16(), r.json().await.unwrap_or_default())
}

#[tokio::test]
async fn buttons_approve_and_answer_once_without_a_login() {
    let r = start().await;
    let base = r.run.web_url.clone();
    let mut rx = r.run.app.events.subscribe();
    r.push("Btn", |_| call("c1", "shell.exec", json!({"cmd": "touch pressed.txt"})));
    r.push("Btn", |_| call("c2", "ask.ask", json!({"question": "tea or coffee?", "options": ["tea", "coffee", "water"]})));
    r.push("Btn", |b| {
        assert_eq!(last_result(b), "coffee");
        text("coffee then")
    });
    let t = r.start_task("Btn", "x").await;
    let b = buttons(&mut rx, &t.id).await;
    assert_eq!(b.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["Allow once", "Deny"]);
    let (status, v) = press(&base, &b[0].1).await;
    assert_eq!((status, v["done"].as_str()), (200, Some("allowed")), "{v}");
    assert_eq!(press(&base, &b[0].1).await.0, 410, "once");
    assert_eq!(press(&base, &b[1].1).await.0, 410, "the call isn't waiting any more");
    let q = buttons(&mut rx, &t.id).await;
    assert_eq!(q.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["tea", "coffee"], "the first two options");
    assert_eq!(press(&base, &q[1].1).await.0, 200);
    r.done(&t.id).await;
    assert!(r.project.path().join("pressed.txt").exists());
    assert_eq!(press(&base, "made-up").await.0, 410);
}

#[tokio::test]
async fn a_merge_button_merges() {
    let r = start().await;
    r.allow_all().await;
    let base = r.run.web_url.clone();
    let mut rx = r.run.app.events.subscribe();
    r.push("MB", |_| call("c1", "git.worktree_start", json!({})));
    r.push("MB", |_| call("c2", "fs.write", json!({"path": "b.txt", "text": "b\n"})));
    r.push("MB", |_| call("c3", "shell.exec", json!({"cmd": "git add -A && git commit -qm b"})));
    r.push("MB", |_| call("c4", "git.worktree_merge", json!({})));
    r.push("MB", |b| {
        assert!(last_result(b).contains("merged "), "{}", last_result(b));
        text("done")
    });
    let t = r.start_task("MB", "x").await;
    let b = buttons(&mut rx, &t.id).await;
    assert_eq!(b[0].0, "Merge");
    assert_eq!(press(&base, &b[0].1).await.0, 200);
    r.done(&t.id).await;
    assert!(r.project.path().join("b.txt").exists());
}
