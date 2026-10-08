//! The designer: a guide that turns a rough goal into work reagent can do
//! well. It runs as a task (origin `design`, `design_profile`) that may only
//! read the project and ask the person (ask.ask, with likely answers), and
//! ends with a proposal: a list of items of any kind - tasks, cron entries,
//! triggers, repo skills - in a ```json block of its report. The person
//! edits them, ticks the ones they want, and they're made. Its instructions
//! are the system skill `reagent-prompt-design`.

use std::path::Path;

use reagent_store::{Budget, Project};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::Config;

/// What a design task proposes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    /// What it assumed, and how the items fit together.
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub items: Vec<Item>,
}

/// One thing to make.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Item {
    /// A task to start now.
    Task {
        title: String,
        prompt: String,
        #[serde(default)]
        skills: Vec<String>,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        profile: Option<String>,
        #[serde(default)]
        budget: Option<Budget>,
        #[serde(default)]
        why: String,
    },
    /// Work that recurs on a schedule.
    Cron {
        expr: String,
        #[serde(default = "utc")]
        tz: String,
        title: String,
        prompt: String,
        #[serde(default)]
        skills: Vec<String>,
        #[serde(default)]
        why: String,
    },
    /// Work that should follow an event, found by a script.
    Trigger {
        name: String,
        mode: String,
        #[serde(default)]
        every: Option<String>,
        #[serde(default)]
        cron: Option<String>,
        #[serde(default)]
        script: String,
        title: String,
        prompt: String,
        /// A webhook's secret (one of the project's secrets).
        #[serde(default)]
        secret: Option<String>,
        /// In the repo (`.agents/triggers/<name>/`) or kept in reagent.
        #[serde(default = "yes")]
        repo: bool,
        #[serde(default)]
        why: String,
    },
    /// A procedure that will come up again: `.agents/skills/<name>/SKILL.md`.
    Skill {
        name: String,
        description: String,
        body: String,
        #[serde(default)]
        why: String,
    },
}

fn utc() -> String {
    "UTC".into()
}
fn yes() -> bool {
    true
}

impl Item {
    /// The item in a line.
    pub fn label(&self) -> String {
        match self {
            Item::Task { title, .. } => format!("task: {title}"),
            Item::Cron { expr, tz, title, .. } => format!("cron entry ({expr} {tz}): {title}"),
            Item::Trigger { name, mode, title, repo, .. } => format!("{mode} trigger {name}{}: {title}", if *repo { " (in the repo)" } else { "" }),
            Item::Skill { name, description, .. } => format!("repo skill {name}: {description}"),
        }
    }
}

/// The proposal in a design task's report: its last ```json block (items
/// that aren't one are left out, and so is what they name that doesn't
/// exist: a kind, a profile, a skill).
pub fn parse_report(report: &str, config: &Config, skills: &[String]) -> Option<Proposal> {
    let block = report.rsplit("```json").next().filter(|_| report.contains("```json"))?;
    let text = block.split("```").next()?;
    let v: Value = serde_json::from_str(text.trim()).ok()?;
    let items = v["items"].as_array()?.iter().filter_map(|i| serde_json::from_value::<Item>(i.clone()).ok()).map(|i| tidy(i, config, skills)).collect();
    Some(Proposal { note: v["note"].as_str().unwrap_or_default().to_string(), items })
}

fn tidy(item: Item, config: &Config, known: &[String]) -> Item {
    match item {
        Item::Task { title, prompt, mut skills, kind, profile, budget, why } => {
            skills.retain(|s| known.contains(s));
            Item::Task { title: title.trim().into(), prompt: prompt.trim().into(), skills, kind: kind.filter(|k| config.kind.contains_key(k)), profile: profile.filter(|p| config.profile.contains_key(p)), budget, why }
        }
        Item::Cron { expr, tz, title, prompt, mut skills, why } => {
            skills.retain(|s| known.contains(s));
            Item::Cron { expr, tz, title, prompt, skills, why }
        }
        other => other,
    }
}

/// Makes an item that isn't a task (a task is started by the caller); a line saying what.
pub async fn make(store: &reagent_store::Store, p: &Project, item: &Item) -> Result<String, String> {
    match item {
        Item::Task { .. } => Err("a task is started, not made".into()),
        Item::Skill { name, description, body, .. } => {
            crate::triggers::check_name(name).map_err(|_| format!("{name:?}: a skill's name is lowercase letters, digits, - and _"))?;
            let dir = Path::new(&p.path).join(".agents/skills").join(name);
            if dir.join("SKILL.md").exists() {
                return Err(format!("the project already has a skill {name} ({})", dir.display()));
            }
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let front = serde_yaml::to_string(&json!({"name": name, "description": description})).unwrap_or_default();
            std::fs::write(dir.join("SKILL.md"), format!("---\n{front}---\n\n{}\n", body.trim())).map_err(|e| e.to_string())?;
            Ok(format!("skill {name} written to {} (commit it)", dir.join("SKILL.md").display()))
        }
        Item::Cron { expr, tz, title, prompt, skills, .. } => {
            let mut c = reagent_store::Cron {
                id: 0,
                project: p.slug.clone(),
                expr: expr.clone(),
                tz: tz.clone(),
                title: title.clone(),
                prompt: prompt.clone(),
                options: reagent_store::Json(reagent_store::CronOptions { skills: skills.clone(), ..Default::default() }),
                overlap: "skip".into(),
                catch_up: true,
                enabled: true,
                last_run: None,
                next_run: None,
                queued: false,
            };
            crate::cron::prepare(&mut c, chrono::Utc::now().timestamp())?;
            let id = store.put_cron(&c).await.map_err(|e| e.to_string())?;
            Ok(format!("cron entry {id} ({expr} {tz}): {title}"))
        }
        Item::Trigger { name, mode, every, cron, script, title, prompt, secret, repo, .. } => {
            let def = crate::triggers::Def { name: name.clone(), mode: mode.clone(), every: every.clone(), cron: cron.clone(), script: Some(script.clone()), title: title.clone(), prompt: prompt.clone(), secret: secret.clone(), ..Default::default() };
            let t = crate::triggers::save(store, p, def.into_trigger(&p.slug, "person")?, true).await?;
            if *repo {
                let t = crate::triggers::move_to(store, p, &t.name, "repo").await?;
                return Ok(format!("trigger {} in the repo ({}; commit it)", t.name, crate::triggers::repo_dir(p, &t.name).display()));
            }
            Ok(format!("trigger {} kept in reagent", t.name))
        }
    }
}

/// What a design task's title says.
pub fn title(goal: &str) -> String {
    let first = goal.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let short: String = first.chars().take(60).collect();
    format!("Design: {short}{}", if first.chars().count() > 60 { "…" } else { "" })
}

/// A design task's prompt. `target`: what the person started from (task,
/// cron, trigger) or `subtask` (a task designs its subtask's work).
pub fn task_prompt(target: &str, goal: &str) -> String {
    let hint = match target {
        "cron" => "The person started from a cron entry: they likely want something done on a schedule. A cron prompt runs again and again, so it must make sense every time (no \"this time\") and say what to do when there's nothing to do.",
        "trigger" => "The person started from a trigger: they likely want work to follow an event. A trigger's title and prompt are templates for each event its script prints: put the event's data in with {{key}}, {{vars.<field>}}, {{message}}, and make the script print exactly those fields.",
        "subtask" => "A task asked for this: it will start the work you propose as its subtask(s). Its title and what it said are in the goal; ask the person only what neither it nor the project tells you.",
        _ => "",
    };
    format!(
        "You are reagent's designer: a guide more than a prompt writer. Help the person work out what they actually want done and the best way for reagent to do it, then propose it. Don't do the work itself.\n\n\
         The goal, as given:\n\n{}\n\n{hint}\n\n\
         How:\n\
         1. Read what you need first (the code, AGENTS.md, the memory, the skills, earlier tasks with tasks.search_history). You can only read and ask.\n\
         2. Ask the person what changes the result and the project doesn't show, with ask.ask: one question a call, always with 2-4 likely answers as options (they may answer in their own words, or say you decide). Point out what they may not have thought of: risks, the checks that show it's done, what must not change, whether it's once or for good. At most about ten questions; if they say to propose now, propose with what you know.\n\
         3. End with a short note and the proposal as a ```json block (the last one in your answer):\n\
         {{\"note\": \"what you assumed, how the items fit\", \"items\": [\n\
           {{\"type\": \"task\", \"title\": \"…\", \"prompt\": \"…\", \"skills\": [], \"kind\": null, \"profile\": null, \"budget\": null, \"why\": \"…\"}},\n\
           {{\"type\": \"cron\", \"expr\": \"0 3 * * 1\", \"tz\": \"UTC\", \"title\": \"…\", \"prompt\": \"…\", \"skills\": [], \"why\": \"…\"}},\n\
           {{\"type\": \"trigger\", \"name\": \"…\", \"mode\": \"poll\", \"every\": \"5m\", \"script\": \"prints one JSON line per event\", \"title\": \"…\", \"prompt\": \"…\", \"repo\": true, \"why\": \"…\"}},\n\
           {{\"type\": \"skill\", \"name\": \"lowercase-name\", \"description\": \"…\", \"body\": \"the SKILL.md instructions\", \"why\": \"…\"}}\n\
         ]}}\n\
         Any number of each, only those that fit; `why` in a line each. Skills, kinds and profiles only by names that exist. The person edits the items, ticks the ones they want, and they're made.\n\n\
         The skill `reagent-prompt-design` (loaded below) says what makes a prompt clear and when each kind of item fits.",
        goal.trim()
    )
}

/// The tools a design task may use (it reads, asks, plans; it changes nothing).
pub fn design_task_may(tool: &str) -> bool {
    matches!(tool, "fs.read" | "fs.grep" | "fs.glob" | "fs.ls" | "memory.memory_read" | "memory.memory_search" | "skills.skill_list" | "skills.skill_load" | "ask.ask" | "tasks.search_history" | "tasks.task_list" | "tasks.cron_list" | "triggers.trigger_list" | "secrets.secrets_list")
        || tool.starts_with("todo.")
}

/// A proposal as text, for agents.
pub fn proposal_text(p: &Proposal) -> String {
    format!("{}\n{}", p.note, serde_json::to_string_pretty(&p.items).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reports_proposal_is_read_and_tidied() {
        let cfg = Config::parse(crate::config::EXAMPLE).unwrap();
        let report = "Here it is.\n```json\n{\"note\": \"n\", \"items\": [\
            {\"type\": \"task\", \"title\": \" Fix CI \", \"prompt\": \"do it\", \"skills\": [\"deploy\", \"made-up\"], \"kind\": \"nope\", \"profile\": \"default\", \"budget\": {\"cost\": 2.0}},\
            {\"type\": \"nonsense\"},\
            {\"type\": \"trigger\", \"name\": \"ci\", \"mode\": \"poll\", \"every\": \"5m\", \"script\": \"x\", \"title\": \"t\", \"prompt\": \"p\"},\
            {\"type\": \"task\", \"title\": \"Second\", \"prompt\": \"p2\"}]}\n```\n";
        let p = parse_report(report, &cfg, &["deploy".into()]).unwrap();
        assert_eq!(p.note, "n");
        assert_eq!(p.items.len(), 3, "the odd one left out; two tasks are fine");
        match &p.items[0] {
            Item::Task { title, skills, kind, profile, budget, .. } => assert_eq!((title.as_str(), skills.as_slice(), kind, profile.as_deref(), budget.as_ref().and_then(|b| b.cost)), ("Fix CI", &["deploy".to_string()][..], &None, Some("default"), Some(2.0))),
            other => panic!("{other:?}"),
        }
        assert!(matches!(&p.items[1], Item::Trigger { repo: true, .. }), "triggers go into the repo unless said");
        assert_eq!(p.items[1].label(), "poll trigger ci (in the repo): t");
        assert!(parse_report("no block", &cfg, &[]).is_none());
        assert!(parse_report("```json\nnot json\n```", &cfg, &[]).is_none());
    }

    #[test]
    fn prompts_and_tools() {
        let p = task_prompt("trigger", "watch CI");
        assert!(p.contains("watch CI") && p.contains("{{vars.<field>}}") && p.contains("\"type\": \"skill\""), "{p}");
        assert_eq!(title("tidy the readme\nmore"), "Design: tidy the readme");
        assert!(design_task_may("fs.read") && design_task_may("todo.todo_add") && !design_task_may("fs.write") && !design_task_may("shell.exec"));
    }
}
