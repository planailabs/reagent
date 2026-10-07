//! reagent end to end: tasks on subnet against a scripted model, with the
//! real tools, hooks, supervisor, git and memory.

mod common;

use common::*;
use serde_json::json;

#[tokio::test]
async fn a_task_reads_edits_runs_and_reports() {
    let r = start().await;
    r.push("Fix", |_| call("c1", "fs.read", json!({"path": "a.txt"})));
    r.push("Fix", |b| {
        assert!(last_result(b).contains("1│hello world"), "{}", last_result(b));
        call("c2", "fs.edit", json!({"path": "a.txt", "old": "world", "new": "reagent"}))
    });
    r.push("Fix", |_| call("c3", "shell.exec", json!({"cmd": "cat a.txt"})));
    r.push("Fix", |b| {
        assert!(last_result(b).starts_with("exit 0") && last_result(b).contains("hello reagent"), "{}", last_result(b));
        text("Changed a.txt; cat shows it.")
    });
    let t = r.start_task("Fix", "change world to reagent in a.txt").await;
    let t = r.done(&t.id).await;
    assert_eq!(t.report.as_deref(), Some("Changed a.txt; cat shows it."));
    assert_eq!(std::fs::read_to_string(r.project.path().join("a.txt")).unwrap(), "hello reagent\n");
    assert!(t.tokens > 0 && t.cost > 0.0, "usage counted: {t:?}");
    // The first call was given the context: the memory indexes.
    let first = &r.requests("Fix")[0];
    assert!(all_text(first).contains("Project memory index"), "context injected");
    let n = r.run.app.store.notifications(10).await.unwrap();
    assert!(n.iter().any(|n| n.kind == "done" && n.task.as_deref() == Some(t.id.as_str())));
}

#[tokio::test]
async fn a_call_the_policy_asks_about_waits_for_approval_and_always_allow_adds_a_rule() {
    let r = start().await;
    r.push("Touch", |_| call("c1", "shell.exec", json!({"cmd": "touch made.txt"})));
    r.push("Touch", |b| {
        assert!(last_result(b).starts_with("exit 0"), "{}", last_result(b));
        call("c2", "shell.exec", json!({"cmd": "touch made.txt"}))
    });
    r.push("Touch", |_| text("touched"));
    let t = r.start_task("Touch", "make two files").await;
    let w = r.until(&t.id, "approval", |t| t.state == "waiting").await;
    let wait = w.wait.unwrap().0;
    assert_eq!(wait["kind"], "approval", "{wait}");
    let call_id = wait["call"]["id"].as_str().unwrap().to_string();
    assert!(!r.project.path().join("made.txt").exists(), "nothing ran yet");
    r.run.app.approve(&t.id, &call_id, true, true).await.unwrap();
    r.done(&t.id).await;
    assert!(r.project.path().join("made.txt").exists());
    assert_eq!(r.requests("Touch").len(), 3, "the second touch ran without asking");
    let rules = r.run.app.store.rules("site").await.unwrap();
    assert_eq!((rules[0].tool.as_str(), rules[0].command.as_deref()), ("shell.exec", Some("touch made.txt")), "always allow: a rule in front");
}

#[tokio::test]
async fn a_denied_call_says_why() {
    let r = start().await;
    r.push("Danger", |_| call("c1", "shell.exec", json!({"cmd": "cargo build && rm -rf /tmp/x"})));
    r.push("Danger", |b| {
        assert!(last_result(b).contains("denied") || all_text(b).contains("not allowed"), "{}", all_text(b));
        text("can't")
    });
    let t = r.start_task("Danger", "x").await;
    r.done(&t.id).await;
}

#[tokio::test]
async fn a_background_job_ending_wakes_its_task() {
    let r = start().await;
    r.push("Bg", |_| call("c1", "shell.exec_bg", json!({"cmd": "sleep 0.5; echo finished-now", "name": "slow"})));
    r.push("Bg", |b| {
        assert!(last_result(b).starts_with("started job j"), "{}", last_result(b));
        text("waiting for it")
    });
    r.push("Bg", |b| {
        let all = all_text(b);
        assert!(all.contains("exited with 0") && all.contains("finished-now"), "{all}");
        text("it finished")
    });
    r.allow_all().await;
    let t = r.start_task("Bg", "run it in the background").await;
    r.until(&t.id, "the second report", |t| t.report.as_deref() == Some("it finished")).await;
}

#[tokio::test]
async fn a_foreground_command_past_its_timeout_goes_on_in_the_background() {
    let r = start().await;
    r.push("Slow", |_| call("c1", "shell.exec", json!({"cmd": "ls; sleep 1.5; echo late", "timeout": 1})));
    r.push("Slow", |b| {
        assert!(last_result(b).contains("goes on in the background as job"), "{}", last_result(b));
        text("moved on")
    });
    r.push("Slow", |b| {
        assert!(all_text(b).contains("late"), "the job's end comes as a message");
        text("done now")
    });
    r.allow_all().await;
    let t = r.start_task("Slow", "x").await;
    r.until(&t.id, "the job's end", |t| t.report.as_deref() == Some("done now")).await;
}

#[tokio::test]
async fn a_worktree_is_merged_once_the_person_approves() {
    let r = start().await;
    r.push("Feature", |_| call("c1", "git.worktree_start", json!({})));
    r.push("Feature", |b| {
        assert!(last_result(b).contains("on branch reagent/feature"), "{}", last_result(b));
        call("c2", "fs.write", json!({"path": "feature.txt", "text": "new\n"}))
    });
    r.push("Feature", |_| call("c3", "shell.exec", json!({"cmd": "git add -A && git commit -qm feature"})));
    r.push("Feature", |b| {
        assert!(last_result(b).starts_with("exit 0"), "{}", last_result(b));
        call("c4", "git.worktree_merge", json!({}))
    });
    r.push("Feature", |b| {
        assert!(last_result(b).contains("merged reagent/feature"), "{}", last_result(b));
        call("c5", "git.worktree_drop", json!({}))
    });
    r.push("Feature", |_| text("merged"));
    let t = r.start_task("Feature", "add a feature in a worktree").await;
    let w = r.until(&t.id, "the merge to wait", |t| t.wait.as_ref().is_some_and(|w| w.0["kind"] == "merge")).await;
    assert!(w.wait.unwrap().0["stat"].as_str().unwrap().contains("feature.txt"));
    assert!(!r.project.path().join("feature.txt").exists(), "not merged before the approval");
    r.run.app.decide_merge(&t.id, reagent_tools::app::MergeAnswer::Merge).unwrap();
    let t = r.done(&t.id).await;
    assert_eq!(std::fs::read_to_string(r.project.path().join("feature.txt")).unwrap(), "new\n");
    assert!(t.worktree.is_none() && t.cwd == r.project.path().canonicalize().unwrap().display().to_string(), "back in the project");
}

#[tokio::test]
async fn a_question_waits_for_the_answer() {
    let r = start().await;
    r.push("Q", |_| call("c1", "ask.ask", json!({"question": "red or blue?", "options": ["red", "blue"]})));
    r.push("Q", |b| {
        assert_eq!(last_result(b), "blue");
        text("blue it is")
    });
    let t = r.start_task("Q", "pick").await;
    let w = r.until(&t.id, "the question", |t| t.wait.as_ref().is_some_and(|w| w.0["kind"] == "question")).await;
    assert_eq!(w.wait.unwrap().0["options"][1], "blue");
    r.run.app.answer(&t.id, "blue").await.unwrap();
    r.done(&t.id).await;
}

#[tokio::test]
async fn memory_written_shows_in_the_next_context() {
    let r = start().await;
    r.push("Mem", |_| call("c1", "memory.memory_write", json!({"scope": "project", "file": "topics/build.md", "text": "cargo test", "about": "how to build and test"})));
    r.push("Mem", |b| {
        let all = all_text(b);
        assert!(all.contains("[from hook context]") && all.contains("- [build](topics/build.md) — how to build and test"), "{all}");
        call("c2", "memory.memory_search", json!({"pattern": "CARGO"}))
    });
    r.push("Mem", |b| {
        assert!(last_result(b).contains("project topics/build.md:1: cargo test"), "{}", last_result(b));
        text("remembered")
    });
    let t = r.start_task("Mem", "note how to build").await;
    r.done(&t.id).await;
}

#[tokio::test]
async fn a_subtask_reports_to_its_parent_and_histories_are_searchable_across_tasks() {
    let r = start().await;
    r.push("Parent", |_| call("c1", "tasks.task_spawn", json!({"title": "Child", "prompt": "say the word"})));
    r.push("Parent", |b| {
        assert!(last_result(b).starts_with("started subtask"), "{}", last_result(b));
        text("waiting for the child")
    });
    r.push("Child", |_| text("the word is zebra-42"));
    r.push("Parent", |b| {
        let all = all_text(b);
        assert!(all.contains("[subtask Child") && all.contains("zebra-42"), "{all}");
        call("c2", "tasks.search_history", json!({"pattern": "zebra-\\d+"}))
    });
    r.push("Parent", |b| {
        let res = last_result(b);
        assert!(res.contains("task Child") && res.contains("zebra-42"), "{res}");
        text("found it")
    });
    let t = r.start_task("Parent", "delegate").await;
    let t = r.until(&t.id, "the parent's last report", |t| t.report.as_deref() == Some("found it")).await;
    let kids = r.run.app.store.tasks(None, Some(&t.id), false, 10).await.unwrap();
    assert_eq!((kids.len(), kids[0].state.as_str(), kids[0].origin.starts_with("task:")), (1, "done", true));
}

#[tokio::test]
async fn cron_starts_a_run_when_due() {
    let r = start().await;
    r.push("Nightly", |_| text("checked"));
    let mut c: reagent_store::Cron = serde_json::from_value(json!({"expr": "0 3 * * *", "title": "Nightly", "prompt": "check things"})).unwrap();
    c.project = "site".into();
    c.next_run = Some(chrono_now() - 10);
    let id = r.run.app.store.put_cron(&c).await.unwrap();
    reagent_tools::cron::tick(&r.run.app, chrono_now()).await;
    let tasks = r.run.app.store.tasks(Some("site"), None, false, 10).await.unwrap();
    let t = tasks.iter().find(|t| t.origin == format!("cron:{id}")).expect("a run started");
    r.done(&t.id).await;
    let c = r.run.app.store.cron(id).await.unwrap().unwrap();
    assert!(c.last_run.is_some() && c.next_run.unwrap() > chrono_now(), "{c:?}");
}

#[tokio::test]
async fn a_task_over_budget_is_paused() {
    let r = start().await;
    r.push("Big", |_| call("c1", "fs.ls", json!({})));
    r.push("Big", |_| call("c2", "fs.ls", json!({})));
    r.push("Big", |_| text("never"));
    let t = r
        .run
        .app
        .start_task(reagent_tools::app::StartTask { project: "site".into(), title: "Big".into(), prompt: "x".into(), budget: Some(reagent_store::Budget { tokens: Some(100), ..Default::default() }), ..Default::default() })
        .await
        .unwrap();
    let w = r.until(&t.id, "the budget stop", |t| t.wait.as_ref().is_some_and(|w| w.0["kind"] == "budget")).await;
    assert!(w.tokens > 100);
    let n = r.run.app.store.notifications(10).await.unwrap();
    assert!(n.iter().any(|n| n.kind == "budget"));
}

#[tokio::test]
async fn files_outside_the_project_are_refused() {
    let r = start().await;
    r.push("Out", |_| call("c1", "fs.read", json!({"path": "/etc/hostname"})));
    r.push("Out", |b| {
        assert!(last_result(b).contains("outside the project"), "{}", last_result(b));
        call("c2", "fs.read", json!({"path": "../../etc/hostname"}))
    });
    r.push("Out", |b| {
        assert!(last_result(b).contains("outside the project"), "{}", last_result(b));
        text("ok")
    });
    let t = r.start_task("Out", "x").await;
    r.done(&t.id).await;
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

#[tokio::test]
async fn a_failed_task_is_retried_where_it_failed() {
    let r = start().await;
    // No reply scripted: the model call fails, and so does the task.
    let t = r.start_task("Flaky", "x").await;
    let f = r.until(&t.id, "the failure", |t| t.state == "failed").await;
    assert!(f.report.is_some_and(|r| !r.is_empty()), "the failure is the report");
    assert!(r.run.app.retry("nope").await.is_err());
    r.push("Flaky", |_| text("worked this time"));
    let t2 = r.run.app.retry(&t.id).await.unwrap();
    assert_eq!(t2.state, "running");
    let d = r.done(&t.id).await;
    assert_eq!(d.report.as_deref(), Some("worked this time"));
    assert!(r.run.app.retry(&t.id).await.unwrap_err().contains("isn't failed"));
}

#[tokio::test]
async fn raising_the_budget_lets_a_paused_task_go_on() {
    let r = start().await;
    r.push("Tight", |_| call("c1", "fs.ls", json!({})));
    r.push("Tight", |_| text("finished after all"));
    let t = r
        .run
        .app
        .start_task(reagent_tools::app::StartTask { project: "site".into(), title: "Tight".into(), prompt: "x".into(), budget: Some(reagent_store::Budget { tokens: Some(100), ..Default::default() }), ..Default::default() })
        .await
        .unwrap();
    let w = r.until(&t.id, "the budget stop", |t| t.wait.as_ref().is_some_and(|w| w.0["kind"] == "budget")).await;
    assert!(r.run.app.raise_budget(&t.id, &reagent_store::Budget { tokens: Some(10), ..Default::default() }).await.unwrap_err().contains("still over"), "{w:?}");
    let raised = r.run.app.raise_budget(&t.id, &reagent_store::Budget { tokens: Some(10_000), ..Default::default() }).await.unwrap();
    assert_eq!(raised.budget.tokens, Some(10_110));
    let d = r.done(&t.id).await;
    assert_eq!(d.report.as_deref(), Some("finished after all"));
}
