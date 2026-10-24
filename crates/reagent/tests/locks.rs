//! Named locks: tasks waiting for, skipping or failing on a busy lock;
//! taken and let go while running; a deadlock refused; the person breaking one.

mod common;

use common::*;
use reagent_tools::app::StartTask;
use serde_json::json;

fn with_locks(title: &str, locks: &[&str], on_busy: Option<&str>) -> StartTask {
    StartTask { project: "site".into(), title: title.into(), prompt: "x".into(), locks: locks.iter().map(|l| l.to_string()).collect(), on_busy: on_busy.map(Into::into), ..Default::default() }
}

#[tokio::test]
async fn a_busy_lock_queues_skips_or_fails_a_task_and_its_end_lets_the_next_go() {
    let r = start().await;
    // The holder waits for an answer, keeping its lock.
    r.push("Holder", |b| {
        assert!(all_text(b).contains("Locks your task tree holds: deploy"));
        call("q1", "ask.ask", json!({"question": "go on?"}))
    });
    r.push("Holder", |_| text("deployed"));
    let holder = r.run.app.start_task(with_locks("Holder", &["deploy"], None)).await.unwrap();
    r.until(&holder.id, "waiting", |t| t.state == "waiting").await;

    let next = r.run.app.start_task(with_locks("Next", &["deploy"], None)).await.unwrap();
    assert_eq!(next.state, "queued");
    assert_eq!(next.wait.as_ref().unwrap().0["kind"], "lock");
    assert!(next.wait.as_ref().unwrap().0["busy"].as_str().unwrap().contains("Holder"), "{:?}", next.wait);
    let e = r.run.app.start_task(with_locks("Skip", &["deploy"], Some("skip"))).await.unwrap_err();
    assert!(e.starts_with("skipped:") && e.contains("deploy"), "{e}");
    let failed = r.run.app.start_task(with_locks("Fail", &["deploy"], Some("fail"))).await.unwrap();
    assert_eq!(failed.state, "failed");
    assert!(failed.report.unwrap().contains("deploy, held by task Holder"));
    // Shared with another lock name, and a shared one beside a shared one: no wait.
    r.push("Reader", |_| text("read"));
    let reader = r.run.app.start_task(with_locks("Reader", &["db:shared"], None)).await.unwrap();
    assert_ne!(reader.state, "queued");
    r.done(&reader.id).await;
    let e = r.run.app.start_now(&next.id).await.unwrap_err();
    assert!(e.contains("waits for a lock"), "{e}");

    // The holder ends: its lock goes, and the waiting task starts.
    r.push("Next", |_| text("next done"));
    r.run.app.answer(&holder.id, "yes").await.unwrap();
    r.done(&holder.id).await;
    r.done(&next.id).await;
    // Let go within the follower's round.
    for _ in 0..100 {
        if r.run.app.store.locks(Some("site")).await.unwrap().is_empty() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("still held: {:?}", r.run.app.store.locks(Some("site")).await.unwrap());
}

#[tokio::test]
async fn a_task_takes_and_releases_locks_and_a_deadlock_is_refused() {
    let r = start().await;
    // A takes a and waits for the person; B takes b and waits for a.
    r.push("A", |_| call("a1", "locks.lock_acquire", json!({"name": "a"})));
    r.push("A", |b| {
        assert!(last_result(b).contains("you hold a now"), "{}", last_result(b));
        call("a2", "ask.ask", json!({"question": "go on?"}))
    });
    // Then A asks for b: that would close the cycle, so it's refused; A lets a go.
    r.push("A", |_| call("a3", "locks.lock_acquire", json!({"name": "b", "timeout": 30})));
    r.push("A", |b| {
        assert!(last_result(b).contains("deadlock"), "{}", last_result(b));
        call("a4", "locks.lock_release", json!({"name": "a"}))
    });
    r.push("A", |b| {
        assert!(last_result(b).contains("released a"), "{}", last_result(b));
        text("A done")
    });
    r.push("B", |_| call("b1", "locks.lock_acquire", json!({"name": "b"})));
    r.push("B", |b| {
        assert!(last_result(b).contains("you hold b now"), "{}", last_result(b));
        call("b2", "locks.lock_acquire", json!({"name": "a", "wait": false}))
    });
    r.push("B", |b| {
        assert!(last_result(b).contains("not taken") && last_result(b).contains("task A"), "{}", last_result(b));
        call("b3", "locks.lock_acquire", json!({"name": "a", "timeout": 30}))
    });
    r.push("B", |b| {
        assert!(last_result(b).contains("you hold a now"), "{}", last_result(b));
        text("B done")
    });
    let a = r.start_task("A", "x").await;
    r.until(&a.id, "A asking", |t| t.state == "waiting").await;
    let b = r.start_task("B", "x").await;
    let mut listed = String::new();
    for _ in 0..200 {
        listed = reagent_tools::locks::list(&r.run.app, "site").await.unwrap().to_string();
        if listed.contains("\"waiting\":[{") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(listed.contains("\"waiting\":[{") && listed.contains("task B"), "{listed}");
    r.run.app.answer(&a.id, "yes").await.unwrap();
    r.done(&a.id).await;
    r.done(&b.id).await;
}

#[tokio::test]
async fn the_person_breaks_a_lock_and_its_holder_is_told() {
    let r = start().await;
    r.push("Stuck", |_| call("q1", "ask.ask", json!({"question": "wait?"})));
    let t = r.run.app.start_task(with_locks("Stuck", &["global:staging"], None)).await.unwrap();
    r.until(&t.id, "waiting", |t| t.state == "waiting").await;
    let rows = r.run.app.store.locks(Some("site")).await.unwrap();
    assert_eq!((rows[0].project.as_str(), rows[0].name.as_str()), ("", "staging"));
    let broken = reagent_tools::locks::break_lock(&r.run.app, "", "staging").await.unwrap();
    assert_eq!(broken, [format!("task:{}", t.id)]);
    assert!(r.run.app.store.locks(None).await.unwrap().is_empty());
    assert!(r.run.app.store.outbox(&t.id).await.unwrap().iter().any(|m| m.text.contains("broke your lock global:staging")));
}

#[tokio::test]
async fn a_trigger_run_skips_or_fails_while_its_lock_is_busy_and_runs_once_free() {
    let r = start().await;
    r.allow_all().await;
    r.push("Holder", |_| call("q1", "ask.ask", json!({"question": "go on?"})));
    r.push("Holder", |_| text("ok"));
    let holder = r.run.app.start_task(with_locks("Holder", &["deploy"], None)).await.unwrap();
    r.until(&holder.id, "waiting", |t| t.state == "waiting").await;
    let p = r.run.app.project("site").await.unwrap();
    let trig = |on_busy: &str| {
        serde_json::from_value::<reagent_store::Trigger>(json!({"name": "ci", "mode": "poll", "every": 3600, "title": "Deploy {{key}}", "prompt": "deploy", "script": "echo '{\"key\": \"1\"}'", "options": {"locks": ["deploy"], "on_busy": on_busy}})).unwrap()
    };
    // skip: no run, no task.
    reagent_tools::triggers::save(&r.run.app.store, &p, trig("skip"), true).await.unwrap();
    for _ in 0..40 {
        if r.run.app.store.trigger("site", "ci").await.unwrap().unwrap().state.0.next_run.is_some_and(|n| n > 0) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(r.run.app.store.trigger_runs("site", "ci").await.unwrap().is_empty());
    // fail: a failed run that says why.
    reagent_tools::triggers::save(&r.run.app.store, &p, trig("fail"), true).await.unwrap();
    reagent_tools::triggers::run_now(&r.run.app, "site", "ci").await.unwrap();
    let mut runs = vec![];
    for _ in 0..60 {
        runs = r.run.app.store.trigger_runs("site", "ci").await.unwrap();
        if !runs.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(!runs[0].ok && runs[0].error.as_deref().unwrap_or_default().contains("deploy, held by task Holder"), "{runs:?}");
    // Free: the run goes, and its task takes the lock in turn.
    r.push("Deploy 1", |b| {
        assert!(all_text(b).contains("Locks your task tree holds: deploy"));
        text("deployed")
    });
    r.run.app.answer(&holder.id, "yes").await.unwrap();
    r.done(&holder.id).await;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    reagent_tools::triggers::run_now(&r.run.app, "site", "ci").await.unwrap();
    for _ in 0..100 {
        if let Some(t) = r.run.app.store.tasks(Some("site"), None, false, 10).await.unwrap().into_iter().find(|t| t.title == "Deploy 1") {
            r.done(&t.id).await;
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("no task from the trigger: {:?}", r.run.app.store.trigger_runs("site", "ci").await.unwrap());
}
