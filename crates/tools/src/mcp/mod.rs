//! The tools tasks have, as MCP servers at `/mcp/<name>` (and the hooks at
//! `/mcp/hooks`). Who calls comes with every call (`_meta.subnet/agent`):
//! paths, jobs and terminals are that task's.

pub mod fs;
pub mod git;
pub mod misc;
pub mod shell;

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use reagent_store::{Project, Task};
use rmcp::RoleServer;
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager};

use crate::app::App;

/// The calling task.
pub async fn caller(app: &App, ctx: &RequestContext<RoleServer>) -> Result<(Task, Project), String> {
    let id = ctx.meta.0.0.get("subnet/agent").and_then(|v| v.as_str()).ok_or("who's calling? (no subnet/agent in the request)")?;
    let t = app.task_of_agent(id).await?;
    let p = app.project(&t.project).await?;
    Ok((t, p))
}

/// `p` without `.` and `..` (lexically).
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// Resolves symlinks of the part that exists (so a link can't lead out).
fn real(p: &Path) -> PathBuf {
    let mut existing = p.to_path_buf();
    let mut rest = vec![];
    while !existing.exists() {
        match (existing.file_name().map(|f| f.to_os_string()), existing.parent()) {
            (Some(f), Some(parent)) => {
                rest.push(f);
                existing = parent.to_path_buf();
            }
            _ => return p.to_path_buf(),
        }
    }
    let mut out = std::fs::canonicalize(&existing).unwrap_or(existing);
    for f in rest.into_iter().rev() {
        out.push(f);
    }
    out
}

/// A path a task names: relative to its working directory, and inside its
/// places (its working directory, the project, the memories; skills for
/// reading), unless a rule with a matching `target` allows it.
pub async fn resolve(app: &App, t: &Task, p: &Project, tool: &str, path: &str, write: bool) -> Result<PathBuf, String> {
    let raw = Path::new(path.trim());
    let full = if raw.is_absolute() { raw.to_path_buf() } else { Path::new(&t.cwd).join(raw) };
    let full = real(&normalize(&full));
    let mut roots = vec![PathBuf::from(&t.cwd), PathBuf::from(&p.path), app.project_memory(p).dir, app.global_memory().dir];
    if !write {
        roots.extend(app.paths.skills.iter().cloned());
        roots.extend(app.skills_for(Path::new(&t.cwd), p).into_iter().map(|s| s.dir));
    }
    if roots.iter().map(|r| real(&normalize(r))).any(|r| full.starts_with(&r)) {
        return Ok(full);
    }
    let rules = app.store.rules(&p.slug).await.map_err(|e| e.to_string())?;
    let shown = full.display().to_string();
    let d = crate::policy::decide(&rules, crate::policy::Action::Deny, &crate::policy::Call { tool, command: None, target: Some(&shown) });
    match (d.action, d.rule.is_some_and(|r| r.target.is_some())) {
        (crate::policy::Action::Allow, true) => Ok(full),
        _ => Err(format!("{shown} is outside the project ({}) and the task's working directory: a rule with a target must allow it", p.path)),
    }
}

/// Only callers with the token.
async fn auth(req: Request, next: Next) -> Response {
    let want = std::env::var(crate::cluster::TOKEN_ENV).unwrap_or_default();
    let got = req.headers().get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or_default();
    if want.is_empty() || got != want {
        return (axum::http::StatusCode::UNAUTHORIZED, "a token is needed").into_response();
    }
    next.run(req).await
}

fn service<S: rmcp::ServerHandler + Clone + Send + Sync + 'static>(s: S) -> StreamableHttpService<S, LocalSessionManager> {
    StreamableHttpService::new(move || Ok(s.clone()), LocalSessionManager::default().into(), StreamableHttpServerConfig::default())
}

/// Every server, behind the token.
pub fn router(app: Arc<App>) -> axum::Router {
    axum::Router::new()
        .nest_service("/mcp/fs", service(fs::FsTools(app.clone())))
        .nest_service("/mcp/shell", service(shell::ShellTools(app.clone())))
        .nest_service("/mcp/pty", service(shell::PtyTools(app.clone())))
        .nest_service("/mcp/git", service(git::GitTools(app.clone())))
        .nest_service("/mcp/memory", service(misc::MemoryTools(app.clone())))
        .nest_service("/mcp/skills", service(misc::SkillTools(app.clone())))
        .nest_service("/mcp/tasks", service(misc::TaskTools(app.clone())))
        .nest_service("/mcp/ask", service(misc::AskTools(app.clone())))
        .nest_service("/mcp/hooks", service(crate::hooks::HookTools(app)))
        .layer(axum::middleware::from_fn(auth))
}

/// A paging note for the end of an answer.
pub fn more(note: impl std::fmt::Display) -> String {
    format!("\n[{note}]")
}
