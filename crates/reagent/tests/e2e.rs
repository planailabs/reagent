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
    r.push("Fix", |_| call("c3", "shell.exec", json!({"cmd": "cat a.txt; echo \"path=$PATH\""})));
    r.push("Fix", |b| {
        assert!(last_result(b).starts_with("exit 0") && last_result(b).contains("hello reagent"), "{}", last_result(b));
        let path = std::env::var("PATH").unwrap();
        let first = path.split(':').next().unwrap();
        assert!(last_result(b).contains(&format!("path={first}")), "commands get reagent's PATH: {}", &last_result(b)[..last_result(b).len().min(600)]);
        text("Changed a.txt; cat shows it.")
    });
    r.allow_all().await;
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
    r.run.app.decide_merge(&t.id, reagent_tools::app::MergeAnswer::Merge).await.unwrap();
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

#[tokio::test]
async fn a_task_keeps_a_todo_list_in_view() {
    let r = start().await;
    r.push("Plan", |b| {
        assert!(tool_names(b).contains(&"todo__todo_add".into()), "the todo tools are there");
        call("c1", "todo.todo_add", json!({"items": ["read the code", "fix the bug", "run the tests"]}))
    });
    r.push("Plan", |b| {
        assert!(last_result(b).starts_with("[ ] 1 read the code\n[ ] 2 fix the bug\n[ ] 3 run the tests\n(3 open of 3)"), "{}", last_result(b));
        // Shown in the context, as it is now.
        assert!(all_text(b).contains("Your todo list"), "{}", all_text(b));
        call("c2", "todo.todo_update", json!({"id": 1, "status": "done"}))
    });
    r.push("Plan", |_| call("c3", "todo.todo_update", json!({"id": 2, "status": "in_progress", "text": "fix the off-by-one"})));
    r.push("Plan", |b| {
        assert!(last_result(b).contains("[x] 1 read the code\n[~] 2 fix the off-by-one"), "{}", last_result(b));
        let shown = all_text(b);
        assert!(shown.contains("[~] 2 fix the off-by-one"), "the changed list is shown again");
        call("c4", "todo.todo_update", json!({"id": 7, "status": "done"}))
    });
    r.push("Plan", |b| {
        assert!(last_result(b).contains("no item 7"), "{}", last_result(b));
        call("c5", "todo.todo_update", json!({"id": 3, "status": "maybe"}))
    });
    r.push("Plan", |b| {
        assert!(last_result(b).contains("pending, in_progress, done or cancelled"), "{}", last_result(b));
        text("planned")
    });
    let t = r.start_task("Plan", "x").await;
    r.done(&t.id).await;
    let todos = r.run.app.store.todos(&t.id).await.unwrap();
    assert_eq!(todos.iter().map(|x| x.status.as_str()).collect::<Vec<_>>(), ["done", "in_progress", "pending"]);
}

#[tokio::test]
async fn a_task_keeps_a_working_memory() {
    let r = start().await;
    r.push("Remember", |b| {
        assert!(tool_names(b).contains(&"wm__wm_get".into()), "the wm tools are there");
        assert!(all_text(b).contains("after your history was summarised"), "the prompt says when to read it");
        call("c1", "wm.wm_set", json!({"key": "pr", "value": 42}))
    });
    r.push("Remember", |_| call("c2", "wm.wm_set", json!({"key": "plan", "value": {"step": 2, "left": ["test"]}})));
    r.push("Remember", |_| call("c3", "wm.wm_set", json!({"key": "pr", "value": "#43"})));
    r.push("Remember", |_| call("c4", "wm.wm_get", json!({})));
    r.push("Remember", |b| {
        let got: serde_json::Value = serde_json::from_str(&last_result(b)).unwrap();
        assert_eq!(got, json!({"plan": {"step": 2, "left": ["test"]}, "pr": "#43"}));
        call("c5", "wm.wm_remove", json!({"key": "plan"}))
    });
    r.push("Remember", |_| call("c6", "wm.wm_remove", json!({"key": "plan"})));
    r.push("Remember", |b| {
        assert!(last_result(b).contains("no slot"), "{}", last_result(b));
        text("remembered")
    });
    let t = r.start_task("Remember", "x").await;
    r.done(&t.id).await;
    assert_eq!(serde_json::Value::Object(r.run.app.store.working_memory(&t.id).await.unwrap()), json!({"pr": "#43"}));
    let other = r.start_task("Other", "x").await;
    assert!(r.run.app.store.working_memory(&other.id).await.unwrap().is_empty(), "each task its own");
}

#[tokio::test]
async fn a_task_sleeps_and_a_message_wakes_it() {
    let r = start().await;
    r.push("Nap", |_| call("c1", "shell.sleep", json!({"duration": "1s"})));
    r.push("Nap", |b| {
        assert_eq!(last_result(b), "slept 1s");
        call("c2", "shell.sleep", json!({"duration": "1h"}))
    });
    r.push("Nap", |b| {
        assert!(last_result(b).starts_with("woken by a message after"), "{}", last_result(b));
        text("up")
    });
    let t = r.start_task("Nap", "x").await;
    // Into the long sleep (the short one's 1s, then a moment), then a message.
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    r.run.app.message(&t.id, "wake up").await.unwrap();
    r.done(&t.id).await;
}

fn tool_names(body: &serde_json::Value) -> Vec<String> {
    body["tools"].as_array().map(|t| t.iter().map(|t| t["function"]["name"].as_str().unwrap_or("").to_string()).collect()).unwrap_or_default()
}

#[tokio::test]
async fn rules_come_into_view_always_by_the_files_touched_or_listed() {
    let r = start().await;
    let rules = r.project.path().join(".agents/rules");
    std::fs::create_dir_all(rules.join("lang")).unwrap();
    std::fs::write(rules.join("tone.md"), "Write plainly.").unwrap();
    std::fs::write(rules.join("lang/txt.mdc"), "---\nglobs: \"*.txt\"\n---\nText files end with a newline.").unwrap();
    std::fs::write(rules.join("release.md"), "---\ndescription: How we release\n---\nTag it.").unwrap();
    r.push("Ruled", |b| {
        let all = all_text(b);
        assert!(all.contains("Write plainly.") && !all.contains("end with a newline"), "{all}");
        assert!(all.contains("rules to read") && all.contains("release (") && !all.contains("Tag it."), "{all}");
        call("c1", "fs.read", json!({"path": "a.txt"}))
    });
    r.push("Ruled", |b| {
        let all = all_text(b);
        assert!(all.contains("Rule lang/txt") && all.contains("Text files end with a newline."), "touched a.txt: {all}");
        assert_eq!(all.matches("Write plainly.").count(), 1, "shown once");
        text("read")
    });
    r.allow_all().await;
    let t = r.start_task("Ruled", "read a.txt").await;
    r.done(&t.id).await;
}

#[tokio::test]
async fn a_plugins_skills_and_rules_reach_tasks() {
    let r = start().await;
    let src = r.data.path().join("plugin-src");
    std::fs::create_dir_all(src.join("skill/greet")).unwrap();
    std::fs::create_dir_all(src.join(".agents/rules")).unwrap();
    std::fs::write(src.join("skill/greet/SKILL.md"), "---\nname: greet\ndescription: say hi nicely\n---\nHi.").unwrap();
    std::fs::write(src.join(".agents/rules/tone.md"), "Keep it short.").unwrap();
    let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(&src).args(args).status().unwrap().success());
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "-A"]);
    git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "one"]);
    let p = r.run.app.plugin_add(&format!("file://{}", src.display()), Some("kit"), None).await.unwrap();
    assert_eq!((p.skills.clone(), p.rules.clone()), (vec!["greet".to_string()], vec!["tone".to_string()]));
    r.push("Plugged", |b| {
        let all = all_text(b);
        assert!(all.contains("greet — say hi nicely") && all.contains("Keep it short."), "{all}");
        text("ok")
    });
    let t = r.start_task("Plugged", "x").await;
    r.done(&t.id).await;
    let up = r.run.app.plugin_update(None).await.unwrap();
    assert_eq!((up[0]["ok"].as_bool(), up[0]["changed"].as_bool()), (Some(true), Some(false)), "{up:?}");
    r.run.app.plugin_remove("kit").await.unwrap();
    assert!(r.run.app.plugins().is_empty());
}

#[tokio::test]
async fn a_task_without_a_title_names_itself() {
    let r = start().await;
    let untitled = reagent_tools::app::UNTITLED;
    r.push(untitled, |b| {
        assert!(all_text(b).contains("name this task with tasks.task_title"), "{}", all_text(b));
        call("c1", "tasks.task_title", json!({"title": "  "}))
    });
    r.push(untitled, |b| {
        assert!(last_result(b).contains("a few words"), "{}", last_result(b));
        call("c2", "tasks.task_title", json!({"title": "Greet the world"}))
    });
    r.push(untitled, |b| {
        assert!(last_result(b).contains("called \"Greet the world\""), "{}", last_result(b));
        text("hello")
    });
    let t = r.start_task("", "say hello").await;
    assert_eq!(t.name(), "untitled task");
    let t = r.done(&t.id).await;
    assert_eq!(t.title, "Greet the world");
    let n = r.run.app.store.notifications(10).await.unwrap();
    assert!(n.iter().any(|n| n.title == "Greet the world — done"), "{n:?}");
}

#[tokio::test]
async fn unlimited_leaves_the_projects_limits_off() {
    let r = start().await;
    let mut p = r.run.app.project("site").await.unwrap();
    p.budget.0.cost = Some(5.0);
    p.budget.0.tokens = Some(1000);
    r.run.app.put_project(p).await.unwrap();
    r.push("Free", |_| text("ok"));
    let t = r.run.app.start_task(reagent_tools::app::StartTask { project: "site".into(), title: "Free".into(), prompt: "x".into(), unlimited: vec!["cost".into()], ..Default::default() }).await.unwrap();
    assert_eq!((t.budget.0.cost, t.budget.0.tokens), (None, Some(1000)), "cost unlimited, tokens the project's");
    r.done(&t.id).await;
}

#[tokio::test]
async fn tasks_start_from_templates_in_the_repo_kept_or_pinned() {
    let r = start().await;
    let dir = r.project.path().join(".agents/templates");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("greet.md"), "---\ntitle: \"Greet {{who}}\"\nfields: [{name: who, required: true}]\n---\nSay hello to {{who}}.").unwrap();
    // Kept in reagent, every project's but pinned to site.
    r.run.app.store.put_template(None, None, "pinned", "---\nproject: site\ntitle: Pinned\nbudget: { minutes: 30 }\n---\nDo the pinned thing.").await.unwrap();
    r.run.app.store.put_template(None, None, "broken", "---\nfields: [{name: a, type: radio}]\n---\nx").await.unwrap();
    r.push("Lead", |_| call("c1", "tasks.template_list", json!({})));
    r.push("Lead", |b| {
        let l = last_result(b);
        assert!(l.contains("\"greet\"") && l.contains("\"pinned\"") && l.contains("no type radio"), "{l}");
        call("c2", "tasks.task_spawn", json!({"template": "greet", "values": {"who": "Ada"}}))
    });
    r.push("Lead", |b| {
        assert!(last_result(b).contains("Greet Ada"), "{}", last_result(b));
        text("spawned")
    });
    r.push("Greet Ada", |b| {
        assert!(all_text(b).contains("Say hello to Ada."));
        text("hello Ada")
    });
    let lead = r.start_task("Lead", "x").await;
    r.done(&lead.id).await;
    // No project: the pin says which.
    r.push("Pinned", |_| text("ok"));
    let t = r.run.app.start_task(reagent_tools::app::StartTask { template: Some("pinned".into()), ..Default::default() }).await.unwrap();
    assert_eq!((t.project.as_str(), t.prompt.as_str(), t.budget.0.minutes), ("site", "Do the pinned thing.", Some(30)));
    r.done(&t.id).await;
    let e = r.run.app.start_task(reagent_tools::app::StartTask { project: "site".into(), template: Some("greet".into()), ..Default::default() }).await.unwrap_err();
    assert!(e.contains("is needed"), "{e}");
    let e = r.run.app.start_task(reagent_tools::app::StartTask { project: "site".into(), template: Some("broken".into()), ..Default::default() }).await.unwrap_err();
    assert!(e.contains("broken"), "{e}");
}

#[tokio::test]
async fn a_message_is_queued_at_once_and_delivered_in_order() {
    let r = start().await;
    r.allow_all().await;
    r.push("Busy", |_| call("c1", "shell.exec", json!({"cmd": "sleep 2; echo done sleeping"})));
    r.push("Busy", |b| {
        let all = all_text(b);
        let (one, two) = (all.find("first note").expect("the first message"), all.find("second note").expect("the second"));
        assert!(one < two, "in order: {all}");
        text("read both")
    });
    let t = r.start_task("Busy", "x").await;
    for _ in 0..100 {
        if r.run.app.sup.jobs(Some(&t.id)).await.unwrap().iter().any(|j| j.running()) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    // The agent is in its command: the messages are taken at once, not when it gets to them.
    let started = std::time::Instant::now();
    r.run.app.message(&t.id, "first note").await.unwrap();
    r.run.app.message(&t.id, "second note").await.unwrap();
    assert!(started.elapsed() < std::time::Duration::from_millis(500), "queued, not delivered: {:?}", started.elapsed());
    let t = r.done(&t.id).await;
    assert_eq!(t.report.as_deref(), Some("read both"));
    assert!(r.run.app.store.outbox(&t.id).await.unwrap().is_empty(), "delivered ones go");
}

#[tokio::test]
async fn helpers_are_subtasks_of_a_kind_and_spawn_agent_says_so() {
    let r = start().await;
    r.allow_all().await;
    r.push("Lead", |_| call("c1", "spawn_agent", json!({"type": "general", "prompt": "look around"})));
    r.push("Lead", |b| {
        let l = last_result(b);
        assert!(l.contains("tasks.task_spawn") && l.contains("explore (") && l.contains("research ("), "{l}");
        call("c2", "tasks.task_spawn", json!({"title": "Look", "prompt": "Find the greeting.", "kind": "explore"}))
    });
    r.push("Look", |b| {
        let all = all_text(b);
        assert!(all.contains("# Your role (explore)") && all.contains("You change nothing") && all.contains("Find the greeting."), "{all}");
        call("e1", "fs.write", json!({"path": "x.txt", "text": "no"}))
    });
    r.push("Look", |b| {
        assert!(last_result(b).contains("doesn't use fs.write"), "{}", last_result(b));
        call("e2", "fs.read", json!({"path": "a.txt"}))
    });
    r.push("Look", |b| {
        assert!(last_result(b).contains("hello world"), "{}", last_result(b));
        text("a.txt:1 says hello world")
    });
    // The report comes as a message: in the next turn, or in a later one.
    for _ in 0..2 {
        r.push("Lead", |b| text(if all_text(b).contains("a.txt:1 says hello world") { "found it" } else { "waiting for it" }));
    }
    let t = r.start_task("Lead", "x").await;
    r.until(&t.id, "the explore report", |t| t.report.as_deref() == Some("found it")).await;
    assert!(!r.project.path().join("x.txt").exists());
    let kids = r.run.app.store.tasks(None, Some(&t.id), false, 10).await.unwrap();
    assert_eq!(kids[0].kind.as_deref(), Some("explore"));
}

#[tokio::test]
async fn a_task_in_a_persistent_worktree_works_there_and_merges_back_into_it() {
    let r = start().await;
    r.allow_all().await;
    let mut p = r.run.app.project("site").await.unwrap();
    p.merge = "auto".into();
    r.run.app.put_project(p).await.unwrap();
    let wts = r.run.app.worktrees();
    let next = wts.add("site", reagent_tools::persistent::AddWorktree { name: "next".into(), ..Default::default() }).await.unwrap();
    assert_eq!((next.worktree.branch.as_str(), next.worktree.base.as_str(), next.sharing_now.as_str()), ("next", "main", "shared"));
    let home = std::path::PathBuf::from(&next.worktree.path);
    assert!(home.join("a.txt").exists());

    r.push("Home", |b| {
        assert!(all_text(b).contains("Persistent worktree: next (branch next, base main)"), "{}", all_text(b));
        call("c1", "fs.write", json!({"path": "home.txt", "text": "direct\n"}))
    });
    r.push("Home", |_| call("c2", "shell.exec", json!({"cmd": "git add -A && git commit -qm home"})));
    r.push("Home", |_| call("c3", "git.worktree_start", json!({"base": "main"})));
    r.push("Home", |b| {
        assert!(last_result(b).contains("merges back into it, not main"), "another base is refused: {}", last_result(b));
        call("c4", "git.worktree_start", json!({}))
    });
    r.push("Home", |b| {
        assert!(last_result(b).contains("(from next)"), "{}", last_result(b));
        call("c5", "shell.exec", json!({"cmd": "echo fix > fix.txt && git add -A && git commit -qm fix"}))
    });
    r.push("Home", |_| call("c6", "git.worktree_merge", json!({})));
    r.push("Home", |b| {
        assert!(last_result(b).contains("into next"), "{}", last_result(b));
        call("c7", "git.worktree_drop", json!({}))
    });
    r.push("Home", |b| {
        assert!(last_result(b).contains("persistent/next again"), "back home: {}", last_result(b));
        call("c8", "tasks.task_spawn", json!({"title": "Sub", "prompt": "look around"}))
    });
    r.push("Sub", |_| text("sub done"));
    r.push("Home", |_| text("waiting"));
    r.push("Home", |_| text("all done"));
    let t = r.run.app.start_task(reagent_tools::app::StartTask { project: "site".into(), title: "Home".into(), prompt: "work in next".into(), worktree: Some("next".into()), ..Default::default() }).await.unwrap();
    assert_eq!((t.home.as_deref(), t.cwd.as_str()), (Some("next"), next.worktree.path.as_str()));
    let t = r.until(&t.id, "the subtask's report", |t| t.report.as_deref() == Some("all done")).await;
    assert_eq!(t.cwd, next.worktree.path);
    assert_eq!(std::fs::read_to_string(home.join("home.txt")).unwrap(), "direct\n");
    assert_eq!(std::fs::read_to_string(home.join("fix.txt")).unwrap(), "fix\n", "the task's worktree merged into next");
    assert!(!r.project.path().join("fix.txt").exists() && !r.project.path().join("home.txt").exists(), "main is untouched");
    let subs = r.run.app.store.tasks(None, Some(&t.id), false, 10).await.unwrap();
    assert_eq!((subs[0].home.as_deref(), subs[0].cwd.as_str()), (Some("next"), next.worktree.path.as_str()), "a subtask starts in its parent's home");
    assert_eq!(wts.tasks("site", "next").await.unwrap().len(), 2);

    // Exclusive: a second task waits for the first.
    wts.set_sharing("site", "next", Some("exclusive")).await.unwrap();
    r.push("Hold", |_| call("q1", "ask.ask", json!({"question": "go on?"})));
    let hold = r.run.app.start_task(reagent_tools::app::StartTask { project: "site".into(), title: "Hold".into(), prompt: "x".into(), worktree: Some("next".into()), ..Default::default() }).await.unwrap();
    r.until(&hold.id, "the question", |t| t.state == "waiting").await;
    r.push("Second", |_| text("second done"));
    let second = r.run.app.start_task(reagent_tools::app::StartTask { project: "site".into(), title: "Second".into(), prompt: "x".into(), worktree: Some("next".into()), ..Default::default() }).await.unwrap();
    assert_eq!(second.state, "queued");
    let elsewhere = r.run.app.start_task(reagent_tools::app::StartTask { project: "site".into(), title: "Elsewhere".into(), prompt: "x".into(), ..Default::default() }).await.unwrap();
    assert_ne!(elsewhere.state, "queued", "the project folder isn't held");
    assert!(wts.remove("site", "next", false, false).await.unwrap_err().contains("tasks work in next"));
    r.run.app.cancel(&hold.id).await.unwrap();
    r.until(&second.id, "the second to run", |t| t.report.as_deref() == Some("second done")).await;

    // The person merges it into main, then removes it.
    let said = wts.merge("site", "next", reagent_tools::git::Strategy::Merge, None).await.unwrap();
    assert!(said.starts_with("merged next into main"), "{said}");
    assert_eq!(std::fs::read_to_string(r.project.path().join("fix.txt")).unwrap(), "fix\n");
    r.run.app.cancel(&elsewhere.id).await.unwrap();
    wts.remove("site", "next", false, false).await.unwrap();
    assert!(!home.exists() && wts.list("site").await.unwrap().0.is_empty());
}

#[tokio::test]
async fn only_the_person_s_queued_messages_can_be_changed_or_cancelled() {
    let r = start().await;
    r.push("Held", |_| call("q1", "ask.ask", json!({"question": "go on?"})));
    let t = r.start_task("Held", "x").await;
    r.until(&t.id, "the question", |t| t.state == "waiting").await;
    r.run.app.pause(&t.id, subnet_core::agent::PauseMode::Quick).await.unwrap();
    r.until(&t.id, "paused", |t| t.state == "paused").await;
    // Paused: both wait in the queue.
    r.run.app.person_message(&t.id, "[from the person] mine").await.unwrap();
    r.run.app.message(&t.id, "[job j1 (build) exited with 0]").await.unwrap();
    let q = r.run.app.store.outbox(&t.id).await.unwrap();
    assert_eq!(q.iter().map(|m| (m.text.as_str(), m.person)).collect::<Vec<_>>(), [("[from the person] mine", true), ("[job j1 (build) exited with 0]", false)]);
    let e = r.run.app.edit_queued(&t.id, q[1].id, "changed").await.unwrap_err();
    assert!(e.contains("not yours"), "{e}");
    assert!(r.run.app.cancel_queued(&t.id, Some(q[1].id)).await.is_err());
    r.run.app.edit_queued(&t.id, q[0].id, "[from the person] mine, changed").await.unwrap();
    assert_eq!(r.run.app.cancel_queued(&t.id, None).await.unwrap(), 1, "all of the person's");
    let left = r.run.app.store.outbox(&t.id).await.unwrap();
    assert_eq!((left.len(), left[0].person), (1, false), "reagent's own stays");
    r.run.app.cancel(&t.id).await.unwrap();
}
