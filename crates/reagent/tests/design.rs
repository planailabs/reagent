//! The designer: a design task that only reads and asks, then proposes
//! items of any kind (tasks, cron entries, triggers, repo skills) that the
//! person picks and creates; for agents a design subtask; reagent's own
//! docs (system skills) for tasks, the web UI and the MCP API.

mod common;

use common::*;
use serde_json::{Value, json};

async fn login(r: &R) -> reqwest::Client {
    r.run.app.store.set_setting("password", &reagent_web::auth::hash("secret-pass").unwrap()).await.unwrap();
    let c = reqwest::Client::builder().cookie_store(true).build().unwrap();
    c.post(format!("{}/api/login", r.run.web_url)).json(&json!({"password": "secret-pass"})).send().await.unwrap();
    c
}

const PROPOSAL: &str = "Here's the plan.\n```json\n{\"note\": \"weekly, on main\", \"items\": [\
  {\"type\": \"task\", \"title\": \"Update deps now\", \"prompt\": \"Update the deps.\", \"skills\": [\"made-up\"], \"why\": \"once now\"},\
  {\"type\": \"cron\", \"expr\": \"0 4 * * 1\", \"tz\": \"UTC\", \"title\": \"Weekly deps\", \"prompt\": \"Update deps.\", \"why\": \"weekly\"},\
  {\"type\": \"trigger\", \"name\": \"ci\", \"mode\": \"poll\", \"every\": \"10m\", \"script\": \"true\", \"title\": \"CI {{key}}\", \"prompt\": \"Fix it.\", \"why\": \"on failure\"},\
  {\"type\": \"skill\", \"name\": \"release\", \"description\": \"How to release\", \"body\": \"1. Tag.\\n2. Push.\", \"why\": \"it recurs\"}]}\n```";

#[tokio::test]
async fn a_design_task_reads_asks_and_proposes_what_the_person_creates() {
    let r = start().await;
    r.allow_all().await;
    let c = login(&r).await;
    let base = r.run.web_url.clone();
    let t: Value = c.post(format!("{base}/api/design")).json(&json!({"project": "site", "goal": "keep the deps fresh", "target": "cron"})).send().await.unwrap().json().await.unwrap();
    let id = t["id"].as_str().unwrap().to_string();
    assert_eq!((t["origin"].as_str(), t["title"].as_str()), (Some("design"), Some("Design: keep the deps fresh")));
    // Its first round: the skill loaded, the goal and the cron hint; it may not write.
    r.push("Design: keep the deps fresh", |b| {
        let all = all_text(b);
        assert!(all.contains("## Skill: reagent-prompt-design") && all.contains("keep the deps fresh") && all.contains("again and again"), "{all}");
        call("c1", "fs.write", json!({"path": "x.txt", "text": "no"}))
    });
    r.push("Design: keep the deps fresh", |b| {
        assert!(last_result(b).contains("a design task only reads"), "{}", last_result(b));
        call("c2", "ask.ask", json!({"question": "How often?", "options": ["weekly", "daily"]}))
    });
    r.push("Design: keep the deps fresh", |b| {
        assert_eq!(last_result(b), "weekly");
        text(PROPOSAL)
    });
    r.until(&id, "its question", |t| t.wait.as_ref().is_some_and(|w| w.0["kind"] == "question")).await;
    assert_eq!(c.get(format!("{base}/api/tasks/{id}/proposal")).send().await.unwrap().json::<Value>().await.unwrap(), Value::Null, "none yet");
    r.run.app.answer(&id, "weekly").await.unwrap();
    r.done(&id).await;
    assert!(!r.project.path().join("x.txt").exists());
    let p: Value = c.get(format!("{base}/api/tasks/{id}/proposal")).send().await.unwrap().json().await.unwrap();
    assert_eq!((p["note"].as_str(), p["items"].as_array().unwrap().len()), (Some("weekly, on main"), 4), "{p}");
    assert_eq!(p["items"][0]["skills"], json!([]), "a skill that isn't there is dropped");
    // The person edits the task, ticks all but the trigger, and creates them.
    let mut items = p["items"].as_array().unwrap().clone();
    items[0]["title"] = json!("Update deps today");
    items.remove(2);
    r.push("Update deps today", |b| {
        assert!(all_text(b).contains("Update the deps."));
        text("updated")
    });
    let out: Value = c.post(format!("{base}/api/design/create")).json(&json!({"project": "site", "items": items})).send().await.unwrap().json().await.unwrap();
    assert!(out.as_array().unwrap().iter().all(|o| o["ok"] == true), "{out}");
    assert!(out[0]["done"].as_str().unwrap().starts_with("task Update deps today started"));
    assert_eq!(r.run.app.store.crons(Some("site")).await.unwrap()[0].title, "Weekly deps");
    let skill = std::fs::read_to_string(r.project.path().join(".agents/skills/release/SKILL.md")).unwrap();
    assert!(skill.starts_with("---\nname: release\ndescription: How to release\n---") && skill.contains("1. Tag."), "{skill}");
    assert!(r.run.app.store.trigger("site", "ci").await.unwrap().is_none(), "not ticked, not made");
    // A trigger in the repo; the same skill isn't overwritten.
    let p2: Value = c.get(format!("{base}/api/tasks/{id}/proposal")).send().await.unwrap().json().await.unwrap();
    let again: Value = c.post(format!("{base}/api/design/create")).json(&json!({"project": "site", "items": [p2["items"][2], p2["items"][3]]})).send().await.unwrap().json().await.unwrap();
    assert_eq!(again[0]["ok"], true);
    assert!(r.project.path().join(".agents/triggers/ci/TRIGGER.md").is_file());
    assert!(again[1]["error"].as_str().unwrap().contains("already has a skill release"));
    assert_eq!(c.post(format!("{base}/api/design")).json(&json!({"project": "site", "goal": " "})).send().await.unwrap().status(), 400);
    let plain = r.start_task("Plain", "x").await;
    assert_eq!(c.get(format!("{base}/api/tasks/{}/proposal", plain.id)).send().await.unwrap().status(), 400, "not a design task");
}

#[tokio::test]
async fn an_agent_has_its_subtasks_work_designed() {
    let r = start().await;
    r.push("Lead", |_| call("c1", "tasks.prompt_design", json!({"goal": "fix the greeting"})));
    r.push("Lead", |b| {
        assert!(last_result(b).starts_with("design task "), "{}", last_result(b));
        text("waiting for the design")
    });
    r.push("Design: fix the greeting", |b| {
        let all = all_text(b);
        assert!(all.contains("(Asked by the task \"Lead\"") && all.contains("subtask"), "{all}");
        text("```json\n{\"note\": \"\", \"items\": [{\"type\": \"task\", \"title\": \"Fix a.txt\", \"prompt\": \"Fix the greeting in a.txt.\"}]}\n```")
    });
    r.push("Lead", |b| {
        assert!(all_text(b).contains("Fix the greeting in a.txt."), "the proposal comes as a message");
        text("got it")
    });
    let t = r.start_task("Lead", "x").await;
    r.until(&t.id, "the design's report", |t| t.report.as_deref() == Some("got it")).await;
    let kids = r.run.app.store.tasks(None, Some(&t.id), false, 10).await.unwrap();
    assert_eq!((kids[0].origin.as_str(), kids[0].state.as_str()), ("design", "done"));
}

#[tokio::test]
async fn reagents_docs_are_skills_pages_and_mcp_tools() {
    let r = start().await;
    // Tasks see them (one line in the context) and load them.
    r.push("Docs", |b| {
        let all = all_text(b);
        assert!(all.contains("reagent's own documentation, as skills") && all.contains("reagent-subtasks"), "{all}");
        call("c1", "skills.skill_load", json!({"name": "reagent-policy"}))
    });
    r.push("Docs", |b| {
        assert!(last_result(b).contains("judged piece by piece"), "{}", last_result(b));
        text("read")
    });
    let t = r.start_task("Docs", "x").await;
    r.done(&t.id).await;
    assert!(r.data.path().join("system-skills/reagent/SKILL.md").is_file());
    let c = login(&r).await;
    let base = r.run.web_url.clone();
    let list: Value = c.get(format!("{base}/api/docs")).send().await.unwrap().json().await.unwrap();
    assert_eq!(list[0]["name"], "reagent");
    let doc: Value = c.get(format!("{base}/api/docs/reagent-secrets")).send().await.unwrap().json().await.unwrap();
    assert!(doc["body"].as_str().unwrap().contains("secrets_set"));
    assert_eq!(c.get(format!("{base}/api/docs/nope")).send().await.unwrap().status(), 404);
    let skills: Value = c.get(format!("{base}/api/projects/site/skills")).send().await.unwrap().json().await.unwrap();
    assert!(skills.as_array().unwrap().iter().any(|s| s["source"] == "system"));
}
