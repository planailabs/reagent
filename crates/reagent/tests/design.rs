//! The prompt designer (questions, then a proposal) for the person, for a
//! task's subtasks and for outside agents; the design task that only reads;
//! reagent's own docs (system skills) for tasks, the web UI and the MCP API.

mod common;

use common::*;
use serde_json::{Value, json};

async fn login(r: &R) -> reqwest::Client {
    r.run.app.store.set_setting("password", &reagent_web::auth::hash("secret-pass").unwrap()).await.unwrap();
    let c = reqwest::Client::builder().cookie_store(true).build().unwrap();
    c.post(format!("{}/api/login", r.run.web_url)).json(&json!({"password": "secret-pass"})).send().await.unwrap();
    c
}

#[tokio::test]
async fn the_designer_asks_then_proposes() {
    let r = start().await;
    std::fs::write(r.project.path().join("AGENTS.md"), "# Site\nBuild with make.\n").unwrap();
    let c = login(&r).await;
    let base = r.run.web_url.clone();
    r.push("designer", |b| {
        let user = b["messages"][1]["content"].as_str().unwrap();
        assert!(user.contains("Build with make.") && user.contains("make the site faster") && user.ends_with("or propose."), "{user}");
        assert!(b["messages"][0]["content"].as_str().unwrap().contains("# What a clear prompt has"), "the skill is its instructions");
        assert_eq!(b["response_format"]["type"], "json_object");
        text(r#"{"questions": [{"question": "Which pages?", "options": ["home", "all"]}]}"#)
    });
    let v: Value = c.post(format!("{base}/api/design")).json(&json!({"project": "site", "goal": "make the site faster"})).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["questions"][0]["options"][1], "all", "{v}");
    r.push("designer", |b| {
        let user = b["messages"][1]["content"].as_str().unwrap();
        assert!(user.contains("- Q: Which pages?\n  A: home") && user.ends_with("Propose now."), "{user}");
        text(r#"```json
{"proposal": {"title": "Speed up the home page", "prompt": "Make / load in under 1s.", "skills": ["nope"], "kind": null, "profile": "default", "budget": {"cost": 1.5}, "note": "assumed the home page"}}
```"#)
    });
    let v: Value = c.post(format!("{base}/api/design")).json(&json!({"project": "site", "goal": "make the site faster", "answers": [{"question": "Which pages?", "answer": "home"}], "propose": true})).send().await.unwrap().json().await.unwrap();
    assert_eq!((v["proposal"]["title"].as_str(), v["proposal"]["skills"].as_array().map(|a| a.len()), v["proposal"]["profile"].as_str()), (Some("Speed up the home page"), Some(0), Some("default")), "{v}");
    assert_eq!(c.post(format!("{base}/api/design")).json(&json!({"project": "site", "goal": " "})).send().await.unwrap().status(), 400);
}

#[tokio::test]
async fn a_task_designs_its_subtasks_prompt() {
    let r = start().await;
    r.push("designer", |b| {
        assert!(b["messages"][0]["content"].as_str().unwrap().contains("subtask"));
        assert!(b["messages"][1]["content"].as_str().unwrap().contains("(Asked by the task \"Lead\""));
        text(r#"{"questions": [{"question": "Which file?", "options": ["a.txt"]}]}"#)
    });
    r.push("designer", |_| text(r#"{"proposal": {"title": "Fix a.txt", "prompt": "Fix the greeting in a.txt.", "note": ""}}"#));
    r.push("Lead", |_| call("c1", "tasks.prompt_design", json!({"goal": "fix the greeting"})));
    r.push("Lead", |b| {
        assert!(last_result(b).contains("1. Which file? (e.g. a.txt)"), "{}", last_result(b));
        call("c2", "tasks.prompt_design", json!({"goal": "fix the greeting", "answers": [{"question": "Which file?", "answer": "a.txt"}]}))
    });
    r.push("Lead", |b| {
        assert!(last_result(b).contains("\"title\": \"Fix a.txt\""), "{}", last_result(b));
        text("designed")
    });
    // Asked without a rule: the starter rules allow it.
    let t = r.start_task("Lead", "x").await;
    r.done(&t.id).await;
}

#[tokio::test]
async fn a_design_task_only_reads_and_reports_a_proposal() {
    let r = start().await;
    r.allow_all().await;
    r.push_design();
    let t = r.run.app.start_design_task("site", "task", "tidy the readme").await.unwrap();
    assert_eq!((t.origin.as_str(), t.title.as_str()), ("design", "Design: tidy the readme"));
    let t = r.done(&t.id).await;
    assert!(t.report.unwrap().contains("```json"));
    assert!(!r.project.path().join("x.txt").exists(), "its write was refused");
}

impl R {
    fn push_design(&self) {
        self.push("Design: tidy the readme", |b| {
            let all = all_text(b);
            assert!(all.contains("## Skill: reagent-prompt-design") && all.contains("tidy the readme"), "the skill is loaded: {all}");
            call("c1", "fs.write", json!({"path": "x.txt", "text": "no"}))
        });
        self.push("Design: tidy the readme", |b| {
            assert!(last_result(b).contains("a design task only reads"), "{}", last_result(b));
            call("c2", "fs.read", json!({"path": "a.txt"}))
        });
        self.push("Design: tidy the readme", |_| text("Here it is.\n```json\n{\"title\": \"Tidy the README\", \"prompt\": \"…\"}\n```"));
    }
}

#[tokio::test]
async fn reagents_docs_are_skills_pages_and_mcp_tools() {
    let r = start().await;
    // Tasks see them (one line in the context) and load them.
    r.push("Docs", |b| {
        let all = all_text(b);
        assert!(all.contains("reagent's own documentation, as skills") && all.contains("reagent-subtasks"), "{all}");
        call("c1", "skills.skill_load", json!({"name": "reagent-triggers-nope"}))
    });
    r.push("Docs", |_| call("c2", "skills.skill_load", json!({"name": "reagent-policy"})));
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

#[tokio::test]
async fn suggestions_the_person_picks_are_made() {
    let r = start().await;
    let c = login(&r).await;
    let base = r.run.web_url.clone();
    let sugs = json!([
        {"type": "skill", "name": "release", "description": "How to release", "body": "1. Tag.\n2. Push.", "why": "it recurs"},
        {"type": "cron", "expr": "0 4 * * 1", "tz": "UTC", "title": "Weekly deps", "prompt": "Update deps.", "why": "weekly"},
        {"type": "trigger", "name": "ci", "mode": "poll", "every": "10m", "script": "true", "title": "CI {{key}}", "prompt": "Fix it.", "why": "on failure"},
        {"type": "skill", "name": "Bad Name", "description": "x", "body": "y"}
    ]);
    let out: Value = c.post(format!("{base}/api/design/apply")).json(&json!({"project": "site", "suggestions": sugs})).send().await.unwrap().json().await.unwrap();
    assert_eq!(out.as_array().unwrap().iter().map(|o| o["ok"].as_bool().unwrap()).collect::<Vec<_>>(), [true, true, true, false], "{out}");
    let skill = std::fs::read_to_string(r.project.path().join(".agents/skills/release/SKILL.md")).unwrap();
    assert!(skill.starts_with("---\nname: release\ndescription: How to release\n---") && skill.contains("1. Tag."), "{skill}");
    assert_eq!(r.run.app.store.crons(Some("site")).await.unwrap()[0].title, "Weekly deps");
    let t = r.run.app.store.trigger("site", "ci").await.unwrap().unwrap();
    assert_eq!((t.source.as_str(), t.every), ("repo", Some(600)), "in the repo by default");
    assert!(r.project.path().join(".agents/triggers/ci/TRIGGER.md").is_file());
    // The same skill again isn't overwritten.
    let again: Value = c.post(format!("{base}/api/design/apply")).json(&json!({"project": "site", "suggestions": [sugs[0]]})).send().await.unwrap().json().await.unwrap();
    assert!(again[0]["error"].as_str().unwrap().contains("already has a skill release"));
}
