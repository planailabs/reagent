//! Stopping and starting again: a running task is paused (its command
//! finishes first) and goes on after the next start.

mod common;

use common::*;
use serde_json::json;

#[tokio::test]
async fn a_task_paused_at_stop_goes_on_after_the_next_start() {
    let r = start().await;
    r.allow_all().await;
    r.push("Long", |_| call("c1", "shell.exec", json!({"cmd": "sleep 1; echo slept"})));
    r.push("Long", |b| {
        assert!(last_result(b).contains("slept"), "the command finished before the stop: {}", last_result(b));
        text("after the restart")
    });
    let t = r.start_task("Long", "x").await;
    // Its command runs; reagent stops.
    for _ in 0..100 {
        if r.run.app.sup.jobs(Some(&t.id)).await.unwrap().iter().any(|j| j.running()) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    reagent::stop(&r.run).await;
    let kept: Vec<String> = serde_json::from_slice(&std::fs::read(r.data.path().join("paused-at-stop.json")).unwrap()).unwrap();
    assert_eq!(kept, [t.id.clone()]);
    assert_eq!(r.requests("Long").len(), 1, "nothing more was asked of the model while stopping");

    // The next start, on the same data.
    let again = reagent::up(reagent::Opts { data: r.data.path().into(), listen: None, in_process_supervisor: true, exe: Default::default(), dist: "/nonexistent".into() }).await.unwrap();
    assert!(!r.data.path().join("paused-at-stop.json").exists());
    let mut done = None;
    for _ in 0..200 {
        let t = again.app.task(&t.id).await.unwrap();
        if t.state == "done" {
            done = Some(t);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(done.expect("it went on").report.as_deref(), Some("after the restart"));
}
