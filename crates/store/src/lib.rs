//! reagent's own data in SQLite (`reagent.db`): projects and their policy
//! rules, tasks, cron, settings, login sessions, push subscriptions and
//! notifications. Schema changes go through `migrations/`.

use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::types::Json;

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
    /// running, waiting, paused, done, failed, cancelled.
    pub state: String,
    pub wait: Option<Json<serde_json::Value>>,
    pub report: Option<String>,
    pub cost: f64,
    pub tokens: i64,
    pub created: i64,
    pub updated: i64,
    pub finished: Option<i64>,
}

impl Task {
    pub fn is_active(&self) -> bool {
        matches!(self.state.as_str(), "running" | "waiting" | "paused")
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
    pub budget: Option<Budget>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
}

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
}

type R<T> = Result<T, sqlx::Error>;

impl Store {
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(30));
        // ponytail: one connection, every query serialised; plenty for one user.
        let pool = SqlitePoolOptions::new().max_connections(1).connect_with(opts).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Store { pool })
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
            "insert into projects (slug, name, path, memory, worktrees, merge, default_action, profile, budget, env)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             on conflict (slug) do update set name = $2, path = $3, memory = $4, worktrees = $5, merge = $6,
               default_action = $7, profile = $8, budget = $9, env = $10",
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
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remove_project(&self, slug: &str) -> R<bool> {
        Ok(sqlx::query("delete from projects where slug = $1").bind(slug).execute(&self.pool).await?.rows_affected() > 0)
    }

    // --- rules ----------------------------------------------------------

    pub async fn rules(&self, project: &str) -> R<Vec<Rule>> {
        sqlx::query_as("select * from rules where project = $1 order by pos, id").bind(project).fetch_all(&self.pool).await
    }

    /// Replaces a project's rules with these, in this order.
    pub async fn set_rules(&self, project: &str, rules: &[Rule]) -> R<()> {
        let mut tx = self.pool.begin().await?;
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
            "insert into tasks (id, project, parent, title, prompt, origin, cwd, profile, budget, skills)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
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
             and (not $3 or state in ('running', 'waiting', 'paused')) order by created desc limit $4",
        )
        .bind(project)
        .bind(parent)
        .bind(active)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn set_agent(&self, id: &str, agent: &str) -> R<()> {
        sqlx::query("update tasks set agent = $2, updated = unixepoch() where id = $1").bind(id).bind(agent).execute(&self.pool).await?;
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

    pub async fn set_report(&self, id: &str, report: &str) -> R<()> {
        sqlx::query("update tasks set report = $2, updated = unixepoch() where id = $1").bind(id).bind(report).execute(&self.pool).await?;
        Ok(())
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

    pub async fn set_usage(&self, id: &str, tokens: i64, cost: f64) -> R<()> {
        sqlx::query("update tasks set tokens = $2, cost = $3 where id = $1").bind(id).bind(tokens).bind(cost).execute(&self.pool).await?;
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
}

#[cfg(test)]
mod tests;
