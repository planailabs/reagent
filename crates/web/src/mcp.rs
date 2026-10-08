//! reagent as an MCP server for agents outside (`/mcp` on the web server):
//! start and manage tasks. They sign in with an API token (`Authorization:
//! Bearer <token>`, made by `reagent token add <name>` or in the settings).

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use reagent_store::Budget;
use reagent_tools::app::{App, MergeAnswer, StartTask};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager};
use rmcp::{schemars, tool, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};
use subnet_core::agent::PauseMode;

use crate::S;

#[derive(Clone)]
pub struct ReagentTools(pub Arc<App>);

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Start {
    /// A project's id (projects lists them).
    pub project: String,
    pub title: String,
    /// What it should do: everything it needs to know.
    pub prompt: String,
    pub profile: Option<String>,
    /// Skills loaded into its first message.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Limits: {tokens?, cost?, minutes?}.
    pub budget: Option<Value>,
    /// A kind of task from reagent's config (it picks the model).
    pub kind: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct List {
    pub project: Option<String>,
    /// Finished ones too.
    #[serde(default)]
    pub all: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Id {
    pub task: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Tail {
    pub task: String,
    /// The last n messages (default 20).
    pub tail: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Message {
    pub task: String,
    pub text: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Pause {
    pub task: String,
    /// quick (default: running commands finish), safe (after this turn) or hard (now).
    pub mode: Option<PauseMode>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Raise {
    pub task: String,
    /// Added to its budget.
    pub tokens: Option<u64>,
    pub cost: Option<f64>,
    pub minutes: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Switch {
    pub task: String,
    /// A profile from reagent's config (`projects` doesn't list them; task_get shows the current one).
    pub profile: String,
    pub why: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Approve {
    pub task: String,
    /// The call it waits on (task_get shows it).
    pub call_id: String,
    pub approved: bool,
    /// Also allow calls like it from now on (a rule).
    #[serde(default)]
    pub always: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Merge {
    pub task: String,
    pub merge: bool,
    /// Why not (when sending it back).
    pub message: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Wait {
    pub task: String,
    /// Seconds (default 300, at most 3600).
    pub timeout: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Search {
    pub pattern: String,
    pub task: Option<String>,
    pub project: Option<String>,
    pub page: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Design {
    pub project: String,
    /// What the task should achieve, roughly.
    pub goal: String,
    /// The answers to the designer's questions so far.
    #[serde(default)]
    pub answers: Vec<reagent_tools::design::Answer>,
    /// Propose now, without more questions.
    #[serde(default)]
    pub propose: bool,
    /// task (default), cron or trigger.
    pub target: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DesignApply {
    pub project: String,
    /// Suggestions from prompt_design's proposal, as they came (the ones to make).
    pub suggestions: Vec<Value>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DocName {
    pub name: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct TriggerAdd {
    /// The project it watches for.
    pub project: String,
    #[serde(flatten)]
    pub def: reagent_tools::triggers::Def,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct TriggerRef {
    pub project: String,
    pub name: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct TriggerMove {
    pub project: String,
    pub name: String,
    /// repo (its files into the project folder) or db (kept in reagent).
    pub to: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Project {
    pub project: Option<String>,
}

fn j(v: impl serde::Serialize) -> Result<String, String> {
    serde_json::to_string_pretty(&v).map_err(|e| e.to_string())
}

#[tool_router(server_handler)]
impl ReagentTools {
    #[tool(description = "The projects reagent works on: id, name, folder, tasks going on.")]
    async fn projects(&self) -> Result<String, String> {
        let mut out = vec![];
        for p in self.0.store.projects().await.map_err(|e| e.to_string())? {
            let active = self.0.store.tasks(Some(&p.slug), None, true, 1000).await.map_err(|e| e.to_string())?.len();
            out.push(json!({"id": p.slug, "name": p.name, "path": p.path, "active": active, "merge": p.merge}));
        }
        j(out)
    }

    #[tool(description = "Start a task in a project: it works on its own; task_get or task_wait follow it.")]
    async fn task_start(&self, Parameters(a): Parameters<Start>) -> Result<String, String> {
        let budget: Option<Budget> = a.budget.map(serde_json::from_value).transpose().map_err(|e| format!("budget: {e}"))?;
        let t = self.0.start_task(StartTask { project: a.project, title: a.title, prompt: a.prompt, profile: a.profile, budget, skills: a.skills, parent: None, origin: Some("mcp".into()), kind: a.kind }).await?;
        j(json!({"task": t.id, "state": t.state}))
    }

    #[tool(description = "Tasks (newest first): of a project or all, going on or (all = true) finished too.")]
    async fn task_list(&self, Parameters(a): Parameters<List>) -> Result<String, String> {
        let v = self.0.store.tasks(a.project.as_deref(), None, !a.all, 200).await.map_err(|e| e.to_string())?;
        j(v.iter().map(|t| json!({"task": t.id, "title": t.title, "project": t.project, "state": t.state, "waits_for": t.wait.as_ref().map(|w| w.0["kind"].clone()), "parent": t.parent})).collect::<Vec<_>>())
    }

    #[tool(description = "A task: its state, what it waits for (an approval's call, a question, a merge, its budget), its report, usage, worktree, subtasks, todo list.")]
    async fn task_get(&self, Parameters(a): Parameters<Id>) -> Result<String, String> {
        let t = self.0.task(&a.task).await?;
        let kids = self.0.store.tasks(None, Some(&a.task), false, 100).await.map_err(|e| e.to_string())?;
        let mut v = json!(t);
        v["subtasks"] = json!(kids.iter().map(|k| json!({"task": k.id, "title": k.title, "state": k.state})).collect::<Vec<_>>());
        v["todos"] = json!(self.0.store.todos(&a.task).await.map_err(|e| e.to_string())?);
        j(v)
    }

    #[tool(description = "A task's last messages (its conversation).")]
    async fn task_transcript(&self, Parameters(a): Parameters<Tail>) -> Result<String, String> {
        let t = self.0.task(&a.task).await?;
        let agent: uuid::Uuid = t.agent.as_deref().ok_or("it never started")?.parse().map_err(|e| format!("{e}"))?;
        let tr = self.0.hub()?.transcript_of(agent, true).await.map_err(|e| e.to_string())?;
        let n = a.tail.unwrap_or(20);
        let msgs: Vec<Value> = tr.messages.iter().rev().take(n).rev().map(|m| json!({"role": m.role, "content": m.content, "tool_calls": m.tool_calls, "tool_call_id": m.tool_call_id})).collect();
        j(msgs)
    }

    #[tool(description = "Send a task a message (read before its next step; a finished task goes on with it).")]
    async fn task_message(&self, Parameters(a): Parameters<Message>) -> Result<String, String> {
        self.0.message(&a.task, &format!("[from an agent outside] {}", a.text)).await?;
        Ok("sent".into())
    }

    #[tool(description = "Pause a task.")]
    async fn task_pause(&self, Parameters(a): Parameters<Pause>) -> Result<String, String> {
        self.0.pause(&a.task, a.mode.unwrap_or(PauseMode::Quick)).await?;
        Ok("paused".into())
    }

    #[tool(description = "Resume a paused task.")]
    async fn task_resume(&self, Parameters(a): Parameters<Id>) -> Result<String, String> {
        self.0.resume(&a.task).await?;
        Ok("resumed".into())
    }

    #[tool(description = "Cancel a task (and its commands and terminals).")]
    async fn task_cancel(&self, Parameters(a): Parameters<Id>) -> Result<String, String> {
        self.0.cancel(&a.task).await?;
        Ok("cancelled".into())
    }

    #[tool(description = "Retry a failed task from where it failed.")]
    async fn task_retry(&self, Parameters(a): Parameters<Id>) -> Result<String, String> {
        let t = self.0.retry(&a.task).await?;
        Ok(format!("retrying ({})", t.state))
    }

    #[tool(description = "Raise a task's budget (it goes on if it was paused over it).")]
    async fn task_raise_budget(&self, Parameters(a): Parameters<Raise>) -> Result<String, String> {
        let t = self.0.raise_budget(&a.task, &Budget { tokens: a.tokens, cost: a.cost, minutes: a.minutes, daily_cost: None }).await?;
        j(json!({"budget": t.budget.0, "state": t.state}))
    }

    #[tool(description = "Move a task onto another profile (another model): its conversation goes on there; a failed one is retried there.")]
    async fn task_switch_profile(&self, Parameters(a): Parameters<Switch>) -> Result<String, String> {
        let t = self.0.switch_profile(&a.task, &a.profile, a.why.as_deref().unwrap_or("an agent outside moved it")).await?;
        Ok(format!("{} runs on {} now", t.title, t.profile))
    }

    #[tool(description = "Approve or deny the call a task waits on.")]
    async fn task_approve(&self, Parameters(a): Parameters<Approve>) -> Result<String, String> {
        self.0.approve(&a.task, &a.call_id, a.approved, a.always).await?;
        Ok(if a.approved { "approved" } else { "denied" }.into())
    }

    #[tool(description = "Answer a task's question.")]
    async fn task_answer(&self, Parameters(a): Parameters<Message>) -> Result<String, String> {
        self.0.answer(&a.task, &a.text).await?;
        Ok("answered".into())
    }

    #[tool(description = "Decide a merge a task waits for: merge, or send it back with a message.")]
    async fn task_merge(&self, Parameters(a): Parameters<Merge>) -> Result<String, String> {
        let ans = if a.merge { MergeAnswer::Merge } else { MergeAnswer::Reject(a.message.unwrap_or_else(|| "not now".into())) };
        self.0.decide_merge(&a.task, ans).await?;
        Ok("decided".into())
    }

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

    #[tool(description = "Design a task's prompt before task_start: give a rough goal; it asks questions (answer them and call again with `answers`) or proposes a title, prompt, skills, kind, profile and budget.")]
    async fn prompt_design(&self, Parameters(a): Parameters<Design>) -> Result<String, String> {
        let target = a.target.as_deref().unwrap_or("task");
        if !matches!(target, "task" | "cron" | "trigger") {
            return Err("target: task, cron or trigger".into());
        }
        let step = self.0.design(&a.project, target, &a.goal, &a.answers, a.propose).await?;
        j(step)
    }

    #[tool(description = "Make suggestions from a prompt_design proposal: a repo skill, a cron entry, a trigger (pass the ones you want, as they came).")]
    async fn design_apply(&self, Parameters(a): Parameters<DesignApply>) -> Result<String, String> {
        let p = self.0.project(&a.project).await?;
        let mut out = vec![];
        for v in a.suggestions {
            let line = match serde_json::from_value::<reagent_tools::design::Suggestion>(v) {
                Ok(s) => reagent_tools::design::apply(&self.0.store, &p, &s).await.unwrap_or_else(|e| format!("not made: {e}")),
                Err(e) => format!("not a suggestion: {e}"),
            };
            out.push(line);
        }
        Ok(out.join("\n"))
    }

    #[tool(description = "reagent's documentation (its system skills): how tasks, subtasks, policy, triggers, memory and the rest work.")]
    async fn docs_list(&self) -> Result<String, String> {
        Ok(reagent_tools::skills::system().into_iter().map(|(n, d, _)| format!("{n} — {d}")).collect::<Vec<_>>().join("\n"))
    }

    #[tool(description = "One of reagent's docs (docs_list names them).")]
    async fn docs_read(&self, Parameters(a): Parameters<DocName>) -> Result<String, String> {
        reagent_tools::skills::system_text(&a.name).map(|t| reagent_tools::skills::parse(&t).2).ok_or_else(|| format!("no doc {:?} (docs_list names them)", a.name))
    }

    #[tool(description = "Triggers (of a project, or all): scripts that watch something and start tasks or message running ones.")]
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

    #[tool(description = "Remove a trigger (a repo trigger's files too).")]
    async fn trigger_remove(&self, Parameters(a): Parameters<TriggerRef>) -> Result<String, String> {
        reagent_tools::triggers::remove(&self.0, &a.project, &a.name).await?;
        Ok("removed".into())
    }

    #[tool(description = "Run a poll trigger now (or restart a watcher).")]
    async fn trigger_run(&self, Parameters(a): Parameters<TriggerRef>) -> Result<String, String> {
        reagent_tools::triggers::run_now(&self.0, &a.project, &a.name).await
    }

    #[tool(description = "Move a trigger into the project's repo (.agents/triggers/<name>/) or back into reagent.")]
    async fn trigger_move(&self, Parameters(a): Parameters<TriggerMove>) -> Result<String, String> {
        let p = self.0.project(&a.project).await?;
        let t = reagent_tools::triggers::move_to(&self.0.store, &p, &a.name, &a.to).await?;
        self.0.emit_trigger(&p.slug, &t.name).await;
        Ok(format!("{} is {} now", t.name, if t.source == "repo" { "in the repo" } else { "kept in reagent" }))
    }

    #[tool(description = "Search tasks' whole conversations for a regex: one task's (paged), or every task's (of a project), a few hits each.")]
    async fn search(&self, Parameters(a): Parameters<Search>) -> Result<String, String> {
        j(self.0.search(&a.pattern, a.task.as_deref(), a.project.as_deref(), None, a.page.unwrap_or(1)).await?)
    }
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

pub fn service(app: Arc<App>) -> StreamableHttpService<ReagentTools, LocalSessionManager> {
    // Callers carry a token, so any host may (reagent sits behind a proxy with its own name).
    // Longer than its longest call (task_wait: an hour), still ending sessions a proxy silently dropped.
    let sessions = reagent_tools::mcp::sessions(Some(std::time::Duration::from_secs(6 * 3600)));
    StreamableHttpService::new(move || Ok(ReagentTools(app.clone())), sessions.into(), StreamableHttpServerConfig::default().disable_allowed_hosts())
}
