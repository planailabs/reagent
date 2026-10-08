//! Triggers: scripts whose events start tasks or message running ones;
//! keys dropped when seen; the policy and the person decide what runs;
//! failures back off and start a repair; watchers; triggers in the repo.

mod common;

use std::time::Duration;

use common::*;
use reagent_store::{Trigger, TriggerState};
use reagent_tools::triggers;
use serde_json::{Value, json};

fn trig(v: Value) -> Trigger {
    let mut base = json!({"name": "ci", "mode": "poll", "every": 3600, "title": "Fix CI {{key}}", "prompt": "Run {{vars.url}} failed.", "script": "cat \"$REAGENT_STATE/out\""});
    base.as_object_mut().unwrap().extend(v.as_object().unwrap().clone());
    serde_json::from_value(base).unwrap()
}

/// Waits until `f` gives something.
async fn until<T>(what: &str, mut f: impl AsyncFnMut() -> Option<T>) -> T {
    for _ in 0..400 {
        if let Some(v) = f().await {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

impl R {
    async fn trigger(&self, name: &str) -> Trigger {
        self.run.app.store.trigger("site", name).await.unwrap().unwrap()
    }

    async fn save(&self, t: Trigger, by_person: bool) -> Trigger {
        let p = self.run.app.project("site").await.unwrap();
        triggers::save(&self.run.app.store, &p, t, by_person).await.unwrap()
    }

    /// The script's output on its next run.
    fn out(&self, name: &str, text: &str) {
        let d = self.data.path().join("triggers/site").join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("out"), text).unwrap();
    }

    async fn runs(&self, name: &str) -> Vec<reagent_store::TriggerRun> {
        self.run.app.store.trigger_runs("site", name).await.unwrap()
    }

    async fn tasks_from(&self, origin: &str) -> Vec<reagent_store::Task> {
        self.run.app.store.tasks(Some("site"), None, false, 100).await.unwrap().into_iter().filter(|t| t.origin == origin).collect()
    }
}

#[tokio::test]
async fn events_start_tasks_message_running_ones_and_keys_are_seen_once() {
    let r = start().await;
    r.allow_all().await;
    r.out("ci", "checking\n{\"key\": \"812\", \"vars\": {\"url\": \"https://ci/812\"}}\n");
    // The task works a while (a command), so it's still going when the next event comes.
    r.push("Fix CI 812", |b| {
        assert!(all_text(b).contains("Run https://ci/812 failed."), "the prompt filled in");
        call("c1", "shell.exec", json!({"cmd": "sleep 4"}))
    });
    r.push("Fix CI 812", |b| {
        assert!(all_text(b).contains("[trigger ci] CI failed again"), "the event's message came: {}", all_text(b));
        text("fixed")
    });
    r.save(trig(json!({"script": "cat \"$REAGENT_STATE/out\"; echo \"tasks=$REAGENT_TASKS\" >&2"})), true).await;
    let t = until("a task", async || r.tasks_from("trigger:site/ci").await.pop()).await;
    assert_eq!(t.title, "Fix CI 812");
    until("the task to run its command", async || r.run.app.sup.jobs(Some(&t.id)).await.unwrap().first().cloned()).await;
    // Another run: the same key again, and a message for the running task.
    r.out("ci", "{\"key\": \"812\"}\n{\"key\": \"813\", \"to\": \"running\", \"message\": \"CI failed again\"}\n");
    triggers::run_now(&r.run.app, "site", "ci").await.unwrap();
    let runs = until("the second run", async || Some(r.runs("ci").await).filter(|x| x.len() == 2)).await;
    assert!(runs[0].ok, "{:?}", runs[0]);
    assert!(runs[0].output.contains("812: seen before, dropped") && runs[0].output.contains(&format!("813: to task {}", t.id)), "{}", runs[0].output);
    assert!(runs[0].output.contains(&format!("\"id\":\"{}\",\"key\":\"812\"", t.id)), "REAGENT_TASKS names it with its key: {}", runs[0].output);
    r.done(&t.id).await;
    assert_eq!(r.tasks_from("trigger:site/ci").await.len(), 1, "no second task");
    // Its task ended: no longer in its state.
    until("the task to leave the state", async || r.trigger("ci").await.state.0.tasks.is_empty().then_some(())).await;
}

#[tokio::test]
async fn queued_events_start_when_the_last_task_ends() {
    let r = start().await;
    r.out("q", "{\"key\": \"a\", \"title\": \"Job A\"}\n{\"key\": \"b\", \"title\": \"Job B\"}\n{\"key\": \"c\", \"to\": \"running\", \"message\": \"for whoever runs\"}\n");
    r.push("Job A", |_| call("c1", "shell.exec", json!({"cmd": "sleep 2"})));
    r.push("Job A", |b| {
        assert!(all_text(b).contains("for whoever runs"));
        text("a done")
    });
    r.push("Job B", |_| text("b done"));
    r.allow_all().await;
    r.save(trig(json!({"name": "q", "overlap": "queue"})), true).await;
    let a = until("task A", async || r.tasks_from("trigger:site/q").await.into_iter().find(|t| t.title == "Job A")).await;
    until("B to wait", async || (r.trigger("q").await.state.0.queue.len() == 1).then_some(())).await;
    r.done(&a.id).await;
    let b = until("task B", async || r.tasks_from("trigger:site/q").await.into_iter().find(|t| t.title == "Job B")).await;
    r.done(&b.id).await;
    assert!(r.trigger("q").await.state.0.queue.is_empty());
}

#[tokio::test]
async fn the_policy_and_the_person_decide_what_runs() {
    let r = start().await;
    r.out("ci", "{\"key\": \"1\"}\n");
    // Made by a task: the policy asks (the project's default).
    let mut t = trig(json!({}));
    t.made_by = "task:x".into();
    r.save(t.clone(), false).await;
    let st = until("the question", async || Some(r.trigger("ci").await.state.0).filter(|s| s.asking.is_some())).await;
    assert_eq!(st.command.as_deref(), Some("cat \"$REAGENT_STATE/out\""));
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(r.runs("ci").await.is_empty(), "not run while it asks");
    let notes = r.run.app.store.notifications(10).await.unwrap();
    assert_eq!(notes.iter().filter(|n| n.kind == "trigger" && n.title.contains("wants to run")).count(), 1, "said once");
    // Allowed once: this script runs.
    r.push("Fix CI 1", |_| text("ok"));
    triggers::approve(&r.run.app, "site", "ci", true, false).await.unwrap();
    until("a run", async || r.runs("ci").await.pop()).await;
    // Another script asks again; denied, it doesn't run.
    t.script = "echo '{\"key\": \"2\"}'".into();
    r.save(t.clone(), false).await;
    until("the question again", async || r.trigger("ci").await.state.0.asking.filter(|h| *h == triggers::script_hash(&t))).await;
    triggers::approve(&r.run.app, "site", "ci", false, false).await.unwrap();
    let st = until("denied", async || Some(r.trigger("ci").await.state.0).filter(|s| s.last_error.as_deref().is_some_and(|e| e.contains("denied")))).await;
    assert!(st.asking.is_none());
    assert_eq!(r.runs("ci").await.len(), 1);
    // A rule allowing it: runs without asking.
    t.script = "cat \"$REAGENT_STATE/out\" # v3".into();
    let mut rules = r.run.app.store.rules("site").await.unwrap();
    rules.insert(0, reagent_store::Rule { id: 0, project: "site".into(), pos: 0, tool: "triggers.run".into(), command: Some("cat *".into()), target: None, action: "allow".into() });
    r.run.app.store.set_rules("site", &rules).await.unwrap();
    r.save(t, false).await;
    until("a run by the rule", async || Some(r.runs("ci").await).filter(|x| x.len() == 2)).await;
    // "Always": a rule for exactly this command.
    let mut t4 = trig(json!({"name": "other", "script": "true"}));
    t4.made_by = "task:x".into();
    r.save(t4, false).await;
    until("its question", async || r.trigger("other").await.state.0.asking).await;
    triggers::approve(&r.run.app, "site", "other", true, true).await.unwrap();
    assert!(r.run.app.store.rules("site").await.unwrap().iter().any(|x| x.tool == "triggers.run" && x.command.as_deref() == Some("true") && x.action == "allow"));
}

#[tokio::test]
async fn failing_runs_back_off_and_start_a_repair() {
    let r = start().await;
    r.push("Repair trigger broken", |b| {
        let all = all_text(b);
        assert!(all.contains("failed 5 times in a row") && all.contains("exit 3") && all.contains("oops"), "{all}");
        text("repaired")
    });
    r.save(trig(json!({"name": "broken", "every": 60, "script": "echo oops >&2; exit 3"})), true).await;
    let mut last_next = 0;
    for n in 1..=5 {
        let st = until("a failed run", async || Some(r.trigger("broken").await.state.0).filter(|s| s.failures == n)).await;
        let next = st.next_run.unwrap();
        assert!(next - st.last_run.unwrap() >= 60 << (n - 1).min(5), "backs off: {n} → {}", next - st.last_run.unwrap());
        assert!(next >= last_next);
        last_next = next;
        if n < 5 {
            triggers::run_now(&r.run.app, "site", "broken").await.unwrap();
        }
    }
    let runs = r.runs("broken").await;
    assert_eq!(runs.len(), 5);
    assert_eq!((runs[0].ok, runs[0].exit, runs[0].error.as_deref()), (false, Some(3), Some("exited with 3")));
    assert!(runs[0].output.contains("oops"));
    let repair = until("the repair task", async || r.tasks_from("trigger-repair:site/broken").await.pop()).await;
    until("the repair noted", async || (r.trigger("broken").await.state.0.repair.as_deref() == Some(repair.id.as_str())).then_some(())).await;
    r.done(&repair.id).await;
    // Once per streak; a good run ends it.
    let mut t = r.trigger("broken").await;
    t.script = "true".into();
    r.save(t, true).await;
    until("a good run", async || Some(r.trigger("broken").await.state.0).filter(|s| s.failures == 0 && s.last_error.is_none() && s.last_run.is_some()).filter(|s: &TriggerState| s.repair.is_none())).await;
    assert_eq!(r.tasks_from("trigger-repair:site/broken").await.len(), 1);
}

#[tokio::test]
async fn a_watcher_runs_in_the_supervisor_and_its_lines_are_events() {
    let r = start().await;
    r.allow_all().await;
    r.push("Deploy a", |_| call("c1", "shell.exec", json!({"cmd": "sleep 3"})));
    r.push("Deploy a", |b| {
        assert!(all_text(b).contains("[trigger w] b came"), "{}", all_text(b));
        text("ok")
    });
    let script = "echo starting\necho '{\"key\": \"a\"}'\nsleep 1.5\necho '{\"key\": \"b\", \"to\": \"running\", \"message\": \"b came\"}'\nsleep 60\n";
    r.save(trig(json!({"name": "w", "mode": "watch", "every": null, "title": "Deploy {{key}}", "prompt": "deploy", "script": script})), true).await;
    let t = until("its task", async || r.tasks_from("trigger:site/w").await.pop()).await;
    let job = r.trigger("w").await.state.0.job.expect("a job");
    let meta = r.run.app.sup.job(&job).await.unwrap();
    assert_eq!(meta.owner.as_deref(), Some("trigger:site/w"));
    r.done(&t.id).await;
    // Off: the watcher stops; the run is recorded.
    r.run.app.store.set_trigger_enabled("site", "w", false).await.unwrap();
    until("the watcher to stop", async || (!r.run.app.sup.job(&job).await.unwrap().running()).then_some(())).await;
    until("no job kept", async || r.trigger("w").await.state.0.job.is_none().then_some(())).await;
}

#[tokio::test]
async fn a_watcher_that_ends_is_started_again() {
    let r = start().await;
    r.save(trig(json!({"name": "w", "mode": "watch", "every": 1, "script": "echo once; exit 0"})), true).await;
    let runs = until("two runs", async || Some(r.runs("w").await).filter(|x| x.len() >= 2)).await;
    assert!(runs.iter().all(|x| x.ok && x.output.contains("once")), "{runs:?}");
    // Not a task's: its end isn't a message to anyone, and it's acknowledged.
    // (A run that just ended is taken within a second.)
    r.run.app.store.set_trigger_enabled("site", "w", false).await.unwrap();
    until("its ended runs taken", async || {
        let jobs = r.run.app.sup.jobs(Some("trigger:site/w")).await.unwrap();
        jobs.iter().all(|j| !j.running() && j.acked).then_some(())
    })
    .await;
}

#[tokio::test]
async fn repo_triggers_are_read_and_move_both_ways() {
    let r = start().await;
    let p = r.run.app.project("site").await.unwrap();
    let dir = r.project.path().join(".agents/triggers/nightly");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("TRIGGER.md"), "---\ndescription: every night\nmode: poll\ncron: \"0 3 * * *\"\ntz: Europe/Vienna\ntitle: Nightly {{key}}\n---\n\nCheck the night's builds.\n").unwrap();
    std::fs::write(dir.join("run"), "#!/bin/sh\necho hi\n").unwrap();
    triggers::sync_repo(&r.run.app).await;
    let t = r.trigger("nightly").await;
    assert_eq!((t.source.as_str(), t.made_by.as_str(), t.cron.as_deref(), t.prompt.as_str()), ("repo", "repo", Some("0 3 * * *"), "Check the night's builds."));
    // Not run now: a cron poll waits for its time; and the policy hasn't allowed it.
    r.run.app.store.set_trigger_enabled("site", "nightly", false).await.unwrap();
    // Off is kept across a scan; a change in the files is read.
    std::fs::write(dir.join("run"), "#!/bin/sh\necho changed\n").unwrap();
    triggers::sync_repo(&r.run.app).await;
    let t = r.trigger("nightly").await;
    assert_eq!((t.enabled, t.script.as_str()), (false, "#!/bin/sh\necho changed\n"));
    // Into reagent: the files go, the state stays.
    triggers::update(&r.run.app, "site", "nightly", |s| s.failures = 2).await;
    let moved = triggers::move_to(&r.run.app.store, &p, "nightly", "db").await.unwrap();
    assert_eq!((moved.source.as_str(), moved.state.0.failures), ("db", 2));
    assert!(!dir.exists());
    triggers::sync_repo(&r.run.app).await;
    assert_eq!(r.trigger("nightly").await.source, "db", "not taken for gone");
    // And back into the repo.
    let back = triggers::move_to(&r.run.app.store, &p, "nightly", "repo").await.unwrap();
    assert_eq!(back.source, "repo");
    assert!(std::fs::read_to_string(dir.join("TRIGGER.md")).unwrap().contains("cron: 0 3 * * *"));
    assert_eq!(std::fs::read_to_string(dir.join("run")).unwrap(), "#!/bin/sh\necho changed\n");
    assert!(triggers::move_to(&r.run.app.store, &p, "nightly", "repo").await.unwrap_err().contains("already"));
    // Files gone: so is the trigger. A kept trigger of a repo one's name is reported.
    std::fs::remove_dir_all(&dir).unwrap();
    triggers::sync_repo(&r.run.app).await;
    assert!(r.run.app.store.trigger("site", "nightly").await.unwrap().is_none());
    r.save(trig(json!({"name": "dup"})), true).await;
    let d2 = r.project.path().join(".agents/triggers/dup");
    std::fs::create_dir_all(&d2).unwrap();
    std::fs::write(d2.join("TRIGGER.md"), "---\nmode: webhook\nsecret: HOOK\ntitle: t\n---\np\n").unwrap();
    triggers::sync_repo(&r.run.app).await;
    assert_eq!(r.trigger("dup").await.source, "db");
    assert!(r.run.app.triggers.repo_errors.lock().unwrap()["site"][0].contains("already keeps a trigger called dup"));
}
