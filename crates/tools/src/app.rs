//! The App: projects and tasks on subnet. A task is a subnet agent (spawned
//! from the `task-<profile>` mixture as root); reagent keeps its record,
//! follows its state, passes reports and job ends on, watches budgets and
//! writes checkpoints when its history is compacted.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use reagent_store::{Budget, NewTask, Project, Store, Task};
use reagent_supervisor as sup;
use serde_json::{Value, json};
use subnet::hub::{Hub, Notice};
use subnet_core::addr::Addr;
use subnet_core::agent::{Event, PauseMode};
use subnet_core::proto::Op;
use tokio::sync::{broadcast, oneshot};

use crate::config::Config;
use crate::memory::Memory;
use crate::skills::{self, Skill};

/// Where reagent keeps things.
#[derive(Debug, Clone)]
pub struct Paths {
    pub data: PathBuf,
    pub socket: PathBuf,
    /// Global skills folders (`~/.agents/skills`, `~/.claude/skills`, `~/.codex/skills`, `<data>/skills`).
    pub skills: Vec<PathBuf>,
    /// Where the system skills are written at every start.
    pub system_skills: PathBuf,
}

impl Paths {
    pub fn new(data: &Path) -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let codex = std::env::var_os("CODEX_HOME").map(PathBuf::from);
        let skills = crate::skills::global_places(home.as_deref(), codex.as_deref(), data);
        Paths { data: data.into(), socket: data.join("supervisor.sock"), skills, system_skills: data.join("system-skills") }
    }
}

/// How a merge waiting for the person ends.
#[derive(Debug, Clone)]
pub enum MergeAnswer {
    Merge,
    Reject(String),
}

/// What starting a task takes.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct StartTask {
    pub project: String,
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub budget: Option<Budget>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
    /// A kind from reagent.hcl (its profile, its escalation).
    #[serde(default)]
    pub kind: Option<String>,
}

pub struct App {
    pub paths: Paths,
    pub config: Config,
    pub store: Store,
    pub sup: Arc<sup::Client>,
    hub: OnceLock<Arc<Hub>>,
    /// What the web UI hears: `{kind: "task" | "notification" | …, …}`.
    pub events: broadcast::Sender<Value>,
    questions: Mutex<HashMap<String, oneshot::Sender<String>>>,
    merges: Mutex<HashMap<String, oneshot::Sender<MergeAnswer>>>,
    /// Foreground jobs a tool waits for (their ends aren't passed on).
    pub fg_waiting: Mutex<HashSet<String>>,
    /// What each task was last shown of its context (hashes by kind).
    pub shown: Mutex<HashMap<String, HashMap<String, u64>>>,
    /// Tasks already told about (approval asked, budget hit): not again.
    notified: Mutex<HashSet<(String, String)>>,
    pub notifier: crate::notify::Notifier,
    /// Where reagent's own MCP servers are (for the cluster file).
    pub mcp_base: OnceLock<String>,
    /// The added MCP servers as the node runs them: name → {ok, error?, tools?}.
    pub mcp_status: Mutex<std::collections::BTreeMap<String, Value>>,
    /// One cluster change at a time.
    applying: tokio::sync::Mutex<()>,
    /// Tool errors in a row, per task (escalation after a kind's `escalate_after`).
    errors_in_a_row: Mutex<HashMap<String, u32>>,
    /// Profiles a task failed over from (no going back and forth).
    fell_back: Mutex<HashMap<String, HashSet<String>>>,
    /// Triggers' runs going on.
    pub triggers: crate::triggers::Runs,
    /// What wakes a task's shell.sleep: a message to it.
    // ponytail: one entry per task that ever slept, never removed; small.
    wakers: Mutex<HashMap<String, Arc<tokio::sync::Notify>>>,
}

/// Prompt, completion and cached prompt tokens from an agent's usage.
fn usage_of(u: &Value) -> (u64, u64, u64) {
    (u["prompt_tokens"].as_u64().unwrap_or(0), u["completion_tokens"].as_u64().unwrap_or(0), u["cached_prompt_tokens"].as_u64().unwrap_or(0))
}

/// What's kept of a token: its sha256 (hex).
pub fn token_hash(t: &str) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(t.as_bytes()).iter().map(|x| format!("{x:02x}")).collect()
}

fn root() -> Addr {
    Addr::root()
}

impl App {
    pub fn new(paths: Paths, config: Config, store: Store, sup: Arc<sup::Client>) -> Arc<App> {
        let notifier = crate::notify::Notifier::new(config.notify.clone());
        Arc::new(App {
            paths,
            config,
            store,
            sup,
            hub: OnceLock::new(),
            events: broadcast::channel(4096).0,
            questions: Default::default(),
            merges: Default::default(),
            fg_waiting: Default::default(),
            shown: Default::default(),
            notified: Default::default(),
            notifier,
            mcp_base: OnceLock::new(),
            mcp_status: Default::default(),
            applying: Default::default(),
            errors_in_a_row: Default::default(),
            fell_back: Default::default(),
            wakers: Default::default(),
            triggers: Default::default(),
        })
    }

    pub fn set_hub(&self, hub: Arc<Hub>) {
        let _ = self.hub.set(hub);
    }

    pub fn hub(&self) -> Result<&Arc<Hub>, String> {
        self.hub.get().ok_or_else(|| "the hub isn't running yet".into())
    }

    fn emit(&self, v: Value) {
        let _ = self.events.send(v);
    }

    /// A trigger changed (or went): the UI hears it.
    pub async fn emit_trigger(&self, project: &str, name: &str) {
        let t = self.store.trigger(project, name).await.ok().flatten();
        self.emit(json!({"kind": "trigger", "project": project, "name": name, "trigger": t}));
    }

    async fn task_changed(&self, id: &str) {
        if let Ok(Some(t)) = self.store.task(id).await {
            self.emit(json!({"kind": "task", "task": t}));
        }
    }

    // --- the cluster and added MCP servers ---------------------------------

    async fn apply_text(&self, text: String) -> Result<(), String> {
        let hub = self.hub()?;
        let _ = std::fs::write(self.paths.data.join("cluster.hcl"), &text);
        hub.apply_cluster(vec![subnet::hub::db::ClusterFile { name: "cluster.hcl".into(), text }], false, &root()).await.map_err(|e| e.to_string())?;
        // Until the node has taken it (configured for this version) and offers the task types.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let nodes = hub.list_nodes().await;
            let types = hub.op(&root(), Op::ListTypes).await?;
            let offered = self.config.profile.keys().all(|p| types.as_array().is_some_and(|ts| ts.iter().any(|t| t["name"] == crate::cluster::mixture(p).as_str() && t["nodes"].as_u64().unwrap_or(0) > 0)));
            if nodes.iter().any(|n| n.configured) && offered {
                return Ok(());
            }
            if tokio::time::Instant::now() > deadline {
                let errors: Vec<String> = nodes.iter().flat_map(|n| n.errors.iter().map(|(k, v)| format!("{k}: {v}"))).collect();
                return Err(format!("the node doesn't offer the task types (is a profile's key_env set?) {}", errors.join("; ")));
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Writes the cluster from reagent.hcl and the added MCP servers, and
    /// applies it: first with the added servers declared, then with those
    /// the node got running in every task's mixture (one that fails is
    /// reported, not given to tasks). Task agents of an older version move
    /// onto the new one.
    pub async fn apply_cluster(&self) -> Result<(), String> {
        let _one = self.applying.lock().await;
        let base = self.mcp_base.get().ok_or("reagent's MCP servers aren't up")?.clone();
        let custom: Vec<_> = self.store.mcp_servers().await.map_err(|e| e.to_string())?;
        self.apply_text(crate::cluster::render(&self.config, &base, &custom, &[])).await?;
        // Until the node runs, or has given up on, each declared server of this version.
        let hub = self.hub()?.clone();
        let ids: Vec<String> = {
            let spec = hub.cluster().spec;
            custom.iter().filter(|m| m.enabled && crate::cluster::check_mcp(m).is_ok()).filter_map(|m| spec.mcp_id(&m.name)).collect()
        };
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        let nodes = loop {
            let nodes = hub.list_nodes().await;
            let settled = ids.iter().all(|id| nodes.iter().any(|n| n.mcps.contains(id) || n.errors.contains_key(id)));
            if settled || tokio::time::Instant::now() > deadline {
                break nodes;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        };
        let mut status = std::collections::BTreeMap::new();
        let mut healthy = vec![];
        for m in &custom {
            let prefix = format!("{}@", m.name);
            let v = if !m.enabled {
                json!({"ok": false, "error": "turned off"})
            } else if let Err(e) = crate::cluster::check_mcp(m) {
                json!({"ok": false, "error": e})
            } else if nodes.iter().any(|n| n.mcps.iter().any(|id| ids.contains(id) && id.starts_with(&prefix))) {
                healthy.push(m.name.clone());
                json!({"ok": true})
            } else {
                let err = nodes.iter().flat_map(|n| n.errors.iter()).find(|(k, _)| k.starts_with(&prefix)).map(|(_, e)| e.clone()).unwrap_or_else(|| "it didn't start".into());
                json!({"ok": false, "error": err})
            };
            status.insert(m.name.clone(), v);
        }
        if !healthy.is_empty() {
            self.apply_text(crate::cluster::render(&self.config, &base, &custom, &healthy)).await?;
        }
        for (name, s) in &status {
            if s["ok"] == false && s["error"] != "turned off" {
                tracing::warn!(mcp = %name, error = %s["error"], "an added MCP server doesn't run: tasks don't get it");
            }
        }
        *self.mcp_status.lock().unwrap() = status;
        self.emit(json!({"kind": "mcp"}));
        self.upgrade_outdated().await;
        Ok(())
    }

    /// Task agents of an older version (reagent.hcl, reagent or a server
    /// changed) move onto the current one: same history, new agent.
    pub async fn upgrade_outdated(&self) {
        let Ok(hub) = self.hub() else { return };
        let agents = hub.list_agents().await;
        for t in self.store.tasks(None, None, true, 10_000).await.unwrap_or_default() {
            let Some(id) = t.agent.as_deref().and_then(|a| a.parse::<uuid::Uuid>().ok()) else { continue };
            let Some(a) = agents.iter().find(|a| a.id == id) else { continue };
            if !a.outdated || a.superseded_by.is_some() {
                continue;
            }
            match hub.upgrade(&root(), id, true, None).await {
                Ok(s) => {
                    let _ = self.move_agent(&t, &s.id.to_string()).await;
                    tracing::info!(task = %t.id, "task moved onto the current version");
                }
                Err(e) => tracing::warn!(task = %t.id, error = %e, "couldn't upgrade a task's agent"),
            }
        }
    }

    /// Applies the cluster again when the added servers changed (from the
    /// CLI, say, which only writes the database).
    pub fn watch_mcp(self: &Arc<Self>) {
        let me = self.clone();
        tokio::spawn(async move {
            let mut seen = me.store.setting("mcp_changed").await.ok().flatten();
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                let now = me.store.setting("mcp_changed").await.ok().flatten();
                if now != seen {
                    seen = now;
                    if let Err(e) = me.apply_cluster().await {
                        tracing::error!(error = %e, "applying the changed MCP servers");
                    }
                }
            }
        });
    }

    // --- projects -------------------------------------------------------

    /// Registers (or changes) a project; a new one gets the starter rules.
    pub async fn put_project(&self, mut p: Project) -> Result<Project, String> {
        if p.slug.is_empty() || !p.slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(format!("{:?}: a project's id is lowercase letters, digits and -", p.slug));
        }
        let path = std::fs::canonicalize(&p.path).map_err(|e| format!("{}: {e}", p.path))?;
        if !path.is_dir() {
            return Err(format!("{} isn't a folder", path.display()));
        }
        p.path = path.display().to_string();
        for (field, v, ok) in [("devshell", &p.devshell, &["off", "auto", "on"][..]), ("memory", &p.memory, &["central", "repo"]), ("worktrees", &p.worktrees, &["central", "repo"]), ("merge", &p.merge, &["approve", "auto"]), ("default_action", &p.default_action, &["allow", "ask", "deny"])] {
            if !ok.contains(&v.as_str()) {
                return Err(format!("{field}: one of {}", ok.join(", ")));
            }
        }
        if let Some(pr) = &p.profile {
            self.config.profile(Some(pr))?;
        }
        if let Some(k) = &p.kind {
            self.config.kind(k)?;
        }
        if p.name.trim().is_empty() {
            p.name = p.slug.clone();
        }
        let new = self.store.project(&p.slug).await.map_err(|e| e.to_string())?.is_none();
        self.store.put_project(&p).await.map_err(|e| e.to_string())?;
        if new {
            self.store.set_rules(&p.slug, &crate::policy::starter_rules(&p.slug)).await.map_err(|e| e.to_string())?;
        }
        self.emit(json!({"kind": "project", "project": p}));
        Ok(p)
    }

    pub async fn project(&self, slug: &str) -> Result<Project, String> {
        self.store.project(slug).await.map_err(|e| e.to_string())?.ok_or_else(|| format!("no project {slug:?}"))
    }

    pub async fn task(&self, id: &str) -> Result<Task, String> {
        self.store.task(id).await.map_err(|e| e.to_string())?.ok_or_else(|| format!("no task {id:?}"))
    }

    /// The task an agent is (from a tool call's `_meta` or a hook's question).
    pub async fn task_of_agent(&self, agent: &str) -> Result<Task, String> {
        self.store.task_by_agent(agent).await.map_err(|e| e.to_string())?.ok_or_else(|| format!("agent {agent} isn't a reagent task"))
    }

    // --- the designer ---------------------------------------------------

    /// A design task: it reads the project (only reads), asks the person,
    /// and reports a proposal (`target`: what the person started from, or
    /// `subtask` with the task that asked as its parent).
    pub async fn start_design_task(&self, project: &str, target: &str, goal: &str, parent: Option<&Task>) -> Result<Task, String> {
        if goal.trim().is_empty() {
            return Err("what do you want done? (a rough goal is enough)".into());
        }
        let goal = match parent {
            Some(t) => format!("{}\n\n(Asked by the task \"{}\", which will start what you propose.)", goal.trim(), t.title),
            None => goal.trim().to_string(),
        };
        self.start_task(StartTask {
            project: project.into(),
            title: crate::design::title(&goal),
            prompt: crate::design::task_prompt(target, &goal),
            skills: vec!["reagent-prompt-design".into()],
            origin: Some("design".into()),
            parent: parent.map(|t| t.id.clone()),
            profile: self.config.design_profile.clone(),
            ..Default::default()
        })
        .await
    }

    /// A design task's proposal (from its report), once it has one.
    pub async fn proposal(&self, id: &str) -> Result<Option<crate::design::Proposal>, String> {
        let t = self.task(id).await?;
        if t.origin != "design" {
            return Err(format!("{} isn't a design task", t.title));
        }
        let p = self.project(&t.project).await?;
        let skills: Vec<String> = self.skills_for(Path::new(&p.path), &p).into_iter().map(|s| s.name).collect();
        Ok(t.report.as_deref().and_then(|r| crate::design::parse_report(r, &self.config, &skills)))
    }

    /// Makes the items the person chose: tasks start, the rest are made; a line (or why not) each.
    pub async fn create_items(&self, project: &str, items: &[crate::design::Item]) -> Result<Vec<Result<String, String>>, String> {
        let p = self.project(project).await?;
        let mut out = vec![];
        for item in items {
            out.push(match item {
                crate::design::Item::Task { title, prompt, skills, kind, profile, budget, .. } => self
                    .start_task(StartTask { project: p.slug.clone(), title: title.clone(), prompt: prompt.clone(), skills: skills.clone(), kind: kind.clone(), profile: profile.clone(), budget: budget.clone(), ..Default::default() })
                    .await
                    .map(|t| format!("task {} {} ({})", t.title, if t.state == "queued" { "queued" } else { "started" }, t.id)),
                other => crate::design::make(&self.store, &p, other).await,
            });
        }
        Ok(out)
    }

    // --- memory and skills ----------------------------------------------

    pub fn global_memory(&self) -> Memory {
        let mut m = Memory::new(self.paths.data.join("memory"), "Global memory");
        m.skip = vec!["projects".into()];
        m
    }

    pub fn project_memory(&self, p: &Project) -> Memory {
        crate::memory::of_project(&self.paths.data, p)
    }

    pub fn skills_for(&self, cwd: &Path, p: &Project) -> Vec<Skill> {
        skills::discover(cwd, Path::new(&p.path), &self.paths.skills, Some(&self.paths.system_skills))
    }

    pub fn worktree_dir(&self, p: &Project, branch: &str) -> PathBuf {
        let leaf = branch.trim_start_matches("reagent/").replace('/', "-");
        match p.worktrees.as_str() {
            "repo" => PathBuf::from(&p.path).join(".worktrees").join(leaf),
            _ => self.paths.data.join("worktrees").join(&p.slug).join(leaf),
        }
    }

    // --- tasks ----------------------------------------------------------

    /// Starts a task: its record, then its agent.
    pub async fn start_task(&self, req: StartTask) -> Result<Task, String> {
        let p = self.project(&req.project).await?;
        let origin = req.origin.clone().unwrap_or_else(|| "ui".into());
        let kind = self.config.kind_for(req.kind.as_deref(), &origin, p.kind.as_deref())?;
        let kind_profile = kind.as_deref().and_then(|k| self.config.kind.get(k)).and_then(|k| k.profile.as_deref());
        // Asked for > the kind's > the project's > the default.
        let (profile, _) = self.config.profile(req.profile.as_deref().or(kind_profile).or(p.profile.as_deref()))?;
        let profile = profile.to_string();
        if req.title.trim().is_empty() || req.prompt.trim().is_empty() {
            return Err("a task needs a title and what to do".into());
        }
        if let Some(limit) = p.budget.daily_cost {
            let spent = self.store.cost_since(&p.slug, chrono::Utc::now().timestamp() - 86_400).await.map_err(|e| e.to_string())?;
            if spent >= limit {
                return Err(format!("{} spent {spent:.2} of its {limit:.2} for the last day: no new tasks until then (or raise its budget)", p.name));
            }
        }
        let mut budget = req.budget.clone().unwrap_or_default();
        budget.tokens = budget.tokens.or(p.budget.tokens);
        budget.cost = budget.cost.or(p.budget.cost);
        budget.minutes = budget.minutes.or(p.budget.minutes);
        let skills = self.skills_for(Path::new(&p.path), &p);
        for name in &req.skills {
            skills.iter().find(|s| &s.name == name).ok_or_else(|| format!("no skill {name:?}"))?;
        }
        // Over the limit of tasks running at once: it waits (subtasks never do: their parent waits for them).
        // ponytail: two starts at the same moment may both find the last place; add a lock if that matters.
        let queue = req.parent.is_none() && !self.has_room(&p).await;
        let t = self
            .store
            .add_task(&NewTask {
                project: p.slug.clone(),
                parent: req.parent.clone(),
                title: req.title.trim().into(),
                prompt: req.prompt.clone(),
                origin,
                cwd: p.path.clone(),
                profile: profile.clone(),
                budget,
                skills: req.skills.clone(),
                kind,
            })
            .await
            .map_err(|e| e.to_string())?;
        if queue {
            self.store.set_state(&t.id, "queued", None).await.map_err(|e| e.to_string())?;
            self.task_changed(&t.id).await;
            tracing::info!(task = %t.id, title = %t.title, project = %p.slug, "task queued: as many as may run at once already do");
            return self.task(&t.id).await;
        }
        self.spawn(&t).await
    }

    /// The limits of tasks running at once: the global one (`max_tasks` in
    /// the settings) and the project's; none: no limit.
    pub async fn max_tasks(&self) -> Option<i64> {
        self.store.setting("max_tasks").await.ok().flatten().and_then(|v| v.parse().ok()).filter(|n| *n > 0)
    }

    /// Whether another task of this project may start now.
    async fn has_room(&self, p: &Project) -> bool {
        if let Some(max) = self.max_tasks().await
            && self.store.running_tasks(None).await.unwrap_or(0) >= max
        {
            return false;
        }
        if let Some(max) = p.max_tasks.filter(|n| *n > 0)
            && self.store.running_tasks(Some(&p.slug)).await.unwrap_or(0) >= max
        {
            return false;
        }
        true
    }

    /// Starts queued tasks, oldest first, while there's room.
    pub async fn start_queued(&self) {
        for t in self.store.queued_tasks().await.unwrap_or_default() {
            let Ok(p) = self.project(&t.project).await else { continue };
            if !self.has_room(&p).await {
                continue;
            }
            match self.spawn(&t).await {
                Ok(_) => tracing::info!(task = %t.id, "queued task started"),
                Err(e) => tracing::warn!(task = %t.id, error = %e, "a queued task didn't start"),
            }
        }
    }

    /// Starts a queued task now, whatever the limits.
    pub async fn start_now(&self, id: &str) -> Result<Task, String> {
        let t = self.task(id).await?;
        if t.state != "queued" {
            return Err(format!("{} isn't queued (it's {})", t.title, t.state));
        }
        self.spawn(&t).await
    }

    /// A task's agent: spawned with its first message (the skills it starts with loaded).
    async fn spawn(&self, t: &Task) -> Result<Task, String> {
        let p = self.project(&t.project).await?;
        let profile = t.profile.clone();
        let skills = self.skills_for(Path::new(&p.path), &p);
        let mut preload = String::new();
        for name in t.skills.iter() {
            let s = skills.iter().find(|s| &s.name == name).ok_or_else(|| format!("no skill {name:?}"))?;
            let (body, files) = skills::load(s)?;
            preload.push_str(&format!("\n\n## Skill: {name} (in {})\n\n{body}\n{}", s.dir.display(), if files.is_empty() { String::new() } else { format!("\nIts files: {}\n", files.join(", ")) }));
        }
        let first = format!(
            "Task: {}\nProject: {} ({})\nWorking directory: {}\n\n{}{}",
            t.title,
            p.name,
            p.slug,
            p.path,
            t.prompt.trim(),
            if preload.is_empty() { String::new() } else { format!("\n\n# Skills loaded for this task{preload}") }
        );
        // A project with servers of its own has its own mixture.
        let own = crate::cluster::project_mixture(&profile, &p.slug);
        let ty = if self.hub()?.cluster().spec.mixtures.contains_key(&own) { own } else { crate::cluster::mixture(&profile) };
        let spawned = match self.hub()?.op(&root(), Op::Spawn { ty, prompt: first, tenant: None }).await {
            Ok(v) => v,
            Err(e) => {
                let _ = self.store.set_state(&t.id, "failed", None).await;
                let _ = self.store.set_report(&t.id, &format!("couldn't start: {e}")).await;
                return Err(e);
            }
        };
        let agent = spawned["id"].as_str().ok_or("the hub didn't say the agent's id")?.to_string();
        self.store.set_agent(&t.id, &agent).await.map_err(|e| e.to_string())?;
        if t.state == "queued" {
            self.store.set_state(&t.id, "running", None).await.map_err(|e| e.to_string())?;
        }
        self.task_changed(&t.id).await;
        tracing::info!(task = %t.id, title = %t.title, project = %p.slug, "task started");
        self.task(&t.id).await
    }

    fn agent_of(t: &Task) -> Result<uuid::Uuid, String> {
        t.agent.as_deref().ok_or("the task has no agent (it didn't start)")?.parse().map_err(|e| format!("{e}"))
    }

    /// A message to a task (it reads it before its next model call; a
    /// finished one goes on with it).
    /// What a sleeping task waits on besides the time.
    pub fn waker(&self, id: &str) -> Arc<tokio::sync::Notify> {
        self.wakers.lock().unwrap().entry(id.into()).or_default().clone()
    }

    pub async fn message(&self, id: &str, text: &str) -> Result<(), String> {
        let t = self.task(id).await?;
        if let Some(w) = self.wakers.lock().unwrap().get(id) {
            w.notify_waiters();
        }
        // Not started yet: it reads it with its prompt.
        if t.state == "queued" {
            self.store.set_prompt(id, &format!("{}\n\n[a message that came while it was queued] {text}", t.prompt.trim_end())).await.map_err(|e| e.to_string())?;
            self.task_changed(id).await;
            return Ok(());
        }
        let agent = Self::agent_of(&t)?;
        self.hub()?.op(&root(), Op::Send { to: Addr::Agent(agent), content: text.into() }).await?;
        if matches!(t.state.as_str(), "done" | "failed") {
            self.store.set_state(id, "running", None).await.map_err(|e| e.to_string())?;
            if t.state == "failed" {
                let _ = self.hub()?.op(&root(), Op::Resume { id: agent, tree: false }).await;
            }
        }
        self.task_changed(id).await;
        Ok(())
    }

    pub async fn pause(&self, id: &str, mode: PauseMode) -> Result<(), String> {
        let t = self.task(id).await?;
        self.hub()?.op(&root(), Op::Pause { id: Self::agent_of(&t)?, mode, tree: true }).await?;
        self.store.set_state(id, "paused", None).await.map_err(|e| e.to_string())?;
        self.task_changed(id).await;
        Ok(())
    }

    pub async fn resume(&self, id: &str) -> Result<(), String> {
        let t = self.task(id).await?;
        self.hub()?.op(&root(), Op::Resume { id: Self::agent_of(&t)?, tree: true }).await?;
        self.store.set_state(id, "running", None).await.map_err(|e| e.to_string())?;
        self.notified.lock().unwrap().retain(|(t, _)| t != id);
        self.task_changed(id).await;
        Ok(())
    }

    pub async fn cancel(&self, id: &str) -> Result<(), String> {
        let t = self.task(id).await?;
        if let Ok(a) = Self::agent_of(&t) {
            self.hub()?.op(&root(), Op::Cancel { id: a }).await?;
        }
        for j in self.sup.jobs(Some(id)).await.unwrap_or_default().into_iter().filter(|j| j.running()) {
            let _ = self.sup.kill(&j.id, None).await;
        }
        for p in self.sup.ptys(Some(id)).await.unwrap_or_default() {
            let _ = self.sup.pty_close(&p.id).await;
        }
        self.store.set_state(id, "cancelled", None).await.map_err(|e| e.to_string())?;
        self.questions.lock().unwrap().remove(id);
        self.merges.lock().unwrap().remove(id);
        self.task_changed(id).await;
        Ok(())
    }

    /// Changes a task's profile or budget (a new profile takes effect when
    /// it's next started: subnet keeps an agent's type). A task paused over
    /// its budget goes on if the new one allows it.
    pub async fn set_limits(&self, id: &str, profile: Option<&str>, budget: Option<Budget>) -> Result<Task, String> {
        let t = self.task(id).await?;
        let profile = match profile {
            Some(p) => self.config.profile(Some(p))?.0.to_string(),
            None => t.profile.clone(),
        };
        let budget = budget.unwrap_or(t.budget.0.clone());
        self.store.set_task_limits(id, &profile, &budget).await.map_err(|e| e.to_string())?;
        self.notified.lock().unwrap().remove(&(id.to_string(), "budget".to_string()));
        if t.wait.as_ref().is_some_and(|w| w.0["kind"] == "budget") {
            let minutes = (chrono::Utc::now().timestamp() - t.started.unwrap_or(t.created)) / 60;
            let over = budget.tokens.is_some_and(|l| t.tokens as u64 > l) || budget.cost.is_some_and(|l| t.cost > l) || budget.minutes.is_some_and(|l| minutes as u64 > l);
            if over {
                self.task_changed(id).await;
                return Err(format!("still over: {} tokens, {:.2} spent, {minutes} min; raise it further", t.tokens, t.cost));
            }
            self.resume(id).await?;
        }
        self.task_changed(id).await;
        self.task(id).await
    }

    /// Moves a task onto another profile (another model): its history goes
    /// on there (subnet's upgrade onto another type). A failed task is
    /// retried there. `why` is said in the notification.
    pub async fn switch_profile(&self, id: &str, profile: &str, why: &str) -> Result<Task, String> {
        let t = self.task(id).await?;
        let (name, _) = self.config.profile(Some(profile))?;
        if name == t.profile {
            return Err(format!("{} is already on {name}", t.title));
        }
        if !matches!(t.state.as_str(), "running" | "waiting" | "paused" | "failed" | "done") {
            return Err(format!("{} is {}", t.title, t.state));
        }
        let p = self.project(&t.project).await?;
        let hub = self.hub()?;
        let own = crate::cluster::project_mixture(name, &p.slug);
        let ty = if hub.cluster().spec.mixtures.contains_key(&own) { own } else { crate::cluster::mixture(name) };
        let s = hub.upgrade(&root(), Self::agent_of(&t)?, true, Some(&ty)).await.map_err(|e| e.to_string())?;
        self.move_agent(&t, &s.id.to_string()).await?;
        self.store.set_task_limits(id, name, &t.budget.0).await.map_err(|e| e.to_string())?;
        // A failed agent's copy is failed too: retried there.
        let failed = hub.list_agents().await.iter().any(|a| a.id == s.id && a.phase == "failed");
        if failed || t.state == "failed" {
            hub.op(&root(), Op::Resume { id: s.id, tree: true }).await?;
            self.store.set_state(id, "running", None).await.map_err(|e| e.to_string())?;
        }
        self.errors_in_a_row.lock().unwrap().remove(id);
        let t2 = self.task(id).await?;
        self.notify("model", Some(&t2), &format!("{} now runs on {name}", t.title), &format!("from {}: {why}", t.profile)).await;
        self.task_changed(id).await;
        tracing::info!(task = %id, from = %t.profile, to = %name, why, "task moved onto another profile");
        Ok(t2)
    }

    /// Where a task escalates: its kind's `escalate` profile.
    pub fn escalation_for(&self, t: &Task) -> Option<String> {
        t.kind.as_deref().and_then(|k| self.config.kind.get(k)).and_then(|k| k.escalate.clone()).filter(|e| *e != t.profile)
    }

    /// A tool result came in: tool errors in a row past the kind's
    /// `escalate_after` move the task onto its escalation profile.
    async fn tool_result(&self, agent: &str, is_error: bool) {
        let Ok(t) = self.task_of_agent(agent).await else { return };
        let n = {
            let mut m = self.errors_in_a_row.lock().unwrap();
            let n = m.entry(t.id.clone()).or_default();
            *n = if is_error { *n + 1 } else { 0 };
            *n
        };
        let after = t.kind.as_deref().and_then(|k| self.config.kind.get(k)).and_then(|k| k.escalate_after);
        if let (Some(after), Some(to)) = (after, self.escalation_for(&t))
            && n >= after
        {
            if let Err(e) = self.switch_profile(&t.id, &to, &format!("{n} tool errors in a row")).await {
                tracing::warn!(task = %t.id, error = %e, "couldn't escalate");
            }
        }
    }

    /// A failed task whose profile has a fallback goes on there (once per profile).
    async fn fall_back(&self, t: &Task, error: &str) -> bool {
        let Some(to) = self.config.profile.get(&t.profile).and_then(|p| p.fallback.clone()) else { return false };
        let fresh = {
            let mut m = self.fell_back.lock().unwrap();
            let tried = m.entry(t.id.clone()).or_default();
            tried.insert(t.profile.clone()) && !tried.contains(&to)
        };
        if !fresh {
            return false;
        }
        match self.switch_profile(&t.id, &to, &format!("{} failed: {}", t.profile, error.chars().take(200).collect::<String>())).await {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!(task = %t.id, error = %e, "couldn't fall back");
                false
            }
        }
    }

    /// Adds to a task's budget (tokens, cost, minutes; what it has none of stays unlimited).
    pub async fn raise_budget(&self, id: &str, more: &Budget) -> Result<Task, String> {
        let t = self.task(id).await?;
        let b = &t.budget.0;
        let add = |have: Option<u64>, more: Option<u64>| match (have, more) {
            (Some(h), Some(m)) => Some(h + m),
            (h, _) => h,
        };
        let raised = Budget {
            tokens: add(b.tokens, more.tokens),
            cost: match (b.cost, more.cost) {
                (Some(h), Some(m)) => Some(h + m),
                (h, _) => h,
            },
            minutes: add(b.minutes, more.minutes),
            daily_cost: b.daily_cost,
        };
        self.set_limits(id, None, Some(raised)).await
    }

    /// Resumes a failed task where it failed (its model call or step again).
    pub async fn retry(&self, id: &str) -> Result<Task, String> {
        let t = self.task(id).await?;
        if t.state != "failed" {
            return Err(format!("{} isn't failed (it's {})", t.title, t.state));
        }
        let agent = Self::agent_of(&t)?;
        self.hub()?.op(&root(), Op::Resume { id: agent, tree: true }).await?;
        self.store.set_state(id, "running", None).await.map_err(|e| e.to_string())?;
        self.notified.lock().unwrap().retain(|(t, _)| t != id);
        self.task_changed(id).await;
        self.task(id).await
    }

    /// Approves or denies the call a task waits on; `always` adds a rule
    /// allowing its kind (the tool, and the command as it was) in front.
    pub async fn approve(&self, id: &str, call_id: &str, approved: bool, always: bool) -> Result<(), String> {
        let t = self.task(id).await?;
        let agent = Self::agent_of(&t)?;
        if always && approved {
            let pending = self.pending_approval(&t).await;
            if let Some(c) = pending.filter(|c| c["id"] == call_id) {
                let tool = c["function"]["name"].as_str().unwrap_or_default().replace("__", ".");
                let args: Value = serde_json::from_str(c["function"]["arguments"].as_str().unwrap_or("{}")).unwrap_or_default();
                let command = crate::hooks::command_of(&tool, &args).map(|c| c.to_string());
                self.store
                    .prepend_rule(&t.project, &reagent_store::Rule { id: 0, project: t.project.clone(), pos: 0, tool, command, target: None, action: "allow".into() })
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        self.hub()?.op(&root(), Op::Approve { id: agent, call_id: call_id.into(), approved }).await?;
        self.notified.lock().unwrap().remove(&(id.to_string(), format!("approval:{call_id}")));
        self.store.set_state(id, "running", None).await.map_err(|e| e.to_string())?;
        self.task_changed(id).await;
        Ok(())
    }

    async fn pending_approval(&self, t: &Task) -> Option<Value> {
        let agent = t.agent.as_deref()?;
        let list = serde_json::to_value(self.hub().ok()?.list_agents().await).ok()?;
        list.as_array()?.iter().find(|a| a["id"] == agent).and_then(|a| a["awaiting_approval"].get("call").cloned().or_else(|| a["awaiting_approval"].as_object().map(|_| a["awaiting_approval"].clone())))
    }

    /// Searches tasks' whole conversations: one task's (paged), or every
    /// task's of a project (`None`: all projects), a few hits each.
    pub async fn search(&self, pattern: &str, task: Option<&str>, project: Option<&str>, except: Option<&str>, page: u32) -> Result<Value, String> {
        regex::RegexBuilder::new(pattern).size_limit(1 << 20).build().map_err(|e| format!("bad pattern: {e}"))?;
        let hub = self.hub()?.clone();
        let search = |agent: uuid::Uuid, page: u32| {
            let (hub, pattern) = (hub.clone(), pattern.to_string());
            async move { hub.op(&Addr::Agent(agent), Op::SearchHistory { pattern, page }).await }
        };
        if let Some(id) = task {
            let t = self.task(id).await?;
            let agent = Self::agent_of(&t)?;
            let mut v = search(agent, page.max(1)).await?;
            v["task"] = json!({"id": t.id, "title": t.title, "state": t.state, "project": t.project});
            return Ok(v);
        }
        let mut out = vec![];
        for t in self.store.tasks(project, None, false, 500).await.map_err(|e| e.to_string())? {
            if Some(t.id.as_str()) == except {
                continue;
            }
            let Ok(agent) = Self::agent_of(&t) else { continue };
            let Ok(v) = search(agent, 1).await else { continue };
            if v["matches"].as_u64().unwrap_or(0) == 0 {
                continue;
            }
            out.push(json!({"task": {"id": t.id, "title": t.title, "state": t.state, "project": t.project}, "matches": v["matches"], "hits": v["hits"].as_array().map(|h| h.iter().take(3).cloned().collect::<Vec<_>>())}));
        }
        Ok(json!({"pattern": pattern, "tasks": out}))
    }

    /// `text` with a project's secret values (4 characters or longer) as ***.
    pub async fn mask(&self, project: &str, text: &str) -> String {
        let mut values: Vec<String> = self.store.secrets_for(project).await.unwrap_or_default().into_iter().map(|s| s.value).filter(|v| v.chars().count() >= 4).collect();
        // Longest first: one value inside another is masked whole.
        values.sort_by_key(|v| std::cmp::Reverse(v.len()));
        let mut out = text.to_string();
        for v in values {
            if out.contains(&v) {
                out = out.replace(&v, "***");
            }
        }
        out
    }

    /// A task's todo list changed: the UI hears it.
    pub async fn todos_changed(&self, id: &str) {
        let todos = self.store.todos(id).await.unwrap_or_default();
        self.emit(json!({"kind": "todos", "task": id, "todos": todos}));
    }

    /// Tells the UI a task's working memory changed.
    pub async fn wm_changed(&self, id: &str) {
        let wm = self.store.working_memory(id).await.unwrap_or_default();
        self.emit(json!({"kind": "wm", "task": id, "wm": wm}));
    }

    // --- questions and merges waiting for the person ---------------------

    /// What a task waits for, as stored, when it's of `kind`.
    async fn stored_wait(&self, id: &str, kind: &str) -> Option<Value> {
        self.task(id).await.ok()?.wait.map(|w| w.0).filter(|w| w["kind"] == kind)
    }

    async fn done_waiting(&self, id: &str) {
        let _ = self.store.set_state(id, "running", None).await;
        self.task_changed(id).await;
    }

    /// A task asks; this waits for the answer. The question is stored, and
    /// so is an answer given while nothing waits (reagent was restarting):
    /// `ask` runs again after a restart (it's idempotent) and finds it.
    pub async fn ask(&self, id: &str, question: &str, options: &[String]) -> Result<String, String> {
        let stored = self.stored_wait(id, "question").await.filter(|w| w["question"] == question);
        if let Some(a) = stored.as_ref().and_then(|w| w["answer"].as_str()) {
            let a = a.to_string();
            self.done_waiting(id).await;
            return Ok(a);
        }
        let (tx, rx) = oneshot::channel();
        self.questions.lock().unwrap().insert(id.into(), tx);
        let wait = json!({"kind": "question", "question": question, "options": options});
        self.store.set_state(id, "waiting", Some(&wait)).await.map_err(|e| e.to_string())?;
        self.task_changed(id).await;
        // Asked again after a restart: the person already heard it.
        if stored.is_none() {
            let t = self.task(id).await?;
            // Buttons for the first two options (a notification shows at most two).
            let buttons = options.iter().take(2).map(|o| (o.clone(), json!({"kind": "answer", "question": question, "answer": o}))).collect();
            self.notify_actions("waiting", Some(&t), &format!("{} asks", t.title), question, buttons).await;
        }
        let answer = rx.await.map_err(|_| "the question went unanswered (the task was cancelled)".to_string());
        self.done_waiting(id).await;
        answer
    }

    /// The person's answer: to the waiting question; when nothing waits
    /// (a restart in between), kept for it; with no question, a message.
    pub async fn answer(&self, id: &str, text: &str) -> Result<(), String> {
        let waiter = self.questions.lock().unwrap().remove(id);
        if let Some(tx) = waiter
            && tx.send(text.to_string()).is_ok()
        {
            return Ok(());
        }
        if let Some(mut w) = self.stored_wait(id, "question").await {
            w["answer"] = json!(text);
            self.store.set_state(id, "waiting", Some(&w)).await.map_err(|e| e.to_string())?;
            self.task_changed(id).await;
            return Ok(());
        }
        self.message(id, &format!("[an answer to your question] {text}")).await
    }

    /// A worktree merge waits for the person (`merge = "approve"`); stored
    /// like a question, and found again when the call runs again.
    pub async fn wait_merge(&self, id: &str, summary: Value) -> Result<MergeAnswer, String> {
        let stored = self.stored_wait(id, "merge").await.filter(|w| w["branch"] == summary["branch"]);
        if let Some(d) = stored.as_ref().map(|w| w["decision"].clone()).filter(|d| !d.is_null()) {
            self.done_waiting(id).await;
            return Ok(match d.as_str() {
                Some("merge") => MergeAnswer::Merge,
                _ => MergeAnswer::Reject(d["reject"].as_str().unwrap_or("not now").to_string()),
            });
        }
        let (tx, rx) = oneshot::channel();
        self.merges.lock().unwrap().insert(id.into(), tx);
        let mut wait = summary;
        wait["kind"] = json!("merge");
        self.store.set_state(id, "waiting", Some(&wait)).await.map_err(|e| e.to_string())?;
        self.task_changed(id).await;
        if stored.is_none() {
            let t = self.task(id).await?;
            let merge = vec![("Merge".to_string(), json!({"kind": "merge", "branch": wait["branch"]}))];
            self.notify_actions("waiting", Some(&t), &format!("{}: ready to merge", t.title), wait["branch"].as_str().unwrap_or(""), merge).await;
        }
        let a = rx.await.map_err(|_| "the merge went unanswered (the task was cancelled)".to_string());
        self.done_waiting(id).await;
        a
    }

    /// The person decides a merge: to the waiting call; kept when nothing waits.
    pub async fn decide_merge(&self, id: &str, a: MergeAnswer) -> Result<(), String> {
        let waiter = self.merges.lock().unwrap().remove(id);
        let stored = match waiter {
            Some(tx) => match tx.send(a) {
                Ok(()) => return Ok(()),
                Err(a) => a,
            },
            None => a,
        };
        let mut w = self.stored_wait(id, "merge").await.ok_or("the task doesn't wait for a merge")?;
        w["decision"] = match stored {
            MergeAnswer::Merge => json!("merge"),
            MergeAnswer::Reject(m) => json!({"reject": m}),
        };
        self.store.set_state(id, "waiting", Some(&w)).await.map_err(|e| e.to_string())?;
        self.task_changed(id).await;
        Ok(())
    }

    pub async fn waits_for_merge(&self, id: &str) -> bool {
        self.merges.lock().unwrap().contains_key(id) || self.stored_wait(id, "merge").await.is_some()
    }

    // --- following tasks ------------------------------------------------

    pub async fn notify(&self, kind: &str, t: Option<&Task>, title: &str, body: &str) {
        self.notify_actions(kind, t, title, body, vec![]).await
    }

    /// A notification with buttons: each gets a one-time token (good for a
    /// day) that does its action without a login (`act`).
    pub async fn notify_actions(&self, kind: &str, t: Option<&Task>, title: &str, body: &str, actions: Vec<(String, Value)>) {
        let id = self.store.add_notification(kind, t.map(|t| t.id.as_str()), title, body).await.ok();
        let mut buttons = vec![];
        if let Some(t) = t {
            for (label, action) in actions {
                let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
                if self.store.add_action_token(&token_hash(&token), &t.id, &action, chrono::Utc::now().timestamp() + 86_400).await.is_ok() {
                    buttons.push(json!({"title": label, "token": token}));
                }
            }
        }
        self.emit(json!({"kind": "notification", "id": id, "type": kind, "task": t.map(|t| &t.id), "title": title, "body": body, "actions": buttons}));
        self.notifier.send(self, kind, t.map(|t| t.id.as_str()), title, body, &buttons).await;
    }

    /// A notification's button: its action, if the task still waits for it.
    pub async fn act(&self, token: &str) -> Result<String, String> {
        let (id, a) = self.store.take_action_token(&token_hash(token)).await.map_err(|e| e.to_string())?.ok_or("this button was used, or has expired")?;
        let t = self.task(&id).await?;
        let w = t.wait.as_ref().map(|w| w.0.clone()).unwrap_or_default();
        let gone = || Err(format!("{} no longer waits for that", t.title));
        match a["kind"].as_str() {
            Some(k @ ("approve" | "deny")) => {
                if w["kind"] != "approval" || w["call"]["id"] != a["call_id"] {
                    return gone();
                }
                self.approve(&id, a["call_id"].as_str().unwrap_or_default(), k == "approve", false).await?;
                Ok(if k == "approve" { "allowed".into() } else { "denied".into() })
            }
            Some("answer") => {
                if w["kind"] != "question" || w["question"] != a["question"] {
                    return gone();
                }
                self.answer(&id, a["answer"].as_str().unwrap_or_default()).await?;
                Ok(format!("answered {}", a["answer"].as_str().unwrap_or_default()))
            }
            Some("merge") => {
                if w["kind"] != "merge" || w["branch"] != a["branch"] {
                    return gone();
                }
                self.decide_merge(&id, MergeAnswer::Merge).await?;
                Ok("merging".into())
            }
            _ => Err("an unknown action".into()),
        }
    }

    /// Reports in root's mailbox: a task ended (or failed).
    pub async fn take_reports(&self) -> Result<(), String> {
        let mails = self.hub()?.wait_inbox(&root(), Some(25_000)).await.map_err(|e| e.to_string())?;
        for m in mails {
            let Addr::Agent(agent) = &m.from else { continue };
            let Ok(t) = self.task_of_agent(&agent.to_string()).await else { continue };
            self.reported(&t, &m.content, m.status).await;
        }
        Ok(())
    }

    async fn reported(&self, t: &Task, content: &str, status: Option<subnet_core::agent::Status>) {
        use subnet_core::agent::Status;
        let (state, kind) = match status {
            Some(Status::Failed) => ("failed", "failed"),
            Some(Status::Cancelled) => ("cancelled", "cancelled"),
            _ => ("done", "done"),
        };
        if t.state == "cancelled" {
            return;
        }
        // A failure on a profile with a fallback: it goes on there instead.
        if kind == "failed" && self.fall_back(t, content).await {
            return;
        }
        self.record_usage(t).await;
        let _ = self.store.set_report(&t.id, content).await;
        if kind != "cancelled" {
            self.notify(kind, Some(t), &format!("{} — {state}", t.title), &content.chars().take(400).collect::<String>()).await;
        }
        let _ = self.store.set_state(&t.id, state, None).await;
        self.task_changed(&t.id).await;
        tracing::info!(task = %t.id, state, "task reported");
        if let Some(parent) = &t.parent
            && let Ok(p) = self.task(parent).await
            && p.is_active()
        {
            let _ = self.message(parent, &format!("[subtask {} ({}) {state}]\n{content}", t.title, t.id)).await;
        }
        crate::cron::task_ended(self, t).await;
        crate::triggers::task_ended(self, t).await;
    }

    /// Pauses a task over its budget (quick: before its next step) and says so.
    pub async fn check_budget(&self, t: &Task) {
        self.record_usage(t).await;
        let Ok(t) = self.task(&t.id).await else { return };
        let b = &t.budget.0;
        let minutes = (chrono::Utc::now().timestamp() - t.started.unwrap_or(t.created)) / 60;
        let over = b.tokens.is_some_and(|l| t.tokens as u64 > l) || b.cost.is_some_and(|l| t.cost > l) || b.minutes.is_some_and(|l| minutes as u64 > l);
        if !over {
            return;
        }
        let (Ok(hub), Ok(agent)) = (self.hub(), Self::agent_of(&t)) else { return };
        let _ = hub.op(&root(), Op::Pause { id: agent, mode: PauseMode::Quick, tree: true }).await;
        let wait = json!({"kind": "budget", "tokens": t.tokens, "cost": t.cost, "minutes": minutes, "budget": b});
        if self.notified.lock().unwrap().insert((t.id.clone(), "budget".into())) {
            self.notify("budget", Some(&t), &format!("{} is over budget", t.title), &format!("{} tokens, {:.2} spent, {minutes} min: paused", t.tokens, t.cost)).await;
        }
        let _ = self.store.set_state(&t.id, "waiting", Some(&wait)).await;
        self.task_changed(&t.id).await;
    }

    /// What a task's current agent used: tokens and cost.
    async fn agent_usage(&self, t: &Task) -> (i64, f64) {
        let (Ok(hub), Some(agent)) = (self.hub(), t.agent.as_deref()) else { return (0, 0.0) };
        let Ok(list) = serde_json::to_value(hub.list_agents().await) else { return (0, 0.0) };
        let Some(a) = list.as_array().and_then(|l| l.iter().find(|a| a["id"] == agent)) else { return (0, 0.0) };
        let (input, output, cached) = usage_of(&a["usage"]);
        let price = self.config.profile.get(&t.profile).map(|p| p.price).unwrap_or_default();
        ((input + output) as i64, price.cost(input, cached, output))
    }

    /// Counts each task's earlier agents again (those its current one
    /// superseded, all the way back): their usage stays with the task.
    pub async fn recount_usage(&self) {
        let Ok(hub) = self.hub() else { return };
        let agents = hub.list_agents().await;
        for t in self.store.tasks(None, None, false, 100_000).await.unwrap_or_default() {
            let Some(current) = t.agent.as_deref().and_then(|a| a.parse::<uuid::Uuid>().ok()) else { continue };
            let price = self.config.profile.get(&t.profile).map(|p| p.price).unwrap_or_default();
            let (mut tokens, mut cost) = (0i64, 0f64);
            let mut wanted = vec![current];
            while let Some(id) = wanted.pop() {
                for a in agents.iter().filter(|a| a.superseded_by == Some(id)) {
                    let u = serde_json::to_value(a.usage).unwrap_or_default();
                    let (input, output, cached) = usage_of(&u);
                    // ponytail: earlier agents counted at the current profile's prices.
                    tokens += (input + output) as i64;
                    cost += price.cost(input, cached, output);
                    wanted.push(a.id);
                }
            }
            if tokens != t.base_tokens {
                let _ = self.store.set_base_usage(&t.id, tokens, cost).await;
            }
        }
    }

    /// A task moves onto a new agent: what the old one used stays counted.
    pub async fn move_agent(&self, t: &Task, agent: &str) -> Result<(), String> {
        let (tokens, cost) = self.agent_usage(t).await;
        self.store.move_agent(&t.id, agent, tokens, cost).await.map_err(|e| e.to_string())
    }

    /// A task's tokens and cost, from its agent's usage.
    async fn record_usage(&self, t: &Task) {
        let (Ok(hub), Some(agent)) = (self.hub(), t.agent.as_deref()) else { return };
        let Ok(list) = serde_json::to_value(hub.list_agents().await) else { return };
        let Some(a) = list.as_array().and_then(|l| l.iter().find(|a| a["id"] == agent)) else { return };
        let (input, output, cached) = usage_of(&a["usage"]);
        let price = self.config.profile.get(&t.profile).map(|p| p.price).unwrap_or_default();
        let _ = self.store.set_usage(&t.id, (input + output) as i64, price.cost(input, cached, output)).await;
    }

    /// Follows the agents: phases, approvals, usage and budgets.
    pub async fn sync(&self) -> Result<(), String> {
        let hub = self.hub()?;
        let agents = serde_json::to_value(hub.list_agents().await).map_err(|e| e.to_string())?;
        let by_id: HashMap<&str, &Value> = agents.as_array().map(|a| a.iter().filter_map(|x| Some((x["id"].as_str()?, x))).collect()).unwrap_or_default();
        for t in self.store.tasks(None, None, true, 10_000).await.map_err(|e| e.to_string())? {
            let Some(a) = t.agent.as_deref().and_then(|id| by_id.get(id)) else { continue };
            // Usage and cost.
            let u = &a["usage"];
            let (input, output, cached) = usage_of(u);
            let price = self.config.profile.get(&t.profile).map(|p| p.price).unwrap_or_default();
            let cost = price.cost(input, cached, output);
            if t.base_tokens + (input + output) as i64 != t.tokens {
                let _ = self.store.set_usage(&t.id, (input + output) as i64, cost).await;
            }
            let phase = a["phase"].as_str().unwrap_or("");
            let mut state = t.state.clone();
            let mut wait = t.wait.as_ref().map(|w| w.0.clone());
            if phase == "failed" {
                if t.state != "failed" {
                    let error = a["error"].as_str().unwrap_or("failed").to_string();
                    if self.fall_back(&t, &error).await {
                        continue;
                    }
                    let _ = self.store.set_report(&t.id, &format!("failed: {error}")).await;
                    let _ = self.store.set_state(&t.id, "failed", None).await;
                    self.task_changed(&t.id).await;
                    self.notify("failed", Some(&t), &format!("{} failed", t.title), &error).await;
                }
                continue;
            }
            if phase == "cancelled" {
                if t.state != "cancelled" {
                    let _ = self.store.set_state(&t.id, "cancelled", None).await;
                    self.task_changed(&t.id).await;
                }
                continue;
            }
            let held = self.questions.lock().unwrap().contains_key(&t.id) || self.merges.lock().unwrap().contains_key(&t.id);
            // A stored question or merge: its call still runs (or runs again after a
            // restart). When it's gone, what was decided goes to the task as a message.
            let stored = wait.as_ref().filter(|w| w["kind"] == "question" || w["kind"] == "merge").cloned();
            let held = held || (stored.is_some() && (phase == "tools" || a["paused"] == true));
            if let Some(w) = stored.filter(|_| !held && (phase == "idle" || phase == "thinking")) {
                let note = match (w["kind"].as_str(), w["answer"].as_str(), &w["decision"]) {
                    (Some("question"), Some(ans), _) => Some(format!("[the answer to your question \"{}\"] {ans}", w["question"].as_str().unwrap_or(""))),
                    (Some("merge"), _, d) if d == "merge" => Some(format!("[the person approved merging {}: call git.worktree_merge again]", w["branch"].as_str().unwrap_or(""))),
                    (Some("merge"), _, d) if !d.is_null() => Some(format!("[the person sent the merge of {} back: {}]", w["branch"].as_str().unwrap_or(""), d["reject"].as_str().unwrap_or(""))),
                    _ => None,
                };
                if let Some(n) = note {
                    let _ = self.message(&t.id, &n).await;
                }
            }
            if !a["awaiting_approval"].is_null() {
                let call = a["awaiting_approval"].get("call").cloned().unwrap_or(a["awaiting_approval"].clone());
                let call_id = call["id"].as_str().unwrap_or_default().to_string();
                state = "waiting".into();
                wait = Some(json!({"kind": "approval", "call": call}));
                let key = (t.id.clone(), format!("approval:{call_id}"));
                if self.notified.lock().unwrap().insert(key) {
                    let tool = call["function"]["name"].as_str().unwrap_or("").replace("__", ".");
                    let buttons = vec![("Allow once".to_string(), json!({"kind": "approve", "call_id": call_id})), ("Deny".to_string(), json!({"kind": "deny", "call_id": call_id}))];
                    self.notify_actions("waiting", Some(&t), &format!("{} needs approval", t.title), &format!("{tool} {}", call["function"]["arguments"].as_str().unwrap_or("")), buttons).await;
                }
            } else if a["paused"] == true {
                // Paused while it waits for the person (a stop, say): it still waits.
                if wait.as_ref().is_some_and(|w| w["kind"] == "question" || w["kind"] == "merge") {
                    state = "waiting".into();
                } else if wait.as_ref().is_none_or(|w| w["kind"] != "budget") {
                    state = "paused".into();
                    wait = None;
                }
            } else if !held && state != "done" {
                state = "running".into();
                wait = None;
            }
            // Over budget: paused (quick), and the person hears it.
            let b = &t.budget.0;
            let minutes = (chrono::Utc::now().timestamp() - t.started.unwrap_or(t.created)) / 60;
            // All the task's agents together.
            let (all_tokens, all_cost) = (t.base_tokens as u64 + input + output, t.base_cost + cost);
            let over = b.tokens.is_some_and(|l| all_tokens > l) || b.cost.is_some_and(|l| all_cost > l) || b.minutes.is_some_and(|l| minutes as u64 > l);
            if over && a["paused"] != true && state != "done" {
                let _ = hub.op(&root(), Op::Pause { id: Self::agent_of(&t)?, mode: PauseMode::Quick, tree: true }).await;
                state = "waiting".into();
                wait = Some(json!({"kind": "budget", "tokens": all_tokens, "cost": all_cost, "minutes": minutes, "budget": b}));
                if self.notified.lock().unwrap().insert((t.id.clone(), "budget".into())) {
                    self.notify("budget", Some(&t), &format!("{} is over budget", t.title), &format!("{all_tokens} tokens, {all_cost:.2} spent, {minutes} min: paused")).await;
                }
            }
            if state != t.state || wait != t.wait.as_ref().map(|w| w.0.clone()) {
                let _ = self.store.set_state(&t.id, &state, wait.as_ref()).await;
                self.task_changed(&t.id).await;
            }
        }
        Ok(())
    }

    /// A job ended: its task hears it, unless a tool waited for it.
    pub async fn job_ended(&self, j: &sup::JobMeta) {
        if j.acked || self.fg_waiting.lock().unwrap().contains(&j.id) {
            return;
        }
        // The event may be older than the tool's acknowledgement: as it is now.
        if self.sup.job(&j.id).await.is_ok_and(|now| now.acked) {
            return;
        }
        let Some(owner) = &j.owner else { return };
        // A watcher's end: its trigger reads it.
        if owner.starts_with("trigger:") {
            return;
        }
        let Ok(t) = self.task(owner).await else { return };
        let how = match (j.exit, j.signal, j.lost) {
            (_, _, true) => "was lost (the supervisor stopped while it ran)".to_string(),
            (Some(c), _, _) => format!("exited with {c}"),
            (_, Some(s), _) => format!("was ended by signal {s}"),
            _ => "ended".to_string(),
        };
        let tail = self.mask(&t.project, &sup::tail(&self.paths.data, &j.id, 20)).await;
        if t.is_active() || t.state == "done" {
            let _ = self.message(&t.id, &format!("[job {} ({}) {how}]\nlast lines:\n{tail}", j.id, j.name.as_deref().unwrap_or(&j.cmd))).await;
        }
        let _ = self.sup.ack(&j.id).await;
        self.emit(json!({"kind": "job", "job": j}));
    }

    /// A compaction summary becomes a checkpoint in the project's memory.
    pub async fn checkpoint(&self, agent: &str, summary: &str) {
        let Ok(t) = self.task_of_agent(agent).await else { return };
        let Ok(p) = self.project(&t.project).await else { return };
        let m = self.project_memory(&p);
        let file = format!("tasks/{}.md", &t.id[..8]);
        let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M");
        let entry = format!("## Checkpoint {stamp}\n\n{}\n", summary.trim());
        let r = match m.read(&file) {
            Ok(_) => m.edit(&file, &crate::edit::Edit { op: Some(crate::edit::Op::Append), text: Some(format!("\n{entry}")), ..Default::default() }, None),
            Err(_) => m.write(&file, &format!("# {}\n\nTask {} ({}).\n\n{entry}", t.title, t.id, t.origin), &format!("checkpoints of task \"{}\"", t.title)),
        };
        if let Err(e) = r {
            tracing::warn!(task = %t.id, error = %e, "writing a checkpoint");
        }
    }

    /// Runs the followers: reports, agent states, job ends, checkpoints.
    pub fn follow(self: &Arc<Self>) {
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = me.take_reports().await {
                    tracing::warn!(error = %e, "taking reports");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        });
        let me = self.clone();
        tokio::spawn(async move {
            let mut n = 0u64;
            loop {
                if let Err(e) = me.sync().await {
                    tracing::warn!(error = %e, "following tasks");
                }
                me.start_queued().await;
                // Agents left on an older version (an upgrade that failed while the
                // node was still coming up, say) are moved on as soon as it can.
                if n % 10 == 0 {
                    me.upgrade_outdated().await;
                }
                n += 1;
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            }
        });
        let me = self.clone();
        tokio::spawn(async move {
            let mut exits = me.sup.exits();
            // Ends missed while reagent was down.
            if let Ok(jobs) = me.sup.jobs(None).await {
                for j in jobs.iter().filter(|j| !j.running() && !j.acked) {
                    me.job_ended(j).await;
                }
            }
            loop {
                match exits.recv().await {
                    Ok(sup::Event::Exit { job }) => me.job_ended(&job).await,
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
                }
            }
        });
        let me = self.clone();
        tokio::spawn(async move {
            let Ok(hub) = me.hub().cloned() else { return };
            let mut rx = hub.subscribe();
            loop {
                match rx.recv().await {
                    Ok(Notice::Agent { agent, event: Event::Compacted { summary, .. }, .. }) => me.checkpoint(&agent.to_string(), &summary).await,
                    Ok(n @ Notice::Agent { event: Event::ToolResult { .. }, .. }) => {
                        if let Notice::Agent { agent, event: Event::ToolResult { is_error, .. }, .. } = &n {
                            let (me, agent, is_error) = (me.clone(), agent.to_string(), *is_error);
                            tokio::spawn(async move { me.tool_result(&agent, is_error).await });
                        }
                        me.emit(json!({"kind": "agent", "notice": n}));
                    }
                    Ok(n @ Notice::Agent { .. }) => me.emit(json!({"kind": "agent", "notice": n})),
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => return,
                }
            }
        });
    }
}
