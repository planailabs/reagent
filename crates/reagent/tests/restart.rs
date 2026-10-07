//! Stopping and starting again: a running task is paused (its command
//! finishes first) and goes on after the next start.

mod common;

use common::*;
use common::R;
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
    let cluster_before = std::fs::read_to_string(r.data.path().join("cluster.hcl")).unwrap();
    reagent::stop(&r.run).await;
    let kept: Vec<String> = serde_json::from_slice(&std::fs::read(r.data.path().join("paused-at-stop.json")).unwrap()).unwrap();
    assert_eq!(kept, [t.id.clone()]);
    assert_eq!(r.requests("Long").len(), 1, "nothing more was asked of the model while stopping");

    // The next start, on the same data.
    let again = reagent::up(reagent::Opts { data: r.data.path().into(), listen: None, in_process_supervisor: true, exe: Default::default(), dist: "/nonexistent".into() }).await.unwrap();
    assert!(!r.data.path().join("paused-at-stop.json").exists());
    assert_eq!(std::fs::read_to_string(r.data.path().join("cluster.hcl")).unwrap(), cluster_before, "the same servers, at the same URLs: running tasks stay current");
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

async fn again(r: &R) -> reagent::Running {
    reagent::up(reagent::Opts { data: r.data.path().into(), listen: None, in_process_supervisor: true, exe: Default::default(), dist: "/nonexistent".into() }).await.unwrap()
}

async fn until_state(run: &reagent::Running, id: &str, what: &str, f: impl Fn(&reagent_store::Task) -> bool) -> reagent_store::Task {
    for _ in 0..300 {
        let t = run.app.task(id).await.unwrap();
        if f(&t) {
            return t;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}: {:?}", run.app.task(id).await.unwrap());
}

#[tokio::test]
async fn a_question_asked_before_a_restart_is_answered_after_it() {
    let r = start().await;
    r.push("Q", |_| call("c1", "ask.ask", json!({"question": "which branch?", "options": ["main", "dev"]})));
    r.push("Q", |b| {
        assert_eq!(last_result(b), "dev", "the answer reaches the same call");
        text("using dev")
    });
    let t = r.start_task("Q", "x").await;
    r.until(&t.id, "the question", |t| t.wait.as_ref().is_some_and(|w| w.0["kind"] == "question")).await;
    let started = std::time::Instant::now();
    reagent::stop(&r.run).await;
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "a task waiting for the person doesn't hold the stop up");
    let notified = r.run.app.store.notifications(100).await.unwrap().iter().filter(|n| n.kind == "waiting").count();

    let run = again(&r).await;
    let w = until_state(&run, &t.id, "the question again", |t| t.state == "waiting" && t.wait.as_ref().is_some_and(|w| w.0["kind"] == "question")).await;
    assert_eq!(w.wait.unwrap().0["options"][1], "dev");
    run.app.answer(&t.id, "dev").await.unwrap();
    let d = until_state(&run, &t.id, "done", |t| t.state == "done").await;
    assert_eq!(d.report.as_deref(), Some("using dev"));
    assert_eq!(run.app.store.notifications(100).await.unwrap().iter().filter(|n| n.kind == "waiting").count(), notified, "not announced twice");
}

#[tokio::test]
async fn an_answer_given_while_nothing_waits_is_kept_for_the_question() {
    let r = start().await;
    r.push("Kept", |_| call("c1", "ask.ask", json!({"question": "go?"})));
    r.push("Kept", |b| {
        assert_eq!(last_result(b), "yes");
        text("went")
    });
    let t = r.start_task("Kept", "x").await;
    r.until(&t.id, "the question", |t| t.wait.as_ref().is_some_and(|w| w.0["kind"] == "question")).await;
    reagent::stop(&r.run).await;
    // Answered while reagent is down (straight into the store, as the web UI would right after a start).
    let mut w = r.run.app.task(&t.id).await.unwrap().wait.unwrap().0;
    w["answer"] = json!("yes");
    r.run.app.store.set_state(&t.id, "waiting", Some(&w)).await.unwrap();
    let run = again(&r).await;
    let d = until_state(&run, &t.id, "done", |t| t.state == "done").await;
    assert_eq!(d.report.as_deref(), Some("went"));
}

#[tokio::test]
async fn a_merge_approved_after_a_restart_is_made() {
    let r = start().await;
    r.allow_all().await;
    r.push("M", |_| call("c1", "git.worktree_start", json!({})));
    r.push("M", |_| call("c2", "fs.write", json!({"path": "m.txt", "text": "m\n"})));
    r.push("M", |_| call("c3", "shell.exec", json!({"cmd": "git add -A && git commit -qm m"})));
    r.push("M", |_| call("c4", "git.worktree_merge", json!({})));
    r.push("M", |b| {
        assert!(last_result(b).contains("merged "), "{}", last_result(b));
        text("merged")
    });
    let t = r.start_task("M", "x").await;
    r.until(&t.id, "the merge", |t| t.wait.as_ref().is_some_and(|w| w.0["kind"] == "merge")).await;
    reagent::stop(&r.run).await;
    let run = again(&r).await;
    until_state(&run, &t.id, "the merge again", |t| t.state == "waiting" && t.wait.as_ref().is_some_and(|w| w.0["kind"] == "merge")).await;
    run.app.decide_merge(&t.id, reagent_tools::app::MergeAnswer::Merge).await.unwrap();
    until_state(&run, &t.id, "done", |t| t.state == "done").await;
    assert_eq!(std::fs::read_to_string(r.project.path().join("m.txt")).unwrap(), "m\n");
}
