//! Services: commands that should always run; started, checked, stopped,
//! restarted on an end, a failed check or a change; repaired by a task after
//! five failures in a row; trusted like triggers; kept in the repo too.

mod common;

use std::time::Duration;

use common::*;
use reagent_store::Service;
use reagent_tools::services;
use serde_json::{Value, json};

fn svc(v: Value) -> Service {
    let mut base = json!({"name": "web", "script": "echo started; exec sleep 600"});
    base.as_object_mut().unwrap().extend(v.as_object().unwrap().clone());
    serde_json::from_value(base).unwrap()
}

impl R {
    async fn service(&self, name: &str) -> Service {
        self.run.app.store.service("site", name).await.unwrap().unwrap()
    }

    async fn save_service(&self, v: Service, by_person: bool) -> Service {
        let p = self.run.app.project("site").await.unwrap();
        services::save(&self.run.app.store, &p, v, by_person).await.unwrap()
    }

    /// Waits (up to 30 s) until `f` holds for the service.
    async fn service_until(&self, name: &str, what: &str, f: impl Fn(&Service) -> bool) -> Service {
        for _ in 0..300 {
            // A repo service shows up with the next scan.
            if let Some(v) = self.run.app.store.service("site", name).await.unwrap()
                && f(&v)
            {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("timed out waiting for {what}: {:?}", self.run.app.store.service("site", name).await.unwrap());
    }
}

#[tokio::test]
async fn a_service_comes_up_is_checked_restarted_stopped_and_changed() {
    let r = start().await;
    let dir = r.project.path().to_path_buf();
    // Up once its check passes.
    r.save_service(svc(json!({"health": "test -f ready", "interval": 1})), true).await;
    let v = r.service_until("web", "it to start", |v| v.state.0.status == "starting").await;
    let first = v.state.0.job.clone().unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(r.service("web").await.state.0.status, "starting", "not up before its check passes");
    std::fs::write(dir.join("ready"), "").unwrap();
    r.service_until("web", "it to be up", |v| v.state.0.status == "up").await;
    let runs = r.run.app.store.service_runs("site", "web").await.unwrap();
    assert!(runs[0].up.is_some() && runs[0].job.as_deref() == Some(first.as_str()));
    assert!(services::logs(&r.run.app, "site", "web", 10).await.unwrap().contains("started"));
    // Three failed checks: stopped, a failure, started again after its backoff.
    std::fs::remove_file(dir.join("ready")).unwrap();
    let v = r.service_until("web", "a failure", |v| v.state.0.failures == 1).await;
    assert!(v.state.0.last_error.as_deref().unwrap().starts_with("unhealthy: 3 failed checks"), "{:?}", v.state.0);
    std::fs::write(dir.join("ready"), "").unwrap();
    let v = r.service_until("web", "it to be up again", |v| v.state.0.status == "up").await;
    let second = v.state.0.job.clone().unwrap();
    assert_ne!(first, second, "a new job");
    // A stop holds.
    services::set_enabled(&r.run.app.store, "site", "web", false).await.unwrap();
    r.service_until("web", "it to stop", |v| v.state.0.status == "stopped" && v.state.0.job.is_none()).await;
    assert!(!r.run.app.sup.job(&second).await.unwrap().running());
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(r.service("web").await.state.0.job.is_none(), "it stays stopped");
    // Switched on, then changed: the change restarts it.
    services::set_enabled(&r.run.app.store, "site", "web", true).await.unwrap();
    let v = r.service_until("web", "it to be up", |v| v.state.0.status == "up").await;
    let third = v.state.0.job.clone().unwrap();
    r.save_service(svc(json!({"script": "echo changed; exec sleep 600", "health": "test -f ready", "interval": 1})), true).await;
    let v = r.service_until("web", "the change to restart it", |v| v.state.0.status == "up" && v.state.0.job.as_deref() != Some(third.as_str())).await;
    assert_eq!(v.state.0.failures, 0, "a change isn't a failure");
    assert!(services::logs(&r.run.app, "site", "web", 10).await.unwrap().contains("changed"));
    // A restart: stopped and started at once.
    let before = v.state.0.job.clone().unwrap();
    services::restart(&r.run.app.store, "site", "web").await.unwrap();
    r.service_until("web", "a restart", |v| v.state.0.job.as_deref().is_some_and(|j| j != before)).await;
    services::remove(&r.run.app, "site", "web").await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!r.run.app.sup.jobs(Some("service:site/web")).await.unwrap().iter().any(|j| j.running()), "removed: stopped");
}

#[tokio::test]
async fn a_service_that_keeps_failing_is_repaired_by_a_task() {
    let r = start().await;
    r.allow_all().await;
    r.push("Repair service crash", |b| {
        let t = all_text(b);
        assert!(t.contains("failed 5 times in a row") && t.contains("exited with 3") && t.contains("Look at the config first.") && t.contains("boom"), "{t}");
        text("fixed it")
    });
    let mut v = r.save_service(svc(json!({"name": "crash", "script": "echo boom; exit 3", "notes": "Look at the config first."})), true).await;
    // Four failures already (the backoff would take minutes): the fifth starts the repair.
    v.state.0.failures = 4;
    r.run.app.store.set_service_state("site", "crash", &v.state.0).await.unwrap();
    let v = r.service_until("crash", "the repair", |v| v.state.0.repair.is_some()).await;
    assert_eq!((v.state.0.failures, v.state.0.status.as_str()), (5, "backoff"));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let next = v.state.0.next_start.unwrap() - now;
    assert!((60..=81).contains(&next), "5 s doubled four times: {next}");
    let task = r.task(v.state.0.repair.as_deref().unwrap()).await;
    assert_eq!(task.origin, "service-repair:site/crash");
    r.done(&task.id).await;
    let runs = r.run.app.store.service_runs("site", "crash").await.unwrap();
    assert_eq!((runs[0].exit, runs[0].error.as_deref()), (Some(3), Some("exited with 3")));
    assert!(runs[0].output.contains("boom"));
    let notes = r.run.app.store.notifications(10).await.unwrap();
    assert!(notes.iter().any(|n| n.kind == "service" && n.title.contains("keeps failing")));
}

#[tokio::test]
async fn an_agent_s_service_waits_for_the_person_and_a_repo_one_is_read() {
    let r = start().await;
    // The project asks: an agent's service waits.
    r.save_service(svc(json!({"name": "agent"})), false).await;
    let v = r.service_until("agent", "the approval", |v| v.state.0.status == "waiting").await;
    assert!(v.state.0.job.is_none() && v.state.0.command.as_deref() == Some("echo started; exec sleep 600"));
    services::approve(&r.run.app, "site", "agent", true, false).await.unwrap();
    r.service_until("agent", "it to be up", |v| v.state.0.status == "up").await;
    // A repo service: read, started once the policy allows the repo's services.
    let d = r.project.path().join(".agents/services/repo");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("SERVICE.md"), "---\ndescription: from the repo\nready_timeout: 30s\n---\n\nIt serves the docs.\n").unwrap();
    std::fs::write(d.join("run"), "exec sleep 600\n").unwrap();
    let mut rules = r.run.app.store.rules("site").await.unwrap();
    rules.insert(0, reagent_store::Rule { id: 0, project: "site".into(), pos: 0, tool: "services.run".into(), command: Some("./.agents/services/*".into()), target: None, action: "allow".into() });
    r.run.app.store.set_rules("site", &rules).await.unwrap();
    let v = r.service_until("repo", "the repo service to be up", |v| v.state.0.status == "up").await;
    assert_eq!((v.source.as_str(), v.ready_timeout, v.notes.as_str()), ("repo", 30, "It serves the docs."));
    // Its files gone: it goes, stopped.
    let job = v.state.0.job.clone().unwrap();
    std::fs::remove_dir_all(&d).unwrap();
    for _ in 0..150 {
        if r.run.app.store.service("site", "repo").await.unwrap().is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(r.run.app.store.service("site", "repo").await.unwrap().is_none());
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!r.run.app.sup.job(&job).await.unwrap().running());
    services::remove(&r.run.app, "site", "agent").await.unwrap();
}

#[tokio::test]
async fn a_task_adds_a_service_and_reads_its_output() {
    let r = start().await;
    r.allow_all().await;
    r.push("Serve", |_| call("c1", "services.service_add", json!({"name": "web", "script": "echo serving; exec sleep 600", "health": "true", "interval": "1s", "notes": "the docs server"})));
    r.push("Serve", |b| {
        assert!(last_result(b).starts_with("added: web"), "{}", last_result(b));
        call("c2", "shell.exec", json!({"cmd": "sleep 4"}))
    });
    r.push("Serve", |_| call("c3", "services.service_list", json!({})));
    r.push("Serve", |b| {
        assert!(last_result(b).contains("web [up"), "{}", last_result(b));
        call("c4", "services.service_logs", json!({"name": "web"}))
    });
    r.push("Serve", |b| {
        assert!(last_result(b).contains("serving"), "{}", last_result(b));
        call("c5", "services.service_stop", json!({"name": "web"}))
    });
    r.push("Serve", |_| text("served"));
    let t = r.start_task("Serve", "run the docs server").await;
    r.done(&t.id).await;
    let v = r.service_until("web", "it to stop", |v| v.state.0.status == "stopped").await;
    assert_eq!((v.made_by, v.enabled, v.notes.as_str()), (format!("task:{}", t.id), false, "the docs server"));
}

#[tokio::test]
async fn a_repair_waits_until_nothing_else_goes_on_in_the_project() {
    let r = start().await;
    r.allow_all().await;
    r.push("Busy", |_| call("q1", "ask.ask", json!({"question": "still there?"})));
    let busy = r.start_task("Busy", "x").await;
    r.until(&busy.id, "the question", |t| t.state == "waiting").await;
    r.push("Repair service crash", |_| text("fixed it"));
    let mut v = r.save_service(svc(json!({"name": "crash", "script": "exit 3"})), true).await;
    v.state.0.failures = 4;
    r.run.app.store.set_service_state("site", "crash", &v.state.0).await.unwrap();
    let v = r.service_until("crash", "the repair", |v| v.state.0.repair.is_some()).await;
    let repair = v.state.0.repair.clone().unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(r.task(&repair).await.state, "queued", "behind the task that waits");
    r.run.app.cancel(&busy.id).await.unwrap();
    let t = r.until(&repair, "the repair to run", |t| t.state == "done").await;
    assert_eq!(t.report.as_deref(), Some("fixed it"));
    services::remove(&r.run.app, "site", "crash").await.unwrap();
}
