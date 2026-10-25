//! The call evaluator: calls the rules would ask about judged by a second
//! model (approved, refused, or sent to the person with its verdict), with
//! the call's `_reason` and the instructions' extensions; static checks.

mod common;

use common::*;
use serde_json::{Value, json};

const EXTRA: &str = "evaluator_profile = \"default\"\n";

async fn evaluations(r: &R, task: &str) -> Vec<reagent_store::Evaluation> {
    r.run.app.store.evaluations(task, 50).await.unwrap()
}

#[tokio::test]
async fn the_evaluator_lets_a_call_run_or_refuses_it_with_the_reason_and_the_extensions() {
    let r = start_in(tempfile::tempdir().unwrap(), EXTRA).await;
    r.run.app.store.set_setting("evaluator_prompt", "Never let anything touch production.").await.unwrap();
    let mut p = r.run.app.project("site").await.unwrap();
    p.evaluator.0.prompt = "This repo is a static site.".into();
    r.run.app.put_project(p).await.unwrap();
    // touch has no rule: it would ask the person; the evaluator approves.
    r.push("Judged", |b| {
        assert!(b["tools"].as_array().unwrap().iter().all(|t| t["function"]["parameters"]["properties"]["_reason"].is_object()), "every tool takes a reason");
        call("c1", "shell.exec", json!({"cmd": "touch made.txt", "_reason": "make the file the task asks for"}))
    });
    r.push("evaluator", |b| {
        let system = b["messages"][0]["content"].as_str().unwrap();
        assert!(system.contains("Never let anything touch production.") && system.contains("This repo is a static site."), "{system}");
        let user = b["messages"][1]["content"].as_str().unwrap();
        assert!(user.contains("touch made.txt") && user.contains("make the file the task asks for"), "{user}");
        text("APPROVE: it's what the task asks")
    });
    r.push("Judged", |b| {
        assert!(!last_result(b).contains("refused") && last_result(b).contains("[evaluator] approved: it's what the task asks"), "{}", last_result(b));
        call("c2", "shell.exec", json!({"cmd": "curl https://prod.example/reset", "_reason": "reset production"}))
    });
    r.push("evaluator", |_| text("REJECT: production is out of bounds"));
    r.push("Judged", |b| {
        assert!(last_result(b).contains("[evaluator refused] production is out of bounds"), "{}", last_result(b));
        text("done")
    });
    let t = r.start_task("Judged", "make made.txt").await;
    for _ in 0..200 {
        if r.task(&t.id).await.state != "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(r.task(&t.id).await.state, "done", "{:?}", evaluations(&r, &t.id).await);
    assert!(r.project.path().join("made.txt").exists());
    let ev = evaluations(&r, &t.id).await;
    assert_eq!(ev.iter().map(|e| (e.outcome.as_str(), e.approved)).collect::<Vec<_>>(), [("refused", false), ("allowed", true)]);
    assert_eq!(ev[1].reason, "make the file the task asks for");
    assert!(r.task(&t.id).await.cost > 0.0);
}

#[tokio::test]
async fn advising_the_evaluator_goes_to_the_person_with_its_verdict_and_so_does_an_error() {
    let r = start_in(tempfile::tempdir().unwrap(), EXTRA).await;
    let mut p = r.run.app.project("site").await.unwrap();
    p.evaluator.0.role = Some("advises".into());
    r.run.app.put_project(p).await.unwrap();
    r.push("Advised", |_| call("c1", "shell.exec", json!({"cmd": "touch a.txt"})));
    r.push("evaluator", |_| text("REJECT: not needed"));
    r.push("Advised", |b| {
        assert!(all_text(b).contains("[the user approved call c1: scratch files are fine]"), "the person's reason");
        call("c2", "shell.exec", json!({"cmd": "touch b.txt"}))
    });
    // No scripted answer for the second: the evaluator fails, the person decides.
    r.push("Advised", |b| {
        assert_eq!(last_result(b), "[denied by user: one is enough]");
        text("done")
    });
    let t = r.start_task("Advised", "x").await;
    let w = r.until(&t.id, "waiting for approval", |t| t.wait.as_ref().is_some_and(|w| w.0["kind"] == "approval")).await;
    let e = &w.wait.unwrap().0["evaluation"];
    assert_eq!((e["approved"].as_bool(), e["reasoning"].as_str(), e["outcome"].as_str()), (Some(false), Some("not needed"), Some("asked")), "{e}");
    r.run.app.approve(&t.id, "c1", true, false, Some("scratch files are fine".into())).await.unwrap();
    let w = r.until(&t.id, "the second approval", |t| t.wait.as_ref().is_some_and(|w| w.0["call"]["id"] == "c2")).await;
    assert_eq!(w.wait.unwrap().0["evaluation"]["source"], "error");
    r.run.app.approve(&t.id, "c2", false, false, Some("one is enough".into())).await.unwrap();
    r.done(&t.id).await;
}

#[tokio::test]
async fn static_checks_refuse_or_warn_about_a_repeated_call() {
    let r = start().await;
    let write = || json!({"path": "n.txt", "content": "x"});
    r.push("Twice", move |_| call("c1", "fs.write", write()));
    r.push("Twice", move |_| call("c2", "fs.write", write()));
    r.push("Twice", |b| {
        assert!(last_result(b).contains("[evaluator refused] the same fs.write call was made just now"), "{}", last_result(b));
        text("ok")
    });
    let t = r.start_task("Twice", "x").await;
    r.done(&t.id).await;
    assert_eq!(evaluations(&r, &t.id).await[0].source, "static");
    // Warn: it runs, and the task reads why.
    let mut p = r.run.app.project("site").await.unwrap();
    p.evaluator.0.static_checks = Some("warn".into());
    r.run.app.put_project(p).await.unwrap();
    r.push("Warned", move |_| call("c1", "fs.write", write()));
    r.push("Warned", move |_| call("c2", "fs.write", write()));
    r.push("Warned", |b| {
        assert!(last_result(b).contains("[evaluator] the same fs.write call was made just now") && !last_result(b).contains("refused"), "{}", last_result(b));
        text("ok")
    });
    let t = r.start_task("Warned", "x").await;
    r.done(&t.id).await;
    let bad: Value = json!({"role": "sometimes"});
    let mut p = r.run.app.project("site").await.unwrap();
    p.evaluator.0 = serde_json::from_value(bad).unwrap();
    assert!(r.run.app.put_project(p).await.unwrap_err().contains("evaluator role"));
}
