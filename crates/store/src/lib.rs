//! reagent's own data in SQLite (`reagent.db`): projects and their policy
//! rules, tasks, cron, settings, login sessions, push subscriptions and
//! notifications. Schema changes go through `migrations/`.

use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
pub use sqlx::types::Json;

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Per-task limits (any may be absent: no limit) and the project's daily cost.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    /// In the profiles' price unit (e.g. dollars).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes: Option<u64>,
    /// The project's cost per day, all its tasks together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_cost: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Project {
    pub slug: String,
    pub name: String,
    pub path: String,
    /// `central` or `repo`.
    pub memory: String,
    pub worktrees: String,
    /// `approve` or `auto`.
    pub merge: String,
    /// `allow`, `ask` or `deny`: what a call no rule matches gets.
    pub default_action: String,
    pub profile: Option<String>,
    pub budget: Json<Budget>,
    pub env: Json<serde_json::Map<String, serde_json::Value>>,
    pub created: i64,
    /// Commands run in the project's nix dev shell: off, auto (when there's a flake.nix) or on.
    #[serde(default = "auto")]
    pub devshell: String,
    /// Which dev shell (`devShells.<attr>`; none: the default).
    #[serde(default)]
    pub devshell_attr: Option<String>,
    /// Its tasks' kind when nothing else picks one.
    #[serde(default)]
    pub kind: Option<String>,
    /// Tasks of this project running at once (none: only the global limit).
    #[serde(default)]
    pub max_tasks: Option<i64>,
    /// `shared` or `exclusive`: tasks in one persistent worktree at once (each may set its own).
    #[serde(default = "shared")]
    pub worktree_sharing: String,
}

fn shared() -> String {
    "shared".into()
}

fn auto() -> String {
    "auto".into()
}

impl Project {
    pub fn new(slug: &str, name: &str, path: &str) -> Self {
        Project {
            slug: slug.into(),
            name: name.into(),
            path: path.into(),
            memory: "central".into(),
            worktrees: "central".into(),
            merge: "approve".into(),
            default_action: "ask".into(),
            profile: None,
            budget: Json(Budget::default()),
            env: Json(Default::default()),
            created: now(),
            devshell: auto(),
            devshell_attr: None,
            kind: None,
            max_tasks: None,
            worktree_sharing: shared(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Rule {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub pos: i64,
    /// A tool pattern (`shell.exec*`, `fs.*`).
    pub tool: String,
    /// A glob over the command line (shell and pty tools).
    #[serde(default)]
    pub command: Option<String>,
    /// For `tasks.task_spawn`: the project it would start in.
    #[serde(default)]
    pub target: Option<String>,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Worktree {
    pub path: String,
    pub branch: String,
    pub base: String,
}

/// A named, long-lived worktree of a project that tasks start in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PersistentWorktree {
    pub project: String,
    pub name: String,
    pub path: String,
    pub branch: String,
    pub base: String,
    /// `shared`, `exclusive`, or none: the project's.
    pub sharing: Option<String>,
    /// Recorded as git had it: removing it only forgets it (unless asked to delete).
    pub adopted: bool,
    pub created: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Task {
    pub id: String,
    pub agent: Option<String>,
    pub project: String,
    pub parent: Option<String>,
    pub title: String,
    pub prompt: String,
    pub origin: String,
    pub cwd: String,
    pub worktree: Option<Json<Worktree>>,
    pub profile: String,
    pub budget: Json<Budget>,
    pub skills: Json<Vec<String>>,
    /// queued, running, waiting, paused, done, failed, cancelled.
    pub state: String,
    pub wait: Option<Json<serde_json::Value>>,
    pub report: Option<String>,
    pub cost: f64,
    pub tokens: i64,
    pub created: i64,
    pub updated: i64,
    pub finished: Option<i64>,
    pub kind: Option<String>,
    /// Used by its earlier agents (before an upgrade or a model switch).
    pub base_tokens: i64,
    pub base_cost: f64,
    /// When its agent started (a queued task hasn't yet).
    #[serde(default)]
    pub started: Option<i64>,
    /// The persistent worktree it works in (none: the project folder).
    #[serde(default)]
    pub home: Option<String>,
}

impl Task {
    /// Its title, or "untitled task" before it has one (a task started
    /// without one names itself with tasks.task_title).
    pub fn name(&self) -> &str {
        if self.title.is_empty() { "untitled task" } else { &self.title }
    }

    pub fn is_active(&self) -> bool {
        matches!(self.state.as_str(), "running" | "waiting" | "paused" | "queued")
    }
}

/// What starting a task needs.
#[derive(Debug, Clone, Default)]
pub struct NewTask {
    pub project: String,
    pub parent: Option<String>,
    pub title: String,
    pub prompt: String,
    pub origin: String,
    pub cwd: String,
    pub profile: String,
    pub budget: Budget,
    pub skills: Vec<String>,
    pub kind: Option<String>,
    pub home: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Cron {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub project: String,
    pub expr: String,
    #[serde(default = "utc")]
    pub tz: String,
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub options: Json<CronOptions>,
    #[serde(default = "skip")]
    pub overlap: String,
    #[serde(default = "yes")]
    pub catch_up: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub last_run: Option<i64>,
    #[serde(default)]
    pub next_run: Option<i64>,
    #[serde(default)]
    pub queued: bool,
}

fn utc() -> String {
    "UTC".into()
}
fn skip() -> String {
    "skip".into()
}
fn yes() -> bool {
    true
}

/// How a cron run's task starts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CronOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<Budget>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
}

/// A trigger: a script that watches something (poll, watch, webhook) and
/// turns what it sees into events that start tasks or message running ones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Trigger {
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub name: String,
    /// db (kept here) or repo (`.agents/triggers/<name>/TRIGGER.md`).
    #[serde(default = "db_source")]
    pub source: String,
    /// person, task:<id>, mcp:<token name> or repo.
    #[serde(default = "person")]
    pub made_by: String,
    #[serde(default)]
    pub description: String,
    /// poll, watch or webhook.
    pub mode: String,
    /// poll: seconds between runs (or `cron`).
    #[serde(default)]
    pub every: Option<i64>,
    #[serde(default)]
    pub cron: Option<String>,
    #[serde(default = "utc")]
    pub tz: String,
    /// The script (sh, unless it starts with `#!`).
    #[serde(default)]
    pub script: String,
    /// A repo trigger's script file, beside its TRIGGER.md.
    #[serde(default = "run_file")]
    pub script_file: String,
    /// Seconds a poll or webhook run may take.
    #[serde(default = "sixty")]
    pub timeout: i64,
    /// skip, queue or parallel (while a task it started still goes).
    #[serde(default = "skip")]
    pub overlap: String,
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub options: Json<CronOptions>,
    /// A webhook's secret: one of the project's secrets, by name.
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default = "yes")]
    pub devshell: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub state: Json<TriggerState>,
    #[serde(default)]
    pub created: i64,
}

impl Trigger {
    /// `<project>/<name>`.
    pub fn id(&self) -> String {
        format!("{}/{}", self.project, self.name)
    }
}

/// A command of a project that should always run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Service {
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub name: String,
    /// db (kept here) or repo (`.agents/services/<name>/SERVICE.md`).
    #[serde(default = "db_source")]
    pub source: String,
    /// person, task:<id>, mcp or repo.
    #[serde(default = "person")]
    pub made_by: String,
    #[serde(default)]
    pub description: String,
    /// The command (sh, unless it starts with `#!`).
    #[serde(default)]
    pub script: String,
    /// A repo service's script file, beside its SERVICE.md.
    #[serde(default = "run_file")]
    pub script_file: String,
    /// A command whose exit 0 says it's healthy.
    #[serde(default)]
    pub health: Option<String>,
    /// Seconds between checks while it's up.
    #[serde(default = "thirty")]
    pub interval: i64,
    /// Seconds it may take to come up.
    #[serde(default = "sixty")]
    pub ready_timeout: i64,
    /// Seconds between SIGTERM and SIGKILL.
    #[serde(default = "ten")]
    pub stop_timeout: i64,
    /// A persistent worktree of the project to run in.
    #[serde(default)]
    pub worktree: Option<String>,
    #[serde(default = "yes")]
    pub devshell: bool,
    /// For whoever repairs it.
    #[serde(default)]
    pub notes: String,
    /// Switched on: it should run.
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub state: Json<ServiceState>,
    #[serde(default)]
    pub created: i64,
}

fn thirty() -> i64 {
    30
}

fn ten() -> i64 {
    10
}

impl Service {
    /// `<project>/<name>`.
    pub fn id(&self) -> String {
        format!("{}/{}", self.project, self.name)
    }
}

/// How a service is doing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServiceState {
    /// stopped, starting, up, backoff, waiting (for the person's approval), denied.
    #[serde(default)]
    pub status: String,
    /// Since when it's in this status.
    #[serde(default)]
    pub since: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// This start's row in service_runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<i64>,
    /// When it may be started again (backoff).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_start: Option<i64>,
    /// Failures in a row.
    #[serde(default)]
    pub failures: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Failed checks in a row while it's up.
    #[serde(default)]
    pub bad_checks: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_check: Option<i64>,
    /// Being stopped: SIGKILL after this time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kill_at: Option<i64>,
    /// Why it's being stopped (none: the person or a task stopped it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopping: Option<String>,
    /// The command (by hash) the person or the policy allowed, or denied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denied: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The repair task started for this failure streak.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair: Option<String>,
    /// Asked to restart (it's stopped, then started at once).
    #[serde(default)]
    pub restart: bool,
    /// The definition (by hash) its job was started from: another restarts it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub def: Option<String>,
}

/// One start of a service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct ServiceRun {
    pub id: i64,
    pub job: Option<String>,
    pub started: i64,
    pub up: Option<i64>,
    pub ended: Option<i64>,
    pub exit: Option<i64>,
    pub error: Option<String>,
    pub output: String,
}

/// What reagent keeps about a trigger's runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TriggerState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run: Option<i64>,
    /// Failed runs in a row.
    #[serde(default)]
    pub failures: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// The script (by hash) the person or the policy allowed, or denied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denied: Option<String>,
    /// Waits for the person to allow this script (hash), and the command shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The repair task started for this failure streak.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair: Option<String>,
    /// Events waiting for a task to end (overlap = queue).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queue: Vec<serde_json::Value>,
    /// Its tasks still going, with the key of the event that started each.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub tasks: std::collections::BTreeMap<String, String>,
    /// A watcher's job, and how many of its output lines were read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    #[serde(default)]
    pub line: usize,
}

/// One run of a trigger's script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct TriggerRun {
    pub id: i64,
    pub started: i64,
    pub ended: Option<i64>,
    pub exit: Option<i64>,
    pub ok: bool,
    pub events: i64,
    pub output: String,
    pub error: Option<String>,
}

fn db_source() -> String {
    "db".into()
}
fn person() -> String {
    "person".into()
}
fn run_file() -> String {
    "run".into()
}
fn sixty() -> i64 {
    60
}

/// A header for an HTTP MCP server, its value from an environment variable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpCredential {
    #[serde(default = "authorization")]
    pub header: String,
    pub env: String,
    #[serde(default)]
    pub prefix: String,
}

fn authorization() -> String {
    "Authorization".into()
}

/// An MCP server the person added.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct McpServer {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub url: Option<String>,
    /// The program and its arguments.
    #[serde(default)]
    pub command: Option<Json<Vec<String>>>,
    #[serde(default)]
    pub env: Json<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    pub credential: Option<Json<McpCredential>>,
    #[serde(default = "yes")]
    pub lazy: bool,
    #[serde(default)]
    pub idempotent: Json<Vec<String>>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub created: i64,
    /// Only this project's tasks get it (none: every task).
    #[serde(default)]
    pub project: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Todo {
    pub id: i64,
    pub text: String,
    /// pending, in_progress, done or cancelled.
    pub status: String,
    pub updated: i64,
}

/// A prompt template kept in reagent (its whole markdown file).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct KeptTemplate {
    pub id: i64,
    /// None: every project's.
    pub project: Option<String>,
    pub name: String,
    pub text: String,
    pub updated: i64,
}

/// A message queued for a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Queued {
    pub id: i64,
    pub text: String,
    #[sqlx(rename = "created")]
    pub at: i64,
    /// Being handed to the task: no longer editable.
    pub sending: bool,
    /// The person's own (they may change or cancel it); else reagent's (a job's end, a report, …).
    pub person: bool,
}

/// One of a task's reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Report {
    pub content: String,
    pub at: i64,
}

pub const TODO_STATES: [&str; 4] = ["pending", "in_progress", "done", "cancelled"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Notification {
    pub id: i64,
    pub at: i64,
    pub kind: String,
    pub task: Option<String>,
    pub title: String,
    pub body: String,
    pub seen: bool,
}

#[derive(Clone)]
pub struct Store {
    pub pool: SqlitePool,
    /// The key secrets are encrypted with (`secret.key` next to the database).
    key: [u8; 32],
}

/// An apprise channel (its URL decrypted).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NotifyChannel {
    pub id: i64,
    pub name: String,
    pub url: String,
    /// The kinds it gets; none: all.
    pub events: Option<Vec<String>>,
    pub enabled: bool,
}

/// A secret as the person sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Secret {
    pub name: String,
    pub value: String,
    /// The project it's for (none: every project).
    pub project: Option<String>,
    pub updated: i64,
}

/// An environment variable's name: letters, digits and _, not first a digit.
pub fn check_secret_name(n: &str) -> Result<(), String> {
    let ok = !n.is_empty() && n.len() <= 100 && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !n.starts_with(|c: char| c.is_ascii_digit());
    if ok { Ok(()) } else { Err(format!("{n:?}: an environment variable's name (letters, digits, _)")) }
}

/// The key file: made the first time (32 random bytes, only the owner may read it).
fn load_key(path: &Path) -> anyhow::Result<[u8; 32]> {
    if let Ok(b) = std::fs::read(path) {
        return b.try_into().map_err(|_| anyhow::anyhow!("{}: not a key (32 bytes)", path.display()));
    }
    use chacha20poly1305::aead::{KeyInit, OsRng};
    let key: [u8; 32] = chacha20poly1305::XChaCha20Poly1305::generate_key(&mut OsRng).into();
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?.write_all(&key)?;
    }
    Ok(key)
}

type R<T> = Result<T, sqlx::Error>;

/// Takes the write lock up front: a deferred transaction that reads, then writes, fails with
/// SQLITE_BUSY at once when another connection wrote meanwhile (busy_timeout can't help it).
const IMMEDIATE: &str = "BEGIN IMMEDIATE";

impl Store {
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(30));
        // WAL: readers run alongside the one writer; writers wait on busy_timeout. One connection
        // queued every query behind the slowest and timed out under load.
        let pool = SqlitePoolOptions::new().max_connections(8).connect_with(opts).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        let key = load_key(&path.with_file_name("secret.key"))?;
        Ok(Store { pool, key })
    }

    // --- projects -------------------------------------------------------

    pub async fn projects(&self) -> R<Vec<Project>> {
        sqlx::query_as("select * from projects order by name").fetch_all(&self.pool).await
    }

    pub async fn project(&self, slug: &str) -> R<Option<Project>> {
        sqlx::query_as("select * from projects where slug = $1").bind(slug).fetch_optional(&self.pool).await
    }

    /// Adds a project, or changes one (by its slug).
    pub async fn put_project(&self, p: &Project) -> R<()> {
        sqlx::query(
            "insert into projects (slug, name, path, memory, worktrees, merge, default_action, profile, budget, env, devshell, devshell_attr, kind, max_tasks, worktree_sharing)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
             on conflict (slug) do update set name = $2, path = $3, memory = $4, worktrees = $5, merge = $6,
               default_action = $7, profile = $8, budget = $9, env = $10, devshell = $11, devshell_attr = $12, kind = $13, max_tasks = $14, worktree_sharing = $15",
        )
        .bind(&p.slug)
        .bind(&p.name)
        .bind(&p.path)
        .bind(&p.memory)
        .bind(&p.worktrees)
        .bind(&p.merge)
        .bind(&p.default_action)
        .bind(&p.profile)
        .bind(&p.budget)
        .bind(&p.env)
        .bind(&p.devshell)
        .bind(&p.devshell_attr)
        .bind(&p.kind)
        .bind(p.max_tasks)
        .bind(&p.worktree_sharing)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remove_project(&self, slug: &str) -> R<bool> {
        let had_servers = self.mcp_servers().await?.iter().any(|m| m.project.as_deref() == Some(slug));
        let gone = sqlx::query("delete from projects where slug = $1").bind(slug).execute(&self.pool).await?.rows_affected() > 0;
        // Its own MCP servers went with it.
        if had_servers {
            self.mcp_changed().await?;
        }
        Ok(gone)
    }

    // --- rules ----------------------------------------------------------

    pub async fn rules(&self, project: &str) -> R<Vec<Rule>> {
        sqlx::query_as("select * from rules where project = $1 order by pos, id").bind(project).fetch_all(&self.pool).await
    }

    /// Replaces a project's rules with these, in this order.
    pub async fn set_rules(&self, project: &str, rules: &[Rule]) -> R<()> {
        let mut tx = self.pool.begin_with(IMMEDIATE).await?;
        sqlx::query("delete from rules where project = $1").bind(project).execute(&mut *tx).await?;
        for (i, r) in rules.iter().enumerate() {
            sqlx::query("insert into rules (project, pos, tool, command, target, action) values ($1, $2, $3, $4, $5, $6)")
                .bind(project)
                .bind(i as i64)
                .bind(&r.tool)
                .bind(&r.command)
                .bind(&r.target)
                .bind(&r.action)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await
    }

    /// Adds a rule in front of the others (an "always allow" from an approval).
    pub async fn prepend_rule(&self, project: &str, r: &Rule) -> R<()> {
        let mut rules = self.rules(project).await?;
        rules.insert(0, r.clone());
        self.set_rules(project, &rules).await
    }

    // --- tasks ----------------------------------------------------------

    pub async fn add_task(&self, t: &NewTask) -> R<Task> {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "insert into tasks (id, project, parent, title, prompt, origin, cwd, profile, budget, skills, kind, home)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(&id)
        .bind(&t.project)
        .bind(&t.parent)
        .bind(&t.title)
        .bind(&t.prompt)
        .bind(&t.origin)
        .bind(&t.cwd)
        .bind(&t.profile)
        .bind(Json(&t.budget))
        .bind(Json(&t.skills))
        .bind(&t.kind)
        .bind(&t.home)
        .execute(&self.pool)
        .await?;
        Ok(self.task(&id).await?.expect("just added"))
    }

    pub async fn task(&self, id: &str) -> R<Option<Task>> {
        sqlx::query_as("select * from tasks where id = $1").bind(id).fetch_optional(&self.pool).await
    }

    pub async fn task_by_agent(&self, agent: &str) -> R<Option<Task>> {
        sqlx::query_as("select * from tasks where agent = $1").bind(agent).fetch_optional(&self.pool).await
    }

    /// Tasks, newest first: of a project, of a parent, or all; `active` only those still going.
    pub async fn tasks(&self, project: Option<&str>, parent: Option<&str>, active: bool, limit: i64) -> R<Vec<Task>> {
        sqlx::query_as(
            "select * from tasks where ($1 is null or project = $1) and ($2 is null or parent = $2)
             and (not $3 or state in ('running', 'waiting', 'paused', 'queued')) order by created desc limit $4",
        )
        .bind(project)
        .bind(parent)
        .bind(active)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    /// What a task's earlier agents used, as counted again.
    pub async fn set_base_usage(&self, id: &str, tokens: i64, cost: f64) -> R<()> {
        sqlx::query("update tasks set tokens = tokens - base_tokens + $2, cost = cost - base_cost + $3, base_tokens = $2, base_cost = $3 where id = $1").bind(id).bind(tokens).bind(cost).execute(&self.pool).await?;
        Ok(())
    }

    /// A new agent for a task whose old one used `tokens` and `cost`: carried over.
    pub async fn move_agent(&self, id: &str, agent: &str, tokens: i64, cost: f64) -> R<()> {
        sqlx::query("update tasks set agent = $2, base_tokens = base_tokens + $3, base_cost = base_cost + $4, tokens = base_tokens + $3, cost = base_cost + $4, updated = unixepoch() where id = $1")
            .bind(id)
            .bind(agent)
            .bind(tokens)
            .bind(cost)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_agent(&self, id: &str, agent: &str) -> R<()> {
        sqlx::query("update tasks set agent = $2, started = coalesce(started, unixepoch()), updated = unixepoch() where id = $1").bind(id).bind(agent).execute(&self.pool).await?;
        Ok(())
    }

    /// Tasks taking a place (running or waiting; not subtasks): of a project, or all.
    pub async fn running_tasks(&self, project: Option<&str>) -> R<i64> {
        sqlx::query_scalar("select count(*) from tasks where parent is null and state in ('running', 'waiting') and ($1 is null or project = $1)").bind(project).fetch_one(&self.pool).await
    }

    /// What a repair task waits behind: the project's other tasks running or
    /// waiting, and those queued that aren't repairs.
    pub async fn busy_besides(&self, project: &str, except: &str) -> R<i64> {
        sqlx::query_scalar("select count(*) from tasks where project = $1 and id != $2 and (state in ('running', 'waiting') or (state = 'queued' and origin not like 'trigger-repair:%' and origin not like 'service-repair:%'))")
            .bind(project)
            .bind(except)
            .fetch_one(&self.pool)
            .await
    }

    /// Top-level tasks working in a persistent worktree now (running, waiting or paused).
    pub async fn tasks_in_home(&self, project: &str, home: &str) -> R<i64> {
        sqlx::query_scalar("select count(*) from tasks where parent is null and state in ('running', 'waiting', 'paused') and project = $1 and home = $2").bind(project).bind(home).fetch_one(&self.pool).await
    }

    /// Tasks that worked or work in a persistent worktree, newest first.
    pub async fn home_tasks(&self, project: &str, home: &str) -> R<Vec<Task>> {
        sqlx::query_as("select * from tasks where project = $1 and home = $2 order by created desc limit 200").bind(project).bind(home).fetch_all(&self.pool).await
    }

    // --- persistent worktrees -------------------------------------------

    pub async fn persistent_worktrees(&self, project: &str) -> R<Vec<PersistentWorktree>> {
        sqlx::query_as("select * from persistent_worktrees where project = $1 order by name").bind(project).fetch_all(&self.pool).await
    }

    pub async fn persistent_worktree(&self, project: &str, name: &str) -> R<Option<PersistentWorktree>> {
        sqlx::query_as("select * from persistent_worktrees where project = $1 and name = $2").bind(project).bind(name).fetch_optional(&self.pool).await
    }

    pub async fn add_persistent_worktree(&self, w: &PersistentWorktree) -> R<()> {
        sqlx::query("insert into persistent_worktrees (project, name, path, branch, base, sharing, adopted) values ($1, $2, $3, $4, $5, $6, $7)")
            .bind(&w.project)
            .bind(&w.name)
            .bind(&w.path)
            .bind(&w.branch)
            .bind(&w.base)
            .bind(&w.sharing)
            .bind(w.adopted)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_worktree_sharing(&self, project: &str, name: &str, sharing: Option<&str>) -> R<bool> {
        Ok(sqlx::query("update persistent_worktrees set sharing = $3 where project = $1 and name = $2").bind(project).bind(name).bind(sharing).execute(&self.pool).await?.rows_affected() > 0)
    }

    pub async fn remove_persistent_worktree(&self, project: &str, name: &str) -> R<bool> {
        Ok(sqlx::query("delete from persistent_worktrees where project = $1 and name = $2").bind(project).bind(name).execute(&self.pool).await?.rows_affected() > 0)
    }

    /// Queued tasks, oldest first.
    pub async fn queued_tasks(&self) -> R<Vec<Task>> {
        sqlx::query_as("select * from tasks where state = 'queued' order by created, rowid").fetch_all(&self.pool).await
    }

    /// A queued task's prompt (a message for it is added before it starts).
    pub async fn set_prompt(&self, id: &str, prompt: &str) -> R<()> {
        sqlx::query("update tasks set prompt = $2, updated = unixepoch() where id = $1").bind(id).bind(prompt).execute(&self.pool).await?;
        Ok(())
    }

    /// Sets a task's state; `wait` is what it waits for (with `waiting`).
    pub async fn set_state(&self, id: &str, state: &str, wait: Option<&serde_json::Value>) -> R<()> {
        let finished = matches!(state, "done" | "failed" | "cancelled").then(now);
        sqlx::query("update tasks set state = $2, wait = $3, updated = unixepoch(), finished = coalesce($4, case when $2 in ('done', 'failed', 'cancelled') then finished end) where id = $1")
            .bind(id)
            .bind(state)
            .bind(wait.map(Json))
            .bind(finished)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_title(&self, id: &str, title: &str) -> R<()> {
        sqlx::query("update tasks set title = $2, updated = unixepoch() where id = $1").bind(id).bind(title).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn set_report(&self, id: &str, report: &str) -> R<()> {
        let mut tx = self.pool.begin_with(IMMEDIATE).await?;
        sqlx::query("update tasks set report = $2, updated = unixepoch() where id = $1").bind(id).bind(report).execute(&mut *tx).await?;
        sqlx::query("insert into reports (task, content) values ($1, $2)").bind(id).bind(report).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn templates(&self) -> R<Vec<KeptTemplate>> {
        sqlx::query_as("select id, project, name, text, updated from templates order by name").fetch_all(&self.pool).await
    }

    /// Adds or changes (by id) a kept template; its id.
    pub async fn put_template(&self, id: Option<i64>, project: Option<&str>, name: &str, text: &str) -> Result<i64, String> {
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || "_-./".contains(c)) || name.starts_with(['.', '/']) {
            return Err("a template's name is letters, digits, _ - . /".into());
        }
        let r = match id {
            Some(id) => sqlx::query_scalar("update templates set project = $2, name = $3, text = $4, updated = unixepoch() where id = $1 returning id").bind(id).bind(project).bind(name).bind(text).fetch_optional(&self.pool).await,
            None => sqlx::query_scalar("insert into templates (project, name, text) values ($1, $2, $3) returning id").bind(project).bind(name).bind(text).fetch_optional(&self.pool).await,
        };
        match r {
            Ok(Some(id)) => Ok(id),
            Ok(None) => Err(format!("no template {}", id.unwrap_or_default())),
            Err(e) if e.to_string().contains("UNIQUE") => Err(format!("there's a template {name} {}already", project.map(|p| format!("for {p} ")).unwrap_or_default())),
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn remove_template(&self, id: i64) -> R<bool> {
        Ok(sqlx::query("delete from templates where id = $1").bind(id).execute(&self.pool).await?.rows_affected() > 0)
    }

    /// Queues a message for a task (delivered, then removed, by the app); its id.
    /// Queues a message; `person`: the person's own, which they may change or cancel until it's taken.
    pub async fn push_outbox(&self, task: &str, text: &str, person: bool) -> R<i64> {
        sqlx::query_scalar("insert into outbox (task, text, person) values ($1, $2, $3) returning id").bind(task).bind(text).bind(person).fetch_one(&self.pool).await
    }

    /// The task's messages not delivered yet, oldest first.
    pub async fn outbox(&self, task: &str) -> R<Vec<Queued>> {
        sqlx::query_as("select id, text, created, sending, person from outbox where task = $1 order by id").bind(task).fetch_all(&self.pool).await
    }

    /// Takes a queued message for delivery: from then on it can't be changed or cancelled.
    pub async fn claim_outbox(&self, id: i64) -> R<bool> {
        Ok(sqlx::query("update outbox set sending = 1 where id = $1 and sending = 0").bind(id).execute(&self.pool).await?.rows_affected() > 0)
    }

    /// A message not yet being delivered, changed.
    pub async fn edit_outbox(&self, task: &str, id: i64, text: &str) -> R<bool> {
        Ok(sqlx::query("update outbox set text = $3 where task = $1 and id = $2 and sending = 0 and person").bind(task).bind(id).bind(text).execute(&self.pool).await?.rows_affected() > 0)
    }

    /// Cancels one of the person's messages not yet being delivered, or (None) all of them: how many.
    pub async fn cancel_outbox(&self, task: &str, id: Option<i64>) -> R<u64> {
        Ok(sqlx::query("delete from outbox where task = $1 and ($2 is null or id = $2) and sending = 0 and person").bind(task).bind(id).execute(&self.pool).await?.rows_affected())
    }

    /// After a restart nothing is being delivered: claimed ones are sent again.
    pub async fn unclaim_outbox(&self) -> R<()> {
        sqlx::query("update outbox set sending = 0").execute(&self.pool).await?;
        Ok(())
    }

    /// Tasks with messages waiting.
    pub async fn outbox_tasks(&self) -> R<Vec<String>> {
        sqlx::query_scalar("select distinct task from outbox").fetch_all(&self.pool).await
    }

    pub async fn remove_outbox(&self, id: i64) -> R<()> {
        sqlx::query("delete from outbox where id = $1").bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// Every report the task made, oldest first (`set_report` appends).
    pub async fn reports(&self, task: &str) -> R<Vec<Report>> {
        sqlx::query_as("select content, at from reports where task = $1 order by id").bind(task).fetch_all(&self.pool).await
    }

    pub async fn set_cwd(&self, id: &str, cwd: &str, worktree: Option<&Worktree>) -> R<()> {
        sqlx::query("update tasks set cwd = $2, worktree = $3, updated = unixepoch() where id = $1")
            .bind(id)
            .bind(cwd)
            .bind(worktree.map(Json))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The current agent's usage (what earlier agents used is added).
    pub async fn set_usage(&self, id: &str, tokens: i64, cost: f64) -> R<()> {
        sqlx::query("update tasks set tokens = base_tokens + $2, cost = base_cost + $3 where id = $1").bind(id).bind(tokens).bind(cost).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn set_task_limits(&self, id: &str, profile: &str, budget: &Budget) -> R<()> {
        sqlx::query("update tasks set profile = $2, budget = $3, updated = unixepoch() where id = $1")
            .bind(id)
            .bind(profile)
            .bind(Json(budget))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// What a project's tasks cost since `since` (unix seconds).
    pub async fn cost_since(&self, project: &str, since: i64) -> R<f64> {
        sqlx::query_scalar("select coalesce(sum(cost), 0.0) from tasks where project = $1 and created >= $2")
            .bind(project)
            .bind(since)
            .fetch_one(&self.pool)
            .await
    }

    // --- cron -----------------------------------------------------------

    pub async fn crons(&self, project: Option<&str>) -> R<Vec<Cron>> {
        sqlx::query_as("select * from cron where $1 is null or project = $1 order by id").bind(project).fetch_all(&self.pool).await
    }

    pub async fn cron(&self, id: i64) -> R<Option<Cron>> {
        sqlx::query_as("select * from cron where id = $1").bind(id).fetch_optional(&self.pool).await
    }

    /// Adds a cron entry (id 0) or changes one; returns its id.
    pub async fn put_cron(&self, c: &Cron) -> R<i64> {
        if c.id == 0 {
            let id = sqlx::query_scalar(
                "insert into cron (project, expr, tz, title, prompt, options, overlap, catch_up, enabled, last_run, next_run)
                 values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) returning id",
            )
            .bind(&c.project)
            .bind(&c.expr)
            .bind(&c.tz)
            .bind(&c.title)
            .bind(&c.prompt)
            .bind(&c.options)
            .bind(&c.overlap)
            .bind(c.catch_up)
            .bind(c.enabled)
            .bind(c.last_run)
            .bind(c.next_run)
            .fetch_one(&self.pool)
            .await?;
            return Ok(id);
        }
        sqlx::query(
            "update cron set project = $2, expr = $3, tz = $4, title = $5, prompt = $6, options = $7, overlap = $8,
               catch_up = $9, enabled = $10, last_run = $11, next_run = $12, queued = $13 where id = $1",
        )
        .bind(c.id)
        .bind(&c.project)
        .bind(&c.expr)
        .bind(&c.tz)
        .bind(&c.title)
        .bind(&c.prompt)
        .bind(&c.options)
        .bind(&c.overlap)
        .bind(c.catch_up)
        .bind(c.enabled)
        .bind(c.last_run)
        .bind(c.next_run)
        .bind(c.queued)
        .execute(&self.pool)
        .await?;
        Ok(c.id)
    }

    pub async fn remove_cron(&self, id: i64) -> R<bool> {
        Ok(sqlx::query("delete from cron where id = $1").bind(id).execute(&self.pool).await?.rows_affected() > 0)
    }

    // --- triggers -------------------------------------------------------

    pub async fn triggers(&self, project: Option<&str>) -> R<Vec<Trigger>> {
        sqlx::query_as("select * from triggers where $1 is null or project = $1 order by project, name").bind(project).fetch_all(&self.pool).await
    }

    pub async fn trigger(&self, project: &str, name: &str) -> R<Option<Trigger>> {
        sqlx::query_as("select * from triggers where project = $1 and name = $2").bind(project).bind(name).fetch_optional(&self.pool).await
    }

    /// Adds or changes a trigger's definition (its state and when it was made stay).
    pub async fn put_trigger(&self, t: &Trigger) -> R<()> {
        sqlx::query(
            "insert into triggers (project, name, source, made_by, description, mode, every, cron, tz, script, script_file, timeout, overlap, title, prompt, options, secret, devshell, enabled, state)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)
             on conflict (project, name) do update set source = $3, made_by = $4, description = $5, mode = $6, every = $7, cron = $8, tz = $9, script = $10,
               script_file = $11, timeout = $12, overlap = $13, title = $14, prompt = $15, options = $16, secret = $17, devshell = $18, enabled = $19",
        )
        .bind(&t.project)
        .bind(&t.name)
        .bind(&t.source)
        .bind(&t.made_by)
        .bind(&t.description)
        .bind(&t.mode)
        .bind(t.every)
        .bind(&t.cron)
        .bind(&t.tz)
        .bind(&t.script)
        .bind(&t.script_file)
        .bind(t.timeout)
        .bind(&t.overlap)
        .bind(&t.title)
        .bind(&t.prompt)
        .bind(&t.options)
        .bind(&t.secret)
        .bind(t.devshell)
        .bind(t.enabled)
        .bind(&t.state)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_trigger_state(&self, project: &str, name: &str, state: &TriggerState) -> R<()> {
        sqlx::query("update triggers set state = $3 where project = $1 and name = $2").bind(project).bind(name).bind(Json(state)).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn set_trigger_enabled(&self, project: &str, name: &str, enabled: bool) -> R<bool> {
        Ok(sqlx::query("update triggers set enabled = $3 where project = $1 and name = $2").bind(project).bind(name).bind(enabled).execute(&self.pool).await?.rows_affected() > 0)
    }

    pub async fn remove_trigger(&self, project: &str, name: &str) -> R<bool> {
        Ok(sqlx::query("delete from triggers where project = $1 and name = $2").bind(project).bind(name).execute(&self.pool).await?.rows_affected() > 0)
    }

    /// Notes an event's key: true the first time (within 30 days).
    pub async fn trigger_key_new(&self, project: &str, name: &str, key: &str) -> R<bool> {
        sqlx::query("delete from trigger_keys where seen < unixepoch() - 30 * 86400").execute(&self.pool).await?;
        Ok(sqlx::query("insert into trigger_keys (project, name, key) values ($1, $2, $3) on conflict do nothing").bind(project).bind(name).bind(key).execute(&self.pool).await?.rows_affected() > 0)
    }

    /// Records a run (the last 20 are kept); its id.
    pub async fn add_trigger_run(&self, project: &str, name: &str, r: &TriggerRun) -> R<i64> {
        let id = sqlx::query_scalar("insert into trigger_runs (project, name, started, ended, exit, ok, events, output, error) values ($1, $2, $3, $4, $5, $6, $7, $8, $9) returning id")
            .bind(project)
            .bind(name)
            .bind(r.started)
            .bind(r.ended)
            .bind(r.exit)
            .bind(r.ok)
            .bind(r.events)
            .bind(&r.output)
            .bind(&r.error)
            .fetch_one(&self.pool)
            .await?;
        sqlx::query("delete from trigger_runs where project = $1 and name = $2 and id not in (select id from trigger_runs where project = $1 and name = $2 order by id desc limit 20)")
            .bind(project)
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }

    /// A trigger's last runs, newest first.
    pub async fn trigger_runs(&self, project: &str, name: &str) -> R<Vec<TriggerRun>> {
        sqlx::query_as("select id, started, ended, exit, ok, events, output, error from trigger_runs where project = $1 and name = $2 order by id desc").bind(project).bind(name).fetch_all(&self.pool).await
    }

    // --- services ---------------------------------------------------------

    pub async fn services(&self, project: Option<&str>) -> R<Vec<Service>> {
        sqlx::query_as("select * from services where $1 is null or project = $1 order by project, name").bind(project).fetch_all(&self.pool).await
    }

    pub async fn service(&self, project: &str, name: &str) -> R<Option<Service>> {
        sqlx::query_as("select * from services where project = $1 and name = $2").bind(project).bind(name).fetch_optional(&self.pool).await
    }

    /// Adds or changes a service's definition (its state, whether it's on and when it was made stay).
    pub async fn put_service(&self, v: &Service) -> R<()> {
        sqlx::query(
            "insert into services (project, name, source, made_by, description, script, script_file, health, interval, ready_timeout, stop_timeout, worktree, devshell, notes, enabled, state)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
             on conflict (project, name) do update set source = $3, made_by = $4, description = $5, script = $6, script_file = $7, health = $8,
               interval = $9, ready_timeout = $10, stop_timeout = $11, worktree = $12, devshell = $13, notes = $14",
        )
        .bind(&v.project)
        .bind(&v.name)
        .bind(&v.source)
        .bind(&v.made_by)
        .bind(&v.description)
        .bind(&v.script)
        .bind(&v.script_file)
        .bind(&v.health)
        .bind(v.interval)
        .bind(v.ready_timeout)
        .bind(v.stop_timeout)
        .bind(&v.worktree)
        .bind(v.devshell)
        .bind(&v.notes)
        .bind(v.enabled)
        .bind(&v.state)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_service_state(&self, project: &str, name: &str, state: &ServiceState) -> R<()> {
        sqlx::query("update services set state = $3 where project = $1 and name = $2").bind(project).bind(name).bind(Json(state)).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn set_service_enabled(&self, project: &str, name: &str, enabled: bool) -> R<bool> {
        Ok(sqlx::query("update services set enabled = $3 where project = $1 and name = $2").bind(project).bind(name).bind(enabled).execute(&self.pool).await?.rows_affected() > 0)
    }

    pub async fn remove_service(&self, project: &str, name: &str) -> R<bool> {
        Ok(sqlx::query("delete from services where project = $1 and name = $2").bind(project).bind(name).execute(&self.pool).await?.rows_affected() > 0)
    }

    /// Records a start (the last 20 are kept); its id.
    pub async fn add_service_run(&self, project: &str, name: &str, job: &str, started: i64) -> R<i64> {
        let id = sqlx::query_scalar("insert into service_runs (project, name, job, started) values ($1, $2, $3, $4) returning id").bind(project).bind(name).bind(job).bind(started).fetch_one(&self.pool).await?;
        sqlx::query("delete from service_runs where project = $1 and name = $2 and id not in (select id from service_runs where project = $1 and name = $2 order by id desc limit 20)")
            .bind(project)
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }

    pub async fn service_run_up(&self, id: i64, at: i64) -> R<()> {
        sqlx::query("update service_runs set up = $2 where id = $1").bind(id).bind(at).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn service_run_ended(&self, id: i64, at: i64, exit: Option<i64>, error: Option<&str>, output: &str) -> R<()> {
        sqlx::query("update service_runs set ended = $2, exit = $3, error = $4, output = $5 where id = $1").bind(id).bind(at).bind(exit).bind(error).bind(output).execute(&self.pool).await?;
        Ok(())
    }

    /// A service's last starts, newest first.
    pub async fn service_runs(&self, project: &str, name: &str) -> R<Vec<ServiceRun>> {
        sqlx::query_as("select id, job, started, up, ended, exit, error, output from service_runs where project = $1 and name = $2 order by id desc").bind(project).bind(name).fetch_all(&self.pool).await
    }

    // --- settings, sessions, push ---------------------------------------

    pub async fn setting(&self, key: &str) -> R<Option<String>> {
        sqlx::query_scalar("select value from settings where key = $1").bind(key).fetch_optional(&self.pool).await
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> R<()> {
        sqlx::query("insert into settings (key, value) values ($1, $2) on conflict (key) do update set value = $2")
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn add_session(&self, hash: &str, expires: i64) -> R<()> {
        sqlx::query("insert into sessions (hash, expires) values ($1, $2)").bind(hash).bind(expires).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn session_valid(&self, hash: &str) -> R<bool> {
        Ok(sqlx::query_scalar::<_, i64>("select count(*) from sessions where hash = $1 and expires > unixepoch()").bind(hash).fetch_one(&self.pool).await? > 0)
    }

    pub async fn end_session(&self, hash: &str) -> R<()> {
        sqlx::query("delete from sessions where hash = $1 or expires <= unixepoch()").bind(hash).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn end_all_sessions(&self) -> R<()> {
        sqlx::query("delete from sessions").execute(&self.pool).await?;
        Ok(())
    }

    pub async fn push_subscriptions(&self) -> R<Vec<serde_json::Value>> {
        let rows: Vec<Json<serde_json::Value>> = sqlx::query_scalar("select subscription from push_subscriptions").fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(|j| j.0).collect())
    }

    pub async fn add_push_subscription(&self, endpoint: &str, sub: &serde_json::Value) -> R<()> {
        sqlx::query("insert into push_subscriptions (endpoint, subscription) values ($1, $2) on conflict (endpoint) do update set subscription = $2")
            .bind(endpoint)
            .bind(Json(sub))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn remove_push_subscription(&self, endpoint: &str) -> R<()> {
        sqlx::query("delete from push_subscriptions where endpoint = $1").bind(endpoint).execute(&self.pool).await?;
        Ok(())
    }

    // --- API tokens -----------------------------------------------------

    pub async fn add_api_token(&self, name: &str, hash: &str) -> R<()> {
        sqlx::query("insert into api_tokens (hash, name) values ($1, $2)").bind(hash).bind(name).execute(&self.pool).await?;
        Ok(())
    }

    /// The name of the token with this hash (and it counts as used now).
    pub async fn api_token(&self, hash: &str) -> R<Option<String>> {
        sqlx::query_scalar("update api_tokens set last_used = unixepoch() where hash = $1 returning name").bind(hash).fetch_optional(&self.pool).await
    }

    /// Names, when made and last used.
    pub async fn api_tokens(&self) -> R<Vec<(String, i64, Option<i64>)>> {
        sqlx::query_as("select name, created, last_used from api_tokens order by name").fetch_all(&self.pool).await
    }

    pub async fn revoke_api_token(&self, name: &str) -> R<bool> {
        Ok(sqlx::query("delete from api_tokens where name = $1").bind(name).execute(&self.pool).await?.rows_affected() > 0)
    }

    // --- MCP servers ----------------------------------------------------

    pub async fn mcp_servers(&self) -> R<Vec<McpServer>> {
        sqlx::query_as("select * from mcp_servers order by name").fetch_all(&self.pool).await
    }

    /// Adds or changes a server (by name); the change is noted for a running reagent.
    pub async fn put_mcp_server(&self, m: &McpServer) -> R<()> {
        sqlx::query(
            "insert into mcp_servers (name, description, url, command, env, credential, lazy, idempotent, enabled, project) values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             on conflict (name) do update set description = $2, url = $3, command = $4, env = $5, credential = $6, lazy = $7, idempotent = $8, enabled = $9, project = $10",
        )
        .bind(&m.name)
        .bind(&m.description)
        .bind(&m.url)
        .bind(&m.command)
        .bind(&m.env)
        .bind(&m.credential)
        .bind(m.lazy)
        .bind(&m.idempotent)
        .bind(m.enabled)
        .bind(&m.project)
        .execute(&self.pool)
        .await?;
        self.mcp_changed().await
    }

    pub async fn remove_mcp_server(&self, name: &str) -> R<bool> {
        let gone = sqlx::query("delete from mcp_servers where name = $1").bind(name).execute(&self.pool).await?.rows_affected() > 0;
        if gone {
            self.mcp_changed().await?;
        }
        Ok(gone)
    }

    async fn mcp_changed(&self) -> R<()> {
        self.set_setting("mcp_changed", &chrono::Utc::now().timestamp_micros().to_string()).await
    }

    // --- secrets --------------------------------------------------------

    fn cipher(&self) -> chacha20poly1305::XChaCha20Poly1305 {
        use chacha20poly1305::KeyInit;
        chacha20poly1305::XChaCha20Poly1305::new((&self.key).into())
    }

    /// A value encrypted: its nonce and the ciphertext.
    fn seal(&self, value: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
        use chacha20poly1305::aead::{Aead, AeadCore, OsRng};
        let nonce = chacha20poly1305::XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let enc = self.cipher().encrypt(&nonce, value.as_bytes()).map_err(|e| e.to_string())?;
        Ok((nonce[..].to_vec(), enc))
    }

    fn unseal(&self, what: &str, nonce: &[u8], value: &[u8]) -> Result<String, String> {
        use chacha20poly1305::aead::Aead;
        let plain = self.cipher().decrypt(nonce.into(), value).map_err(|_| format!("{what}: can't be decrypted (another secret.key?)"))?;
        Ok(String::from_utf8_lossy(&plain).into_owned())
    }

    /// Sets a secret (every project's, or one project's).
    pub async fn set_secret(&self, project: Option<&str>, name: &str, value: &str) -> Result<(), String> {
        check_secret_name(name)?;
        let (nonce, enc) = self.seal(value)?;
        let mut tx = self.pool.begin_with(IMMEDIATE).await.map_err(|e| e.to_string())?;
        sqlx::query("delete from secrets where coalesce(project, '') = coalesce($1, '') and name = $2").bind(project).bind(name).execute(&mut *tx).await.map_err(|e| e.to_string())?;
        sqlx::query("insert into secrets (project, name, nonce, value) values ($1, $2, $3, $4)")
            .bind(project)
            .bind(name)
            .bind(nonce)
            .bind(enc)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        tx.commit().await.map_err(|e| e.to_string())
    }

    pub async fn remove_secret(&self, project: Option<&str>, name: &str) -> R<bool> {
        Ok(sqlx::query("delete from secrets where coalesce(project, '') = coalesce($1, '') and name = $2").bind(project).bind(name).execute(&self.pool).await?.rows_affected() > 0)
    }

    /// The secrets of one scope: every project's (`None`) or one project's own.
    pub async fn secrets(&self, project: Option<&str>) -> Result<Vec<Secret>, String> {
        let rows: Vec<(Option<String>, String, Vec<u8>, Vec<u8>, i64)> = sqlx::query_as("select project, name, nonce, value, updated from secrets where coalesce(project, '') = coalesce($1, '') order by name")
            .bind(project)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        rows.into_iter()
            .map(|(project, name, nonce, value, updated)| {
                let value = self.unseal(&name, &nonce, &value)?;
                Ok(Secret { name, value, project, updated })
            })
            .collect()
    }

    /// What a project's commands get: every project's secrets, its own over them.
    pub async fn secrets_for(&self, project: &str) -> Result<Vec<Secret>, String> {
        let mut all = self.secrets(None).await?;
        for own in self.secrets(Some(project)).await? {
            all.retain(|s| s.name != own.name);
            all.push(own);
        }
        all.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(all)
    }

    // --- notification action tokens -------------------------------------

    pub async fn add_action_token(&self, hash: &str, task: &str, action: &serde_json::Value, expires: i64) -> R<()> {
        sqlx::query("insert into action_tokens (hash, task, action, expires) values ($1, $2, $3, $4)").bind(hash).bind(task).bind(Json(action)).bind(expires).execute(&self.pool).await?;
        Ok(())
    }

    /// Takes a token (once): its task and action, if it's unused and not expired.
    pub async fn take_action_token(&self, hash: &str) -> R<Option<(String, serde_json::Value)>> {
        let row: Option<(String, Json<serde_json::Value>)> = sqlx::query_as("update action_tokens set used = true where hash = $1 and not used and expires > unixepoch() returning task, action")
            .bind(hash)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|(t, a)| (t, a.0)))
    }

    // --- todos ----------------------------------------------------------

    pub async fn todos(&self, task: &str) -> R<Vec<Todo>> {
        sqlx::query_as("select id, text, status, updated from todos where task = $1 order by id").bind(task).fetch_all(&self.pool).await
    }

    /// Adds items at the end; their ids.
    pub async fn add_todos(&self, task: &str, items: &[String]) -> R<Vec<i64>> {
        let mut tx = self.pool.begin_with(IMMEDIATE).await?;
        let mut next: i64 = sqlx::query_scalar("select coalesce(max(id), 0) + 1 from todos where task = $1").bind(task).fetch_one(&mut *tx).await?;
        let mut ids = vec![];
        for t in items {
            sqlx::query("insert into todos (task, id, text) values ($1, $2, $3)").bind(task).bind(next).bind(t).execute(&mut *tx).await?;
            ids.push(next);
            next += 1;
        }
        tx.commit().await?;
        Ok(ids)
    }

    /// Changes an item's status and/or text; false: there's no such item.
    pub async fn update_todo(&self, task: &str, id: i64, status: Option<&str>, text: Option<&str>) -> R<bool> {
        Ok(sqlx::query("update todos set status = coalesce($3, status), text = coalesce($4, text), updated = unixepoch() where task = $1 and id = $2")
            .bind(task)
            .bind(id)
            .bind(status)
            .bind(text)
            .execute(&self.pool)
            .await?
            .rows_affected()
            > 0)
    }

    /// Empties a task's list.
    pub async fn clear_todos(&self, task: &str) -> R<()> {
        sqlx::query("delete from todos where task = $1").bind(task).execute(&self.pool).await?;
        Ok(())
    }

    // --- working memory ---------------------------------------------------

    /// A task's working memory: its slots, by key.
    pub async fn working_memory(&self, task: &str) -> R<serde_json::Map<String, serde_json::Value>> {
        let rows: Vec<(String, Json<serde_json::Value>)> = sqlx::query_as("select key, value from working_memory where task = $1 order by key").bind(task).fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(|(k, v)| (k, v.0)).collect())
    }

    /// Sets a slot (a new one or a new value).
    pub async fn set_working_memory(&self, task: &str, key: &str, value: &serde_json::Value) -> R<()> {
        sqlx::query("insert into working_memory (task, key, value) values ($1, $2, $3) on conflict (task, key) do update set value = excluded.value, updated = unixepoch()")
            .bind(task)
            .bind(key)
            .bind(Json(value))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Removes a slot; false when there was none.
    pub async fn remove_working_memory(&self, task: &str, key: &str) -> R<bool> {
        Ok(sqlx::query("delete from working_memory where task = $1 and key = $2").bind(task).bind(key).execute(&self.pool).await?.rows_affected() > 0)
    }

    // --- apprise channels -------------------------------------------------

    pub async fn notify_channels(&self) -> Result<Vec<NotifyChannel>, String> {
        let rows: Vec<(i64, String, Vec<u8>, Vec<u8>, Option<Json<Vec<String>>>, bool)> =
            sqlx::query_as("select id, name, nonce, url, events, enabled from notify_channels order by name").fetch_all(&self.pool).await.map_err(|e| e.to_string())?;
        rows.into_iter()
            .map(|(id, name, nonce, url, events, enabled)| Ok(NotifyChannel { id, url: self.unseal(&name, &nonce, &url)?, name, events: events.map(|e| e.0), enabled }))
            .collect()
    }

    /// Adds a channel; its id.
    pub async fn add_notify_channel(&self, name: &str, url: &str, events: Option<&[String]>) -> Result<i64, String> {
        if name.trim().is_empty() || !url.contains("://") {
            return Err("a channel needs a name and an apprise URL (tgram://…, ntfys://…, mailto://…)".into());
        }
        let (nonce, enc) = self.seal(url.trim())?;
        sqlx::query_scalar("insert into notify_channels (name, nonce, url, events) values ($1, $2, $3, $4) returning id")
            .bind(name.trim())
            .bind(nonce)
            .bind(enc)
            .bind(events.map(Json))
            .fetch_one(&self.pool)
            .await
            .map_err(|e| if e.to_string().contains("UNIQUE") { format!("there's a channel {} already", name.trim()) } else { e.to_string() })
    }

    pub async fn set_notify_channel_enabled(&self, id: i64, enabled: bool) -> R<bool> {
        Ok(sqlx::query("update notify_channels set enabled = $2 where id = $1").bind(id).bind(enabled).execute(&self.pool).await?.rows_affected() > 0)
    }

    pub async fn remove_notify_channel(&self, id: i64) -> R<bool> {
        Ok(sqlx::query("delete from notify_channels where id = $1").bind(id).execute(&self.pool).await?.rows_affected() > 0)
    }

    // --- notifications --------------------------------------------------

    pub async fn add_notification(&self, kind: &str, task: Option<&str>, title: &str, body: &str) -> R<i64> {
        sqlx::query_scalar("insert into notifications (kind, task, title, body) values ($1, $2, $3, $4) returning id")
            .bind(kind)
            .bind(task)
            .bind(title)
            .bind(body)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn notifications(&self, limit: i64) -> R<Vec<Notification>> {
        sqlx::query_as("select * from notifications order by id desc limit $1").bind(limit).fetch_all(&self.pool).await
    }

    pub async fn mark_seen(&self, upto: i64) -> R<()> {
        sqlx::query("update notifications set seen = true where id <= $1").bind(upto).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn mark_one_seen(&self, id: i64) -> R<()> {
        sqlx::query("update notifications set seen = true where id = $1").bind(id).execute(&self.pool).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
