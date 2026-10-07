use super::*;

async fn store() -> (Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (Store::open(&dir.path().join("reagent.db")).await.unwrap(), dir)
}

#[tokio::test]
async fn projects_and_their_rules_in_order() {
    let (s, _d) = store().await;
    let mut p = Project::new("site", "Site", "/src/site");
    s.put_project(&p).await.unwrap();
    p.merge = "auto".into();
    p.budget = Json(Budget { cost: Some(2.5), ..Default::default() });
    s.put_project(&p).await.unwrap();
    let got = s.project("site").await.unwrap().unwrap();
    assert_eq!((got.merge.as_str(), got.budget.cost), ("auto", Some(2.5)));
    assert!(s.put_project(&Project::new("other", "Other", "/src/site")).await.is_err(), "one project per folder");

    let r = |tool: &str, action: &str| Rule { id: 0, project: String::new(), pos: 0, tool: tool.into(), command: None, target: None, action: action.into() };
    s.set_rules("site", &[r("fs.*", "allow"), r("shell.*", "ask")]).await.unwrap();
    s.prepend_rule("site", &Rule { command: Some("cargo *".into()), ..r("shell.exec", "allow") }).await.unwrap();
    let rules = s.rules("site").await.unwrap();
    assert_eq!(rules.iter().map(|r| (r.tool.as_str(), r.action.as_str())).collect::<Vec<_>>(), [("shell.exec", "allow"), ("fs.*", "allow"), ("shell.*", "ask")]);
    assert_eq!(rules[0].command.as_deref(), Some("cargo *"));

    assert!(s.remove_project("site").await.unwrap());
    assert!(s.rules("site").await.unwrap().is_empty(), "rules go with it");
}

#[tokio::test]
async fn tasks_keep_their_state_and_usage() {
    let (s, _d) = store().await;
    s.put_project(&Project::new("site", "Site", "/src/site")).await.unwrap();
    let t = s
        .add_task(&NewTask { project: "site".into(), title: "Fix".into(), prompt: "fix it".into(), origin: "ui".into(), cwd: "/src/site".into(), profile: "default".into(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(t.state, "running");
    s.set_agent(&t.id, "a-1").await.unwrap();
    assert_eq!(s.task_by_agent("a-1").await.unwrap().unwrap().id, t.id);
    let sub = s
        .add_task(&NewTask { project: "site".into(), parent: Some(t.id.clone()), title: "Sub".into(), prompt: "p".into(), origin: format!("task:{}", t.id), cwd: "/src/site".into(), profile: "default".into(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(s.tasks(None, Some(&t.id), false, 10).await.unwrap()[0].id, sub.id);

    s.set_state(&t.id, "waiting", Some(&serde_json::json!({"kind": "question", "question": "which?"}))).await.unwrap();
    let w = s.task(&t.id).await.unwrap().unwrap();
    assert_eq!(w.wait.unwrap().0["question"], "which?");
    s.set_cwd(&t.id, "/wt/fix", Some(&Worktree { path: "/wt/fix".into(), branch: "reagent/fix".into(), base: "main".into() })).await.unwrap();
    s.set_usage(&t.id, 1200, 0.25).await.unwrap();
    s.set_state(&t.id, "done", None).await.unwrap();
    let d = s.task(&t.id).await.unwrap().unwrap();
    assert!(d.finished.is_some() && d.wait.is_none() && !d.is_active());
    assert_eq!(d.worktree.unwrap().0.branch, "reagent/fix");
    assert_eq!(s.cost_since("site", 0).await.unwrap(), 0.25);
    assert_eq!(s.tasks(Some("site"), None, true, 10).await.unwrap().len(), 1, "only the subtask is still going");
}

#[tokio::test]
async fn cron_settings_sessions_and_notifications() {
    let (s, _d) = store().await;
    s.put_project(&Project::new("site", "Site", "/src/site")).await.unwrap();
    let c: Cron = serde_json::from_value(serde_json::json!({"expr": "0 3 * * *", "title": "Nightly", "prompt": "check"})).unwrap();
    let id = s.put_cron(&Cron { project: "site".into(), ..c }).await.unwrap();
    let mut c = s.cron(id).await.unwrap().unwrap();
    assert_eq!((c.tz.as_str(), c.overlap.as_str(), c.catch_up, c.enabled), ("UTC", "skip", true, true));
    c.next_run = Some(42);
    c.options = Json(CronOptions { skills: vec!["deploy".into()], ..Default::default() });
    s.put_cron(&c).await.unwrap();
    assert_eq!(s.crons(Some("site")).await.unwrap()[0].options.skills, ["deploy"]);
    assert!(s.remove_cron(id).await.unwrap());

    s.set_setting("password", "x").await.unwrap();
    s.set_setting("password", "y").await.unwrap();
    assert_eq!(s.setting("password").await.unwrap().as_deref(), Some("y"));
    s.add_session("h", now() + 60).await.unwrap();
    assert!(s.session_valid("h").await.unwrap());
    s.end_session("h").await.unwrap();
    assert!(!s.session_valid("h").await.unwrap());

    s.add_push_subscription("https://push/1", &serde_json::json!({"endpoint": "https://push/1"})).await.unwrap();
    assert_eq!(s.push_subscriptions().await.unwrap().len(), 1);
    s.add_api_token("ci", "h1").await.unwrap();
    assert!(s.add_api_token("ci", "h2").await.is_err(), "one token per name");
    assert_eq!(s.api_token("h1").await.unwrap().as_deref(), Some("ci"));
    assert_eq!(s.api_token("nope").await.unwrap(), None);
    assert!(s.api_tokens().await.unwrap()[0].2.is_some(), "last used");
    assert!(s.revoke_api_token("ci").await.unwrap());
    assert_eq!(s.api_token("h1").await.unwrap(), None);
    let n = s.add_notification("done", None, "Fix", "fixed").await.unwrap();
    s.mark_seen(n).await.unwrap();
    assert!(s.notifications(10).await.unwrap()[0].seen);
}

#[tokio::test]
async fn mcp_servers_are_kept_and_changes_noted() {
    let (s, _d) = store().await;
    let m: McpServer = serde_json::from_value(serde_json::json!({"name": "web", "url": "https://mcp.example/mcp", "credential": {"env": "WEB_TOKEN", "prefix": "Bearer "}})).unwrap();
    assert!(m.lazy && m.enabled, "lazy and on by default");
    s.put_mcp_server(&m).await.unwrap();
    let first = s.setting("mcp_changed").await.unwrap().unwrap();
    let mut m2 = s.mcp_servers().await.unwrap().remove(0);
    assert_eq!(m2.credential.as_ref().unwrap().0.header, "Authorization");
    m2.lazy = false;
    m2.idempotent = Json(vec!["search".into()]);
    s.put_mcp_server(&m2).await.unwrap();
    assert!(!s.mcp_servers().await.unwrap()[0].lazy);
    assert_ne!(s.setting("mcp_changed").await.unwrap().unwrap(), first);
    assert!(s.remove_mcp_server("web").await.unwrap());
    assert!(!s.remove_mcp_server("web").await.unwrap());
    // One project's own: gone with the project.
    s.put_project(&Project::new("site", "Site", "/src/site")).await.unwrap();
    s.put_mcp_server(&McpServer { project: Some("site".into()), ..m }).await.unwrap();
    assert_eq!(s.mcp_servers().await.unwrap()[0].project.as_deref(), Some("site"));
    assert!(s.put_mcp_server(&McpServer { name: "x".into(), project: Some("nope".into()), ..s.mcp_servers().await.unwrap()[0].clone() }).await.is_err(), "only a project there is");
    s.remove_project("site").await.unwrap();
    assert!(s.mcp_servers().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_tasks_todos() {
    let (s, _d) = store().await;
    s.put_project(&Project::new("site", "Site", "/src/site")).await.unwrap();
    let t = s.add_task(&NewTask { project: "site".into(), title: "T".into(), prompt: "p".into(), origin: "ui".into(), cwd: "/".into(), profile: "default".into(), ..Default::default() }).await.unwrap();
    assert_eq!(s.add_todos(&t.id, &["read".into(), "fix".into()]).await.unwrap(), [1, 2]);
    assert_eq!(s.add_todos(&t.id, &["test".into()]).await.unwrap(), [3]);
    assert!(s.update_todo(&t.id, 2, Some("in_progress"), None).await.unwrap());
    assert!(s.update_todo(&t.id, 3, None, Some("run the tests")).await.unwrap());
    assert!(!s.update_todo(&t.id, 9, Some("done"), None).await.unwrap());
    let v = s.todos(&t.id).await.unwrap();
    assert_eq!(v.iter().map(|t| (t.id, t.text.as_str(), t.status.as_str())).collect::<Vec<_>>(), [(1, "read", "pending"), (2, "fix", "in_progress"), (3, "run the tests", "pending")]);
    s.clear_todos(&t.id).await.unwrap();
    assert!(s.todos(&t.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn secrets_are_encrypted_and_a_projects_own_win() {
    let (s, d) = store().await;
    s.put_project(&Project::new("site", "Site", "/src/site")).await.unwrap();
    s.set_secret(None, "GH_TOKEN", "ghp_global").await.unwrap();
    s.set_secret(None, "NPM_TOKEN", "npm_1").await.unwrap();
    s.set_secret(Some("site"), "GH_TOKEN", "ghp_site").await.unwrap();
    s.set_secret(Some("site"), "GH_TOKEN", "ghp_site_2").await.unwrap();
    assert!(s.set_secret(None, "1BAD", "x").await.is_err() && s.set_secret(None, "A-B", "x").await.is_err());
    let all = s.secrets_for("site").await.unwrap();
    assert_eq!(all.iter().map(|x| (x.name.as_str(), x.value.as_str(), x.project.as_deref())).collect::<Vec<_>>(), [("GH_TOKEN", "ghp_site_2", Some("site")), ("NPM_TOKEN", "npm_1", None)]);
    assert_eq!(s.secrets(None).await.unwrap()[0].value, "ghp_global");
    // Not readable in the database file, and the key file is the owner's only.
    let raw = std::fs::read(d.path().join("reagent.db")).unwrap_or_default();
    let wal = std::fs::read(d.path().join("reagent.db-wal")).unwrap_or_default();
    assert!(!String::from_utf8_lossy(&raw).contains("ghp_site_2") && !String::from_utf8_lossy(&wal).contains("ghp_site_2"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(d.path().join("secret.key")).unwrap().permissions().mode() & 0o777, 0o600);
    }
    // The same key next time; another key can't read them.
    let again = Store::open(&d.path().join("reagent.db")).await.unwrap();
    assert_eq!(again.secrets(Some("site")).await.unwrap()[0].value, "ghp_site_2");
    std::fs::write(d.path().join("secret.key"), [7u8; 32]).unwrap();
    let wrong = Store::open(&d.path().join("reagent.db")).await.unwrap();
    assert!(wrong.secrets(None).await.unwrap_err().contains("can't be decrypted"));
    assert!(s.remove_secret(Some("site"), "GH_TOKEN").await.unwrap());
    assert_eq!(s.secrets_for("site").await.unwrap()[0].value, "ghp_global");
    s.remove_project("site").await.unwrap();
}

#[tokio::test]
async fn action_tokens_are_taken_once_and_expire() {
    let (s, _d) = store().await;
    s.put_project(&Project::new("site", "Site", "/src/site")).await.unwrap();
    let t = s.add_task(&NewTask { project: "site".into(), title: "T".into(), prompt: "p".into(), origin: "ui".into(), cwd: "/".into(), profile: "default".into(), ..Default::default() }).await.unwrap();
    s.add_action_token("h1", &t.id, &serde_json::json!({"kind": "approve", "call_id": "c1"}), now() + 60).await.unwrap();
    s.add_action_token("h2", &t.id, &serde_json::json!({"kind": "deny"}), now() - 1).await.unwrap();
    let (task, a) = s.take_action_token("h1").await.unwrap().unwrap();
    assert_eq!((task.as_str(), a["call_id"].as_str()), (t.id.as_str(), Some("c1")));
    assert!(s.take_action_token("h1").await.unwrap().is_none(), "once");
    assert!(s.take_action_token("h2").await.unwrap().is_none(), "expired");
    assert!(s.take_action_token("nope").await.unwrap().is_none());
}
