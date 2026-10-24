//! The JSON API the web UI uses. Most of it is declared once in
//! [`endpoints`] and served to agents outside as the MCP API's tools too; what
//! only a browser can use (login, live streams, terminals, web push, a
//! notification's button) and API tokens are routes of their own.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::Engine;
use futures::{SinkExt, StreamExt};
use reagent_store::{Budget, Cron, Project, Rule};
use reagent_supervisor as sup;
use reagent_tools::app::{MergeAnswer, StartTask};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use subnet_core::agent::PauseMode;

use crate::endpoints::{Endpoints, Who};
use crate::{COOKIE, S, auth, client_ip, session_of};

/// An error as `{error}` with a status.
#[derive(Debug)]
pub struct E(pub StatusCode, pub String);

impl IntoResponse for E {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}

impl From<String> for E {
    fn from(e: String) -> Self {
        let code = if e.starts_with("no ") { StatusCode::NOT_FOUND } else { StatusCode::BAD_REQUEST };
        E(code, e)
    }
}

pub type R<T = Value> = Result<Json<T>, E>;

fn db<T>(r: Result<T, impl std::fmt::Display>) -> Result<T, E> {
    r.map_err(|e| E(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// The web API: the endpoints' routes and the browser's own.
pub fn routes(eps: &Endpoints) -> Router<S> {
    Router::new()
        .route("/api/session", get(session))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/events", get(events))
        .route("/api/push/key", get(push_key))
        .route("/api/push/subscribe", post(push_subscribe))
        .route("/api/push/unsubscribe", post(push_unsubscribe))
        .route("/api/plugins/{id}/update", post(update_plugin))
        .route("/hook/{project}/{name}", post(hook))
        .route("/api/jobs/{job}/stream", get(job_stream))
        .route("/api/ptys/{pty}", get(pty_ws))
        .route("/api/action", post(action))
        .route("/api/tokens", get(tokens).post(add_token))
        .route("/api/tokens/{name}", delete(revoke_token))
        .merge(eps.routes())
}

/// Everything else, each route also an MCP tool (named for agents: what they do).
pub fn endpoints() -> Endpoints {
    let mut e = Endpoints::default();
    e.get("/api/config", "config", "reagent's configuration: the kinds of task, the profiles (models), routing, notifications.", config)
        .get("/api/settings", "settings", "How many tasks may run at once (none: no limit), how many run and wait.", settings)
        .put("/api/settings", "settings_set", "Set how many tasks may run at once (none or 0: no limit); queued ones start if there's room.", put_settings)
        .post("/api/design", "prompt_design", "Start reagent's designer for a rough goal: a design task reads the project, asks the person what it can't find out (task_wait shows its questions; task_answer answers them), and reports a proposal: tasks, cron entries, triggers, repo skills. design_proposal reads it, design_create makes the items you pick.", design_task)
        .post("/api/design/create", "design_create", "Make items from a design proposal (pass the ones you want, as they came or edited): tasks start, cron entries, triggers and repo skills are made. What became of each.", design_create)
        .get("/api/tasks/{task}/proposal", "design_proposal", "A design task's proposal: its note and items (null while it isn't done).", proposal)
        .get("/api/docs", "docs_list", "reagent's documentation (its system skills): how tasks, subtasks, policy, triggers, memory and the rest work.", docs)
        .get("/api/docs/{name}", "docs_read", "One of reagent's docs (docs_list names them).", doc)
        .get("/api/inbox", "inbox", "Everything waiting for the person: tasks waiting or failed, unseen notifications, triggers and services waiting for an approval or failing.", inbox)
        .get("/api/notifications", "notifications", "The last notifications (done, failed, waiting, budget, …).", notifications)
        .post("/api/notifications/seen", "notifications_seen", "Mark notifications seen: every one up to an id, or one.", seen)
        .get("/api/notify/channels", "channel_list", "The apprise notification channels (URLs masked), and how many URLs reagent.hcl has.", channels)
        .post("/api/notify/channels", "channel_add", "Add an apprise notification channel.", add_channel)
        .put("/api/notify/channels/{id}", "channel_set", "Switch a notification channel on or off.", set_channel)
        .delete("/api/notify/channels/{id}", "channel_remove", "Remove a notification channel.", remove_channel)
        .post("/api/notify/channels/{id}/test", "channel_test", "Send a test through a notification channel: what apprise said.", test_channel)
        .get("/api/templates", "template_list", "Prompt templates: a project's (its repo's, kept in reagent, global, plugins'), or the global ones. Each one parsed: what it's for, its fields (a form: task_start's values) or its prompt, what it sets; broken ones with why.", templates)
        .put("/api/templates", "template_put", "Add or change a template kept in reagent (the whole file); a broken one is refused.", put_template)
        .delete("/api/templates/{id}", "template_remove", "Remove a template kept in reagent.", remove_template)
        .get("/api/plugins", "plugin_list", "The plugins: git repositories in reagent's data folder whose skills and rules every task gets (id, URL, ref, commit, skills, rules).", plugins)
        .post("/api/plugins", "plugin_add", "Add a plugin: clone a git repository whose skills (skills/, skill/, .agents/skills/) and rules (rules/, .agents/rules/) every task then gets; a marketplace (.claude-plugin/marketplace.json, .agents/plugins/marketplace.json) brings its local plugins' too.", add_plugin)
        .post("/api/plugins/update", "plugin_update", "Update a plugin (or every one): fetch what it follows and move onto it. Each one's commits before and after, or why not.", update_plugins)
        .delete("/api/plugins/{id}", "plugin_remove", "Remove a plugin (its folder).", remove_plugin)
        .put("/api/plugins/{id}/rules-off", "plugin_rules_off", "Switch a plugin's rules off (kept across updates): tasks don't get them. Lists every one that's off; the rest are on.", plugin_rules_off)
        .get("/api/projects", "projects", "The projects reagent works on (slug is the id other tools take), each with how many tasks go on.", projects)
        .get("/api/projects/{project}", "project_get", "A project's settings.", project)
        .put("/api/projects/{project}", "project_put", "Add a project, or change its settings (the fields given; a new one needs name and path).", put_project)
        .delete("/api/projects/{project}", "project_remove", "Forget a project (refused while tasks go on there; its folder stays).", remove_project)
        .get("/api/projects/{project}/rules", "policy_rules", "A project's policy: its rules, in order.", rules)
        .put("/api/projects/{project}/rules", "policy_rules_set", "Replace a project's policy rules (in order; body: the list).", set_rules)
        .get("/api/projects/{project}/skills", "project_skills", "The skills a project's tasks get, with where each is from and its files.", skills)
        .get("/api/projects/{project}/agent-rules", "project_agent_rules", "The rules (.agents/rules, global, plugins') a project's tasks get, and when.", project_rules)
        .get("/api/projects/{project}/cron", "cron_list", "A project's cron entries.", crons)
        .get("/api/projects/{project}/triggers", "project_triggers", "A project's triggers, whole (each with its webhook path), and its repo triggers that couldn't be read.", triggers)
        .get("/api/projects/{project}/services", "project_services", "A project's services, whole, and its repo services that couldn't be read.", services)
        .put("/api/services/{project}/{name}", "service_put", "Add or change a service, whole (service_add takes a shorter form). The person's command is allowed; an agent's goes by the project's policy.", put_service)
        .delete("/api/services/{project}/{name}", "service_remove", "Remove a service: it's stopped (a repo service's files go too).", remove_service)
        .post("/api/services/{project}/{name}/start", "service_start", "Switch a service on: started at once, its failure streak cleared.", start_service)
        .post("/api/services/{project}/{name}/stop", "service_stop", "Switch a service off: stopped; it stays off until started.", stop_service)
        .post("/api/services/{project}/{name}/restart", "service_restart", "Restart a service: stopped and started again at once.", restart_service)
        .post("/api/services/{project}/{name}/approve", "service_approve", "Allow or deny the command a service waits to run (always: from now on).", approve_service)
        .post("/api/services/{project}/{name}/move", "service_move", "Move a service into the project's repo (.agents/services/<name>/) or back into reagent.", move_service)
        .get("/api/services/{project}/{name}/runs", "service_runs", "A service's last starts and how each ended.", service_runs)
        .get("/api/projects/{project}/worktrees", "worktree_list", "A project's persistent worktrees (named, long-lived checkouts tasks start in: task_start's worktree): branch, base, commits ahead, uncommitted changes, tasks there now; and the worktrees git has that could be adopted.", worktrees)
        .post("/api/projects/{project}/worktrees", "worktree_add", "Make a persistent worktree of a project (a branch checked out apart, for tasks to start in), or adopt one git has (path).", add_worktree)
        .get("/api/projects/{project}/worktrees/{name}", "worktree_get", "A persistent worktree: how it is, its status and diff against its base, the tasks that worked there.", worktree)
        .put("/api/projects/{project}/worktrees/{name}", "worktree_set", "Set whether a persistent worktree is shared or exclusive (one task at a time; none: the project's).", set_worktree)
        .delete("/api/projects/{project}/worktrees/{name}", "worktree_remove", "Remove a persistent worktree and its branch (refused while tasks work there, or while it has work not in its base, unmerged commits or uncommitted files, unless force: that work is discarded); an adopted one is only forgotten unless delete.", remove_worktree)
        .post("/api/projects/{project}/worktrees/{name}/merge", "worktree_merge", "Merge a persistent worktree's branch into its base (commit everything there first); conflicts are named and nothing changes.", merge_worktree)
        .get("/api/projects/{project}/sync", "sync_list", "A project's git sync, a branch each (its folder's, each persistent worktree's): on or off, interval, strategy, remote, what the checkout's .agents/sync.md overrides, and how it's doing (ok, skipped, fixing, stuck, error).", syncs)
        .put("/api/projects/{project}/sync/{place}", "sync_put", "Set a branch's git sync, whole (sync_set changes some fields). place: a persistent worktree's name, or - for the project folder.", put_sync)
        .post("/api/projects/{project}/sync/{place}/run", "sync_run", "Sync a branch now (within seconds; a stuck one goes on). place: a persistent worktree's name, or - for the project folder.", run_sync)
        .put("/api/triggers/{project}/{name}", "trigger_put", "Add or change a trigger, whole (trigger_add takes a shorter form). The person's script is allowed; an agent's goes by the project's policy.", put_trigger)
        .delete("/api/triggers/{project}/{name}", "trigger_remove", "Remove a trigger (a repo trigger's files too).", remove_trigger)
        .post("/api/triggers/{project}/{name}/run", "trigger_run", "Run a poll trigger now (or restart a watcher).", run_trigger)
        .post("/api/triggers/{project}/{name}/enabled", "trigger_enable", "Switch a trigger on or off.", enable_trigger)
        .post("/api/triggers/{project}/{name}/move", "trigger_move", "Move a trigger into the project's repo (.agents/triggers/<name>/) or back into reagent.", move_trigger)
        .post("/api/triggers/{project}/{name}/approve", "trigger_approve", "Allow or deny the script a trigger waits to run (always: from now on).", approve_trigger)
        .get("/api/triggers/{project}/{name}/runs", "trigger_runs", "A trigger's last runs: output, events and what became of them, errors.", trigger_runs)
        .get("/api/memory", "memory_read", "A memory (global or a project's): its index and files, or one file.", memory_read)
        .put("/api/memory", "memory_write", "Write a memory file (and its line in the index).", memory_write)
        .delete("/api/memory", "memory_remove", "Remove a memory file.", memory_remove)
        .post("/api/cron", "cron_put", "Add (id 0) or change a cron entry: a prompt started as a task on a schedule.", put_cron)
        .delete("/api/cron/{id}", "cron_remove", "Remove a cron entry.", remove_cron)
        .post("/api/cron/{id}/run", "cron_run", "Run a cron entry now.", run_cron)
        .get("/api/tasks", "task_list", "Tasks (newest first): of a project or all, of a parent, only those going on (active), at most limit (default 200).", tasks)
        .post("/api/tasks", "task_start", "Start a task in a project (from a prompt, or a template and its values): it works on its own; task_get or task_wait follow it.", start_task)
        .get("/api/tasks/{task}", "task_get", "A task: its state, what it waits for (an approval's call, a question, a merge, its budget), its report (the last; reports: every one, oldest first), usage, worktree, subtasks, todo list, working memory, queued messages.", task)
        .get("/api/tasks/{task}/transcript", "task_transcript", "A task's conversation: what its model is sent now (full: everything as it happened), the last tail messages if given.", transcript)
        .post("/api/tasks/{task}/message", "task_message", "Send a task a message (read before its next step; a finished task goes on with it).", message)
        .delete("/api/tasks/{task}/queued", "task_queued_cancel", "Cancel every message queued for a task not taken yet.", cancel_queued)
        .put("/api/tasks/{task}/queued/{mid}", "task_queued_edit", "Change a message queued for a task (the person's) before it's taken.", edit_queued)
        .delete("/api/tasks/{task}/queued/{mid}", "task_queued_cancel_one", "Cancel one message queued for a task.", cancel_one_queued)
        .post("/api/tasks/{task}/pause", "task_pause", "Pause a task.", pause)
        .post("/api/tasks/{task}/resume", "task_resume", "Resume a paused task.", resume)
        .post("/api/tasks/{task}/cancel", "task_cancel", "Cancel a task (and its commands and terminals).", cancel)
        .post("/api/tasks/{task}/retry", "task_retry", "Retry a failed task from where it failed.", retry)
        .post("/api/tasks/{task}/start", "task_start_now", "Start a queued task now, past the limits on tasks at once.", start_now)
        .post("/api/tasks/{task}/limits", "task_limits", "Set a queued task's profile or budget before it starts.", limits)
        .post("/api/tasks/{task}/raise", "task_raise_budget", "Raise a task's budget by these amounts (it goes on if it was paused over it).", raise)
        .post("/api/tasks/{task}/profile", "task_switch_profile", "Move a task onto another profile (another model): its conversation goes on there; a failed one is retried there.", switch_profile)
        .post("/api/tasks/{task}/approve", "task_approve", "Approve or deny the call a task waits on.", approve)
        .post("/api/tasks/{task}/answer", "task_answer", "Answer a task's question.", answer)
        .post("/api/tasks/{task}/merge", "task_merge", "Decide a merge a task waits for: merge, or send it back with a message.", merge)
        .get("/api/tasks/{task}/diff", "task_diff", "A task's worktree: its branch and base, diffstat, diff and status.", diff)
        .get("/api/tasks/{task}/jobs", "task_jobs", "A task's commands (foreground and background jobs).", jobs)
        .get("/api/tasks/{task}/ptys", "task_ptys", "A task's terminals.", ptys)
        .post("/api/tasks/{task}/ptys", "task_pty_open", "Open a terminal in a task's working directory (the web UI shows it live).", pty_open)
        .get("/api/search", "search", "Search tasks' whole conversations for a regex: one task's (paged), or every task's (of a project), a few hits each.", search)
        .get("/api/jobs/{job}/output", "job_output", "A job's output: lines from..to, the last tail, or the lines matching a pattern (secrets masked).", job_output)
        .post("/api/jobs/{job}/background", "job_background", "Move a foreground job to the background (its task goes on; its end wakes the task).", job_background)
        .post("/api/jobs/{job}/kill", "job_kill", "Kill a job (a signal, default TERM).", job_kill)
        .delete("/api/ptys/{pty}", "pty_close", "Close a terminal.", pty_close)
        .get("/api/mcp", "mcp_server_list", "The MCP servers added for tasks (every task's, a project's, or all), each with whether it runs.", mcp_servers)
        .put("/api/mcp/{name}", "mcp_server_put", "Add or change an MCP server for tasks (every task's, or a project's) and apply it: whether it runs.", put_mcp)
        .delete("/api/mcp/{name}", "mcp_server_remove", "Remove an added MCP server.", remove_mcp)
        .get("/api/secrets", "secret_list", "Secrets: every project's, or a project's own (the person sees the values; an agent the names).", secrets)
        .put("/api/secrets", "secret_set", "Set a secret (every project's, or a project's own).", set_secret)
        .delete("/api/secrets", "secret_remove", "Remove a secret.", remove_secret);
    e
}

#[derive(Deserialize, JsonSchema)]
pub struct Nothing {}

#[derive(Deserialize, JsonSchema)]
pub struct TaskRef {
    pub task: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ProjectRef {
    /// A project's id (projects lists them).
    pub project: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct Named {
    pub project: String,
    pub name: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct Id {
    pub id: i64,
}

#[derive(Deserialize, JsonSchema)]
pub struct Text {
    pub task: String,
    pub text: String,
}

// --- login --------------------------------------------------------------

async fn session(State(s): State<S>, headers: HeaderMap) -> R {
    let set = db(s.w.app.store.setting("password").await)?.is_some();
    let ok = match session_of(&headers) {
        Some(t) => db(s.w.app.store.session_valid(&auth::token_hash(&t)).await)?,
        None => false,
    };
    Ok(Json(json!({"logged_in": ok, "password_set": set})))
}

#[derive(Deserialize)]
struct Login {
    password: String,
}

async fn login(State(s): State<S>, ci: ConnectInfo<SocketAddr>, headers: HeaderMap, Json(l): Json<Login>) -> Result<Response, E> {
    let ip = client_ip(&ci, &headers);
    {
        let mut a = s.attempts.lock().unwrap();
        let e = a.entry(ip).or_insert((0, std::time::Instant::now()));
        if e.1.elapsed() > Duration::from_secs(60) {
            *e = (0, std::time::Instant::now());
        }
        if e.0 >= 5 {
            return Err(E(StatusCode::TOO_MANY_REQUESTS, "too many tries: wait a minute".into()));
        }
    }
    let Some(hash) = db(s.w.app.store.setting("password").await)? else {
        return Err(E(StatusCode::CONFLICT, "no password set yet: run `reagent passwd`".into()));
    };
    if !auth::verify(&l.password, &hash) {
        s.attempts.lock().unwrap().entry(ip).or_insert((0, std::time::Instant::now())).0 += 1;
        return Err(E(StatusCode::UNAUTHORIZED, "wrong password".into()));
    }
    s.attempts.lock().unwrap().remove(&ip);
    let token = auth::new_token();
    let days = 30;
    db(s.w.app.store.add_session(&auth::token_hash(&token), chrono::Utc::now().timestamp() + days * 86_400).await)?;
    // Secure when it came over TLS (through the proxy).
    let secure = headers.get("x-forwarded-proto").is_some_and(|v| v == "https");
    let cookie = format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}", days * 86_400, if secure { "; Secure" } else { "" });
    Ok(([(header::SET_COOKIE, cookie)], Json(json!({"ok": true}))).into_response())
}

async fn logout(State(s): State<S>, headers: HeaderMap) -> Result<Response, E> {
    if let Some(t) = session_of(&headers) {
        db(s.w.app.store.end_session(&auth::token_hash(&t)).await)?;
    }
    Ok(([(header::SET_COOKIE, format!("{COOKIE}=; Path=/; Max-Age=0"))], Json(json!({"ok": true}))).into_response())
}

// --- overview -----------------------------------------------------------

async fn config(s: S, _: Who, _: Nothing) -> R {
    let c = &s.w.app.config;
    let profiles: Vec<Value> = c.profile.iter().map(|(n, p)| json!({"name": n, "model": p.model, "provider": p.provider, "price": p.price, "context": p.context, "fallback": p.fallback})).collect();
    let kinds: Vec<Value> = c.kind.iter().map(|(n, k)| json!({"name": n, "description": k.description, "profile": k.profile, "escalate": k.escalate, "escalate_after": k.escalate_after})).collect();
    Ok(Json(json!({"kinds": kinds, "default_kind": c.default_kind, "routing": c.routing, "profiles": profiles, "default_profile": c.default_profile, "grep_results": c.grep_results, "notify": {"apprise": !c.notify.apprise_urls().is_empty(), "events": c.notify.events, "url": c.notify.url}})))
}

/// What the person sets in the web UI: how many tasks may run at once (none: no limit).
async fn settings(s: S, _: Who, _: Nothing) -> R {
    let running = db(s.w.app.store.running_tasks(None).await)?;
    let queued = db(s.w.app.store.queued_tasks().await)?.len();
    Ok(Json(json!({"max_tasks": s.w.app.max_tasks().await, "running": running, "queued": queued})))
}

#[derive(Deserialize, JsonSchema)]
struct Settings {
    max_tasks: Option<i64>,
}

async fn put_settings(s: S, who: Who, v: Settings) -> R {
    let n = v.max_tasks.filter(|n| *n > 0);
    db(s.w.app.store.set_setting("max_tasks", &n.map(|n| n.to_string()).unwrap_or_default()).await)?;
    // More room: queued tasks start.
    s.w.app.start_queued().await;
    settings(s, who, Nothing {}).await
}

#[derive(Deserialize, JsonSchema)]
struct DesignReq {
    project: String,
    /// What should be done, roughly.
    goal: String,
    /// What the person started from: task (default), cron or trigger.
    #[serde(default)]
    target: Option<String>,
}

/// The designer: a design task that reads the project, asks, and proposes.
async fn design_task(s: S, _: Who, d: DesignReq) -> R {
    let target = d.target.as_deref().unwrap_or("task");
    if !matches!(target, "task" | "cron" | "trigger") {
        return Err(E(StatusCode::BAD_REQUEST, "target: task, cron or trigger".into()));
    }
    Ok(Json(json!(s.w.app.start_design_task(&d.project, target, &d.goal, None).await?)))
}

/// A design task's proposal (null until it has one).
async fn proposal(s: S, _: Who, t: TaskRef) -> R {
    Ok(Json(json!(s.w.app.proposal(&t.task).await?)))
}

#[derive(Deserialize, JsonSchema)]
struct Create {
    project: String,
    /// Items from design_proposal (as they came, edited if you like): the ones to make.
    items: Vec<reagent_tools::design::Item>,
}

/// Makes the items the person chose; what became of each.
async fn design_create(s: S, _: Who, c: Create) -> R {
    let out = s.w.app.create_items(&c.project, &c.items).await?;
    Ok(Json(json!(out.into_iter().map(|r| match r {
        Ok(done) => json!({"ok": true, "done": done}),
        Err(e) => json!({"ok": false, "error": e}),
    }).collect::<Vec<_>>())))
}

/// reagent's own documentation (the system skills).
async fn docs(_: S, _: Who, _: Nothing) -> R {
    Ok(Json(json!(reagent_tools::skills::system().into_iter().map(|(name, description, _)| json!({"name": name, "description": description})).collect::<Vec<_>>())))
}

#[derive(Deserialize, JsonSchema)]
struct DocName {
    name: String,
}

async fn doc(_: S, _: Who, d: DocName) -> R {
    let text = reagent_tools::skills::system_text(&d.name).ok_or_else(|| E(StatusCode::NOT_FOUND, format!("no doc {:?}", d.name)))?;
    let (_, description, body) = reagent_tools::skills::parse(&text);
    Ok(Json(json!({"name": d.name, "description": description, "body": body})))
}

/// Live: task changes, notifications, agent events, job ends.
async fn events(State(s): State<S>) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let rx = s.w.app.events.subscribe();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|e| async move { e.ok().map(|v| Ok(Event::default().data(v.to_string()))) });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Everything waiting for the person.
async fn inbox(s: S, _: Who, _: Nothing) -> R {
    let tasks = db(s.w.app.store.tasks(None, None, true, 1000).await)?;
    let waiting: Vec<_> = tasks.into_iter().filter(|t| t.state == "waiting" || t.state == "failed").collect();
    let failed = db(s.w.app.store.tasks(None, None, false, 200).await)?.into_iter().filter(|t| t.state == "failed").collect::<Vec<_>>();
    let unseen: Vec<_> = db(s.w.app.store.notifications(100).await)?.into_iter().filter(|n| !n.seen).collect();
    // Triggers whose script waits for the person's approval.
    let triggers: Vec<_> = db(s.w.app.store.triggers(None).await)?.into_iter().filter(|t| t.state.0.asking.is_some()).collect();
    // Services waiting for an approval, or failing.
    let services: Vec<_> = db(s.w.app.store.services(None).await)?.into_iter().filter(|v| v.enabled && (v.state.0.asking.is_some() || v.state.0.failures > 0)).collect();
    Ok(Json(json!({"waiting": waiting, "failed": failed, "notifications": unseen, "triggers": triggers, "services": services})))
}

async fn notifications(s: S, _: Who, _: Nothing) -> R {
    Ok(Json(json!(db(s.w.app.store.notifications(200).await)?)))
}

#[derive(Deserialize, JsonSchema)]
struct Upto {
    /// Every one up to this id, or
    upto: Option<i64>,
    /// this one alone.
    id: Option<i64>,
}

async fn seen(s: S, _: Who, u: Upto) -> R {
    match (u.upto, u.id) {
        (Some(upto), None) => db(s.w.app.store.mark_seen(upto).await)?,
        (None, Some(id)) => db(s.w.app.store.mark_one_seen(id).await)?,
        _ => return Err(E(StatusCode::BAD_REQUEST, "upto or id".into())),
    }
    Ok(Json(json!({"ok": true})))
}

/// An apprise URL shown without its token: the scheme and its end.
fn masked(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let tail: String = rest.chars().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{scheme}://…{tail}")
}

/// The person's apprise channels (URLs masked), and reagent.hcl's (how many).
async fn channels(s: S, _: Who, _: Nothing) -> R {
    let c = s.w.app.store.notify_channels().await?;
    Ok(Json(json!({
        "channels": c.iter().map(|c| json!({"id": c.id, "name": c.name, "url": masked(&c.url), "events": c.events, "enabled": c.enabled})).collect::<Vec<_>>(),
        "config_urls": s.w.app.config.notify.apprise_urls().len(),
    })))
}

#[derive(Deserialize, JsonSchema)]
struct AddChannel {
    name: String,
    /// An apprise URL.
    url: String,
    /// The kinds it gets (done, failed, waiting, …); none or empty: every kind.
    events: Option<Vec<String>>,
}

async fn add_channel(s: S, _: Who, b: AddChannel) -> R {
    let events: Option<Vec<String>> = b.events.map(|e| e.into_iter().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect()).filter(|e: &Vec<String>| !e.is_empty());
    let id = s.w.app.store.add_notify_channel(&b.name, &b.url, events.as_deref()).await.map_err(|e| E(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(json!({"id": id})))
}

#[derive(Deserialize, JsonSchema)]
struct SetChannel {
    id: i64,
    enabled: bool,
}

async fn set_channel(s: S, _: Who, b: SetChannel) -> R {
    if !db(s.w.app.store.set_notify_channel_enabled(b.id, b.enabled).await)? {
        return Err(E(StatusCode::NOT_FOUND, format!("no channel {}", b.id)));
    }
    Ok(Json(json!({"ok": true})))
}

async fn remove_channel(s: S, _: Who, i: Id) -> R {
    Ok(Json(json!({"removed": db(s.w.app.store.remove_notify_channel(i.id).await)?})))
}

/// Sends a test through one channel and says what apprise said.
async fn test_channel(s: S, _: Who, i: Id) -> R {
    let c = s.w.app.store.notify_channels().await?.into_iter().find(|c| c.id == i.id).ok_or_else(|| E(StatusCode::NOT_FOUND, format!("no channel {}", i.id)))?;
    match reagent_tools::notify::apprise(std::slice::from_ref(&c.url), "reagent", &format!("A test from reagent through the channel **{}**.", c.name)).await {
        Ok(()) => Ok(Json(json!({"ok": true}))),
        Err(e) => Ok(Json(json!({"ok": false, "error": e}))),
    }
}

#[derive(Deserialize, JsonSchema)]
struct TemplatesQ {
    /// A project's id (none: the global templates).
    project: Option<String>,
}

/// A project's templates (none: the global ones), each parsed, broken ones with their error.
async fn templates(s: S, _: Who, q: TemplatesQ) -> R {
    Ok(Json(json!(s.w.app.templates(q.project.as_deref()).await?)))
}

#[derive(Deserialize, JsonSchema)]
struct PutTemplate {
    /// The one to change (none: a new one).
    id: Option<i64>,
    /// A project's (none: every project's).
    project: Option<String>,
    name: String,
    /// The whole file: YAML frontmatter, then the prompt with {{field}} placeholders.
    text: String,
}

/// Adds or changes a template kept in reagent; a broken one is refused.
async fn put_template(s: S, _: Who, b: PutTemplate) -> R {
    let project = b.project.filter(|p| !p.is_empty());
    if let Some(p) = &project {
        s.w.app.project(p).await?;
    }
    let t = reagent_tools::templates::parse(b.name.trim(), &b.text, "db");
    if let Some(e) = t.error {
        return Err(E(StatusCode::BAD_REQUEST, e));
    }
    let id = s.w.app.store.put_template(b.id, project.as_deref(), &b.name, &b.text).await?;
    Ok(Json(json!({"id": id})))
}

async fn remove_template(s: S, _: Who, i: Id) -> R {
    if !db(s.w.app.store.remove_template(i.id).await)? {
        return Err(E(StatusCode::NOT_FOUND, format!("no template {}", i.id)));
    }
    Ok(Json(json!({"removed": true})))
}

async fn plugins(s: S, _: Who, _: Nothing) -> R {
    Ok(Json(json!(s.w.app.plugins())))
}

#[derive(Deserialize, JsonSchema)]
struct AddPlugin {
    /// A git URL (https, ssh, file).
    url: String,
    /// Its folder's name (default: the URL's last part).
    id: Option<String>,
    /// A branch or tag to follow (default: the remote's default branch).
    #[serde(rename = "ref")]
    reference: Option<String>,
}

async fn add_plugin(s: S, _: Who, b: AddPlugin) -> R {
    Ok(Json(json!(s.w.app.plugin_add(&b.url, b.id.as_deref(), b.reference.as_deref()).await?)))
}

#[derive(Deserialize, JsonSchema)]
struct PluginUpdate {
    /// One plugin (default: every one).
    id: Option<String>,
}

async fn update_plugins(s: S, _: Who, b: PluginUpdate) -> R {
    Ok(Json(json!(s.w.app.plugin_update(b.id.as_deref()).await?)))
}

/// One (the web UI's button): plugin_update with an id covers it for agents.
async fn update_plugin(State(s): State<S>, Path(id): Path<String>) -> R {
    Ok(Json(json!(s.w.app.plugin_update(Some(&id)).await?[0])))
}

#[derive(Deserialize, JsonSchema)]
struct RulesOff {
    id: String,
    /// Its rules no task gets (by name, as plugin_list shows them); the others are on. None: all on.
    #[serde(default)]
    rules: Vec<String>,
}

async fn plugin_rules_off(s: S, _: Who, b: RulesOff) -> R {
    Ok(Json(json!(s.w.app.plugin_rules_off(&b.id, &b.rules).await?)))
}

#[derive(Deserialize, JsonSchema)]
struct PluginId {
    id: String,
}

async fn remove_plugin(s: S, _: Who, b: PluginId) -> R {
    s.w.app.plugin_remove(&b.id).await?;
    Ok(Json(json!({"removed": true})))
}

async fn push_key(State(s): State<S>) -> R {
    Ok(Json(json!({"key": reagent_tools::notify::vapid_public(&s.w.app.store).await?})))
}

async fn push_subscribe(State(s): State<S>, Json(sub): Json<Value>) -> R {
    let endpoint = sub["endpoint"].as_str().ok_or_else(|| E(StatusCode::BAD_REQUEST, "no endpoint".into()))?;
    db(s.w.app.store.add_push_subscription(endpoint, &sub).await)?;
    Ok(Json(json!({"ok": true})))
}

async fn push_unsubscribe(State(s): State<S>, Json(sub): Json<Value>) -> R {
    db(s.w.app.store.remove_push_subscription(sub["endpoint"].as_str().unwrap_or_default()).await)?;
    Ok(Json(json!({"ok": true})))
}

// --- projects -------------------------------------------------------------

async fn projects(s: S, _: Who, _: Nothing) -> R {
    let mut out = vec![];
    for p in db(s.w.app.store.projects().await)? {
        let active = db(s.w.app.store.tasks(Some(&p.slug), None, true, 1000).await)?.len();
        let mut v = json!(p);
        v["active"] = json!(active);
        out.push(v);
    }
    Ok(Json(json!(out)))
}

async fn project(s: S, _: Who, p: ProjectRef) -> R {
    Ok(Json(json!(s.w.app.project(&p.project).await?)))
}

#[derive(Deserialize, JsonSchema)]
struct PutProject {
    project: String,
    /// The settings to change (name, path, merge, budget, …: as project_get shows them).
    #[serde(flatten)]
    fields: Map<String, Value>,
}

async fn put_project(s: S, _: Who, b: PutProject) -> R {
    let slug = b.project;
    let base = serde_json::to_value(Project::new(&slug, &slug, "")).unwrap();
    let mut merged = match db(s.w.app.store.project(&slug).await)? {
        Some(old) => serde_json::to_value(old).unwrap(),
        None => base,
    };
    for (k, v) in b.fields {
        merged[k] = v;
    }
    merged["slug"] = json!(slug);
    let p: Project = serde_json::from_value(merged).map_err(|e| E(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(json!(s.w.app.put_project(p).await?)))
}

async fn remove_project(s: S, _: Who, p: ProjectRef) -> R {
    if !db(s.w.app.store.tasks(Some(&p.project), None, true, 1).await)?.is_empty() {
        return Err(E(StatusCode::CONFLICT, "it has tasks going on: cancel them first".into()));
    }
    Ok(Json(json!({"removed": db(s.w.app.store.remove_project(&p.project).await)?})))
}

async fn rules(s: S, _: Who, p: ProjectRef) -> R {
    Ok(Json(json!(db(s.w.app.store.rules(&p.project).await)?)))
}

#[derive(Deserialize, JsonSchema)]
struct SetRules {
    project: String,
    /// The rules, in order: {tool (a glob over server.tool), args?, target?, action (allow, ask, deny), …} as policy_rules shows them.
    #[schemars(with = "Vec<Value>")]
    body: Vec<Rule>,
}

async fn set_rules(s: S, _: Who, b: SetRules) -> R {
    let slug = b.project;
    s.w.app.project(&slug).await?;
    for r in &b.body {
        reagent_tools::policy::Action::parse(&r.action)?;
        if r.tool.trim().is_empty() {
            return Err(E(StatusCode::BAD_REQUEST, "a rule needs a tool pattern".into()));
        }
    }
    db(s.w.app.store.set_rules(&slug, &b.body).await)?;
    Ok(Json(json!(db(s.w.app.store.rules(&slug).await)?)))
}

/// A project's persistent worktrees, and those git has that could be adopted.
async fn worktrees(s: S, _: Who, p: ProjectRef) -> R {
    let (kept, adoptable) = s.w.app.worktrees().list(&p.project).await?;
    Ok(Json(json!({"worktrees": kept, "adoptable": adoptable})))
}

#[derive(Deserialize, JsonSchema)]
struct AddWorktree {
    project: String,
    #[serde(flatten)]
    worktree: reagent_tools::persistent::AddWorktree,
}

async fn add_worktree(s: S, _: Who, b: AddWorktree) -> R {
    Ok(Json(json!(s.w.app.worktrees().add(&b.project, b.worktree).await?)))
}

/// One: how it is, its status and diff against its base, the tasks that worked there.
async fn worktree(s: S, _: Who, n: Named) -> R {
    let (slug, name) = (&n.project, &n.name);
    let a = s.w.app.worktrees();
    let info = a.info_of(slug, name).await?;
    let (stat, diff, status) = if info.missing {
        (String::new(), String::new(), String::new())
    } else {
        (a.diff(slug, name, true).await.unwrap_or_else(|e| e), a.diff(slug, name, false).await.unwrap_or_default(), a.status(slug, name).await.unwrap_or_default())
    };
    let tasks: Vec<Value> = a.tasks(slug, name).await?.iter().map(|t| json!({"id": t.id, "title": t.name(), "state": t.state, "created": t.created, "parent": t.parent})).collect();
    Ok(Json(json!({"worktree": info, "stat": stat, "diff": diff, "status": status, "tasks": tasks})))
}

#[derive(Deserialize, JsonSchema)]
struct SetWorktree {
    project: String,
    name: String,
    /// shared, exclusive, or null: the project's.
    sharing: Option<String>,
}

async fn set_worktree(s: S, _: Who, b: SetWorktree) -> R {
    Ok(Json(json!(s.w.app.worktrees().set_sharing(&b.project, &b.name, b.sharing.as_deref().filter(|x| !x.is_empty())).await?)))
}

async fn syncs(s: S, _: Who, p: ProjectRef) -> R {
    let p = s.w.app.project(&p.project).await?;
    Ok(Json(json!(reagent_tools::gitsync::list(&s.w.app.store, &p).await)))
}

#[derive(Deserialize, JsonSchema)]
struct PutSync {
    project: String,
    place: String,
    /// enabled, every (seconds), strategy (rebase, merge, ff-only), remote, branch, notes: as sync_list shows them.
    #[serde(flatten)]
    fields: Map<String, Value>,
}

async fn put_sync(s: S, _: Who, b: PutSync) -> R {
    let p = s.w.app.project(&b.project).await?;
    let mut v = Value::Object(b.fields);
    v["project"] = json!(b.project);
    v["place"] = json!(b.place);
    let v: reagent_store::GitSync = serde_json::from_value(v).map_err(|e| E(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(json!(reagent_tools::gitsync::put(&s.w.app.store, &p, v).await?)))
}

#[derive(Deserialize, JsonSchema)]
struct SyncPlace {
    project: String,
    /// A persistent worktree's name, or - (or .) for the project folder.
    place: String,
}

async fn run_sync(s: S, _: Who, b: SyncPlace) -> R {
    s.w.app.project(&b.project).await?;
    reagent_tools::gitsync::run_now(&s.w.app.store, &b.project, reagent_tools::gitsync::place_of(&b.place)).await?;
    Ok(Json(json!({"run": true})))
}

#[derive(Deserialize, JsonSchema)]
struct MergeWorktree {
    project: String,
    name: String,
    /// merge (default), squash or rebase.
    strategy: Option<reagent_tools::git::Strategy>,
    message: Option<String>,
}

async fn merge_worktree(s: S, _: Who, b: MergeWorktree) -> R {
    let said = s.w.app.worktrees().merge(&b.project, &b.name, b.strategy.unwrap_or(reagent_tools::git::Strategy::Merge), b.message).await?;
    Ok(Json(json!({"merged": said})))
}

#[derive(Deserialize, JsonSchema)]
struct RemoveWorktree {
    project: String,
    name: String,
    /// Discard its work not in its base (unmerged commits, uncommitted files).
    #[serde(default)]
    force: bool,
    /// An adopted one: delete its folder and branch too (default: only forget it).
    #[serde(default)]
    delete: bool,
}

async fn remove_worktree(s: S, _: Who, q: RemoveWorktree) -> R {
    let said = s.w.app.worktrees().remove(&q.project, &q.name, q.force, q.delete).await?;
    Ok(Json(json!({"removed": said})))
}

/// The rules the project's tasks get (`.agents/rules`, global, plugins') and when.
async fn project_rules(s: S, _: Who, p: ProjectRef) -> R {
    let p = s.w.app.project(&p.project).await?;
    let out: Vec<Value> = s
        .w
        .app
        .rules_for(std::path::Path::new(&p.path), &p)
        .iter()
        .map(|r| {
            let when = if r.always { "always".to_string() } else if !r.globs.is_empty() { format!("for {}", r.globs.join(", ")) } else { "listed".into() };
            json!({"name": r.name, "path": r.path, "when": when, "description": r.description, "body": r.body})
        })
        .collect();
    Ok(Json(json!(out)))
}

async fn skills(s: S, _: Who, p: ProjectRef) -> R {
    let p = s.w.app.project(&p.project).await?;
    let mut list = s.w.app.skills_for(std::path::Path::new(&p.path), &p);
    // The project's and global ones first; reagent's own docs after.
    list.sort_by_key(|sk| sk.source == "system");
    let out: Vec<Value> = list
        .iter()
        .map(|sk| {
            let files = reagent_tools::skills::load(sk).map(|(_, f)| f).unwrap_or_default();
            json!({"name": sk.name, "description": sk.description, "source": sk.source, "dir": sk.dir, "files": files})
        })
        .collect();
    Ok(Json(json!(out)))
}

// --- memory ---------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
struct MemQ {
    /// `global` or a project's id.
    scope: String,
    /// A file of it (like notes.md).
    file: Option<String>,
}

async fn memory_of(s: &S, scope: &str) -> Result<reagent_tools::memory::Memory, E> {
    Ok(if scope == "global" { s.w.app.global_memory() } else { s.w.app.project_memory(&s.w.app.project(scope).await?) })
}

async fn memory_read(s: S, _: Who, q: MemQ) -> R {
    let m = memory_of(&s, &q.scope).await?;
    match q.file {
        Some(f) => Ok(Json(json!({"file": f, "text": m.read(&f)?}))),
        None => {
            let mut files = vec![];
            if m.dir.exists() {
                for e in ignore::WalkBuilder::new(&m.dir).hidden(false).build().flatten() {
                    let rel = e.path().strip_prefix(&m.dir).unwrap_or(e.path()).display().to_string();
                    if rel.ends_with(".md") && rel != "INDEX.md" && !m.skip.iter().any(|sk| rel.starts_with(&format!("{sk}/"))) {
                        files.push(rel);
                    }
                }
            }
            files.sort();
            Ok(Json(json!({"index": m.index(), "files": files, "dir": m.dir})))
        }
    }
}

#[derive(Deserialize, JsonSchema)]
struct MemW {
    /// `global` or a project's id.
    scope: String,
    file: String,
    text: String,
    /// Its line in the index: what it's about.
    about: String,
}

async fn memory_write(s: S, _: Who, w: MemW) -> R {
    Ok(Json(json!({"ok": memory_of(&s, &w.scope).await?.write(&w.file, &w.text, &w.about)?})))
}

async fn memory_remove(s: S, _: Who, q: MemQ) -> R {
    let f = q.file.ok_or_else(|| E(StatusCode::BAD_REQUEST, "which file?".into()))?;
    Ok(Json(json!({"ok": memory_of(&s, &q.scope).await?.remove(&f)?})))
}

// --- cron -----------------------------------------------------------------

async fn crons(s: S, _: Who, p: ProjectRef) -> R {
    Ok(Json(json!(db(s.w.app.store.crons(Some(&p.project)).await)?)))
}

#[derive(Deserialize, JsonSchema)]
struct PutCron {
    /// id (0: a new one), project, expr, tz, title, prompt, skills, enabled, …: as cron_list shows them.
    #[serde(flatten)]
    fields: Map<String, Value>,
}

async fn put_cron(s: S, _: Who, b: PutCron) -> R {
    let mut c: Cron = serde_json::from_value(Value::Object(b.fields)).map_err(|e| E(StatusCode::BAD_REQUEST, e.to_string()))?;
    s.w.app.project(&c.project).await?;
    if c.id != 0 {
        let old = db(s.w.app.store.cron(c.id).await)?.ok_or_else(|| E(StatusCode::NOT_FOUND, "no such cron entry".into()))?;
        c.last_run = old.last_run;
    }
    reagent_tools::cron::prepare(&mut c, chrono::Utc::now().timestamp())?;
    let id = db(s.w.app.store.put_cron(&c).await)?;
    Ok(Json(json!(db(s.w.app.store.cron(id).await)?)))
}

async fn remove_cron(s: S, _: Who, i: Id) -> R {
    Ok(Json(json!({"removed": db(s.w.app.store.remove_cron(i.id).await)?})))
}

async fn run_cron(s: S, _: Who, i: Id) -> R {
    let c = db(s.w.app.store.cron(i.id).await)?.ok_or_else(|| E(StatusCode::NOT_FOUND, "no such cron entry".into()))?;
    Ok(Json(json!(reagent_tools::cron::run(&s.w.app, &c).await?)))
}

// --- triggers -------------------------------------------------------------

/// A project's triggers (each with its webhook path), and repo files that couldn't be read.
async fn triggers(s: S, _: Who, p: ProjectRef) -> R {
    let slug = p.project;
    s.w.app.project(&slug).await?;
    let list: Vec<Value> = db(s.w.app.store.triggers(Some(&slug)).await)?
        .into_iter()
        .map(|t| {
            let mut v = json!(t);
            v["hash"] = json!(reagent_tools::triggers::script_hash(&t));
            if t.mode == "webhook" {
                v["hook"] = json!(format!("/hook/{}/{}", t.project, t.name));
            }
            v
        })
        .collect();
    let errors = s.w.app.triggers.repo_errors.lock().unwrap().get(&slug).cloned().unwrap_or_default();
    Ok(Json(json!({"triggers": list, "errors": errors})))
}

/// Who made a trigger or a service, and whether its script is allowed as it is.
fn maker(who: Who) -> (&'static str, bool) {
    match who {
        Who::Person => ("person", true),
        Who::Agent => ("mcp", false),
    }
}

#[derive(Deserialize, JsonSchema)]
struct PutNamed {
    project: String,
    name: String,
    /// Its fields, as project_triggers / project_services show them.
    #[serde(flatten)]
    fields: Map<String, Value>,
}

impl PutNamed {
    fn parse<T: serde::de::DeserializeOwned>(self) -> Result<T, E> {
        let mut v = Value::Object(self.fields);
        v["project"] = json!(self.project);
        v["name"] = json!(self.name);
        serde_json::from_value(v).map_err(|e| E(StatusCode::BAD_REQUEST, e.to_string()))
    }
}

/// Adds or changes a trigger (the person's: its script is allowed).
async fn put_trigger(s: S, who: Who, b: PutNamed) -> R {
    let p = s.w.app.project(&b.project).await?;
    let mut t: reagent_store::Trigger = b.parse()?;
    let (by, allowed) = maker(who);
    t.made_by = by.into();
    let t = reagent_tools::triggers::save(&s.w.app.store, &p, t, allowed).await?;
    s.w.app.emit_trigger(&p.slug, &t.name).await;
    Ok(Json(json!(t)))
}

async fn remove_trigger(s: S, _: Who, n: Named) -> R {
    reagent_tools::triggers::remove(&s.w.app, &n.project, &n.name).await?;
    Ok(Json(json!({"removed": true})))
}

async fn run_trigger(s: S, _: Who, n: Named) -> R {
    Ok(Json(json!({"ok": reagent_tools::triggers::run_now(&s.w.app, &n.project, &n.name).await?})))
}

#[derive(Deserialize, JsonSchema)]
struct Enabled {
    project: String,
    name: String,
    enabled: bool,
}

async fn enable_trigger(s: S, _: Who, e: Enabled) -> R {
    if !db(s.w.app.store.set_trigger_enabled(&e.project, &e.name, e.enabled).await)? {
        return Err(E(StatusCode::NOT_FOUND, format!("no trigger {:?} in {}", e.name, e.project)));
    }
    s.w.app.emit_trigger(&e.project, &e.name).await;
    Ok(Json(json!({"enabled": e.enabled})))
}

#[derive(Deserialize, JsonSchema)]
struct MoveTo {
    project: String,
    name: String,
    /// repo (its files into the project folder) or db (kept in reagent).
    to: String,
}

async fn move_trigger(s: S, _: Who, m: MoveTo) -> R {
    let p = s.w.app.project(&m.project).await?;
    let t = reagent_tools::triggers::move_to(&s.w.app.store, &p, &m.name, &m.to).await?;
    s.w.app.emit_trigger(&m.project, &m.name).await;
    Ok(Json(json!(t)))
}

#[derive(Deserialize, JsonSchema)]
struct Approval {
    project: String,
    name: String,
    approved: bool,
    /// Also allow it from now on (a rule).
    #[serde(default)]
    always: bool,
}

async fn approve_trigger(s: S, _: Who, a: Approval) -> R {
    Ok(Json(json!(reagent_tools::triggers::approve(&s.w.app, &a.project, &a.name, a.approved, a.always).await?)))
}

async fn trigger_runs(s: S, _: Who, n: Named) -> R {
    Ok(Json(json!(db(s.w.app.store.trigger_runs(&n.project, &n.name).await)?)))
}

/// A project's services, and its repo services that couldn't be read.
async fn services(s: S, _: Who, p: ProjectRef) -> R {
    let slug = p.project;
    s.w.app.project(&slug).await?;
    let list: Vec<Value> = db(s.w.app.store.services(Some(&slug)).await)?
        .into_iter()
        .map(|v| {
            let mut j = json!(v);
            j["hash"] = json!(reagent_tools::services::script_hash(&v));
            j
        })
        .collect();
    let errors = s.w.app.services.repo_errors.lock().unwrap().get(&slug).cloned().unwrap_or_default();
    Ok(Json(json!({"services": list, "errors": errors})))
}

/// Adds or changes a service (the person's: its command is allowed).
async fn put_service(s: S, who: Who, b: PutNamed) -> R {
    let p = s.w.app.project(&b.project).await?;
    let mut v: reagent_store::Service = b.parse()?;
    let (by, allowed) = maker(who);
    v.made_by = by.into();
    v.health = v.health.filter(|h| !h.trim().is_empty());
    v.worktree = v.worktree.filter(|w| !w.trim().is_empty());
    let v = reagent_tools::services::save(&s.w.app.store, &p, v, allowed).await?;
    s.w.app.emit_service(&p.slug, &v.name).await;
    Ok(Json(json!(v)))
}

async fn remove_service(s: S, _: Who, n: Named) -> R {
    reagent_tools::services::remove(&s.w.app, &n.project, &n.name).await?;
    Ok(Json(json!({"removed": true})))
}

async fn start_service(s: S, _: Who, n: Named) -> R {
    let v = reagent_tools::services::set_enabled(&s.w.app.store, &n.project, &n.name, true).await?;
    s.w.app.emit_service(&n.project, &n.name).await;
    Ok(Json(json!(v)))
}

async fn stop_service(s: S, _: Who, n: Named) -> R {
    let v = reagent_tools::services::set_enabled(&s.w.app.store, &n.project, &n.name, false).await?;
    s.w.app.emit_service(&n.project, &n.name).await;
    Ok(Json(json!(v)))
}

async fn restart_service(s: S, _: Who, n: Named) -> R {
    let v = reagent_tools::services::restart(&s.w.app.store, &n.project, &n.name).await?;
    s.w.app.emit_service(&n.project, &n.name).await;
    Ok(Json(json!(v)))
}

async fn approve_service(s: S, _: Who, a: Approval) -> R {
    Ok(Json(json!(reagent_tools::services::approve(&s.w.app, &a.project, &a.name, a.approved, a.always).await?)))
}

async fn move_service(s: S, _: Who, m: MoveTo) -> R {
    let p = s.w.app.project(&m.project).await?;
    let v = reagent_tools::services::move_to(&s.w.app.store, &p, &m.name, &m.to).await?;
    s.w.app.emit_service(&m.project, &m.name).await;
    Ok(Json(json!(v)))
}

async fn service_runs(s: S, _: Who, n: Named) -> R {
    Ok(Json(json!(db(s.w.app.store.service_runs(&n.project, &n.name).await)?)))
}

/// A webhook (no login: the trigger's secret signs it).
async fn hook(State(s): State<S>, Path((project, name)): Path<(String, String)>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    match reagent_tools::triggers::webhook(s.w.app.clone(), &project, &name, &headers, body).await {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({"ok": true}))).into_response(),
        Err((code, e)) => (StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_REQUEST), Json(json!({"error": e}))).into_response(),
    }
}

// --- tasks ----------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
struct TasksQ {
    project: Option<String>,
    /// A task's subtasks.
    parent: Option<String>,
    /// Only those going on.
    #[serde(default)]
    active: bool,
    limit: Option<i64>,
}

async fn tasks(s: S, _: Who, q: TasksQ) -> R {
    Ok(Json(json!(db(s.w.app.store.tasks(q.project.as_deref(), q.parent.as_deref(), q.active, q.limit.unwrap_or(200)).await)?)))
}

async fn start_task(s: S, who: Who, mut t: StartTask) -> R {
    t.origin = Some(if who == Who::Person { "ui" } else { "mcp" }.into());
    t.parent = None;
    Ok(Json(json!(s.w.app.start_task(t).await?)))
}

async fn task(s: S, _: Who, r: TaskRef) -> R {
    let id = r.task;
    let t = s.w.app.task(&id).await?;
    let kids = db(s.w.app.store.tasks(None, Some(&id), false, 100).await)?;
    let mut v = json!(t);
    v["subtasks"] = json!(kids);
    v["waits_for_merge"] = json!(s.w.app.waits_for_merge(&id).await);
    v["todos"] = json!(db(s.w.app.store.todos(&id).await)?);
    v["working_memory"] = json!(db(s.w.app.store.working_memory(&id).await)?);
    v["reports"] = json!(db(s.w.app.store.reports(&id).await)?);
    v["queued_messages"] = json!(db(s.w.app.store.outbox(&id).await)?);
    Ok(Json(v))
}

#[derive(Deserialize, JsonSchema)]
struct TranscriptQ {
    task: String,
    /// Everything as it happened (default: what its model is sent now).
    #[serde(default)]
    full: bool,
    /// Only the last n messages.
    tail: Option<usize>,
}

async fn transcript(s: S, _: Who, q: TranscriptQ) -> R {
    let t = s.w.app.task(&q.task).await?;
    let agent: uuid::Uuid = t.agent.as_deref().ok_or_else(|| E(StatusCode::NOT_FOUND, "it never started".into()))?.parse().map_err(|e| E(StatusCode::BAD_REQUEST, format!("{e}")))?;
    let mut tr = s.w.app.hub()?.transcript_of(agent, q.full).await.map_err(|e| E(StatusCode::NOT_FOUND, e.to_string()))?;
    if let Some(n) = q.tail {
        tr.messages.drain(..tr.messages.len().saturating_sub(n));
    }
    Ok(Json(json!(tr)))
}

async fn message(s: S, who: Who, m: Text) -> R {
    match who {
        Who::Person => s.w.app.person_message(&m.task, &format!("[from the person] {}", m.text)).await?,
        Who::Agent => s.w.app.message(&m.task, &format!("[from an agent outside] {}", m.text)).await?,
    }
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize, JsonSchema)]
struct Queued {
    task: String,
    /// The queued message's id (task_get's queued_messages).
    mid: i64,
}

#[derive(Deserialize, JsonSchema)]
struct EditQueued {
    task: String,
    mid: i64,
    text: String,
}

async fn edit_queued(s: S, _: Who, m: EditQueued) -> R {
    s.w.app.edit_queued(&m.task, m.mid, &format!("[from the person] {}", m.text.strip_prefix("[from the person] ").unwrap_or(&m.text))).await?;
    Ok(Json(json!({"ok": true})))
}

async fn cancel_one_queued(s: S, _: Who, q: Queued) -> R {
    s.w.app.cancel_queued(&q.task, Some(q.mid)).await?;
    Ok(Json(json!({"cancelled": 1})))
}

async fn cancel_queued(s: S, _: Who, t: TaskRef) -> R {
    Ok(Json(json!({"cancelled": s.w.app.cancel_queued(&t.task, None).await?})))
}

#[derive(Deserialize, JsonSchema)]
struct Pause {
    task: String,
    /// quick (default: running commands finish), safe (after this turn) or hard (now).
    mode: Option<PauseMode>,
}

async fn pause(s: S, _: Who, p: Pause) -> R {
    s.w.app.pause(&p.task, p.mode.unwrap_or(PauseMode::Quick)).await?;
    Ok(Json(json!({"ok": true})))
}

async fn resume(s: S, _: Who, t: TaskRef) -> R {
    s.w.app.resume(&t.task).await?;
    Ok(Json(json!({"ok": true})))
}

async fn cancel(s: S, _: Who, t: TaskRef) -> R {
    s.w.app.cancel(&t.task).await?;
    Ok(Json(json!({"ok": true})))
}

async fn start_now(s: S, _: Who, t: TaskRef) -> R {
    Ok(Json(json!(s.w.app.start_now(&t.task).await?)))
}

async fn retry(s: S, _: Who, t: TaskRef) -> R {
    Ok(Json(json!(s.w.app.retry(&t.task).await?)))
}

#[derive(Deserialize, JsonSchema)]
struct Limits {
    task: String,
    profile: Option<String>,
    budget: Option<Budget>,
}

async fn limits(s: S, _: Who, l: Limits) -> R {
    Ok(Json(json!(s.w.app.set_limits(&l.task, l.profile.as_deref(), l.budget).await?)))
}

#[derive(Deserialize, JsonSchema)]
struct Switch {
    task: String,
    /// A profile from reagent's config (config lists them; task_get shows the current one).
    profile: String,
    #[serde(default)]
    why: Option<String>,
}

/// The person (or an agent) moves a task onto another profile (another model), history and all.
async fn switch_profile(s: S, who: Who, b: Switch) -> R {
    let why = b.why.unwrap_or_else(|| if who == Who::Person { "the person moved it" } else { "an agent outside moved it" }.into());
    Ok(Json(json!(s.w.app.switch_profile(&b.task, &b.profile, &why).await?)))
}

#[derive(Deserialize, JsonSchema)]
struct Raise {
    task: String,
    /// Added to its budget.
    #[serde(flatten)]
    budget: Budget,
}

async fn raise(s: S, _: Who, b: Raise) -> R {
    Ok(Json(json!(s.w.app.raise_budget(&b.task, &b.budget).await?)))
}

#[derive(Deserialize, JsonSchema)]
struct Approve {
    task: String,
    /// The call it waits on (task_get shows it).
    call_id: String,
    approved: bool,
    /// Also allow calls like it from now on (a rule).
    #[serde(default)]
    always: bool,
}

async fn approve(s: S, _: Who, a: Approve) -> R {
    s.w.app.approve(&a.task, &a.call_id, a.approved, a.always).await?;
    Ok(Json(json!({"ok": true})))
}

async fn answer(s: S, _: Who, a: Text) -> R {
    s.w.app.answer(&a.task, &a.text).await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize, JsonSchema)]
struct MergeQ {
    task: String,
    merge: bool,
    /// Why not (when sending it back).
    #[serde(default)]
    message: String,
}

async fn merge(s: S, _: Who, m: MergeQ) -> R {
    s.w.app.decide_merge(&m.task, if m.merge { MergeAnswer::Merge } else { MergeAnswer::Reject(if m.message.is_empty() { "not now".into() } else { m.message }) }).await?;
    Ok(Json(json!({"ok": true})))
}

async fn diff(s: S, _: Who, r: TaskRef) -> R {
    let t = s.w.app.task(&r.task).await?;
    let w = t.worktree.map(|w| w.0).ok_or_else(|| E(StatusCode::NOT_FOUND, "it doesn't work in a worktree".into()))?;
    let dir = std::path::Path::new(&w.path);
    Ok(Json(json!({
        "branch": w.branch, "base": w.base, "path": w.path,
        "stat": reagent_tools::git::diff(dir, &w.base, true)?,
        "diff": reagent_tools::git::diff(dir, &w.base, false)?,
        "status": reagent_tools::git::status(dir)?,
    })))
}

#[derive(Deserialize, JsonSchema)]
struct SearchQ {
    /// A regex.
    #[serde(alias = "pattern")]
    q: String,
    task: Option<String>,
    project: Option<String>,
    page: Option<u32>,
}

/// Across tasks' conversations.
async fn search(s: S, _: Who, q: SearchQ) -> R {
    Ok(Json(s.w.app.search(&q.q, q.task.as_deref(), q.project.as_deref(), None, q.page.unwrap_or(1)).await?))
}

// --- added MCP servers ----------------------------------------------------

#[derive(Deserialize, JsonSchema)]
struct McpQ {
    /// A project's servers; `global` the ones every task gets; none: all.
    project: Option<String>,
}

async fn mcp_servers(s: S, _: Who, q: McpQ) -> R {
    let list: Vec<_> = db(s.w.app.store.mcp_servers().await)?
        .into_iter()
        .filter(|m| match q.project.as_deref() {
            None => true,
            Some("global") => m.project.is_none(),
            Some(p) => m.project.as_deref() == Some(p),
        })
        .collect();
    let status = s.w.app.mcp_status.lock().unwrap().clone();
    Ok(Json(json!(list.iter().map(|m| {
        let mut v = json!(m);
        v["status"] = status.get(&m.name).cloned().unwrap_or(json!({"ok": false, "error": "not applied yet"}));
        v
    }).collect::<Vec<_>>())))
}

#[derive(Deserialize, JsonSchema)]
struct PutMcp {
    name: String,
    /// url or command, headers, env, project, lazy, idempotent, enabled, …: as mcp_server_list shows them.
    #[serde(flatten)]
    fields: Map<String, Value>,
}

/// Adds or changes a server and applies it: the answer says whether it runs.
async fn put_mcp(s: S, _: Who, b: PutMcp) -> R {
    let mut m = Value::Object(b.fields);
    m["name"] = json!(b.name);
    let m: reagent_store::McpServer = serde_json::from_value(m).map_err(|e| E(StatusCode::BAD_REQUEST, e.to_string()))?;
    reagent_tools::cluster::check_mcp(&m)?;
    if let Some(p) = &m.project {
        s.w.app.project(p).await?;
    }
    db(s.w.app.store.put_mcp_server(&m).await)?;
    s.w.app.apply_cluster().await?;
    let status = s.w.app.mcp_status.lock().unwrap().get(&m.name).cloned().unwrap_or_default();
    Ok(Json(json!({"server": m, "status": status})))
}

#[derive(Deserialize, JsonSchema)]
struct McpName {
    name: String,
}

async fn remove_mcp(s: S, _: Who, b: McpName) -> R {
    let gone = db(s.w.app.store.remove_mcp_server(&b.name).await)?;
    if gone {
        s.w.app.apply_cluster().await?;
    }
    Ok(Json(json!({"removed": gone})))
}

// --- notification buttons ---------------------------------------------------

#[derive(Deserialize)]
struct Action {
    token: String,
}

/// A notification's button: the token is its authority (no login).
async fn action(State(s): State<S>, Json(a): Json<Action>) -> R {
    match s.w.app.act(&a.token).await {
        Ok(done) => Ok(Json(json!({"ok": true, "done": done}))),
        Err(e) => Err(E(StatusCode::GONE, e)),
    }
}

// --- secrets ----------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
struct SecretQ {
    /// A project's own; none or `global`: every project's.
    project: Option<String>,
    name: Option<String>,
}

fn scope(p: &Option<String>) -> Option<&str> {
    p.as_deref().filter(|p| *p != "global" && !p.is_empty())
}

/// The person sees them all, values too; an agent outside only their names
/// (tasks get them masked, and so does it).
async fn secrets(s: S, who: Who, q: SecretQ) -> R {
    if let Some(p) = scope(&q.project) {
        s.w.app.project(p).await?;
    }
    let mut v = json!(s.w.app.store.secrets(scope(&q.project)).await?);
    if who == Who::Agent {
        for x in v.as_array_mut().into_iter().flatten() {
            x.as_object_mut().map(|o| o.remove("value"));
        }
    }
    Ok(Json(v))
}

#[derive(Deserialize, JsonSchema)]
struct SetSecret {
    /// A project's own; none or `global`: every project's.
    project: Option<String>,
    name: String,
    value: String,
}

async fn set_secret(s: S, _: Who, b: SetSecret) -> R {
    if let Some(p) = scope(&b.project) {
        s.w.app.project(p).await?;
    }
    s.w.app.store.set_secret(scope(&b.project), b.name.trim(), &b.value).await?;
    Ok(Json(json!({"ok": true})))
}

async fn remove_secret(s: S, _: Who, q: SecretQ) -> R {
    let name = q.name.as_deref().ok_or_else(|| E(StatusCode::BAD_REQUEST, "which secret?".into()))?;
    Ok(Json(json!({"removed": db(s.w.app.store.remove_secret(scope(&q.project), name).await)?})))
}

// --- API tokens (the MCP API; the person's only) -----------------------------

async fn tokens(State(s): State<S>) -> R {
    let v = db(s.w.app.store.api_tokens().await)?;
    Ok(Json(json!(v.iter().map(|(n, c, u)| json!({"name": n, "created": c, "last_used": u})).collect::<Vec<_>>())))
}

#[derive(Deserialize)]
struct Name {
    name: String,
}

/// A new token: shown once.
async fn add_token(State(s): State<S>, Json(n): Json<Name>) -> R {
    let name = n.name.trim();
    if name.is_empty() {
        return Err(E(StatusCode::BAD_REQUEST, "a token needs a name".into()));
    }
    let token = format!("rgt_{}", auth::new_token());
    s.w.app.store.add_api_token(name, &auth::token_hash(&token)).await.map_err(|_| E(StatusCode::CONFLICT, format!("there's a token called {name:?}")))?;
    Ok(Json(json!({"name": name, "token": token})))
}

async fn revoke_token(State(s): State<S>, Path(name): Path<String>) -> R {
    Ok(Json(json!({"revoked": db(s.w.app.store.revoke_api_token(&name).await)?})))
}

// --- jobs and terminals ---------------------------------------------------

async fn jobs(s: S, _: Who, t: TaskRef) -> R {
    Ok(Json(json!(s.w.app.sup.jobs(Some(&t.task)).await?)))
}

#[derive(Deserialize, JsonSchema)]
struct OutQ {
    job: String,
    from: Option<usize>,
    to: Option<usize>,
    tail: Option<usize>,
    /// A regex: only the lines matching it.
    pattern: Option<String>,
}

/// The project whose secrets a job's output is masked with.
async fn job_project(s: &S, job: &str) -> Option<String> {
    let owner = s.w.app.sup.job(job).await.ok()?.owner?;
    // A watcher's job: its trigger's project.
    if let Some(id) = owner.strip_prefix("trigger:").or_else(|| owner.strip_prefix("service:")) {
        return id.split_once('/').map(|(p, _)| p.to_string());
    }
    Some(s.w.app.task(&owner).await.ok()?.project)
}

async fn job_output(s: S, _: Who, q: OutQ) -> R {
    let mut v = sup::output(&s.w.app.paths.data, &q.job, q.from, q.to, q.tail, q.pattern.as_deref())?;
    // Secret values never leave as they are.
    if let Some(p) = job_project(&s, &q.job).await {
        for key in ["text", "matches"] {
            for l in v[key].as_array_mut().into_iter().flatten() {
                if let Some(t) = l["text"].as_str() {
                    l["text"] = json!(s.w.app.mask(&p, t).await);
                }
            }
        }
    }
    Ok(Json(v))
}

/// A job's output as it comes (SSE), ending with its end.
async fn job_stream(State(s): State<S>, Path(id): Path<String>) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, E> {
    let rx = sup::client::stream(&s.w.app.paths.socket, Some(&id), None).await?;
    let project = job_project(&s, &id).await;
    let app = s.w.app.clone();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(move |e| {
        let (app, project) = (app.clone(), project.clone());
        async move {
        match e.ok()? {
            sup::Event::Output { text, .. } => {
                let text = match &project {
                    Some(p) => app.mask(p, &text).await,
                    None => text,
                };
                Some(Ok(Event::default().event("output").data(json!({"text": text}).to_string())))
            }
            sup::Event::Exit { job } => Some(Ok(Event::default().event("exit").data(json!(job).to_string()))),
            sup::Event::Backgrounded { job } => Some(Ok(Event::default().event("backgrounded").data(json!({"job": job}).to_string()))),
            _ => None,
        }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[derive(Deserialize, JsonSchema)]
struct JobRef {
    job: String,
}

async fn job_background(s: S, _: Who, j: JobRef) -> R {
    Ok(Json(json!(s.w.app.sup.background(&j.job).await?)))
}

#[derive(Deserialize, JsonSchema)]
struct Signal {
    job: String,
    /// A signal's number (default 15, TERM).
    signal: Option<i32>,
}

async fn job_kill(s: S, _: Who, k: Signal) -> R {
    s.w.app.sup.kill(&k.job, k.signal).await?;
    Ok(Json(json!({"ok": true})))
}

async fn ptys(s: S, _: Who, t: TaskRef) -> R {
    Ok(Json(json!(s.w.app.sup.ptys(Some(&t.task)).await?)))
}

#[derive(Deserialize, JsonSchema)]
struct PtyOpen {
    task: String,
    /// What it runs (default: a shell).
    cmd: Option<String>,
    /// In the project's dev shell (default true).
    devshell: Option<bool>,
    cols: Option<u16>,
    rows: Option<u16>,
}

/// The person opens a terminal in a task's working directory.
async fn pty_open(s: S, _: Who, o: PtyOpen) -> R {
    let t = s.w.app.task(&o.task).await?;
    let p = s.w.app.project(&t.project).await?;
    let cmd = reagent_tools::devshell::wrap_terminal(&p, std::path::Path::new(&t.cwd), o.cmd.as_deref(), o.devshell.unwrap_or(true));
    let m = s.w.app.sup.pty_open(sup::PtyArgs { cmd, cwd: t.cwd.clone(), env: reagent_tools::mcp::shell::env(&s.w.app, &t, &p).await, owner: Some(t.id), cols: o.cols.unwrap_or(120), rows: o.rows.unwrap_or(32) }).await?;
    Ok(Json(json!(m)))
}

#[derive(Deserialize, JsonSchema)]
struct PtyRef {
    pty: String,
}

async fn pty_close(s: S, _: Who, p: PtyRef) -> R {
    s.w.app.sup.pty_close(&p.pty).await?;
    Ok(Json(json!({"ok": true})))
}

/// A terminal, live: bytes out (base64 text frames: `{data}`), keys in
/// (`{data}`: base64) and size changes (`{cols, rows}`).
async fn pty_ws(State(s): State<S>, Path(id): Path<String>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| pty_session(s, id, socket))
}

async fn pty_session(s: S, id: String, socket: WebSocket) {
    let Ok(mut rx) = sup::client::stream(&s.w.app.paths.socket, None, Some(&id)).await else { return };
    let (mut tx, mut inbound) = socket.split();
    let out = tokio::spawn(async move {
        while let Ok(e) = rx.recv().await {
            let msg = match e {
                sup::Event::Pty { data, .. } => json!({"data": data}),
                sup::Event::PtyExit { .. } => json!({"exit": true}),
                _ => continue,
            };
            if tx.send(Message::Text(msg.to_string().into())).await.is_err() {
                return;
            }
        }
    });
    while let Some(Ok(m)) = inbound.next().await {
        let Message::Text(t) = m else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&t) else { continue };
        if let Some(d) = v["data"].as_str()
            && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(d)
        {
            let _ = s.w.app.sup.pty_send(&id, &bytes).await;
        }
        if let (Some(c), Some(r)) = (v["cols"].as_u64(), v["rows"].as_u64()) {
            let _ = s.w.app.sup.pty_resize(&id, c as u16, r as u16).await;
        }
    }
    out.abort();
}
