//! Secrets: commands get them as env, the task can read them with a tool,
//! other results and job logs show them as ***, the person manages them.

mod common;

use common::*;
use serde_json::{Value, json};

#[tokio::test]
async fn commands_get_secrets_and_results_mask_them() {
    let r = start().await;
    r.allow_all().await;
    r.run.app.store.set_secret(None, "GH_TOKEN", "ghp_every_project").await.unwrap();
    r.run.app.store.set_secret(None, "NPM_TOKEN", "npm_secret_value").await.unwrap();
    r.run.app.store.set_secret(Some("site"), "GH_TOKEN", "ghp_site_only").await.unwrap();
    r.push("Sec", |b| {
        let all = all_text(b);
        assert!(all.contains("Environment variables your commands get") && all.contains("GH_TOKEN, NPM_TOKEN"), "names in the context: {all}");
        assert!(!all.contains("ghp_site_only"), "not the values");
        call("c1", "shell.exec", json!({"cmd": "echo gh=$GH_TOKEN npm=$NPM_TOKEN"}))
    });
    r.push("Sec", |b| {
        assert!(last_result(b).contains("gh=*** npm=***"), "masked: {}", last_result(b));
        call("c2", "secrets.secrets_list", json!({}))
    });
    r.push("Sec", |b| {
        assert_eq!(last_result(b), "GH_TOKEN (this project's)\nNPM_TOKEN (every project's)");
        call("c3", "secrets.secrets_get", json!({"name": "GH_TOKEN"}))
    });
    r.push("Sec", |b| {
        assert_eq!(last_result(b), "ghp_site_only", "the tool gives the value: the project's own");
        text("ok")
    });
    let t = r.start_task("Sec", "x").await;
    r.done(&t.id).await;
    // The command really got them; the log has them, the API masks them.
    let job = r.run.app.sup.jobs(Some(&t.id)).await.unwrap().remove(0);
    assert!(reagent_supervisor::tail(&r.run.app.paths.data, &job.id, 5).contains("gh=ghp_site_only npm=npm_secret_value"));
    r.run.app.store.set_setting("password", &reagent_web::auth::hash("secret-pass").unwrap()).await.unwrap();
    let c = reqwest::Client::builder().cookie_store(true).build().unwrap();
    let base = r.run.web_url.clone();
    c.post(format!("{base}/api/login")).json(&json!({"password": "secret-pass"})).send().await.unwrap();
    let out: Value = c.get(format!("{base}/api/jobs/{}/output", job.id)).send().await.unwrap().json().await.unwrap();
    assert_eq!(out["text"][0]["text"], "gh=*** npm=***");
    // The stored conversation has them masked too.
    let tr: Value = c.get(format!("{base}/api/tasks/{}/transcript?full=true", t.id)).send().await.unwrap().json().await.unwrap();
    let raw = tr.to_string();
    if let Some(i) = raw.find("npm_secret_value") {
        panic!("never in a tool result but the secrets tool's: …{}…", &raw[i.saturating_sub(400)..(i + 100).min(raw.len())]);
    }
}

#[tokio::test]
async fn the_person_manages_secrets() {
    let r = start().await;
    r.run.app.store.set_setting("password", &reagent_web::auth::hash("secret-pass").unwrap()).await.unwrap();
    let c = reqwest::Client::builder().cookie_store(true).build().unwrap();
    let base = r.run.web_url.clone();
    c.post(format!("{base}/api/login")).json(&json!({"password": "secret-pass"})).send().await.unwrap();
    assert!(c.put(format!("{base}/api/secrets")).json(&json!({"name": "API_KEY", "value": "k1"})).send().await.unwrap().status().is_success());
    assert!(c.put(format!("{base}/api/secrets")).json(&json!({"project": "site", "name": "API_KEY", "value": "k2"})).send().await.unwrap().status().is_success());
    assert_eq!(c.put(format!("{base}/api/secrets")).json(&json!({"name": "bad-name", "value": "x"})).send().await.unwrap().status(), 400);
    assert_eq!(c.put(format!("{base}/api/secrets")).json(&json!({"project": "nope", "name": "X", "value": "x"})).send().await.unwrap().status(), 404);
    let global: Value = c.get(format!("{base}/api/secrets?project=global")).send().await.unwrap().json().await.unwrap();
    assert_eq!((global[0]["name"].as_str(), global[0]["value"].as_str()), (Some("API_KEY"), Some("k1")), "the person sees values");
    let site: Value = c.get(format!("{base}/api/secrets?project=site")).send().await.unwrap().json().await.unwrap();
    assert_eq!(site[0]["value"], "k2");
    assert!(c.delete(format!("{base}/api/secrets?project=site&name=API_KEY")).send().await.unwrap().status().is_success());
    assert_eq!(r.run.app.store.secrets_for("site").await.unwrap()[0].value, "k1");
    assert_eq!(reqwest::get(format!("{base}/api/secrets")).await.unwrap().status(), 401);
}

#[tokio::test]
async fn a_task_sets_and_removes_its_projects_secrets() {
    let r = start().await;
    r.allow_all().await;
    r.run.app.store.set_secret(None, "SHARED", "every-project-value").await.unwrap();
    r.push("Keep", |_| call("c1", "secrets.secrets_set", json!({"name": "DEPLOY_KEY", "value": "dk_new_value_1"})));
    r.push("Keep", |b| {
        assert_eq!(last_result(b), "DEPLOY_KEY set for this project (commands get it as $DEPLOY_KEY)");
        call("c2", "shell.exec", json!({"cmd": "echo key=$DEPLOY_KEY"}))
    });
    r.push("Keep", |b| {
        assert!(last_result(b).contains("key=***"), "its commands get it, masked: {}", last_result(b));
        call("c3", "secrets.secrets_set", json!({"name": "bad-name", "value": "x"}))
    });
    r.push("Keep", |b| {
        assert!(last_result(b).contains("environment variable"), "{}", last_result(b));
        call("c4", "secrets.secrets_remove", json!({"name": "SHARED"}))
    });
    r.push("Keep", |b| {
        assert!(last_result(b).contains("no secret \"SHARED\" of its own"), "every project's isn't the task's: {}", last_result(b));
        call("c5", "secrets.secrets_remove", json!({"name": "DEPLOY_KEY"}))
    });
    r.push("Keep", |b| {
        assert_eq!(last_result(b), "removed DEPLOY_KEY");
        text("kept and dropped")
    });
    let t = r.start_task("Keep", "x").await;
    r.done(&t.id).await;
    assert!(r.run.app.store.secrets(Some("site")).await.unwrap().is_empty());
    assert_eq!(r.run.app.store.secrets(None).await.unwrap()[0].value, "every-project-value");
    let notes = r.run.app.store.notifications(20).await.unwrap();
    assert!(notes.iter().any(|n| n.kind == "secret" && n.title == "Keep set the secret DEPLOY_KEY"), "{notes:?}");
    assert!(notes.iter().any(|n| n.kind == "secret" && n.title == "Keep removed the secret DEPLOY_KEY"));
    assert!(notes.iter().all(|n| !n.body.contains("dk_new_value_1")), "the value is never told");
}
