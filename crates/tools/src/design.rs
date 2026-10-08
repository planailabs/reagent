//! The prompt designer: a rough goal becomes a task prompt an agent can act
//! on alone. A round at a time, a model (`design_profile`) either asks
//! questions (each with likely answers) or proposes a title, a prompt and
//! fitting skills, kind, profile and budget. Stateless: the caller keeps the
//! answers and sends them back. Its instructions are the system skill
//! `reagent-prompt-design`.

use std::path::{Path, PathBuf};

use reagent_store::{Budget, Project};
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::Config;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Answer {
    pub question: String,
    pub answer: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub question: String,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub budget: Option<Budget>,
    /// What it assumed, in a line.
    #[serde(default)]
    pub note: String,
    /// What else to keep: a repo skill, a cron entry, a trigger (the person picks).
    #[serde(default)]
    pub suggestions: Vec<Suggestion>,
}

/// Something the designer suggests keeping beside the task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Suggestion {
    /// A procedure that will come up again: `.agents/skills/<name>/SKILL.md` in the project.
    Skill {
        name: String,
        description: String,
        body: String,
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
}

fn utc() -> String {
    "UTC".into()
}
fn yes() -> bool {
    true
}

/// Makes what a suggestion says, for the person who chose it; a line saying what.
pub async fn apply(store: &reagent_store::Store, p: &Project, s: &Suggestion) -> Result<String, String> {
    match s {
        Suggestion::Skill { name, description, body, .. } => {
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
        Suggestion::Cron { expr, tz, title, prompt, .. } => {
            let mut c = reagent_store::Cron {
                id: 0,
                project: p.slug.clone(),
                expr: expr.clone(),
                tz: tz.clone(),
                title: title.clone(),
                prompt: prompt.clone(),
                options: Default::default(),
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
        Suggestion::Trigger { name, mode, every, cron, script, title, prompt, secret, repo, .. } => {
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

/// A round: questions, or a proposal.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Step {
    #[serde(default)]
    pub questions: Vec<Question>,
    #[serde(default)]
    pub proposal: Option<Proposal>,
}

/// What the designer is shown about a project: its folder, AGENTS.md, its
/// memory's index, its skills, the kinds and profiles there are.
pub fn context(data: &Path, global_skills: &[PathBuf], system: &Path, p: &Project, config: &Config) -> String {
    let mut out = format!("Project: {} ({}), folder {}\n", p.name, p.slug, p.path);
    if let Ok(a) = std::fs::read_to_string(Path::new(&p.path).join("AGENTS.md")) {
        out.push_str(&format!("\n## Its AGENTS.md\n\n{}\n", a.chars().take(8000).collect::<String>()));
    }
    let index = crate::memory::of_project(data, p).index();
    out.push_str(&format!("\n## Its memory's index\n\n{}\n", index.chars().take(4000).collect::<String>()));
    let skills = crate::skills::discover(Path::new(&p.path), Path::new(&p.path), global_skills, Some(system));
    let own: Vec<_> = skills.iter().filter(|s| s.source != "system").cloned().collect();
    if !own.is_empty() {
        out.push_str(&format!("\n## Skills a task can start with\n\n{}\n", crate::skills::listing(&own)));
    }
    if !config.kind.is_empty() {
        out.push_str(&format!(
            "\n## Kinds of task\n\n{}\n",
            config.kind.iter().map(|(n, k)| format!("- {n}: {}{}", k.description, k.profile.as_ref().map(|p| format!(" (profile {p})")).unwrap_or_default())).collect::<Vec<_>>().join("\n")
        ));
    }
    out.push_str(&format!(
        "\n## Model profiles\n\n{}\n(default: {})\n",
        config.profile.iter().map(|(n, pr)| format!("- {n}: {}", pr.model)).collect::<Vec<_>>().join("\n"),
        p.profile.as_deref().unwrap_or(&config.default_profile)
    ));
    out
}

/// The designer's instructions: the system skill, and the answer's form.
pub fn instructions(target: &str) -> String {
    let skill = crate::skills::system_text("reagent-prompt-design").map(|t| crate::skills::parse(&t).2).unwrap_or_default();
    let what = match target {
        "cron" => "The prompt is a cron entry's: it runs on a schedule, again and again, so it must make sense every time (no \"this time\"), and say what to do when there's nothing to do.",
        "trigger" => "The title and prompt are a trigger's templates: a task starts from them for each event its script prints. Put the event's data in with {{key}}, {{vars.<field>}}, {{message}}, and say in the note which fields you assume the script gives.",
        "subtask" => "A task asks you to design a prompt for a subtask it will start. It answers your questions itself; ask what it knows that the subtask must be told.",
        _ => "The prompt starts a task now.",
    };
    format!(
        "You are reagent's prompt designer, and more a guide than a prompt writer: you help the person work out what they actually want done and the best way for reagent to do it (once now, as a skill to repeat, on a schedule, when something happens; with which skills, model and budget), point out what they may not have thought of (risks, checks, what \"done\" means), and keep it short and friendly. The prompt is one of the things you hand over, not the whole of it. {what}\n\n{skill}\n\n# Your answer\n\nAnswer with one JSON object and nothing else, either\n\
         {{\"questions\": [{{\"question\": \"…\", \"options\": [\"…\", \"…\"]}}]}}  (1 to 4 questions, each with 2 to 4 likely answers; the person may also answer in their own words)\n\
         or\n\
         {{\"proposal\": {{\"title\": \"…\", \"prompt\": \"…\", \"skills\": [], \"kind\": null, \"profile\": null, \"budget\": null, \"note\": \"…\", \"suggestions\": []}}}}\n\
         (skills, kind and profile only from the lists you're shown; budget, if any, as {{\"tokens\": n, \"cost\": x, \"minutes\": n}}; note: one line on what you assumed).\n\
         suggestions: what else to keep, each the person may take or leave, with `why` (one line):\n\
         {{\"type\": \"skill\", \"name\": \"lowercase-name\", \"description\": \"…\", \"body\": \"the SKILL.md instructions\", \"why\": \"…\"}}\n\
         {{\"type\": \"cron\", \"expr\": \"0 3 * * *\", \"tz\": \"UTC\", \"title\": \"…\", \"prompt\": \"…\", \"why\": \"…\"}}\n\
         {{\"type\": \"trigger\", \"name\": \"…\", \"mode\": \"poll\", \"every\": \"5m\", \"script\": \"prints one JSON line per event\", \"title\": \"…\", \"prompt\": \"…\", \"repo\": true, \"why\": \"…\"}}\n\
         When you're told to propose now, propose."
    )
}

/// The text of the first message: the context, the goal, the answers so far.
pub fn request(ctx: &str, goal: &str, answers: &[Answer], propose: bool) -> String {
    let mut out = format!("{ctx}\n## The goal, as given\n\n{}\n", goal.trim());
    if !answers.is_empty() {
        out.push_str("\n## Questions answered so far\n\n");
        for a in answers {
            out.push_str(&format!("- Q: {}\n  A: {}\n", a.question.trim(), a.answer.trim()));
        }
    }
    out.push_str(if propose || answers.len() >= 12 { "\nPropose now." } else { "\nAsk what you still need, or propose." });
    out
}

/// The JSON object in a model's answer (fences and talk around it ignored).
pub fn parse_step(text: &str) -> Result<Step, String> {
    let (Some(a), Some(b)) = (text.find('{'), text.rfind('}')) else { return Err(format!("the designer didn't answer with JSON: {}", text.chars().take(300).collect::<String>())) };
    let mut v: Value = serde_json::from_str(&text[a..=b]).map_err(|e| format!("the designer's answer isn't JSON ({e}): {}", text.chars().take(300).collect::<String>()))?;
    // A suggestion that isn't one is left out, not the whole answer.
    let raw = v.get_mut("proposal").and_then(|p| p.get_mut("suggestions")).map(|s| std::mem::replace(s, json!([])));
    let mut step: Step = serde_json::from_value(v).map_err(|e| format!("the designer's answer isn't one ({e}): {}", text.chars().take(300).collect::<String>()))?;
    if let (Some(p), Some(Value::Array(items))) = (step.proposal.as_mut(), raw) {
        p.suggestions = items.into_iter().filter_map(|i| serde_json::from_value(i).ok()).collect();
    }
    if step.questions.is_empty() && step.proposal.is_none() {
        return Err("the designer neither asked nor proposed".into());
    }
    Ok(step)
}

/// Drops what the proposal names that isn't there (a skill, kind or profile).
pub fn tidy(mut step: Step, config: &Config, skills: &[String]) -> Step {
    if let Some(p) = &mut step.proposal {
        p.skills.retain(|s| skills.contains(s));
        p.kind = p.kind.take().filter(|k| config.kind.contains_key(k));
        p.profile = p.profile.take().filter(|k| config.profile.contains_key(k));
        p.title = p.title.trim().to_string();
        p.prompt = p.prompt.trim().to_string();
        step.questions.clear();
    }
    step.questions.truncate(4);
    step
}

/// One round with the designer's model.
pub async fn ask_model(config: &Config, system: &str, user: &str) -> Result<String, String> {
    let (_, prof) = config.profile(config.design_profile.as_deref())?;
    let provider = config.provider.get(&prof.provider).ok_or_else(|| format!("no provider {:?}", prof.provider))?;
    let key = match &provider.key_env {
        Some(k) => Some(std::env::var(k).map_err(|_| format!("{k} isn't set (the designer's profile needs it)"))?),
        None => None,
    };
    let mut body = json!({"model": prof.model, "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}], "response_format": {"type": "json_object"}});
    for (k, v) in &prof.params {
        body[k] = v.clone();
    }
    body["stream"] = json!(false);
    let mut req = reqwest::Client::new().post(format!("{}/chat/completions", provider.base_url.trim_end_matches('/'))).json(&body).timeout(std::time::Duration::from_secs(300));
    if let Some(k) = key {
        req = req.bearer_auth(k);
    }
    let res = req.send().await.map_err(|e| format!("the designer's model can't be reached: {e}"))?;
    let status = res.status();
    let v: Value = res.json().await.map_err(|e| format!("the designer's model answered oddly ({status}): {e}"))?;
    if !status.is_success() {
        return Err(format!("the designer's model said {status}: {}", v["error"]["message"].as_str().unwrap_or(&v.to_string())));
    }
    v["choices"][0]["message"]["content"].as_str().map(String::from).ok_or_else(|| format!("no answer from the designer's model: {v}"))
}

/// A round of design for a project: questions, or a proposal.
pub async fn step(config: &Config, data: &Path, global_skills: &[PathBuf], system: &Path, p: &Project, target: &str, goal: &str, answers: &[Answer], propose: bool) -> Result<Step, String> {
    if goal.trim().is_empty() {
        return Err("what should the task do? (a rough goal is enough)".into());
    }
    let ctx = context(data, global_skills, system, p, config);
    let text = ask_model(config, &instructions(target), &request(&ctx, goal, answers, propose)).await?;
    let skills: Vec<String> = crate::skills::discover(Path::new(&p.path), Path::new(&p.path), global_skills, Some(system)).into_iter().map(|s| s.name).collect();
    Ok(tidy(parse_step(&text)?, config, &skills))
}

/// A round as text, for agents (questions numbered, a proposal as JSON).
pub fn step_text(s: &Step) -> String {
    match &s.proposal {
        Some(p) => format!(
            "Proposal (pass title and prompt to tasks.task_spawn; change what you like){}:\n{}",
            if p.suggestions.is_empty() { "" } else { ". Its suggestions you may act on yourself: a skill with fs.write (.agents/skills/<name>/SKILL.md), a cron entry with tasks.cron_add, a trigger with triggers.trigger_add" },
            serde_json::to_string_pretty(&json!({"title": p.title, "prompt": p.prompt, "skills": p.skills, "kind": p.kind, "profile": p.profile, "budget": p.budget, "note": p.note, "suggestions": p.suggestions})).unwrap_or_default()
        ),
        None => format!(
            "Questions (answer them and call prompt_design again with `answers`, or with propose: true):\n{}",
            s.questions.iter().enumerate().map(|(i, q)| format!("{}. {}{}", i + 1, q.question, if q.options.is_empty() { String::new() } else { format!(" (e.g. {})", q.options.join(" / ")) })).collect::<Vec<_>>().join("\n")
        ),
    }
}

/// The prompt of a design task: it reads the project, asks the person,
/// and reports a proposal (a ```json block).
pub fn task_prompt(target: &str, goal: &str) -> String {
    format!(
        "Design the prompt for a {target} in this project; don't do the work itself.\n\nThe goal, as the person gave it:\n\n{}\n\n\
         1. Read what you need to understand it (the code, the memory, the skills): you can only read.\n\
         2. Ask the person what you still need with ask.ask (one question a call, with options), as `reagent-prompt-design` says.\n\
         3. Answer with a short note and the proposal as a ```json block: {{\"title\", \"prompt\", \"skills\", \"kind\", \"profile\", \"budget\", \"note\"}}.\n\n\
         Load the skill `reagent-prompt-design` first.",
        goal.trim()
    )
}

/// The tools a design task may use (it reads, asks, plans; it changes nothing).
pub fn design_task_may(tool: &str) -> bool {
    matches!(tool, "fs.read" | "fs.grep" | "fs.glob" | "fs.ls" | "memory.memory_read" | "memory.memory_search" | "skills.skill_list" | "skills.skill_load" | "ask.ask" | "tasks.search_history" | "tasks.task_list" | "secrets.secrets_list")
        || tool.starts_with("todo.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_parsed_and_tidied() {
        let cfg = Config::parse(crate::config::EXAMPLE).unwrap();
        let s = parse_step("Sure:\n```json\n{\"questions\": [{\"question\": \"Which branch?\", \"options\": [\"main\", \"dev\"]}]}\n```").unwrap();
        assert_eq!(s.questions[0].options, ["main", "dev"]);
        assert!(parse_step("no json here").is_err());
        assert!(parse_step("{}").unwrap_err().contains("neither"));
        let p = parse_step(r#"{"proposal": {"title": " Fix CI ", "prompt": "do it", "skills": ["deploy", "made-up"], "kind": "nope", "profile": "default", "budget": {"cost": 2.0}}}"#).unwrap();
        let t = tidy(p, &cfg, &["deploy".into()]);
        let pr = t.proposal.unwrap();
        assert_eq!((pr.title.as_str(), pr.skills.as_slice(), pr.kind, pr.profile.as_deref(), pr.budget.unwrap().cost), ("Fix CI", &["deploy".to_string()][..], None, Some("default"), Some(2.0)));
        let s = parse_step(r#"{"proposal": {"title": "t", "prompt": "p", "suggestions": [{"type": "skill", "name": "deploy", "description": "d", "body": "b"}, {"type": "nonsense"}, {"type": "trigger", "name": "ci", "mode": "poll", "every": "5m", "script": "x", "title": "t", "prompt": "p"}]}}"#).unwrap();
        let sug = &s.proposal.as_ref().unwrap().suggestions;
        assert_eq!(sug.len(), 2, "the odd one left out");
        assert!(matches!(&sug[1], Suggestion::Trigger { repo: true, .. }), "triggers go into the repo unless said");
        let text = step_text(&Step { questions: vec![Question { question: "Which?".into(), options: vec!["a".into(), "b".into()] }], proposal: None });
        assert!(text.contains("1. Which? (e.g. a / b)"));
    }

    #[test]
    fn requests_and_instructions() {
        let r = request("ctx\n", "make it fast", &[Answer { question: "How fast?".into(), answer: "2x".into() }], false);
        assert!(r.contains("make it fast") && r.contains("- Q: How fast?\n  A: 2x") && r.ends_with("or propose."));
        assert!(request("", "x", &[], true).ends_with("Propose now."));
        let i = instructions("trigger");
        assert!(i.contains("{{vars.<field>}}") && i.contains("# What a clear prompt has") && i.contains("\"proposal\""), "{i}");
        assert!(design_task_may("fs.read") && design_task_may("todo.todo_add") && !design_task_may("fs.write") && !design_task_may("shell.exec"));
    }
}
