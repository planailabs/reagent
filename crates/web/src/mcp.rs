//! reagent as an MCP server for agents outside (`/mcp` on the web server):
//! start and manage tasks. They sign in with an API token (`Authorization:
//! Bearer <token>`, made by `reagent token add <name>` or in the settings).
//! Its tools are the web API's endpoints (`api::endpoints`, the same handlers
//! and inputs) and the few below that only agents need.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use reagent_tools::app::App;
use rmcp::handler::server::router::tool::{ToolRoute, ToolRouter};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::{CallToolResult, ContentBlock, Tool, ToolAnnotations};
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager};
use rmcp::{schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::S;
use crate::endpoints::{Endpoint, Endpoints, Who};

/// The app, the web server's state (for the endpoints) and every tool.
#[derive(Clone)]
pub struct ReagentTools(pub Arc<App>, S, ToolRouter<ReagentTools>);

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Wait {
    pub task: String,
    /// Seconds (default 300, at most 3600).
    pub timeout: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct TriggerAdd {
    /// The project it watches for.
    pub project: String,
    #[serde(flatten)]
    pub def: reagent_tools::triggers::Def,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ServiceAdd {
    /// The project it belongs to.
    pub project: String,
    #[serde(flatten)]
    pub def: reagent_tools::services::Def,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ServiceLogs {
    pub project: String,
    pub name: String,
    /// How many of its last lines (default 100).
    pub lines: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SourceLs {
    /// A folder of the repository (default: the top), like crates/tools/src.
    #[serde(default)]
    pub path: String,
    /// Every file under it, not only its entries.
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SourceRead {
    /// A file of the repository, like crates/tools/src/app.rs or DESIGN.md.
    pub path: String,
    /// The first line (default 1).
    pub from: Option<usize>,
    /// The last line (default: 400 lines on).
    pub to: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SyncSet {
    pub project: String,
    /// A persistent worktree (none: the project folder).
    pub worktree: Option<String>,
    pub enabled: Option<bool>,
    /// Between rounds, like 15m (at least a minute).
    pub every: Option<String>,
    /// rebase, merge or ff-only.
    pub strategy: Option<String>,
    /// The remote ("": the branch's upstream, else origin).
    pub remote: Option<String>,
    /// The remote's branch ("": the upstream's, else the same name).
    pub branch: Option<String>,
    /// For the task that resolves a conflict or decides about uncommitted changes.
    pub notes: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Project {
    pub project: Option<String>,
}

fn j(v: impl serde::Serialize) -> Result<String, String> {
    serde_json::to_string_pretty(&v).map_err(|e| e.to_string())
}

#[tool_router]
impl ReagentTools {
    #[tool(description = "Wait until a task ends or waits for someone (or the timeout): its state, what it waits for, its report.")]
    async fn task_wait(&self, Parameters(a): Parameters<Wait>) -> Result<String, String> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(a.timeout.unwrap_or(300).clamp(1, 3600));
        let mut rx = self.0.events.subscribe();
        loop {
            let t = self.0.task(&a.task).await?;
            if !matches!(t.state.as_str(), "running") {
                return j(json!({"task": t.id, "state": t.state, "waits_for": t.wait.map(|w| w.0), "report": t.report}));
            }
            if tokio::time::timeout_at(deadline, rx.recv()).await.is_err() {
                return j(json!({"task": t.id, "state": t.state, "timed_out": true}));
            }
        }
    }

    #[tool(description = "Triggers (of a project, or all), a line each: scripts that watch something and start tasks or message running ones.")]
    async fn trigger_list(&self, Parameters(a): Parameters<Project>) -> Result<String, String> {
        let v = self.0.store.triggers(a.project.as_deref()).await.map_err(|e| e.to_string())?;
        Ok(if v.is_empty() { "no triggers".into() } else { v.iter().map(|t| format!("{}: {}", t.project, reagent_tools::triggers::line(t))).collect::<Vec<_>>().join("\n") })
    }

    #[tool(description = "Add (or replace) a trigger in a project: a script that prints one JSON line per event; each new event starts a task from the title and prompt templates, or messages a running one. The project's policy decides whether its script runs (it may ask the person).")]
    async fn trigger_add(&self, Parameters(a): Parameters<TriggerAdd>) -> Result<String, String> {
        let p = self.0.project(&a.project).await?;
        let t = reagent_tools::triggers::save(&self.0.store, &p, a.def.into_trigger(&p.slug, "mcp")?, false).await?;
        self.0.emit_trigger(&p.slug, &t.name).await;
        Ok(format!("added: {}{}", reagent_tools::triggers::line(&t), if t.mode == "webhook" { format!("; its URL: <reagent>/hook/{}/{}", p.slug, t.name) } else { String::new() }))
    }

    #[tool(description = "Change some of a branch's git sync settings (kept in reagent; the checkout's .agents/sync.md overrides what it sets). On, reagent pulls with the strategy and pushes every interval; a conflict, or uncommitted changes no task works on, start a task to sort them out, then reagent pushes.")]
    async fn sync_set(&self, Parameters(a): Parameters<SyncSet>) -> Result<String, String> {
        let p = self.0.project(&a.project).await?;
        let place = a.worktree.unwrap_or_default();
        let mut v = self.0.store.git_sync(&p.slug, &place).await.map_err(|e| e.to_string())?.unwrap_or_else(|| reagent_store::GitSync::new(&p.slug, &place));
        if let Some(x) = a.enabled {
            v.enabled = x;
        }
        if let Some(x) = a.every {
            v.every = reagent_tools::triggers::parse_every(&x)?;
        }
        if let Some(x) = a.strategy {
            v.strategy = x;
        }
        if let Some(x) = a.remote {
            v.remote = Some(x);
        }
        if let Some(x) = a.branch {
            v.branch = Some(x);
        }
        if let Some(x) = a.notes {
            v.notes = x;
        }
        let v = reagent_tools::gitsync::put(&self.0.store, &p, v).await?;
        Ok(format!("{}, every {}m, {}", if v.enabled { "on" } else { "off" }, v.every / 60, v.strategy))
    }

    #[tool(description = "Services (of a project, or all), a line each: commands that should always run, each with its state (starting, up, stopping, backoff, waiting for approval, stopped), since when, failures and last error.")]
    async fn service_list(&self, Parameters(a): Parameters<Project>) -> Result<String, String> {
        let v = self.0.store.services(a.project.as_deref()).await.map_err(|e| e.to_string())?;
        Ok(if v.is_empty() { "no services".into() } else { v.iter().map(|x| format!("{}: {}", x.project, reagent_tools::services::line(x))).collect::<Vec<_>>().join("\n") })
    }

    #[tool(description = "Add (or replace) a service of a project: a command that should always run. reagent starts it, checks it (health), restarts it with backoff, and after 5 failures in a row starts a task to repair it. The project's policy decides whether its command runs (it may ask the person).")]
    async fn service_add(&self, Parameters(a): Parameters<ServiceAdd>) -> Result<String, String> {
        let p = self.0.project(&a.project).await?;
        let v = reagent_tools::services::save(&self.0.store, &p, a.def.into_service(&p.slug, "mcp")?, false).await?;
        self.0.emit_service(&p.slug, &v.name).await;
        Ok(format!("added: {}", reagent_tools::services::line(&v)))
    }

    #[tool(description = "The end of a service's output.")]
    async fn service_logs(&self, Parameters(a): Parameters<ServiceLogs>) -> Result<String, String> {
        reagent_tools::services::logs(&self.0, &a.project, &a.name, a.lines.unwrap_or(100)).await
    }

    #[tool(description = "List reagent's own source (as this running build has it): a folder's entries, or every file under it.")]
    async fn reagent_source_ls(&self, Parameters(a): Parameters<SourceLs>) -> Result<String, String> {
        reagent_tools::source::ls(&a.path, a.recursive)
    }

    #[tool(description = "Read a file of reagent's own source (as this running build has it), numbered lines, 400 at a time.")]
    async fn reagent_source_read(&self, Parameters(a): Parameters<SourceRead>) -> Result<String, String> {
        reagent_tools::source::read(&a.path, a.from, a.to)
    }
}

#[tool_handler(router = self.2)]
impl rmcp::ServerHandler for ReagentTools {}

/// An endpoint as a tool: its input schema, its answer as JSON text.
fn route(ep: Arc<Endpoint>) -> ToolRoute<ReagentTools> {
    let annotations = ToolAnnotations::new().read_only(ep.method == Method::GET).destructive(ep.method == Method::DELETE);
    let tool = Tool::new(ep.tool, ep.description, ep.schema.clone()).annotate(annotations);
    ToolRoute::new_dyn(tool, move |ctx: ToolCallContext<'_, ReagentTools>| {
        let (ep, s) = (ep.clone(), ctx.service.1.clone());
        let input = Value::Object(ctx.arguments.unwrap_or_default());
        Box::pin(async move {
            Ok(match ep.call(s, Who::Agent, input).await {
                Ok(v) => CallToolResult::success(vec![ContentBlock::text(serde_json::to_string_pretty(&v).unwrap_or_default())]),
                Err(e) => CallToolResult::error(vec![ContentBlock::text(e.1)]),
            }
            .into())
        })
    })
}

/// Only callers with an API token.
pub async fn auth(State(s): State<S>, req: Request, next: Next) -> Response {
    let tok = req.headers().get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or_default().to_string();
    let who = if tok.is_empty() { None } else { s.w.app.store.api_token(&crate::auth::token_hash(&tok)).await.ok().flatten() };
    if who.is_none() {
        return (StatusCode::UNAUTHORIZED, "an API token is needed (reagent token add <name>)").into_response();
    }
    next.run(req).await
}

pub fn service(s: S, eps: &Endpoints) -> StreamableHttpService<ReagentTools, LocalSessionManager> {
    let mut router = ReagentTools::tool_router();
    for ep in &eps.0 {
        assert!(!router.has_route(ep.tool), "the MCP API has its own {}", ep.tool);
        router.add_route(route(ep.clone()));
    }
    let tools = ReagentTools(s.w.app.clone(), s, router);
    // Callers carry a token, so any host may (reagent sits behind a proxy with its own name).
    // Longer than its longest call (task_wait: an hour), still ending sessions a proxy silently dropped.
    let sessions = reagent_tools::mcp::sessions(Some(std::time::Duration::from_secs(6 * 3600)));
    StreamableHttpService::new(move || Ok(tools.clone()), sessions.into(), StreamableHttpServerConfig::default().disable_allowed_hosts())
}
