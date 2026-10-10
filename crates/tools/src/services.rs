//! Services: commands of a project that should always run (a dev server, a
//! database, a worker). Each runs as a supervisor job (owner
//! `service:<project>/<name>`, outliving restarts of reagent); a loop every
//! second starts it, checks it (its `health` command), stops it when it
//! doesn't come up or stops answering, and starts it again after a backoff.
//! Five failures in a row start a task to repair it, as for triggers.
//!
//! Kept in reagent.db, or in the repo as `.agents/services/<name>/SERVICE.md`
//! (YAML frontmatter; the body: notes for whoever repairs it) with its script
//! beside it. Start, stop and restart only change what's stored (the CLI has
//! no running reagent); the loop does the rest. The policy decides whether a
//! command runs (`services.run`), unless the person allowed it (by hash).

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use reagent_store::{Json, Project, Service, ServiceState, Store};
use reagent_supervisor::SpawnArgs;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::app::{App, StartTask};

/// Failures in a row before a repair task starts.
pub const REPAIR_AFTER: u32 = 5;
/// Backoff: the first, and the ceiling (seconds).
const BACKOFF: i64 = 5;
const MAX_BACKOFF: i64 = 3600;
/// Up this long ends a failure streak.
const STABLE: i64 = 60;
/// Without a health command it's up once it has lived this long.
const ALIVE_IS_UP: i64 = 3;
/// While starting, it's checked this often.
const STARTING_EVERY: i64 = 2;
/// Failed checks in a row that make it unhealthy.
const BAD_CHECKS: u32 = 3;
/// A check is killed after this long.
const CHECK_TIMEOUT: u64 = 10;
/// Where repo services live in a project (the first is written).
pub const DIRS: [&str; 2] = [".agents/services", ".agent/services"];

/// What the loop keeps in memory: checks running now, one state change at a time.
#[derive(Default)]
pub struct Services {
    checking: Mutex<HashSet<String>>,
    lock: tokio::sync::Mutex<()>,
    /// Repo service files that couldn't be read, per project.
    pub repo_errors: Mutex<BTreeMap<String, Vec<String>>>,
}

// --- definitions ------------------------------------------------------------

pub fn check_name(n: &str) -> Result<(), String> {
    if n.is_empty() || n.len() > 60 || !n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(format!("{n:?}: a service's name is lowercase letters, digits and -"));
    }
    Ok(())
}

pub fn check(v: &Service) -> Result<(), String> {
    check_name(&v.name)?;
    if v.script.trim().is_empty() {
        return Err("a service needs a script (its command)".into());
    }
    if v.health.as_deref().is_some_and(|h| h.trim().is_empty()) {
        return Err("health: a command, or none".into());
    }
    if !(1..=86_400).contains(&v.interval) {
        return Err("interval: 1 second to a day".into());
    }
    if !(1..=3600).contains(&v.ready_timeout) {
        return Err("ready_timeout: 1 to 3600 seconds".into());
    }
    if !(1..=600).contains(&v.stop_timeout) {
        return Err("stop_timeout: 1 to 600 seconds".into());
    }
    if let Some(w) = &v.worktree
        && !crate::persistent::valid_name(w)
    {
        return Err(format!("worktree {w:?}: a persistent worktree's name"));
    }
    if v.script_file.is_empty() || v.script_file.contains(['/', '\\']) || v.script_file == "SERVICE.md" || v.script_file.starts_with('.') {
        return Err(format!("{:?}: the script's file name, beside SERVICE.md", v.script_file));
    }
    Ok(())
}

/// What an approval holds for: its command and its check.
pub fn script_hash(v: &Service) -> String {
    crate::app::token_hash(&format!("{}\0{}", v.script, v.health.as_deref().unwrap_or("")))
}

/// What it was started from: another restarts it.
fn def_hash(v: &Service) -> String {
    crate::app::token_hash(&format!("{}\0{}\0{}\0{:?}\0{}", v.script, v.script_file, v.health.as_deref().unwrap_or(""), v.worktree, v.devshell))
}

/// The command lines the policy judges: a repo service's file (or a kept one's script), and its check.
pub fn command_lines(v: &Service) -> Vec<String> {
    let mut out = vec![if v.source == "repo" { format!("./{}/{}/{}", DIRS[0], v.name, v.script_file) } else { v.script.trim().to_string() }];
    out.extend(v.health.as_deref().map(|h| h.trim().to_string()));
    out
}

fn duration(v: Option<serde_yaml::Value>, field: &str) -> Result<Option<i64>, String> {
    match v {
        None => Ok(None),
        Some(serde_yaml::Value::Number(n)) => Ok(Some(n.as_i64().ok_or(format!("{field}: whole seconds"))?)),
        Some(serde_yaml::Value::String(s)) => Ok(Some(crate::triggers::parse_every(&s)?)),
        Some(_) => Err(format!("{field}: seconds, or a duration like 30s")),
    }
}

fn shown(secs: i64) -> serde_yaml::Value {
    serde_yaml::Value::String(if secs % 3600 == 0 { format!("{}h", secs / 3600) } else if secs % 60 == 0 { format!("{}m", secs / 60) } else { format!("{secs}s") })
}

/// The frontmatter of a SERVICE.md.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Front {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    script: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    health: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    interval: Option<serde_yaml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ready_timeout: Option<serde_yaml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stop_timeout: Option<serde_yaml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    worktree: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    devshell: Option<bool>,
}

/// A service from its SERVICE.md and the folder it's in (the script read from beside it).
pub fn from_markdown(project: &str, dir: &Path, text: &str) -> Result<Service, String> {
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
    check_name(&name)?;
    let t = text.trim_start_matches('\u{feff}');
    let (f, body) = match t.strip_prefix("---") {
        Some(rest) => {
            let rest = rest.trim_start_matches(['\r', '\n']);
            let end = rest.find("\n---").ok_or("the frontmatter isn't closed (---)")?;
            let f: Front = serde_yaml::from_str::<Option<Front>>(&rest[..end]).map_err(|e| format!("frontmatter: {e}"))?.unwrap_or_default();
            (f, rest[end + 4..].trim_start_matches('-').trim().to_string())
        }
        None => (Front::default(), t.trim().to_string()),
    };
    let script_file = f.script.unwrap_or_else(|| "run".into());
    let script = std::fs::read_to_string(dir.join(&script_file)).map_err(|e| format!("{script_file}: {e}"))?;
    let v = Service {
        project: project.into(),
        name,
        source: "repo".into(),
        made_by: "repo".into(),
        description: f.description,
        script,
        script_file,
        health: f.health,
        interval: duration(f.interval, "interval")?.unwrap_or(30),
        ready_timeout: duration(f.ready_timeout, "ready_timeout")?.unwrap_or(60),
        stop_timeout: duration(f.stop_timeout, "stop_timeout")?.unwrap_or(10),
        worktree: f.worktree,
        devshell: f.devshell.unwrap_or(true),
        notes: body,
        enabled: true,
        state: Default::default(),
        created: 0,
    };
    check(&v)?;
    Ok(v)
}

/// A service as a SERVICE.md.
pub fn to_markdown(v: &Service) -> String {
    let f = Front {
        description: v.description.clone(),
        script: (v.script_file != "run").then(|| v.script_file.clone()),
        health: v.health.clone(),
        interval: (v.interval != 30).then(|| shown(v.interval)),
        ready_timeout: (v.ready_timeout != 60).then(|| shown(v.ready_timeout)),
        stop_timeout: (v.stop_timeout != 10).then(|| shown(v.stop_timeout)),
        worktree: v.worktree.clone(),
        devshell: (!v.devshell).then_some(false),
    };
    format!("---\n{}---\n\n{}\n", serde_yaml::to_string(&f).unwrap_or_default(), v.notes.trim())
}

/// A repo service's folder (where it is, else where it would be written).
pub fn repo_dir(p: &Project, name: &str) -> PathBuf {
    DIRS.iter().map(|d| Path::new(&p.path).join(d).join(name)).find(|d| d.join("SERVICE.md").is_file()).unwrap_or_else(|| Path::new(&p.path).join(DIRS[0]).join(name))
}

/// The services in a project's folder, and what couldn't be read.
pub fn read_repo(p: &Project) -> (Vec<Service>, Vec<String>) {
    let (mut found, mut errors, mut names) = (vec![], vec![], HashSet::new());
    for d in DIRS {
        let Ok(entries) = std::fs::read_dir(Path::new(&p.path).join(d)) else { continue };
        let mut dirs: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("SERVICE.md").is_file()).collect();
        dirs.sort();
        for dir in dirs {
            let shown = format!("{d}/{}", dir.file_name().unwrap_or_default().to_string_lossy());
            match std::fs::read_to_string(dir.join("SERVICE.md")).map_err(|e| e.to_string()).and_then(|text| from_markdown(&p.slug, &dir, &text)) {
                Ok(v) if names.insert(v.name.clone()) => found.push(v),
                Ok(_) => {}
                Err(e) => errors.push(format!("{shown}: {e}")),
            }
        }
    }
    (found, errors)
}

/// Writes a service's files into the project (SERVICE.md and its script).
pub fn write_repo(p: &Project, v: &Service) -> Result<PathBuf, String> {
    let dir = repo_dir(p, &v.name);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(dir.join("SERVICE.md"), to_markdown(v)).map_err(|e| e.to_string())?;
    let f = dir.join(&v.script_file);
    std::fs::write(&f, &v.script).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755));
    }
    Ok(dir)
}

/// Removes a service's files from the project (its folder, if nothing else is in it).
pub fn remove_repo(p: &Project, v: &Service) -> Result<(), String> {
    let dir = repo_dir(p, &v.name);
    for f in ["SERVICE.md", v.script_file.as_str()] {
        match std::fs::remove_file(dir.join(f)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", dir.join(f).display())),
        }
    }
    let _ = std::fs::remove_dir(&dir);
    Ok(())
}

/// A service as the tools take it (tasks', outside agents').
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
pub struct Def {
    /// Lowercase letters, digits and -. An existing service of this name is replaced.
    pub name: String,
    /// What runs, for good (sh, or a #! line): it shouldn't end. It gets the project's env and secrets and $REAGENT_SERVICE.
    pub script: String,
    /// A command whose exit 0 says it's healthy (curl -fs localhost:8080/health); none: it's up while it runs.
    pub health: Option<String>,
    /// Between checks while it's up (default 30s).
    pub interval: Option<String>,
    /// How long it may take to come up (default 60s).
    pub ready_timeout: Option<String>,
    /// SIGTERM, then SIGKILL after this (default 10s).
    pub stop_timeout: Option<String>,
    /// A persistent worktree of the project to run in (default: the project folder).
    pub worktree: Option<String>,
    /// false: not in the project's nix dev shell.
    pub devshell: Option<bool>,
    pub description: Option<String>,
    /// For whoever repairs it: what it is, where its config is, how it's fixed.
    pub notes: Option<String>,
}

impl Def {
    pub fn into_service(self, project: &str, made_by: &str) -> Result<Service, String> {
        let secs = |s: Option<String>, default: i64| s.as_deref().map(crate::triggers::parse_every).transpose().map(|v| v.unwrap_or(default));
        Ok(Service {
            project: project.into(),
            name: self.name,
            source: "db".into(),
            made_by: made_by.into(),
            description: self.description.unwrap_or_default(),
            script: self.script,
            script_file: "run".into(),
            health: self.health.filter(|h| !h.trim().is_empty()),
            interval: secs(self.interval, 30)?,
            ready_timeout: secs(self.ready_timeout, 60)?,
            stop_timeout: secs(self.stop_timeout, 10)?,
            worktree: self.worktree.filter(|w| !w.trim().is_empty()),
            devshell: self.devshell.unwrap_or(true),
            notes: self.notes.unwrap_or_default(),
            enabled: true,
            state: Default::default(),
            created: 0,
        })
    }
}

/// A service in a line, for tools.
pub fn line(v: &Service) -> String {
    let st = &v.state.0;
    let status = if !v.enabled && st.job.is_none() { "stopped" } else if st.status.is_empty() { "new" } else { st.status.as_str() };
    let mut flags = vec![];
    if st.failures > 0 {
        flags.push(format!("{} failures: {}", st.failures, st.last_error.as_deref().unwrap_or("").lines().next().unwrap_or("")));
    }
    if let Some(r) = st.repair.as_deref().filter(|r| !r.is_empty()) {
        flags.push(format!("repair task {r}"));
    }
    if let Some(j) = &st.job {
        flags.push(format!("job {j}"));
    }
    format!(
        "{} [{status}{}] (in {}, by {}{}{}){}",
        v.name,
        if st.since > 0 { format!(" since {}", chrono::DateTime::from_timestamp(st.since, 0).map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string()).unwrap_or_default()) } else { String::new() },
        if v.source == "repo" { "the repo" } else { "reagent" },
        v.made_by,
        v.health.as_deref().map(|h| format!(", health: {h}")).unwrap_or_default(),
        v.worktree.as_deref().map(|w| format!(", worktree {w}")).unwrap_or_default(),
        if flags.is_empty() { String::new() } else { format!(" — {}", flags.join("; ")) },
    )
}

// --- what the person, tasks and agents do (stored; the loop acts) ------------

async fn get(store: &Store, project: &str, name: &str) -> Result<Service, String> {
    store.service(project, name).await.map_err(|e| e.to_string())?.ok_or_else(|| format!("no service {name:?} in {project}"))
}

/// Adds or changes a service (by the person when `by_person`: its command is
/// allowed). A repo service's files are written. A running one restarts with it.
pub async fn save(store: &Store, p: &Project, mut v: Service, by_person: bool) -> Result<Service, String> {
    v.project = p.slug.clone();
    check(&v)?;
    let old = store.service(&p.slug, &v.name).await.map_err(|e| e.to_string())?;
    v.source = old.as_ref().map(|o| o.source.clone()).unwrap_or_else(|| "db".into());
    if let Some(o) = &old {
        v.made_by = if v.source == "repo" { "repo".into() } else { o.made_by.clone() };
        v.enabled = o.enabled;
    }
    if v.source == "repo" {
        write_repo(p, &v)?;
    }
    let mut state = old.map(|o| o.state.0).unwrap_or_default();
    if by_person {
        state.approved = Some(script_hash(&v));
        state.denied = None;
        state.asking = None;
    }
    // Changed: a new streak, started (or judged) again at once.
    state.failures = 0;
    state.repair = None;
    state.next_start = None;
    v.state = Json(state);
    store.put_service(&v).await.map_err(|e| e.to_string())?;
    store.set_service_state(&p.slug, &v.name, &v.state.0).await.map_err(|e| e.to_string())?;
    get(store, &p.slug, &v.name).await
}

/// Switches a service on (started at once, a new streak) or off (stopped; it stays so).
pub async fn set_enabled(store: &Store, project: &str, name: &str, on: bool) -> Result<Service, String> {
    let v = get(store, project, name).await?;
    store.set_service_enabled(project, name, on).await.map_err(|e| e.to_string())?;
    if on {
        let mut st = v.state.0;
        st.failures = 0;
        st.repair = None;
        st.next_start = None;
        st.last_error = None;
        store.set_service_state(project, name, &st).await.map_err(|e| e.to_string())?;
    }
    get(store, project, name).await
}

/// Stops a service and starts it again (a stopped one just starts).
pub async fn restart(store: &Store, project: &str, name: &str) -> Result<Service, String> {
    let v = set_enabled(store, project, name, true).await?;
    let mut st = v.state.0;
    if st.job.is_some() {
        st.restart = true;
    }
    store.set_service_state(project, name, &st).await.map_err(|e| e.to_string())?;
    get(store, project, name).await
}

/// Moves a service into the repo (its files, left for the person to commit)
/// or back into reagent.db (its files removed). Its state goes with it.
pub async fn move_to(store: &Store, p: &Project, name: &str, to: &str) -> Result<Service, String> {
    let mut v = get(store, &p.slug, name).await?;
    match (to, v.source.as_str()) {
        ("repo", "repo") | ("db", "db") => return Err(format!("{name} is already in {}", if to == "repo" { "the repo" } else { "reagent" })),
        ("repo", _) => {
            write_repo(p, &v)?;
            v.source = "repo".into();
            v.made_by = "repo".into();
            store.put_service(&v).await.map_err(|e| e.to_string())?;
        }
        ("db", _) => {
            // Kept here first: the repo scan doesn't take it for gone.
            v.source = "db".into();
            store.put_service(&v).await.map_err(|e| e.to_string())?;
            remove_repo(p, &v)?;
        }
        _ => return Err("to: repo or db".into()),
    }
    get(store, &p.slug, name).await
}

/// The end of a service's output: its job's, else its last start's.
pub async fn logs(app: &App, project: &str, name: &str, lines: usize) -> Result<String, String> {
    let v = get(&app.store, project, name).await?;
    let job = match v.state.0.job.clone() {
        Some(j) => Some(j),
        None => app.store.service_runs(project, name).await.map_err(|e| e.to_string())?.into_iter().find_map(|r| r.job),
    };
    let Some(job) = job else { return Ok(format!("{name} hasn't run yet")) };
    Ok(app.mask(project, &reagent_supervisor::tail(&app.paths.data, &job, lines.clamp(1, 2000))).await)
}

/// Removes a service: it's stopped, a repo service's files go too.
pub async fn remove(app: &App, project: &str, name: &str) -> Result<(), String> {
    let v = get(&app.store, project, name).await?;
    if let Some(j) = &v.state.0.job {
        terminate(app, j, v.stop_timeout);
    }
    if v.source == "repo" {
        let p = app.project(project).await?;
        app.store.put_service(&Service { source: "db".into(), ..v.clone() }).await.map_err(|e| e.to_string())?;
        remove_repo(&p, &v)?;
    }
    app.store.remove_service(project, name).await.map_err(|e| e.to_string())?;
    app.emit_service(project, name).await;
    Ok(())
}

/// SIGTERM now, SIGKILL after `grace` seconds if it still runs; its end isn't looked at.
fn terminate(app: &App, job: &str, grace: i64) {
    let (sup, job) = (app.sup.clone(), job.to_string());
    tokio::spawn(async move {
        let _ = sup.kill(&job, Some(15)).await;
        tokio::time::sleep(std::time::Duration::from_secs(grace as u64)).await;
        let _ = sup.kill(&job, Some(9)).await;
        let _ = sup.ack(&job).await;
    });
}

// --- the loop -----------------------------------------------------------------

/// Changes a service's state (read fresh, one change at a time).
pub async fn update(app: &App, project: &str, name: &str, f: impl FnOnce(&mut ServiceState)) -> Option<ServiceState> {
    let _one = app.services.lock.lock().await;
    let v = app.store.service(project, name).await.ok()??;
    let mut s = v.state.0;
    f(&mut s);
    app.store.set_service_state(project, name, &s).await.ok()?;
    app.emit_service(project, name).await;
    Some(s)
}

fn set_status(s: &mut ServiceState, status: &str, now: i64) {
    if s.status != status {
        s.status = status.into();
        s.since = now;
    }
}

enum Trust {
    Run,
    Ask,
    Deny(String),
}

/// The person's approval of this command, else the policy (`services.run`, its command and its check).
async fn trust(app: &App, v: &Service, p: &Project) -> Trust {
    let hash = script_hash(v);
    let st = &v.state.0;
    if st.approved.as_deref() == Some(&hash) {
        return Trust::Run;
    }
    if st.denied.as_deref() == Some(&hash) {
        return Trust::Deny("the person denied this command".into());
    }
    let rules = app.store.rules(&p.slug).await.unwrap_or_default();
    let default = crate::policy::Action::parse(&p.default_action).unwrap_or(crate::policy::Action::Ask);
    let mut ask = false;
    for cmd in command_lines(v) {
        let d = crate::policy::decide(&rules, default, &crate::policy::Call { tool: "services.run", command: Some(&cmd), target: None });
        match d.action {
            crate::policy::Action::Allow => {}
            crate::policy::Action::Deny => return Trust::Deny(format!("the policy denies `{cmd}`{}", d.rule.map(|r| format!(" (rule {} {})", r.tool, r.command.unwrap_or_default())).unwrap_or_default())),
            crate::policy::Action::Ask => ask = true,
        }
    }
    if !ask {
        return Trust::Run;
    }
    if st.asking.as_deref() != Some(&hash) {
        let (h, c) = (hash.clone(), command_lines(v).join("\n"));
        let now = chrono::Utc::now().timestamp();
        update(app, &p.slug, &v.name, |s| {
            s.asking = Some(h);
            s.command = Some(c);
            set_status(s, "waiting", now);
        })
        .await;
        let excerpt: String = v.script.lines().take(15).collect::<Vec<_>>().join("\n");
        let health = v.health.as_deref().map(|h| format!("\n\nIts check: `{h}`")).unwrap_or_default();
        app.notify("service", None, &format!("service {} in {} wants to run", v.name, p.name), &format!("Made by {}. Its command:\n\n```\n{excerpt}\n```{health}\n\nAllow or deny it on the project's services tab or in the inbox.", v.made_by)).await;
    }
    Trust::Ask
}

/// The person allows (once: this command; always: rules) or denies a service's command.
pub async fn approve(app: &App, project: &str, name: &str, approved: bool, always: bool) -> Result<Service, String> {
    let v = get(&app.store, project, name).await?;
    let hash = script_hash(&v);
    if always && approved {
        for cmd in command_lines(&v) {
            let rule = reagent_store::Rule { id: 0, project: project.into(), pos: 0, tool: "services.run".into(), command: Some(cmd), target: None, action: "allow".into() };
            app.store.prepend_rule(project, &rule).await.map_err(|e| e.to_string())?;
        }
    }
    update(app, project, name, |s| {
        s.asking = None;
        if approved {
            s.approved = Some(hash);
            s.denied = None;
            s.next_start = None;
        } else {
            s.denied = Some(hash);
        }
    })
    .await;
    get(&app.store, project, name).await
}

/// Where it runs: its persistent worktree, else the project folder.
async fn place(app: &App, v: &Service, p: &Project) -> Result<PathBuf, String> {
    match &v.worktree {
        Some(w) => {
            let w = app.worktrees().get(&p.slug, w).await?;
            let d = PathBuf::from(&w.path);
            if d.is_dir() { Ok(d) } else { Err(format!("its worktree {} is gone ({})", w.name, w.path)) }
        }
        None => Ok(PathBuf::from(&p.path)),
    }
}

/// The folder a kept service's script is written to.
pub fn state_dir(app: &App, v: &Service) -> PathBuf {
    app.paths.data.join("services").join(&v.project).join(&v.name)
}

/// The command that runs it: its file in the repo, or written from reagent.db.
fn script_command(app: &App, v: &Service, p: &Project, dir: &Path) -> Result<String, String> {
    let file = if v.source == "repo" {
        repo_dir(p, &v.name).join(&v.script_file)
    } else {
        let d = state_dir(app, v);
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        let f = d.join("script");
        std::fs::write(&f, &v.script).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755));
        }
        f
    };
    let q = crate::devshell::sh_quote(&file.display().to_string());
    let cmd = if v.script.starts_with("#!") { q } else { format!("sh {q}") };
    Ok(crate::devshell::wrap(p, dir, &cmd, v.devshell))
}

async fn env(app: &App, v: &Service, p: &Project) -> Vec<(String, String)> {
    let mut e = crate::mcp::shell::project_env(app, p).await;
    e.push(("REAGENT_SERVICE".into(), v.name.clone()));
    e
}

/// Runs its health command once: Ok when it exits 0 within CHECK_TIMEOUT.
async fn run_check(app: &App, v: &Service, p: &Project) -> Result<(), String> {
    let Some(h) = &v.health else { return Ok(()) };
    let dir = place(app, v, p).await?;
    let cmd = crate::devshell::wrap(p, &dir, h, v.devshell);
    let mut c = tokio::process::Command::new("sh");
    c.arg("-c").arg(&cmd).current_dir(&dir).envs(env(app, v, p).await).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    #[cfg(unix)]
    c.process_group(0);
    let child = c.spawn().map_err(|e| format!("its check couldn't start: {e}"))?;
    let pid = child.id();
    match tokio::time::timeout(std::time::Duration::from_secs(CHECK_TIMEOUT), child.wait_with_output()).await {
        Ok(Ok(o)) if o.status.success() => Ok(()),
        Ok(Ok(o)) => {
            let out = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
            let tail: String = out.trim().lines().rev().take(5).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
            Err(format!("its check exited with {}{}", o.status.code().map(|c| c.to_string()).unwrap_or("a signal".into()), if tail.is_empty() { String::new() } else { format!(": {tail}") }))
        }
        Ok(Err(e)) => Err(format!("its check: {e}")),
        Err(_) => {
            #[cfg(unix)]
            if let Some(pid) = pid {
                let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGKILL);
            }
            Err(format!("its check took longer than {CHECK_TIMEOUT}s"))
        }
    }
}

/// Starts it, if its command may run.
async fn start(app: &App, v: &Service, p: &Project, now: i64) {
    match trust(app, v, p).await {
        Trust::Run => {}
        Trust::Ask => return,
        Trust::Deny(why) => {
            let msg = format!("not started: {why}");
            if v.state.0.last_error.as_deref() != Some(msg.as_str()) {
                app.notify("service", None, &format!("service {} in {} doesn't start", v.name, p.name), &why).await;
            }
            update(app, &p.slug, &v.name, |s| {
                s.last_error = Some(msg);
                s.next_start = Some(now + 60);
                set_status(s, "denied", now);
            })
            .await;
            return;
        }
    }
    let started = async {
        let dir = place(app, v, p).await?;
        let cmd = script_command(app, v, p, &dir)?;
        app.sup.spawn(SpawnArgs { cmd, cwd: dir.display().to_string(), env: env(app, v, p).await, owner: Some(format!("service:{}", v.id())), name: Some(format!("service {}", v.name)), fg: false, stdin: None }).await.map_err(|e| format!("couldn't start: {e}"))
    }
    .await;
    match started {
        Ok(j) => {
            tracing::info!(service = %v.id(), job = %j.id, "service started");
            let run = app.store.add_service_run(&p.slug, &v.name, &j.id, now).await.ok();
            let def = def_hash(v);
            update(app, &p.slug, &v.name, |s| {
                s.job = Some(j.id.clone());
                s.run = run;
                s.def = Some(def);
                s.asking = None;
                s.next_start = None;
                s.restart = false;
                s.stopping = None;
                s.kill_at = None;
                s.bad_checks = 0;
                s.last_check = None;
                s.status = "starting".into();
                s.since = now;
            })
            .await;
        }
        Err(e) => failed(app, v, p, &e, "", now).await,
    }
}

/// Asks it to stop: SIGTERM now, SIGKILL at `kill_at`. `why`: a failure (it
/// didn't come up, or stopped answering); none: it was asked to.
async fn stop(app: &App, v: &Service, job: &str, why: Option<String>, now: i64) {
    let _ = app.sup.kill(job, Some(15)).await;
    let grace = v.stop_timeout;
    update(app, &v.project, &v.name, |s| {
        s.kill_at = Some(now + grace);
        s.stopping = why;
        set_status(s, "stopping", now);
    })
    .await;
}

/// A failure: backs off (doubling, up to an hour); the fifth in a row starts a repair task (once per streak).
async fn failed(app: &App, v: &Service, p: &Project, error: &str, log: &str, now: i64) {
    let st = update(app, &p.slug, &v.name, |s| {
        s.failures += 1;
        s.last_error = Some(error.chars().take(2000).collect());
        s.next_start = Some(now + (BACKOFF << (s.failures - 1).min(12)).min(MAX_BACKOFF));
        s.status = "backoff".into();
        s.since = now;
    })
    .await;
    tracing::warn!(service = %v.id(), error, "service failed");
    if let Some(st) = st
        && st.failures >= REPAIR_AFTER
        && st.repair.is_none()
    {
        repair(app, v, p, error, log, st.failures).await;
    }
}

async fn repair(app: &App, v: &Service, p: &Project, error: &str, log: &str, failures: u32) {
    let kept = if v.source == "repo" { format!("in the repo: {} (edit the files; they're read again within seconds, and a changed command restarts it)", repo_dir(p, &v.name).display()) } else { "in reagent's database (services.service_add with the same name replaces it, and restarts it)".to_string() };
    let prompt = format!(
        "The service `{}` of this project failed {failures} times in a row (it ended, didn't come up within {}s, or stopped answering its check). Find out why and repair it, so it comes up and stays up. services.service_logs shows its output, services.service_restart starts it again at once.\n\nKept {kept}.\nRuns in: {}\nHealth check: {}\n\nCommand:\n```\n{}\n```\n{}\nLast error:\n```\n{}\n```\n\nLast output:\n```\n{}\n```",
        v.name,
        v.ready_timeout,
        v.worktree.as_deref().map(|w| format!("the persistent worktree {w}")).unwrap_or_else(|| "the project folder".into()),
        v.health.as_deref().map(|h| format!("`{h}` every {}s", v.interval)).unwrap_or_else(|| "none (up while it runs)".into()),
        v.script.trim(),
        if v.notes.trim().is_empty() { String::new() } else { format!("\nNotes:\n{}\n", v.notes.trim()) },
        error.trim(),
        log.lines().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"),
    );
    let started = app.start_task(StartTask { project: p.slug.clone(), title: format!("Repair service {}", v.name), prompt, origin: Some(format!("service-repair:{}", v.id())), ..Default::default() }).await;
    match started {
        Ok(task) => {
            update(app, &p.slug, &v.name, |s| s.repair = Some(task.id.clone())).await;
            app.notify("service", Some(&task), &format!("service {} keeps failing", v.name), &format!("{failures} failures in a row; a task repairs it.\n\n{}", error.chars().take(300).collect::<String>())).await;
        }
        Err(e) => {
            update(app, &p.slug, &v.name, |s| s.repair = Some(String::new())).await;
            app.notify("service", None, &format!("service {} keeps failing", v.name), &format!("{failures} failures in a row; the repair task didn't start: {e}")).await;
        }
    }
}

/// Runs a check in the background; its result counts only for the job it checked.
fn check_now(app: &Arc<App>, v: &Service, p: &Project, job: &str) {
    if !app.services.checking.lock().unwrap().insert(v.id()) {
        return;
    }
    let (app, v, p, job) = (app.clone(), v.clone(), p.clone(), job.to_string());
    tokio::spawn(async move {
        let r = run_check(&app, &v, &p).await;
        app.services.checking.lock().unwrap().remove(&v.id());
        let now = chrono::Utc::now().timestamp();
        let mut became_up = None;
        let mut unhealthy = None;
        update(&app, &p.slug, &v.name, |s| {
            if s.job.as_deref() != Some(&job) || s.kill_at.is_some() {
                return;
            }
            s.last_check = Some(now);
            match &r {
                Ok(()) => {
                    s.bad_checks = 0;
                    if s.status == "starting" {
                        set_status(s, "up", now);
                        became_up = s.run;
                    }
                }
                Err(e) if s.status == "up" => {
                    s.bad_checks += 1;
                    s.last_error = Some(e.clone());
                    if s.bad_checks >= BAD_CHECKS {
                        unhealthy = Some(format!("unhealthy: {BAD_CHECKS} failed checks in a row ({e})"));
                    }
                }
                Err(e) => s.last_error = Some(e.clone()),
            }
        })
        .await;
        if let Some(run) = became_up {
            let _ = app.store.service_run_up(run, now).await;
            tracing::info!(service = %v.id(), "service up");
        }
        if let Some(why) = unhealthy {
            stop(&app, &v, &job, Some(why), now).await;
        }
    });
}

/// One look at a service: its job's end, its checks, a stop or a (re)start due.
async fn look(app: &Arc<App>, v: &Service, p: &Project, now: i64) {
    let st = &v.state.0;
    if let Some(job) = &st.job {
        let meta = app.sup.job(job).await;
        if let Ok(m) = &meta
            && m.running()
        {
            if let Some(k) = st.kill_at {
                if now >= k {
                    let _ = app.sup.kill(job, Some(9)).await;
                }
                return;
            }
            let changed = st.def.as_deref().is_some_and(|d| d != def_hash(v));
            if !v.enabled || st.restart || changed {
                if changed {
                    update(app, &p.slug, &v.name, |s| s.restart = true).await;
                }
                return stop(app, v, job, None, now).await;
            }
            match st.status.as_str() {
                "starting" if now - st.since >= v.ready_timeout => stop(app, v, job, Some(format!("not up within {}s{}", v.ready_timeout, st.last_error.as_deref().map(|e| format!(" (last check: {e})")).unwrap_or_default())), now).await,
                "starting" if v.health.is_none() => {
                    if now - st.since >= ALIVE_IS_UP {
                        let run = st.run;
                        update(app, &p.slug, &v.name, |s| set_status(s, "up", now)).await;
                        if let Some(r) = run {
                            let _ = app.store.service_run_up(r, now).await;
                        }
                    }
                }
                "starting" => {
                    if st.last_check.is_none_or(|c| now - c >= STARTING_EVERY) {
                        check_now(app, v, p, job);
                    }
                }
                "up" => {
                    if st.failures > 0 && now - st.since >= STABLE {
                        update(app, &p.slug, &v.name, |s| {
                            s.failures = 0;
                            s.repair = None;
                            s.last_error = None;
                        })
                        .await;
                    }
                    if v.health.is_some() && st.last_check.is_none_or(|c| now - c >= v.interval) {
                        check_now(app, v, p, job);
                    }
                }
                _ => {}
            }
            return;
        }
        // It ended (or its job is gone): recorded, then a failure unless it was asked to stop.
        let m = meta.ok();
        let ended = match &m {
            None => "its job is gone".to_string(),
            Some(m) if m.lost => "lost (the supervisor stopped while it ran)".to_string(),
            Some(m) => match (m.exit, m.signal) {
                (Some(c), _) => format!("exited with {c}"),
                (_, Some(sig)) => format!("ended by signal {sig}"),
                _ => "ended".to_string(),
            },
        };
        let tail = app.mask(&p.slug, &reagent_supervisor::tail(&app.paths.data, job, 200)).await;
        let asked = st.stopping.is_none() && (st.kill_at.is_some() || !v.enabled || st.restart);
        let error = st.stopping.clone().unwrap_or_else(|| ended.clone());
        if let Some(run) = st.run {
            let _ = app.store.service_run_ended(run, now, m.as_ref().and_then(|m| m.exit).map(i64::from), (!asked).then_some(error.as_str()), &tail).await;
        }
        let _ = app.sup.ack(job).await;
        let restart = st.restart;
        update(app, &p.slug, &v.name, |s| {
            s.job = None;
            s.run = None;
            s.kill_at = None;
            s.stopping = None;
            s.restart = false;
            s.bad_checks = 0;
            s.last_check = None;
            set_status(s, if v.enabled { "starting" } else { "stopped" }, now);
        })
        .await;
        if asked {
            tracing::info!(service = %v.id(), "service stopped");
            if restart && v.enabled {
                let fresh = app.store.service(&p.slug, &v.name).await.ok().flatten();
                if let Some(fresh) = fresh {
                    start(app, &fresh, p, now).await;
                }
            }
        } else {
            failed(app, v, p, &error, &tail, now).await;
        }
        return;
    }
    if !v.enabled {
        if st.status != "stopped" {
            update(app, &p.slug, &v.name, |s| set_status(s, "stopped", now)).await;
        }
        return;
    }
    if st.next_start.is_some_and(|n| n > now) {
        return;
    }
    start(app, v, p, now).await;
}

/// Reads every project's repo services into reagent.db (a kept one of the
/// same name wins; one whose files are gone goes, stopped).
pub async fn sync_repo(app: &App) {
    for p in app.store.projects().await.unwrap_or_default() {
        let (found, mut errors) = read_repo(&p);
        let names: HashSet<String> = found.iter().map(|v| v.name.clone()).collect();
        let have = app.store.services(Some(&p.slug)).await.unwrap_or_default();
        for v in found {
            match have.iter().find(|h| h.name == v.name) {
                Some(h) if h.source == "db" => {
                    errors.push(format!("{}: reagent already keeps a service called {} (move one, or rename it)", DIRS[0], v.name));
                    continue;
                }
                Some(h) if (Service { enabled: h.enabled, state: h.state.clone(), created: h.created, ..v.clone() }) == *h => continue,
                _ => {}
            }
            if let Err(e) = app.store.put_service(&v).await {
                errors.push(format!("{}: {e}", v.name));
            }
            app.emit_service(&p.slug, &v.name).await;
        }
        for gone in have.iter().filter(|h| h.source == "repo" && !names.contains(&h.name)) {
            if let Some(j) = &gone.state.0.job {
                terminate(app, j, gone.stop_timeout);
            }
            let _ = app.store.remove_service(&p.slug, &gone.name).await;
            app.emit_service(&p.slug, &gone.name).await;
        }
        let mut all = app.services.repo_errors.lock().unwrap();
        if errors.is_empty() {
            all.remove(&p.slug);
        } else {
            all.insert(p.slug.clone(), errors);
        }
    }
}

/// One pass over every service.
pub async fn tick(app: &Arc<App>, now: i64) {
    let projects: BTreeMap<String, Project> = app.store.projects().await.unwrap_or_default().into_iter().map(|p| (p.slug.clone(), p)).collect();
    for v in app.store.services(None).await.unwrap_or_default() {
        if let Some(p) = projects.get(&v.project) {
            look(app, &v, p, now).await;
        }
    }
}

/// Looks after the services for as long as reagent runs.
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

    #[test]
    fn a_service_md_reads_and_writes_back() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("web");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("serve"), "python -m http.server 8080\n").unwrap();
        let text = "---\ndescription: the dev server\nscript: serve\nhealth: curl -fs localhost:8080\ninterval: 1m\nready_timeout: 90\n---\n\nRestart it after changing the config.\n";
        let v = from_markdown("site", &dir, text).unwrap();
        assert_eq!((v.name.as_str(), v.script_file.as_str(), v.interval, v.ready_timeout, v.stop_timeout, v.notes.as_str()), ("web", "serve", 60, 90, 10, "Restart it after changing the config."));
        assert_eq!(from_markdown("site", &dir, &to_markdown(&v)).unwrap(), v);
        assert!(from_markdown("site", &dir, "---\nport: 80\n---\nx").unwrap_err().contains("port"), "unknown keys are errors");
        assert!(check(&Service { interval: 0, ..v.clone() }).is_err());
        assert_eq!(command_lines(&Service { source: "db".into(), ..v.clone() }), ["python -m http.server 8080", "curl -fs localhost:8080"]);
        assert_eq!(command_lines(&v)[0], "./.agents/services/web/serve");
        assert_ne!(def_hash(&v), def_hash(&Service { worktree: Some("next".into()), ..v.clone() }), "another place restarts it");
        assert_eq!(def_hash(&v), def_hash(&Service { interval: 5, ..v }), "another interval doesn't");
    }
}
