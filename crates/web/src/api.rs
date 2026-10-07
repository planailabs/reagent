//! The JSON API the web UI uses.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use base64::Engine;
use futures::{SinkExt, StreamExt};
use reagent_store::{Budget, Cron, Project, Rule};
use reagent_supervisor as sup;
use reagent_tools::app::{MergeAnswer, StartTask};
use serde::Deserialize;
use serde_json::{Value, json};
use subnet_core::agent::PauseMode;

use crate::{COOKIE, S, auth, client_ip, session_of};

/// An error as `{error}` with a status.
pub struct E(StatusCode, String);

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

type R<T = Value> = Result<Json<T>, E>;

fn db<T>(r: Result<T, impl std::fmt::Display>) -> Result<T, E> {
    r.map_err(|e| E(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub fn routes() -> Router<S> {
    Router::new()
        .route("/api/session", get(session))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/config", get(config))
        .route("/api/events", get(events))
        .route("/api/inbox", get(inbox))
        .route("/api/notifications", get(notifications))
        .route("/api/notifications/seen", post(seen))
        .route("/api/push/key", get(push_key))
        .route("/api/push/subscribe", post(push_subscribe))
        .route("/api/push/unsubscribe", post(push_unsubscribe))
        .route("/api/projects", get(projects))
        .route("/api/projects/{slug}", put(put_project).delete(remove_project).get(project))
        .route("/api/projects/{slug}/rules", get(rules).put(set_rules))
        .route("/api/projects/{slug}/skills", get(skills))
        .route("/api/projects/{slug}/cron", get(crons))
        .route("/api/memory", get(memory_read).put(memory_write).delete(memory_remove))
        .route("/api/cron", post(put_cron))
        .route("/api/cron/{id}", delete(remove_cron))
        .route("/api/cron/{id}/run", post(run_cron))
        .route("/api/tasks", get(tasks).post(start_task))
        .route("/api/tasks/{id}", get(task))
        .route("/api/tasks/{id}/transcript", get(transcript))
        .route("/api/tasks/{id}/message", post(message))
        .route("/api/tasks/{id}/pause", post(pause))
        .route("/api/tasks/{id}/resume", post(resume))
        .route("/api/tasks/{id}/cancel", post(cancel))
        .route("/api/tasks/{id}/retry", post(retry))
        .route("/api/tasks/{id}/limits", post(limits))
        .route("/api/tasks/{id}/raise", post(raise))
        .route("/api/tasks/{id}/approve", post(approve))
        .route("/api/tasks/{id}/answer", post(answer))
        .route("/api/tasks/{id}/merge", post(merge))
        .route("/api/tasks/{id}/diff", get(diff))
        .route("/api/tasks/{id}/jobs", get(jobs))
        .route("/api/tasks/{id}/ptys", get(ptys).post(pty_open))
        .route("/api/search", get(search))
        .route("/api/jobs/{id}/output", get(job_output))
        .route("/api/jobs/{id}/stream", get(job_stream))
        .route("/api/jobs/{id}/background", post(job_background))
        .route("/api/jobs/{id}/kill", post(job_kill))
        .route("/api/ptys/{id}", get(pty_ws).delete(pty_close))
        .route("/api/mcp", get(mcp_servers))
        .route("/api/mcp/{name}", put(put_mcp).delete(remove_mcp))
        .route("/api/secrets", get(secrets).put(set_secret).delete(remove_secret))
        .route("/api/tokens", get(tokens).post(add_token))
        .route("/api/tokens/{name}", delete(revoke_token))
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

async fn config(State(s): State<S>) -> R {
    let c = &s.w.app.config;
    let profiles: Vec<Value> = c.profile.iter().map(|(n, p)| json!({"name": n, "model": p.model, "provider": p.provider, "price": p.price, "context": p.context})).collect();
    Ok(Json(json!({"profiles": profiles, "default_profile": c.default_profile, "grep_results": c.grep_results, "notify": {"apprise": !c.notify.apprise_urls().is_empty(), "events": c.notify.events, "url": c.notify.url}})))
}

/// Live: task changes, notifications, agent events, job ends.
async fn events(State(s): State<S>) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let rx = s.w.app.events.subscribe();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|e| async move { e.ok().map(|v| Ok(Event::default().data(v.to_string()))) });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Everything waiting for the person.
async fn inbox(State(s): State<S>) -> R {
    let tasks = db(s.w.app.store.tasks(None, None, true, 1000).await)?;
    let waiting: Vec<_> = tasks.into_iter().filter(|t| t.state == "waiting" || t.state == "failed").collect();
    let failed = db(s.w.app.store.tasks(None, None, false, 200).await)?.into_iter().filter(|t| t.state == "failed").collect::<Vec<_>>();
    let unseen: Vec<_> = db(s.w.app.store.notifications(100).await)?.into_iter().filter(|n| !n.seen).collect();
    Ok(Json(json!({"waiting": waiting, "failed": failed, "notifications": unseen})))
}

async fn notifications(State(s): State<S>) -> R {
    Ok(Json(json!(db(s.w.app.store.notifications(200).await)?)))
}

#[derive(Deserialize)]
struct Upto {
    upto: i64,
}

async fn seen(State(s): State<S>, Json(u): Json<Upto>) -> R {
    db(s.w.app.store.mark_seen(u.upto).await)?;
    Ok(Json(json!({"ok": true})))
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

async fn projects(State(s): State<S>) -> R {
    let mut out = vec![];
    for p in db(s.w.app.store.projects().await)? {
        let active = db(s.w.app.store.tasks(Some(&p.slug), None, true, 1000).await)?.len();
        let mut v = json!(p);
        v["active"] = json!(active);
        out.push(v);
    }
    Ok(Json(json!(out)))
}

async fn project(State(s): State<S>, Path(slug): Path<String>) -> R {
    Ok(Json(json!(s.w.app.project(&slug).await?)))
}

async fn put_project(State(s): State<S>, Path(slug): Path<String>, Json(mut p): Json<Value>) -> R {
    p["slug"] = json!(slug);
    let base = serde_json::to_value(Project::new(&slug, &slug, "")).unwrap();
    let mut merged = match db(s.w.app.store.project(&slug).await)? {
        Some(old) => serde_json::to_value(old).unwrap(),
        None => base,
    };
    for (k, v) in p.as_object().cloned().unwrap_or_default() {
        merged[k] = v;
    }
    let p: Project = serde_json::from_value(merged).map_err(|e| E(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(json!(s.w.app.put_project(p).await?)))
}

async fn remove_project(State(s): State<S>, Path(slug): Path<String>) -> R {
    if !db(s.w.app.store.tasks(Some(&slug), None, true, 1).await)?.is_empty() {
        return Err(E(StatusCode::CONFLICT, "it has tasks going on: cancel them first".into()));
    }
    Ok(Json(json!({"removed": db(s.w.app.store.remove_project(&slug).await)?})))
}

async fn rules(State(s): State<S>, Path(slug): Path<String>) -> R {
    Ok(Json(json!(db(s.w.app.store.rules(&slug).await)?)))
}

async fn set_rules(State(s): State<S>, Path(slug): Path<String>, Json(rules): Json<Vec<Rule>>) -> R {
    s.w.app.project(&slug).await?;
    for r in &rules {
        reagent_tools::policy::Action::parse(&r.action)?;
        if r.tool.trim().is_empty() {
            return Err(E(StatusCode::BAD_REQUEST, "a rule needs a tool pattern".into()));
        }
    }
    db(s.w.app.store.set_rules(&slug, &rules).await)?;
    Ok(Json(json!(db(s.w.app.store.rules(&slug).await)?)))
}

async fn skills(State(s): State<S>, Path(slug): Path<String>) -> R {
    let p = s.w.app.project(&slug).await?;
    let list = s.w.app.skills_for(std::path::Path::new(&p.path), &p);
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

#[derive(Deserialize)]
struct MemQ {
    /// `global` or a project's id.
    scope: String,
    file: Option<String>,
}

async fn memory_of(s: &S, scope: &str) -> Result<reagent_tools::memory::Memory, E> {
    Ok(if scope == "global" { s.w.app.global_memory() } else { s.w.app.project_memory(&s.w.app.project(scope).await?) })
}

async fn memory_read(State(s): State<S>, Query(q): Query<MemQ>) -> R {
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

#[derive(Deserialize)]
struct MemW {
    scope: String,
    file: String,
    text: String,
    about: String,
}

async fn memory_write(State(s): State<S>, Json(w): Json<MemW>) -> R {
    Ok(Json(json!({"ok": memory_of(&s, &w.scope).await?.write(&w.file, &w.text, &w.about)?})))
}

async fn memory_remove(State(s): State<S>, Query(q): Query<MemQ>) -> R {
    let f = q.file.ok_or_else(|| E(StatusCode::BAD_REQUEST, "which file?".into()))?;
    Ok(Json(json!({"ok": memory_of(&s, &q.scope).await?.remove(&f)?})))
}

// --- cron -----------------------------------------------------------------

async fn crons(State(s): State<S>, Path(slug): Path<String>) -> R {
    Ok(Json(json!(db(s.w.app.store.crons(Some(&slug)).await)?)))
}

async fn put_cron(State(s): State<S>, Json(mut c): Json<Cron>) -> R {
    s.w.app.project(&c.project).await?;
    if c.id != 0 {
        let old = db(s.w.app.store.cron(c.id).await)?.ok_or_else(|| E(StatusCode::NOT_FOUND, "no such cron entry".into()))?;
        c.last_run = old.last_run;
    }
    reagent_tools::cron::prepare(&mut c, chrono::Utc::now().timestamp())?;
    let id = db(s.w.app.store.put_cron(&c).await)?;
    Ok(Json(json!(db(s.w.app.store.cron(id).await)?)))
}

async fn remove_cron(State(s): State<S>, Path(id): Path<i64>) -> R {
    Ok(Json(json!({"removed": db(s.w.app.store.remove_cron(id).await)?})))
}

async fn run_cron(State(s): State<S>, Path(id): Path<i64>) -> R {
    let c = db(s.w.app.store.cron(id).await)?.ok_or_else(|| E(StatusCode::NOT_FOUND, "no such cron entry".into()))?;
    Ok(Json(json!(reagent_tools::cron::run(&s.w.app, &c).await?)))
}

// --- tasks ----------------------------------------------------------------

#[derive(Deserialize)]
struct TasksQ {
    project: Option<String>,
    parent: Option<String>,
    #[serde(default)]
    active: bool,
    limit: Option<i64>,
}

async fn tasks(State(s): State<S>, Query(q): Query<TasksQ>) -> R {
    Ok(Json(json!(db(s.w.app.store.tasks(q.project.as_deref(), q.parent.as_deref(), q.active, q.limit.unwrap_or(200)).await)?)))
}

async fn start_task(State(s): State<S>, Json(mut t): Json<StartTask>) -> R {
    t.origin = Some("ui".into());
    t.parent = None;
    Ok(Json(json!(s.w.app.start_task(t).await?)))
}

async fn task(State(s): State<S>, Path(id): Path<String>) -> R {
    let t = s.w.app.task(&id).await?;
    let kids = db(s.w.app.store.tasks(None, Some(&id), false, 100).await)?;
    let mut v = json!(t);
    v["subtasks"] = json!(kids);
    v["waits_for_merge"] = json!(s.w.app.waits_for_merge(&id).await);
    v["todos"] = json!(db(s.w.app.store.todos(&id).await)?);
    Ok(Json(v))
}

#[derive(Deserialize)]
struct Full {
    #[serde(default)]
    full: bool,
}

async fn transcript(State(s): State<S>, Path(id): Path<String>, Query(f): Query<Full>) -> R {
    let t = s.w.app.task(&id).await?;
    let agent: uuid::Uuid = t.agent.as_deref().ok_or_else(|| E(StatusCode::NOT_FOUND, "it never started".into()))?.parse().map_err(|e| E(StatusCode::BAD_REQUEST, format!("{e}")))?;
    let tr = s.w.app.hub()?.transcript_of(agent, f.full).await.map_err(|e| E(StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(json!(tr)))
}

#[derive(Deserialize)]
struct Text {
    text: String,
}

async fn message(State(s): State<S>, Path(id): Path<String>, Json(m): Json<Text>) -> R {
    s.w.app.message(&id, &format!("[from the person] {}", m.text)).await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct Pause {
    mode: Option<PauseMode>,
}

async fn pause(State(s): State<S>, Path(id): Path<String>, Json(p): Json<Pause>) -> R {
    s.w.app.pause(&id, p.mode.unwrap_or(PauseMode::Quick)).await?;
    Ok(Json(json!({"ok": true})))
}

async fn resume(State(s): State<S>, Path(id): Path<String>) -> R {
    s.w.app.resume(&id).await?;
    Ok(Json(json!({"ok": true})))
}

async fn cancel(State(s): State<S>, Path(id): Path<String>) -> R {
    s.w.app.cancel(&id).await?;
    Ok(Json(json!({"ok": true})))
}

async fn retry(State(s): State<S>, Path(id): Path<String>) -> R {
    Ok(Json(json!(s.w.app.retry(&id).await?)))
}

#[derive(Deserialize)]
struct Limits {
    profile: Option<String>,
    budget: Option<Budget>,
}

async fn limits(State(s): State<S>, Path(id): Path<String>, Json(l): Json<Limits>) -> R {
    Ok(Json(json!(s.w.app.set_limits(&id, l.profile.as_deref(), l.budget).await?)))
}

async fn raise(State(s): State<S>, Path(id): Path<String>, Json(b): Json<Budget>) -> R {
    Ok(Json(json!(s.w.app.raise_budget(&id, &b).await?)))
}

#[derive(Deserialize)]
struct Approve {
    call_id: String,
    approved: bool,
    #[serde(default)]
    always: bool,
}

async fn approve(State(s): State<S>, Path(id): Path<String>, Json(a): Json<Approve>) -> R {
    s.w.app.approve(&id, &a.call_id, a.approved, a.always).await?;
    Ok(Json(json!({"ok": true})))
}

async fn answer(State(s): State<S>, Path(id): Path<String>, Json(a): Json<Text>) -> R {
    s.w.app.answer(&id, &a.text).await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct MergeQ {
    merge: bool,
    #[serde(default)]
    message: String,
}

async fn merge(State(s): State<S>, Path(id): Path<String>, Json(m): Json<MergeQ>) -> R {
    s.w.app.decide_merge(&id, if m.merge { MergeAnswer::Merge } else { MergeAnswer::Reject(if m.message.is_empty() { "not now".into() } else { m.message }) }).await?;
    Ok(Json(json!({"ok": true})))
}

async fn diff(State(s): State<S>, Path(id): Path<String>) -> R {
    let t = s.w.app.task(&id).await?;
    let w = t.worktree.map(|w| w.0).ok_or_else(|| E(StatusCode::NOT_FOUND, "it doesn't work in a worktree".into()))?;
    let dir = std::path::Path::new(&w.path);
    Ok(Json(json!({
        "branch": w.branch, "base": w.base, "path": w.path,
        "stat": reagent_tools::git::diff(dir, &w.base, true)?,
        "diff": reagent_tools::git::diff(dir, &w.base, false)?,
        "status": reagent_tools::git::status(dir)?,
    })))
}

#[derive(Deserialize)]
struct SearchQ {
    q: String,
    task: Option<String>,
    project: Option<String>,
    page: Option<u32>,
}

/// Across tasks' conversations.
async fn search(State(s): State<S>, Query(q): Query<SearchQ>) -> R {
    Ok(Json(s.w.app.search(&q.q, q.task.as_deref(), q.project.as_deref(), None, q.page.unwrap_or(1)).await?))
}

// --- added MCP servers ----------------------------------------------------

#[derive(Deserialize)]
struct McpQ {
    /// A project's servers; `global` the ones every task gets; none: all.
    project: Option<String>,
}

async fn mcp_servers(State(s): State<S>, Query(q): Query<McpQ>) -> R {
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

/// Adds or changes a server and applies it: the answer says whether it runs.
async fn put_mcp(State(s): State<S>, Path(name): Path<String>, Json(mut m): Json<Value>) -> R {
    m["name"] = json!(name);
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

async fn remove_mcp(State(s): State<S>, Path(name): Path<String>) -> R {
    let gone = db(s.w.app.store.remove_mcp_server(&name).await)?;
    if gone {
        s.w.app.apply_cluster().await?;
    }
    Ok(Json(json!({"removed": gone})))
}

// --- secrets ----------------------------------------------------------------

#[derive(Deserialize)]
struct SecretQ {
    /// A project's own; none or `global`: every project's.
    project: Option<String>,
    name: Option<String>,
}

fn scope(p: &Option<String>) -> Option<&str> {
    p.as_deref().filter(|p| *p != "global" && !p.is_empty())
}

/// The person sees them all, values too.
async fn secrets(State(s): State<S>, Query(q): Query<SecretQ>) -> R {
    if let Some(p) = scope(&q.project) {
        s.w.app.project(p).await?;
    }
    Ok(Json(json!(s.w.app.store.secrets(scope(&q.project)).await?)))
}

#[derive(Deserialize)]
struct SetSecret {
    project: Option<String>,
    name: String,
    value: String,
}

async fn set_secret(State(s): State<S>, Json(b): Json<SetSecret>) -> R {
    if let Some(p) = scope(&b.project) {
        s.w.app.project(p).await?;
    }
    s.w.app.store.set_secret(scope(&b.project), b.name.trim(), &b.value).await?;
    Ok(Json(json!({"ok": true})))
}

async fn remove_secret(State(s): State<S>, Query(q): Query<SecretQ>) -> R {
    let name = q.name.as_deref().ok_or_else(|| E(StatusCode::BAD_REQUEST, "which secret?".into()))?;
    Ok(Json(json!({"removed": db(s.w.app.store.remove_secret(scope(&q.project), name).await)?})))
}

// --- API tokens (the MCP API) ---------------------------------------------

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

async fn jobs(State(s): State<S>, Path(id): Path<String>) -> R {
    Ok(Json(json!(s.w.app.sup.jobs(Some(&id)).await?)))
}

#[derive(Deserialize)]
struct OutQ {
    from: Option<usize>,
    to: Option<usize>,
    tail: Option<usize>,
    pattern: Option<String>,
}

/// The project whose secrets a job's output is masked with.
async fn job_project(s: &S, job: &str) -> Option<String> {
    let owner = s.w.app.sup.job(job).await.ok()?.owner?;
    Some(s.w.app.task(&owner).await.ok()?.project)
}

async fn job_output(State(s): State<S>, Path(id): Path<String>, Query(q): Query<OutQ>) -> R {
    let mut v = sup::output(&s.w.app.paths.data, &id, q.from, q.to, q.tail, q.pattern.as_deref())?;
    // Secret values never leave as they are.
    if let Some(p) = job_project(&s, &id).await {
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

async fn job_background(State(s): State<S>, Path(id): Path<String>) -> R {
    Ok(Json(json!(s.w.app.sup.background(&id).await?)))
}

#[derive(Deserialize)]
struct Signal {
    signal: Option<i32>,
}

async fn job_kill(State(s): State<S>, Path(id): Path<String>, Json(k): Json<Signal>) -> R {
    s.w.app.sup.kill(&id, k.signal).await?;
    Ok(Json(json!({"ok": true})))
}

async fn ptys(State(s): State<S>, Path(id): Path<String>) -> R {
    Ok(Json(json!(s.w.app.sup.ptys(Some(&id)).await?)))
}

#[derive(Deserialize)]
struct PtyOpen {
    cmd: Option<String>,
    devshell: Option<bool>,
    cols: Option<u16>,
    rows: Option<u16>,
}

/// The person opens a terminal in a task's working directory.
async fn pty_open(State(s): State<S>, Path(id): Path<String>, Json(o): Json<PtyOpen>) -> R {
    let t = s.w.app.task(&id).await?;
    let p = s.w.app.project(&t.project).await?;
    let cmd = reagent_tools::devshell::wrap_terminal(&p, std::path::Path::new(&t.cwd), o.cmd.as_deref(), o.devshell.unwrap_or(true));
    let m = s.w.app.sup.pty_open(sup::PtyArgs { cmd, cwd: t.cwd.clone(), env: reagent_tools::mcp::shell::env(&s.w.app, &t, &p).await, owner: Some(t.id), cols: o.cols.unwrap_or(120), rows: o.rows.unwrap_or(32) }).await?;
    Ok(Json(json!(m)))
}

async fn pty_close(State(s): State<S>, Path(id): Path<String>) -> R {
    s.w.app.sup.pty_close(&id).await?;
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
