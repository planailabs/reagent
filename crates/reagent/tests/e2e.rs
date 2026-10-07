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
