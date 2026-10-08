//! Triggers: a script per project that watches something (a CI pipeline, a
//! queue, an inbox) and prints events, one JSON object per line on stdout:
//! `{"key", "to", "title", "message", "vars"}`. An event starts a task from
//! the trigger's templates (`to: "new"`, the default), or goes to a running
//! one as a message (`"running"`: the trigger's latest active task; or a
//! task's id). A key seen before is dropped.
//!
//! Modes: `poll` (every `every` seconds, or on a cron expression), `watch`
//! (runs for good in the supervisor, restarted when it ends) and `webhook`
//! (`POST /hook/<project>/<name>`, checked against one of the project's
//! secrets; the request goes to the script on stdin, or is the event).
//!
//! Kept in reagent.db, or in the repo as `.agents/triggers/<name>/TRIGGER.md`
//! (YAML frontmatter, the prompt as the body) with the script beside it;
//! either moves to the other. The policy decides whether a script runs
//! (`triggers.run`), unless the person allowed that script (by hash). Runs
//! that fail back off; after five in a row a task is started to repair it.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use reagent_store::{CronOptions, Json, Project, Store, Task, Trigger, TriggerRun, TriggerState};
use reagent_supervisor::SpawnArgs;
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::{App, StartTask};

/// Failed runs in a row before a repair task starts.
pub const REPAIR_AFTER: u32 = 5;
/// Backoff ceiling (seconds).
const MAX_BACKOFF: i64 = 3600;
/// Where repo triggers live in a project (the first is written).
pub const DIRS: [&str; 2] = [".agents/triggers", ".agent/triggers"];
/// Output kept per run (the end).
const KEEP_OUTPUT: usize = 20_000;

/// What runs keep in memory: polls going now, and one state change at a time.
#[derive(Default)]
pub struct Runs {
    running: Mutex<HashSet<String>>,
    lock: tokio::sync::Mutex<()>,
    /// Repo trigger files that couldn't be read, per project.
    pub repo_errors: Mutex<BTreeMap<String, Vec<String>>>,
}

// --- definitions ------------------------------------------------------------

/// `90`, `90s`, `2m`, `1h`, `1d` as seconds.
pub fn parse_every(s: &str) -> Result<i64, String> {
    let s = s.trim();
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: i64 = n.parse().map_err(|_| format!("{s:?}: a duration like 90s, 2m, 1h"))?;
    let mul = match unit.trim() {
        "" | "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => return Err(format!("{s:?}: a duration like 90s, 2m, 1h")),
    };
    Ok(n * mul)
}

pub fn check_name(n: &str) -> Result<(), String> {
    if n.is_empty() || n.len() > 60 || !n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
        return Err(format!("{n:?}: a trigger's name is lowercase letters, digits, - and _"));
    }
    Ok(())
}

/// Checks a definition.
pub fn check(t: &Trigger) -> Result<(), String> {
    check_name(&t.name)?;
    match t.mode.as_str() {
        "poll" => match (&t.every, &t.cron) {
            (Some(e), None) if *e >= 10 => {}
            (Some(_), None) => return Err("every: at least 10 seconds".into()),
            (None, Some(c)) => {
                crate::cron::next_run(c, &t.tz, 0)?;
            }
            _ => return Err("a poll runs `every` so often or on a `cron` expression: one of them".into()),
        },
        "watch" | "webhook" => {}
        _ => return Err("mode: poll, watch or webhook".into()),
    }
    if t.mode != "webhook" && t.script.trim().is_empty() {
        return Err(format!("a {} trigger needs a script", t.mode));
    }
    if t.mode == "webhook" {
        let s = t.secret.as_deref().ok_or("a webhook needs a secret: the name of one of the project's secrets")?;
        reagent_store::check_secret_name(s)?;
    }
    if !matches!(t.overlap.as_str(), "skip" | "queue" | "parallel") {
        return Err("overlap: skip, queue or parallel".into());
    }
    if !(1..=3600).contains(&t.timeout) {
        return Err("timeout: 1 to 3600 seconds".into());
    }
    if t.title.trim().is_empty() || t.prompt.trim().is_empty() {
        return Err("a trigger needs a title and a prompt (templates for the tasks it starts)".into());
    }
    if t.script_file.is_empty() || t.script_file.contains(['/', '\\']) || t.script_file == "TRIGGER.md" || t.script_file.starts_with('.') {
        return Err(format!("{:?}: the script's file name, beside TRIGGER.md", t.script_file));
    }
    Ok(())
}

/// The script's hash: an approval holds for exactly this script.
pub fn script_hash(t: &Trigger) -> String {
    crate::app::token_hash(&t.script)
}

/// The command line the policy judges: a repo trigger's file, a kept one's script.
pub fn command_line(t: &Trigger) -> String {
    if t.source == "repo" { format!("./{}/{}/{}", DIRS[0], t.name, t.script_file) } else { t.script.trim().to_string() }
}

/// The frontmatter of a TRIGGER.md.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Front {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    description: String,
    mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    every: Option<serde_yaml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cron: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tz: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    script: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    overlap: Option<String>,
    title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    budget: Option<reagent_store::Budget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    devshell: Option<bool>,
}

/// A trigger from its TRIGGER.md and the folder it's in (the script read from beside it).
pub fn from_markdown(project: &str, dir: &Path, text: &str) -> Result<Trigger, String> {
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
    check_name(&name)?;
    let t = text.trim_start_matches('\u{feff}');
    let rest = t.strip_prefix("---").ok_or("TRIGGER.md starts with YAML frontmatter (---)")?.trim_start_matches(['\r', '\n']);
    let end = rest.find("\n---").ok_or("the frontmatter isn't closed (---)")?;
    let f: Front = serde_yaml::from_str(&rest[..end]).map_err(|e| format!("frontmatter: {e}"))?;
    let body = rest[end + 4..].trim_start_matches('-').trim().to_string();
    let every = match f.every {
        None => None,
        Some(serde_yaml::Value::Number(n)) => Some(n.as_i64().ok_or("every: whole seconds")?),
        Some(serde_yaml::Value::String(s)) => Some(parse_every(&s)?),
        Some(_) => return Err("every: seconds, or a duration like 2m".into()),
    };
    let script_file = f.script.unwrap_or_else(|| "run".into());
    let script = if f.mode == "webhook" && !dir.join(&script_file).exists() { String::new() } else { std::fs::read_to_string(dir.join(&script_file)).map_err(|e| format!("{script_file}: {e}"))? };
    let tr = Trigger {
        project: project.into(),
        name,
        source: "repo".into(),
        made_by: "repo".into(),
        description: f.description,
        mode: f.mode,
        every,
        cron: f.cron,
        tz: f.tz.unwrap_or_else(|| "UTC".into()),
        script,
        script_file,
        timeout: f.timeout.unwrap_or(60),
        overlap: f.overlap.unwrap_or_else(|| "skip".into()),
        title: f.title,
        prompt: body,
        options: Json(CronOptions { profile: f.profile, kind: f.kind, budget: f.budget, skills: f.skills }),
        secret: f.secret,
        devshell: f.devshell.unwrap_or(true),
        enabled: true,
        state: Default::default(),
        created: 0,
    };
    check(&tr)?;
    Ok(tr)
}

/// A trigger as a TRIGGER.md.
pub fn to_markdown(t: &Trigger) -> String {
    let o = &t.options.0;
    let f = Front {
        description: t.description.clone(),
        mode: t.mode.clone(),
        every: t.every.map(|e| serde_yaml::Value::String(if e % 3600 == 0 { format!("{}h", e / 3600) } else if e % 60 == 0 { format!("{}m", e / 60) } else { format!("{e}s") })),
        cron: t.cron.clone(),
        tz: (t.tz != "UTC").then(|| t.tz.clone()),
        script: (t.script_file != "run").then(|| t.script_file.clone()),
        timeout: (t.timeout != 60).then_some(t.timeout),
        overlap: (t.overlap != "skip").then(|| t.overlap.clone()),
        title: t.title.clone(),
        profile: o.profile.clone(),
        kind: o.kind.clone(),
        skills: o.skills.clone(),
        budget: o.budget.clone(),
        secret: t.secret.clone(),
        devshell: (!t.devshell).then_some(false),
    };
    format!("---\n{}---\n\n{}\n", serde_yaml::to_string(&f).unwrap_or_default(), t.prompt.trim())
}

/// A repo trigger's folder (where it is, else where it would be written).
pub fn repo_dir(p: &Project, name: &str) -> PathBuf {
    DIRS.iter().map(|d| Path::new(&p.path).join(d).join(name)).find(|d| d.join("TRIGGER.md").is_file()).unwrap_or_else(|| Path::new(&p.path).join(DIRS[0]).join(name))
}

/// The triggers in a project's folder, and what couldn't be read.
pub fn read_repo(p: &Project) -> (Vec<Trigger>, Vec<String>) {
    let (mut found, mut errors, mut names) = (vec![], vec![], HashSet::new());
    for d in DIRS {
        let Ok(entries) = std::fs::read_dir(Path::new(&p.path).join(d)) else { continue };
        let mut dirs: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("TRIGGER.md").is_file()).collect();
        dirs.sort();
        for dir in dirs {
            let shown = format!("{d}/{}", dir.file_name().unwrap_or_default().to_string_lossy());
            match std::fs::read_to_string(dir.join("TRIGGER.md")).map_err(|e| e.to_string()).and_then(|text| from_markdown(&p.slug, &dir, &text)) {
                Ok(t) if names.insert(t.name.clone()) => found.push(t),
                Ok(_) => {}
                Err(e) => errors.push(format!("{shown}: {e}")),
            }
        }
    }
    (found, errors)
}

/// Writes a trigger's files into the project (TRIGGER.md and its script).
pub fn write_repo(p: &Project, t: &Trigger) -> Result<PathBuf, String> {
    let dir = repo_dir(p, &t.name);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(dir.join("TRIGGER.md"), to_markdown(t)).map_err(|e| e.to_string())?;
    if !t.script.is_empty() {
        let f = dir.join(&t.script_file);
        std::fs::write(&f, &t.script).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755));
        }
    }
    Ok(dir)
}

/// Removes a trigger's files from the project (its folder, if nothing else is in it).
pub fn remove_repo(p: &Project, t: &Trigger) -> Result<(), String> {
    let dir = repo_dir(p, &t.name);
    for f in ["TRIGGER.md", t.script_file.as_str()] {
        match std::fs::remove_file(dir.join(f)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", dir.join(f).display())),
        }
    }
    let _ = std::fs::remove_dir(&dir);
    Ok(())
}

/// Adds or changes a trigger (by the person when `by_person`: the script
/// they wrote is allowed). A repo trigger's files are written.
pub async fn save(store: &Store, p: &Project, mut t: Trigger, by_person: bool) -> Result<Trigger, String> {
    t.project = p.slug.clone();
    check(&t)?;
    let old = store.trigger(&p.slug, &t.name).await.map_err(|e| e.to_string())?;
    t.source = old.as_ref().map(|o| o.source.clone()).unwrap_or_else(|| "db".into());
    if let Some(o) = &old {
        t.made_by = if t.source == "repo" { "repo".into() } else { o.made_by.clone() };
    }
    if t.source == "repo" {
        write_repo(p, &t)?;
    }
    let mut state = old.map(|o| o.state.0).unwrap_or_default();
    if by_person {
        state.approved = Some(script_hash(&t));
        state.denied = None;
        state.asking = None;
    }
    // Changed: it runs (or is judged) again now.
    state.next_run = None;
    state.failures = 0;
    state.repair = None;
    t.state = Json(state);
    store.put_trigger(&t).await.map_err(|e| e.to_string())?;
    store.set_trigger_state(&p.slug, &t.name, &t.state.0).await.map_err(|e| e.to_string())?;
    store.trigger(&p.slug, &t.name).await.map_err(|e| e.to_string())?.ok_or_else(|| "it went away".into())
}

/// Moves a trigger into the repo (its files, left for the person to commit)
/// or back into reagent.db (its files removed). Its state goes with it.
pub async fn move_to(store: &Store, p: &Project, name: &str, to: &str) -> Result<Trigger, String> {
    let mut t = store.trigger(&p.slug, name).await.map_err(|e| e.to_string())?.ok_or_else(|| format!("no trigger {name:?} in {}", p.slug))?;
    match (to, t.source.as_str()) {
        ("repo", "repo") | ("db", "db") => return Err(format!("{name} is already in {}", if to == "repo" { "the repo" } else { "reagent" })),
        ("repo", _) => {
            write_repo(p, &t)?;
            t.source = "repo".into();
            t.made_by = "repo".into();
            store.put_trigger(&t).await.map_err(|e| e.to_string())?;
        }
        ("db", _) => {
            // Kept here first: the repo scan doesn't take it for gone.
            t.source = "db".into();
            store.put_trigger(&t).await.map_err(|e| e.to_string())?;
            remove_repo(p, &t)?;
        }
        _ => return Err("to: repo or db".into()),
    }
    store.trigger(&p.slug, name).await.map_err(|e| e.to_string())?.ok_or_else(|| "it went away".into())
}

/// A trigger as the tools take it (tasks', outside agents').
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
pub struct Def {
    /// Lowercase letters, digits, - and _. An existing trigger of this name is replaced.
    pub name: String,
    /// poll (runs every so often), watch (runs for good, a line per event) or webhook (POST /hook/<project>/<name>).
    pub mode: String,
    /// poll: how often (90s, 2m, 1h), or `cron`.
    pub every: Option<String>,
    /// poll: a five-field cron expression instead of `every`.
    pub cron: Option<String>,
    /// The cron's time zone (default UTC).
    pub tz: Option<String>,
    /// The script (sh, or a #! line). It prints one JSON object per line per event:
    /// {"key": "unique id", "to": "new" | "running" | "<task id>", "title"?, "message"?, "vars": {…}}.
    /// It gets $REAGENT_STATE (a folder it keeps), $REAGENT_LAST_RUN, $REAGENT_TASKS (its tasks going, with keys), the project's env and secrets.
    /// A webhook's script reads {headers, body} on stdin (none: the body is the event).
    pub script: Option<String>,
    /// Seconds a run may take (default 60).
    pub timeout: Option<i64>,
    /// While a task it started still goes: skip (default), queue or parallel.
    pub overlap: Option<String>,
    /// The tasks' title and prompt: templates with {{key}}, {{vars.x}}, {{message}}.
    pub title: String,
    pub prompt: String,
    pub description: Option<String>,
    pub profile: Option<String>,
    pub kind: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    /// A webhook's secret: the name of one of the project's secrets.
    pub secret: Option<String>,
    /// false: not in the project's nix dev shell.
    pub devshell: Option<bool>,
}

impl Def {
    pub fn into_trigger(self, project: &str, made_by: &str) -> Result<Trigger, String> {
        Ok(Trigger {
            project: project.into(),
            name: self.name,
            source: "db".into(),
            made_by: made_by.into(),
            description: self.description.unwrap_or_default(),
            mode: self.mode,
            every: self.every.as_deref().map(parse_every).transpose()?,
            cron: self.cron,
            tz: self.tz.unwrap_or_else(|| "UTC".into()),
            script: self.script.unwrap_or_default(),
            script_file: "run".into(),
            timeout: self.timeout.unwrap_or(60),
            overlap: self.overlap.unwrap_or_else(|| "skip".into()),
            title: self.title,
            prompt: self.prompt,
            options: Json(CronOptions { profile: self.profile, kind: self.kind, budget: None, skills: self.skills }),
            secret: self.secret,
            devshell: self.devshell.unwrap_or(true),
            enabled: true,
            state: Default::default(),
            created: 0,
        })
    }
}

/// A trigger in a line or two, for tools.
pub fn line(t: &Trigger) -> String {
    let when = match (&t.mode[..], &t.cron, t.every) {
        ("poll", Some(c), _) => format!("cron `{c}` {}", t.tz),
        ("poll", _, Some(e)) => format!("every {e}s"),
        (m, _, _) => m.to_string(),
    };
    let st = &t.state.0;
    let mut flags = vec![];
    if !t.enabled {
        flags.push("off".to_string());
    }
    if st.asking.is_some() {
        flags.push("waits for approval".into());
    }
    if st.failures > 0 {
        flags.push(format!("{} failures: {}", st.failures, st.last_error.as_deref().unwrap_or("").lines().next().unwrap_or("")));
    }
    format!("{} ({}, {}, in {}, by {}){}  → {}", t.name, t.mode, when, if t.source == "repo" { "the repo" } else { "reagent" }, t.made_by, if flags.is_empty() { String::new() } else { format!(" [{}]", flags.join("; ")) }, t.title)
}

// --- events -----------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Event {
    #[serde(default)]
    pub key: Option<String>,
    /// new (default), running, or a task's id.
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub vars: Value,
}

/// The events in a script's output: lines starting with `{` (others are its
/// log); a line that isn't a JSON object is an error.
pub fn parse_events(out: &str) -> (Vec<Event>, Vec<String>) {
    let (mut evs, mut errs) = (vec![], vec![]);
    for l in out.lines().map(str::trim).filter(|l| l.starts_with('{')) {
        match serde_json::from_str::<Event>(l) {
            Ok(mut e) => {
                if e.key.as_deref().is_none_or(str::is_empty) {
                    e.key = Some(crate::app::token_hash(l)[..16].to_string());
                }
                evs.push(e);
            }
            Err(err) => errs.push(format!("not an event ({err}): {}", l.chars().take(200).collect::<String>())),
        }
    }
    (evs, errs)
}

/// `{{key}}`, `{{vars.a.b}}`, `{{message}}`, … filled in from `ctx` (text as
/// it is, anything else as JSON; missing: nothing).
pub fn render(template: &str, ctx: &Value) -> String {
    let re = regex::Regex::new(r"\{\{\s*([A-Za-z0-9_.\-]+)\s*\}\}").expect("a fixed pattern");
    re.replace_all(template, |c: &regex::Captures| {
        let mut v = ctx;
        for part in c[1].split('.') {
            v = match v {
                Value::Array(a) => part.parse::<usize>().ok().and_then(|i| a.get(i)).unwrap_or(&Value::Null),
                _ => v.get(part).unwrap_or(&Value::Null),
            };
        }
        match v {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    })
    .into_owned()
}

fn origin(t: &Trigger) -> String {
    format!("trigger:{}", t.id())
}

// --- state --------------------------------------------------------------------

/// Changes a trigger's state (read fresh, one change at a time).
pub async fn update(app: &App, project: &str, name: &str, f: impl FnOnce(&mut TriggerState)) -> Option<TriggerState> {
    let _one = app.triggers.lock.lock().await;
    let t = app.store.trigger(project, name).await.ok()??;
    let mut s = t.state.0;
    f(&mut s);
    app.store.set_trigger_state(project, name, &s).await.ok()?;
    app.emit_trigger(project, name).await;
    Some(s)
}

/// What a trigger's script may do now.
enum Trust {
    Run,
    Ask,
    Deny(String),
}

/// The person's approval of this script, else the policy (`triggers.run`).
async fn trust(app: &App, t: &Trigger, p: &Project) -> Trust {
    let hash = script_hash(t);
    let st = &t.state.0;
    if st.approved.as_deref() == Some(&hash) {
        return Trust::Run;
    }
    if st.denied.as_deref() == Some(&hash) {
        return Trust::Deny("the person denied this script".into());
    }
    let rules = app.store.rules(&p.slug).await.unwrap_or_default();
    let default = crate::policy::Action::parse(&p.default_action).unwrap_or(crate::policy::Action::Ask);
    let cmd = command_line(t);
    let d = crate::policy::decide(&rules, default, &crate::policy::Call { tool: "triggers.run", command: Some(&cmd), target: None });
    match d.action {
        crate::policy::Action::Allow => Trust::Run,
        crate::policy::Action::Deny => Trust::Deny(format!("the policy denies it{}", d.rule.map(|r| format!(" (rule {} {})", r.tool, r.command.unwrap_or_default())).unwrap_or_default())),
        crate::policy::Action::Ask => {
            if st.asking.as_deref() != Some(&hash) {
                let (h, c) = (hash.clone(), cmd.clone());
                update(app, &p.slug, &t.name, |s| {
                    s.asking = Some(h);
                    s.command = Some(c);
                })
                .await;
                let excerpt: String = t.script.lines().take(15).collect::<Vec<_>>().join("\n");
                app.notify("trigger", None, &format!("trigger {} in {} wants to run", t.name, p.name), &format!("Made by {}. Its script:\n\n```\n{excerpt}\n```\n\nAllow or deny it on the project's triggers tab or in the inbox.", t.made_by)).await;
            }
            Trust::Ask
        }
    }
}

/// The person allows (once: this script; always: a rule) or denies a trigger's script.
pub async fn approve(app: &App, project: &str, name: &str, approved: bool, always: bool) -> Result<Trigger, String> {
    let t = app.store.trigger(project, name).await.map_err(|e| e.to_string())?.ok_or_else(|| format!("no trigger {name:?} in {project}"))?;
    let hash = script_hash(&t);
    if always && approved {
        let rule = reagent_store::Rule { id: 0, project: project.into(), pos: 0, tool: "triggers.run".into(), command: Some(command_line(&t)), target: None, action: "allow".into() };
        app.store.prepend_rule(project, &rule).await.map_err(|e| e.to_string())?;
    }
    update(app, project, name, |s| {
        s.asking = None;
        if approved {
            s.approved = Some(hash);
            s.denied = None;
            s.next_run = None;
        } else {
            s.denied = Some(hash);
        }
    })
    .await;
    app.store.trigger(project, name).await.map_err(|e| e.to_string())?.ok_or_else(|| "it went away".into())
}

// --- running scripts ----------------------------------------------------------

/// What a script gets: the project's environment and secrets, PATH, who it is.
async fn env(app: &App, t: &Trigger, p: &Project) -> Vec<(String, String)> {
    let mut e = crate::mcp::shell::project_env(app, p).await;
    let active: BTreeMap<String, String> = active_tasks(app, t).await.into_iter().map(|x| (x.id.clone(), t.state.0.tasks.get(&x.id).cloned().unwrap_or_default())).collect();
    e.push(("REAGENT_TRIGGER".into(), t.name.clone()));
    e.push(("REAGENT_STATE".into(), state_dir(app, t).display().to_string()));
    e.push(("REAGENT_LAST_RUN".into(), t.state.0.last_run.unwrap_or(0).to_string()));
    e.push(("REAGENT_TASKS".into(), json!(active.iter().map(|(id, key)| json!({"id": id, "key": key})).collect::<Vec<_>>()).to_string()));
    e
}

/// The folder a trigger's script keeps its own state in.
pub fn state_dir(app: &App, t: &Trigger) -> PathBuf {
    app.paths.data.join("triggers").join(&t.project).join(&t.name)
}

/// The command that runs the script: its file in the repo, or written from reagent.db.
fn script_command(app: &App, t: &Trigger, p: &Project) -> Result<String, String> {
    let file = if t.source == "repo" {
        repo_dir(p, &t.name).join(&t.script_file)
    } else {
        let d = state_dir(app, t);
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        let f = d.join("script");
        std::fs::write(&f, &t.script).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755));
        }
        f
    };
    std::fs::create_dir_all(state_dir(app, t)).map_err(|e| e.to_string())?;
    let q = crate::devshell::sh_quote(&file.display().to_string());
    let cmd = if t.script.starts_with("#!") { q } else { format!("sh {q}") };
    Ok(crate::devshell::wrap(p, Path::new(&p.path), &cmd, t.devshell))
}

/// How a poll or webhook run went.
struct Out {
    exit: Option<i32>,
    stdout: String,
    log: String,
    error: Option<String>,
}

/// Runs a script to its end (or its timeout: then it and what it started are killed).
async fn run_script(app: &App, t: &Trigger, p: &Project, stdin: Option<String>) -> Out {
    let cmd = match script_command(app, t, p) {
        Ok(c) => c,
        Err(e) => return Out { exit: None, stdout: String::new(), log: String::new(), error: Some(e) },
    };
    let mut c = tokio::process::Command::new("sh");
    c.arg("-c").arg(&cmd).current_dir(&p.path).envs(env(app, t, p).await).stdin(if stdin.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() }).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    #[cfg(unix)]
    c.process_group(0);
    let mut child = match c.spawn() {
        Ok(c) => c,
        Err(e) => return Out { exit: None, stdout: String::new(), log: String::new(), error: Some(format!("couldn't start: {e}")) },
    };
    let pid = child.id();
    if let (Some(input), Some(mut w)) = (stdin, child.stdin.take()) {
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let _ = w.write_all(input.as_bytes()).await;
        });
    }
    match tokio::time::timeout(std::time::Duration::from_secs(t.timeout as u64), child.wait_with_output()).await {
        Ok(Ok(o)) => {
            let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&o.stderr);
            Out { exit: o.status.code(), log: format!("{stdout}{stderr}"), stdout, error: (!o.status.success()).then(|| format!("exited with {}", o.status.code().map(|c| c.to_string()).unwrap_or("a signal".into()))) }
        }
        Ok(Err(e)) => Out { exit: None, stdout: String::new(), log: String::new(), error: Some(e.to_string()) },
        Err(_) => {
            #[cfg(unix)]
            if let Some(pid) = pid {
                let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGKILL);
            }
            Out { exit: None, stdout: String::new(), log: String::new(), error: Some(format!("took longer than {}s: killed", t.timeout)) }
        }
    }
}

fn keep_end(s: &str) -> String {
    let n = s.chars().count();
    if n <= KEEP_OUTPUT { s.to_string() } else { format!("[…]\n{}", s.chars().skip(n - KEEP_OUTPUT).collect::<String>()) }
}

/// A run's output and events: the events dispatched, the run recorded,
/// failures counted (backing off; a repair task after REPAIR_AFTER).
async fn finish(app: &App, t: &Trigger, p: &Project, started: i64, out: Out) {
    let (events, mut errors) = parse_events(&out.stdout);
    let mut notes = vec![];
    for e in &events {
        match dispatch(app, t, p, e).await {
            Ok(n) => notes.push(n),
            Err(err) => errors.push(err),
        }
    }
    let error = out.error.clone().or_else(|| (!errors.is_empty()).then(|| errors.join("\n")));
    let log = format!("{}{}", out.log, if notes.is_empty() { String::new() } else { format!("\n[reagent] {}", notes.join("\n[reagent] ")) });
    let log = app.mask(&p.slug, &keep_end(&log)).await;
    let now = chrono::Utc::now().timestamp();
    let run = TriggerRun { id: 0, started, ended: Some(now), exit: out.exit.map(i64::from), ok: error.is_none(), events: events.len() as i64, output: log.clone(), error: error.clone() };
    let _ = app.store.add_trigger_run(&p.slug, &t.name, &run).await;
    ended(app, t, p, error, &log).await;
}

/// Counts a run's end: a success ends a failure streak; a failure backs
/// off, and the fifth in a row starts a repair task (once per streak).
async fn ended(app: &App, t: &Trigger, p: &Project, error: Option<String>, log: &str) {
    let now = chrono::Utc::now().timestamp();
    let normal = next_due(t, now);
    let st = update(app, &p.slug, &t.name, |s| {
        s.last_run = Some(now);
        match &error {
            None => {
                s.failures = 0;
                s.last_error = None;
                s.repair = None;
                s.next_run = Some(normal);
            }
            Some(e) => {
                s.failures += 1;
                s.last_error = Some(e.chars().take(2000).collect());
                let base = t.every.unwrap_or(60).max(10);
                let backoff = (base << (s.failures - 1).min(12)).min(MAX_BACKOFF);
                s.next_run = Some(normal.max(now + backoff));
            }
        }
    })
    .await;
    if let (Some(e), Some(st)) = (&error, st)
        && st.failures >= REPAIR_AFTER
        && st.repair.is_none()
    {
        tracing::warn!(trigger = %t.id(), failures = st.failures, error = %e, "a trigger keeps failing: starting a task to repair it");
        repair(app, t, p, e, log, st.failures).await;
    }
}

/// When a poll runs next (after `now`); a watcher is started again after `every` (default 5 s).
fn next_due(t: &Trigger, now: i64) -> i64 {
    match (&t.cron, t.every) {
        (Some(c), _) if t.mode == "poll" => crate::cron::next_run(c, &t.tz, now).unwrap_or(now + 3600),
        (_, Some(e)) => now + e,
        _ => now + 5,
    }
}

async fn repair(app: &App, t: &Trigger, p: &Project, error: &str, log: &str, failures: u32) {
    let kept = if t.source == "repo" { format!("in the repo: {} (edit the files; it's read again within seconds)", repo_dir(p, &t.name).display()) } else { "in reagent's database (triggers.trigger_add with the same name replaces it)".to_string() };
    let prompt = format!(
        "The trigger `{}` of this project failed {failures} times in a row. Find out why and repair it.\n\nMode: {}{}\nKept {kept}.\nIts state folder: {}\n\nScript:\n```\n{}\n```\n\nLast error:\n```\n{}\n```\n\nLast output:\n```\n{}\n```",
        t.name,
        t.mode,
        match (&t.cron, t.every) {
            (Some(c), _) => format!(", cron `{c}` ({})", t.tz),
            (_, Some(e)) => format!(", every {e}s"),
            _ => String::new(),
        },
        state_dir(app, t).display(),
        t.script.trim(),
        error.trim(),
        log.lines().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"),
    );
    let started = app.start_task(StartTask { project: p.slug.clone(), title: format!("Repair trigger {}", t.name), prompt, origin: Some(format!("trigger-repair:{}", t.id())), ..Default::default() }).await;
    match started {
        Ok(task) => {
            update(app, &p.slug, &t.name, |s| s.repair = Some(task.id.clone())).await;
            app.notify("trigger", Some(&task), &format!("trigger {} keeps failing", t.name), &format!("{failures} runs in a row; a task repairs it.\n\n{}", error.chars().take(300).collect::<String>())).await;
        }
        Err(e) => {
            update(app, &p.slug, &t.name, |s| s.repair = Some(String::new())).await;
            app.notify("trigger", None, &format!("trigger {} keeps failing", t.name), &format!("{failures} runs in a row; the repair task didn't start: {e}")).await;
        }
    }
}

/// The trigger's tasks still going, newest first.
async fn active_tasks(app: &App, t: &Trigger) -> Vec<Task> {
    let o = origin(t);
    app.store.tasks(Some(&t.project), None, true, 1000).await.unwrap_or_default().into_iter().filter(|x| x.origin == o).collect()
}

/// Does what an event says; a line for the run's log.
async fn dispatch(app: &App, t: &Trigger, p: &Project, e: &Event) -> Result<String, String> {
    let key = e.key.clone().unwrap_or_default();
    if !app.store.trigger_key_new(&p.slug, &t.name, &key).await.map_err(|e| e.to_string())? {
        return Ok(format!("{key}: seen before, dropped"));
    }
    deliver(app, t, p, e).await
}

/// An event past the key check: a message to a task, or a new task (as the overlap allows).
async fn deliver(app: &App, t: &Trigger, p: &Project, e: &Event) -> Result<String, String> {
    let key = e.key.clone().unwrap_or_default();
    let ctx = json!({"key": key, "title": e.title, "message": e.message, "vars": e.vars, "trigger": t.name, "project": p.slug});
    let note = || format!("[trigger {}] {}", t.name, e.message.clone().or(e.title.clone()).unwrap_or_else(|| render(&t.title, &ctx)));
    match e.to.as_deref().unwrap_or("new") {
        "new" => {}
        "running" => {
            if let Some(x) = active_tasks(app, t).await.first() {
                app.message(&x.id, &note()).await?;
                return Ok(format!("{key}: to task {}", x.id));
            }
        }
        id => {
            let x = app.task(id).await.map_err(|_| format!("{key}: no task {id:?}"))?;
            if x.project != p.slug {
                return Err(format!("{key}: task {id} is another project's"));
            }
            app.message(&x.id, &note()).await?;
            return Ok(format!("{key}: to task {}", x.id));
        }
    }
    if t.overlap != "parallel" && !active_tasks(app, t).await.is_empty() {
        if t.overlap == "queue" {
            let ev = serde_json::to_value(e).unwrap_or_default();
            update(app, &p.slug, &t.name, |s| s.queue.push(ev)).await;
            return Ok(format!("{key}: queued (a task of this trigger still goes)"));
        }
        return Ok(format!("{key}: skipped (a task of this trigger still goes)"));
    }
    let task = start(app, t, p, e, &ctx).await?;
    Ok(format!("{key}: started task {}", task.id))
}

async fn start(app: &App, t: &Trigger, p: &Project, e: &Event, ctx: &Value) -> Result<Task, String> {
    let o = &t.options.0;
    let mut prompt = render(&t.prompt, ctx);
    if let Some(m) = &e.message
        && !t.prompt.contains("message")
    {
        prompt.push_str(&format!("\n\n{m}"));
    }
    let title = render(e.title.as_deref().unwrap_or(&t.title), ctx);
    let task = app
        .start_task(StartTask { project: p.slug.clone(), title, prompt, profile: o.profile.clone(), budget: o.budget.clone(), skills: o.skills.clone(), parent: None, origin: Some(origin(t)), kind: o.kind.clone() })
        .await?;
    let (id, key) = (task.id.clone(), e.key.clone().unwrap_or_default());
    update(app, &p.slug, &t.name, |s| {
        s.tasks.insert(id, key);
    })
    .await;
    Ok(task)
}

/// A task ended: a trigger's event queued behind it starts now.
pub async fn task_ended(app: &App, task: &Task) {
    let Some((project, name)) = task.origin.strip_prefix("trigger:").and_then(|o| o.split_once('/')) else { return };
    let mut next = None;
    update(app, project, name, |s| {
        s.tasks.remove(&task.id);
        if !s.queue.is_empty() {
            next = Some(s.queue.remove(0));
        }
    })
    .await;
    let (Some(ev), Ok(Some(t)), Ok(p)) = (next, app.store.trigger(project, name).await, app.project(project).await) else { return };
    let Ok(ev) = serde_json::from_value::<Event>(ev) else { return };
    if let Err(e) = deliver(app, &t, &p, &ev).await {
        tracing::warn!(trigger = %t.id(), error = %e, "a queued event didn't start");
    }
}

// --- the modes ------------------------------------------------------------------

/// A poll run (or a run asked for now).
async fn poll(app: Arc<App>, t: Trigger, p: Project) {
    let started = chrono::Utc::now().timestamp();
    let out = run_script(&app, &t, &p, None).await;
    finish(&app, &t, &p, started, out).await;
}

/// A watcher: started in the supervisor when it doesn't run; its new
/// complete output lines read for events; its end counted like a run's.
async fn watch(app: &App, t: &Trigger, p: &Project, now: i64) {
    let st = &t.state.0;
    if let Some(job) = &st.job {
        let meta = app.sup.job(job).await;
        let log = reagent_supervisor::server::job_dir(&app.paths.data, job).join("output.log");
        let (lines, upto) = read_from(&log, st.line);
        let mut errors = vec![];
        if !lines.is_empty() {
            let (events, errs) = parse_events(&lines);
            errors.extend(errs);
            for e in &events {
                match dispatch(app, t, p, e).await {
                    Ok(n) => tracing::info!(trigger = %t.id(), "{n}"),
                    Err(err) => errors.push(err),
                }
            }
            update(app, &p.slug, &t.name, |s| s.line = upto).await;
        }
        for e in &errors {
            tracing::warn!(trigger = %t.id(), error = %e, "a watcher's event");
        }
        match meta {
            Ok(m) if m.running() => return,
            Ok(m) => {
                // Ended: what it printed last, then its end.
                let (rest, _) = read_from(&log, upto);
                let (events, _) = parse_events(&rest);
                for e in &events {
                    let _ = dispatch(app, t, p, e).await;
                }
                let error = match (m.exit, m.lost) {
                    (_, true) => Some("lost (the supervisor stopped while it ran)".to_string()),
                    (Some(0), _) => None,
                    (Some(c), _) => Some(format!("exited with {c}")),
                    _ => Some(format!("ended by signal {}", m.signal.unwrap_or(0))),
                };
                let tail = app.mask(&p.slug, &reagent_supervisor::tail(&app.paths.data, job, 200)).await;
                let run = TriggerRun { id: 0, started: m.started, ended: m.ended, exit: m.exit.map(i64::from), ok: error.is_none(), events: 0, output: format!("[job {job}]\n{tail}"), error: error.clone() };
                let _ = app.store.add_trigger_run(&p.slug, &t.name, &run).await;
                let _ = app.sup.ack(job).await;
                update(app, &p.slug, &t.name, |s| {
                    s.job = None;
                    s.line = 0;
                })
                .await;
                ended(app, t, p, error, &tail).await;
                return;
            }
            Err(_) => {
                update(app, &p.slug, &t.name, |s| {
                    s.job = None;
                    s.line = 0;
                })
                .await;
            }
        }
    }
    if st.next_run.is_some_and(|n| n > now) {
        return;
    }
    match trust(app, t, p).await {
        Trust::Run => {}
        Trust::Ask => return,
        Trust::Deny(why) => return refused(app, t, p, &why).await,
    }
    let cmd = match script_command(app, t, p) {
        Ok(c) => c,
        Err(e) => return ended(app, t, p, Some(e.clone()), &e).await,
    };
    match app.sup.spawn(SpawnArgs { cmd, cwd: p.path.clone(), env: env(app, t, p).await, owner: Some(origin(t)), name: Some(format!("trigger {}", t.name)), fg: false, stdin: None }).await {
        Ok(j) => {
            tracing::info!(trigger = %t.id(), job = %j.id, "watcher started");
            update(app, &p.slug, &t.name, |s| {
                s.job = Some(j.id.clone());
                s.line = 0;
                s.last_run = Some(now);
            })
            .await;
        }
        Err(e) => ended(app, t, p, Some(format!("couldn't start: {e}")), "").await,
    }
}

/// The complete lines of a file after byte `from`, and where they end.
fn read_from(path: &Path, from: usize) -> (String, usize) {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return (String::new(), from) };
    if f.seek(SeekFrom::Start(from as u64)).is_err() {
        return (String::new(), from);
    }
    let mut buf = vec![];
    let _ = f.take(4 << 20).read_to_end(&mut buf);
    let Some(end) = buf.iter().rposition(|b| *b == b'\n') else { return (String::new(), from) };
    (String::from_utf8_lossy(&buf[..=end]).into_owned(), from + end + 1)
}

/// The policy denies a trigger's script: not run, said once.
async fn refused(app: &App, t: &Trigger, p: &Project, why: &str) {
    let msg = format!("not run: {why}");
    if t.state.0.last_error.as_deref() != Some(msg.as_str()) {
        app.notify("trigger", None, &format!("trigger {} in {} doesn't run", t.name, p.name), why).await;
    }
    let next = next_due(t, chrono::Utc::now().timestamp()).max(chrono::Utc::now().timestamp() + 60);
    update(app, &p.slug, &t.name, |s| {
        s.last_error = Some(msg);
        s.next_run = Some(next);
    })
    .await;
}

/// A webhook call: checked against the trigger's secret, then (in the
/// background) through its script, or as one event.
pub async fn webhook(app: Arc<App>, project: &str, name: &str, headers: &axum::http::HeaderMap, body: axum::body::Bytes) -> Result<(), (u16, String)> {
    let t = app.store.trigger(project, name).await.map_err(|e| (500, e.to_string()))?.filter(|t| t.mode == "webhook" && t.enabled).ok_or((404, "no such webhook".to_string()))?;
    let p = app.project(project).await.map_err(|e| (404, e))?;
    let want = t.secret.as_deref().ok_or((403, "the trigger has no secret".to_string()))?;
    let secret = app.store.secrets_for(project).await.map_err(|e| (500, e))?.into_iter().find(|s| s.name == want).map(|s| s.value).ok_or((403, format!("the secret {want} isn't set")))?;
    if !signed(headers, &body, &secret) {
        return Err((401, "a bad or missing signature".into()));
    }
    let text = String::from_utf8_lossy(&body).into_owned();
    let hdrs: BTreeMap<String, String> = headers.iter().filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string()))).filter(|(k, _)| !matches!(k.as_str(), "authorization" | "x-gitlab-token" | "cookie")).collect();
    tokio::spawn(async move {
        let started = chrono::Utc::now().timestamp();
        if t.script.trim().is_empty() {
            let delivery = ["x-github-delivery", "x-gitlab-event-uuid", "x-request-id"].iter().find_map(|h| hdrs.get(*h).cloned());
            let vars = serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({"body": text}));
            let key = delivery.unwrap_or_else(|| crate::app::token_hash(&text)[..16].to_string());
            let line = json!({"key": key, "vars": vars}).to_string();
            return finish(&app, &t, &p, started, Out { exit: Some(0), stdout: line.clone(), log: String::new(), error: None }).await;
        }
        match trust(&app, &t, &p).await {
            Trust::Run => {}
            Trust::Ask => return,
            Trust::Deny(why) => return refused(&app, &t, &p, &why).await,
        }
        let input = json!({"headers": hdrs, "body": serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text))}).to_string();
        let out = run_script(&app, &t, &p, Some(input)).await;
        finish(&app, &t, &p, started, out).await;
    });
    Ok(())
}

/// GitHub's HMAC (`X-Hub-Signature-256`), GitLab's token (`X-Gitlab-Token`) or `Authorization: Bearer`.
pub fn signed(headers: &axum::http::HeaderMap, body: &[u8], secret: &str) -> bool {
    use hmac::Mac;
    let h = |n: &str| headers.get(n).and_then(|v| v.to_str().ok());
    if let Some(sig) = h("x-hub-signature-256").and_then(|s| s.strip_prefix("sha256=")) {
        let Ok(want) = (0..sig.len()).step_by(2).map(|i| sig.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok())).collect::<Option<Vec<u8>>>().ok_or(()) else { return false };
        let Ok(mut m) = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()) else { return false };
        m.update(body);
        return m.verify_slice(&want).is_ok();
    }
    // Compared as hashes: no early exit on the secret's bytes.
    let same = |given: &str| crate::app::token_hash(given) == crate::app::token_hash(secret);
    if let Some(tok) = h("x-gitlab-token") {
        return same(tok);
    }
    h("authorization").and_then(|a| a.strip_prefix("Bearer ")).is_some_and(same)
}

/// Runs a poll now (or restarts a watcher).
pub async fn run_now(app: &Arc<App>, project: &str, name: &str) -> Result<String, String> {
    let t = app.store.trigger(project, name).await.map_err(|e| e.to_string())?.ok_or_else(|| format!("no trigger {name:?} in {project}"))?;
    match t.mode.as_str() {
        "webhook" => Err("a webhook runs when it's called".into()),
        "watch" => {
            if let Some(j) = &t.state.0.job {
                let _ = app.sup.kill(j, None).await;
            }
            update(app, project, name, |s| s.next_run = None).await;
            Ok("the watcher starts again".into())
        }
        _ => {
            update(app, project, name, |s| s.next_run = Some(0)).await;
            Ok("it runs within a second".into())
        }
    }
}

/// Removes a trigger: its watcher stops, a repo trigger's files go too.
pub async fn remove(app: &App, project: &str, name: &str) -> Result<(), String> {
    let t = app.store.trigger(project, name).await.map_err(|e| e.to_string())?.ok_or_else(|| format!("no trigger {name:?} in {project}"))?;
    if let Some(j) = &t.state.0.job {
        let _ = app.sup.kill(j, None).await;
    }
    if t.source == "repo" {
        let p = app.project(project).await?;
        app.store.put_trigger(&Trigger { source: "db".into(), ..t.clone() }).await.map_err(|e| e.to_string())?;
        remove_repo(&p, &t)?;
    }
    app.store.remove_trigger(project, name).await.map_err(|e| e.to_string())?;
    app.emit_trigger(project, name).await;
    Ok(())
}

// --- the schedule ---------------------------------------------------------------

/// Reads every project's repo triggers into reagent.db (a db trigger of the
/// same name wins; a repo trigger whose files are gone goes).
pub async fn sync_repo(app: &App) {
    let _one = app.triggers.lock.lock().await;
    for p in app.store.projects().await.unwrap_or_default() {
        let (found, mut errors) = read_repo(&p);
        let names: HashSet<String> = found.iter().map(|t| t.name.clone()).collect();
        let have = app.store.triggers(Some(&p.slug)).await.unwrap_or_default();
        for mut t in found {
            match have.iter().find(|h| h.name == t.name) {
                Some(h) if h.source == "db" => {
                    errors.push(format!("{}: reagent already keeps a trigger called {} (move one, or rename it)", DIRS[0], t.name));
                    continue;
                }
                Some(h) => {
                    let same = Trigger { enabled: h.enabled, state: h.state.clone(), created: h.created, ..t.clone() } == *h;
                    if same {
                        continue;
                    }
                    t.enabled = h.enabled;
                    t.state = h.state.clone();
                }
                None => {}
            }
            if let Err(e) = app.store.put_trigger(&t).await {
                errors.push(format!("{}: {e}", t.name));
            }
            app.emit_trigger(&p.slug, &t.name).await;
        }
        for gone in have.iter().filter(|h| h.source == "repo" && !names.contains(&h.name)) {
            if let Some(j) = &gone.state.0.job {
                let _ = app.sup.kill(j, None).await;
            }
            let _ = app.store.remove_trigger(&p.slug, &gone.name).await;
            app.emit_trigger(&p.slug, &gone.name).await;
        }
        let mut all = app.triggers.repo_errors.lock().unwrap();
        if errors.is_empty() {
            all.remove(&p.slug);
        } else {
            all.insert(p.slug.clone(), errors);
        }
    }
}

/// One pass: due polls start, watchers are looked after.
pub async fn tick(app: &Arc<App>, now: i64) {
    let projects: BTreeMap<String, Project> = app.store.projects().await.unwrap_or_default().into_iter().map(|p| (p.slug.clone(), p)).collect();
    for t in app.store.triggers(None).await.unwrap_or_default() {
        let Some(p) = projects.get(&t.project) else { continue };
        if !t.enabled {
            if let Some(j) = &t.state.0.job {
                let _ = app.sup.kill(j, None).await;
                let _ = app.sup.ack(j).await;
                update(app, &p.slug, &t.name, |s| {
                    s.job = None;
                    s.line = 0;
                })
                .await;
            }
            continue;
        }
        match t.mode.as_str() {
            "poll" => {
                let due = t.state.0.next_run.is_none_or(|n| n <= now);
                // A cron poll's first run is at its first time, not now.
                if t.state.0.next_run.is_none() && t.cron.is_some() && t.state.0.last_run.is_none() {
                    let next = next_due(&t, now);
                    update(app, &p.slug, &t.name, |s| s.next_run = Some(next)).await;
                    continue;
                }
                if !due || !app.triggers.running.lock().unwrap().insert(t.id()) {
                    continue;
                }
                match trust(app, &t, p).await {
                    Trust::Run => {
                        // Not again before this run ends.
                        let (app, p) = (app.clone(), p.clone());
                        tokio::spawn(async move {
                            let id = t.id();
                            poll(app.clone(), t, p).await;
                            app.triggers.running.lock().unwrap().remove(&id);
                        });
                    }
                    Trust::Ask => {
                        app.triggers.running.lock().unwrap().remove(&t.id());
                    }
                    Trust::Deny(why) => {
                        refused(app, &t, p, &why).await;
                        app.triggers.running.lock().unwrap().remove(&t.id());
                    }
                }
            }
            "watch" => watch(app, &t, p, now).await,
            _ => {}
        }
    }
}

/// Runs the triggers for as long as reagent runs.
pub fn schedule(app: Arc<App>) {
    tokio::spawn(async move {
        let mut n = 0u64;
        loop {
            if n % 10 == 0 {
                sync_repo(&app).await;
            }
            tick(&app, chrono::Utc::now().timestamp()).await;
            n += 1;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trig(v: Value) -> Trigger {
        let mut base = json!({"project": "site", "name": "ci", "mode": "poll", "every": 60, "title": "Fix {{key}}", "prompt": "p", "script": "echo"});
        base.as_object_mut().unwrap().extend(v.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn durations_names_and_checks() {
        assert_eq!(parse_every("90").unwrap(), 90);
        assert_eq!(parse_every("2m").unwrap(), 120);
        assert_eq!(parse_every("1h").unwrap(), 3600);
        assert!(parse_every("soon").is_err());
        assert!(check_name("Bad Name").is_err());
        assert!(check(&trig(json!({}))).is_ok());
        assert!(check(&trig(json!({"every": 5}))).unwrap_err().contains("10 seconds"));
        assert!(check(&trig(json!({"every": null}))).unwrap_err().contains("one of them"));
        assert!(check(&trig(json!({"every": null, "cron": "*/5 * * * *"}))).is_ok());
        assert!(check(&trig(json!({"every": null, "cron": "nope"}))).is_err());
        assert!(check(&trig(json!({"mode": "watch", "script": " "}))).unwrap_err().contains("needs a script"));
        assert!(check(&trig(json!({"mode": "webhook", "script": ""}))).unwrap_err().contains("secret"));
        assert!(check(&trig(json!({"mode": "webhook", "script": "", "secret": "HOOK"}))).is_ok());
        assert!(check(&trig(json!({"overlap": "sometimes"}))).is_err());
        assert!(check(&trig(json!({"script_file": "../x"}))).is_err());
    }

    #[test]
    fn events_are_json_lines_and_keys_default_to_a_hash() {
        let (ev, errs) = parse_events("checking…\n{\"key\": \"812\", \"vars\": {\"url\": \"u\"}}\n{\"to\": \"running\", \"message\": \"again\"}\n{broken\n");
        assert_eq!(ev.len(), 2);
        assert_eq!((ev[0].key.as_deref(), ev[0].vars["url"].as_str()), (Some("812"), Some("u")));
        assert_eq!((ev[1].to.as_deref(), ev[1].key.as_ref().map(|k| k.len())), (Some("running"), Some(16)));
        assert_eq!(errs.len(), 1);
        let (again, _) = parse_events("{\"to\": \"running\", \"message\": \"again\"}");
        assert_eq!(again[0].key, ev[1].key, "the same line, the same key");
    }

    #[test]
    fn templates_fill_in_paths() {
        let ctx = json!({"key": "812", "vars": {"url": "https://ci/812", "n": 3, "list": ["a", "b"], "o": {"x": 1}}});
        assert_eq!(render("Fix {{key}}: {{ vars.url }} ({{vars.n}}) {{vars.list.1}} {{vars.o}} {{vars.missing}}.", &ctx), "Fix 812: https://ci/812 (3) b {\"x\":1} .");
    }

    #[test]
    fn markdown_goes_both_ways() {
        let d = tempfile::tempdir().unwrap();
        let mut p = Project::new("site", "Site", &d.path().display().to_string());
        p.slug = "site".into();
        let mut t = trig(json!({"every": 120, "description": "CI on main", "overlap": "queue", "options": {"profile": "big", "skills": ["fix-ci"]}, "devshell": false}));
        t.script = "#!/bin/sh\ngh run list\n".into();
        let dir = write_repo(&p, &t).unwrap();
        assert_eq!(dir, d.path().join(".agents/triggers/ci"));
        let md = std::fs::read_to_string(dir.join("TRIGGER.md")).unwrap();
        assert!(md.contains("every: 2m") && md.contains("overlap: queue") && md.contains("devshell: false") && md.trim_end().ends_with("\np"), "{md}");
        let (found, errors) = read_repo(&p);
        assert!(errors.is_empty(), "{errors:?}");
        let back = &found[0];
        assert_eq!((back.source.as_str(), back.every, back.script.as_str(), back.options.0.profile.as_deref(), back.devshell), ("repo", Some(120), t.script.as_str(), Some("big"), false));
        assert_eq!(command_line(back), "./.agents/triggers/ci/run");
        // A broken one is reported, not taken.
        std::fs::create_dir_all(d.path().join(".agent/triggers/bad")).unwrap();
        std::fs::write(d.path().join(".agent/triggers/bad/TRIGGER.md"), "---\nmode: poll\ntitle: t\nwhat: x\n---\nbody").unwrap();
        let (found, errors) = read_repo(&p);
        assert_eq!(found.len(), 1);
        assert!(errors[0].contains(".agent/triggers/bad") && errors[0].contains("what"), "{errors:?}");
        remove_repo(&p, back).unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn webhook_signatures() {
        use hmac::Mac;
        let body = b"{\"a\":1}";
        let mut m = hmac::Hmac::<sha2::Sha256>::new_from_slice(b"s3cret").unwrap();
        m.update(body);
        let sig: String = m.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect();
        let mut h = axum::http::HeaderMap::new();
        h.insert("x-hub-signature-256", format!("sha256={sig}").parse().unwrap());
        assert!(signed(&h, body, "s3cret"));
        assert!(!signed(&h, b"{\"a\":2}", "s3cret"), "another body");
        assert!(!signed(&h, body, "other"));
        let mut g = axum::http::HeaderMap::new();
        g.insert("x-gitlab-token", "s3cret".parse().unwrap());
        assert!(signed(&g, body, "s3cret") && !signed(&g, body, "nope"));
        let mut b = axum::http::HeaderMap::new();
        b.insert("authorization", "Bearer s3cret".parse().unwrap());
        assert!(signed(&b, body, "s3cret"));
        assert!(!signed(&axum::http::HeaderMap::new(), body, "s3cret"));
    }
}
