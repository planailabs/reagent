//! A task's triggers tools: scripts that watch something and start tasks
//! (or message running ones) when it happens.

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::service::RequestContext;
use rmcp::{RoleServer, schemars, tool, tool_router};
use serde::Deserialize;

use super::caller;
use crate::app::App;
use crate::triggers::{self, Def};

#[derive(Clone)]
pub struct TriggerTools(pub Arc<App>);

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Name {
    pub name: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Move {
    pub name: String,
    /// repo (into .agents/triggers/<name>/ in the project folder) or db (into reagent).
    pub to: String,
}

#[tool_router(server_handler)]
impl TriggerTools {
    #[tool(description = "This project's triggers: what each watches, how, and how it's doing.")]
    async fn trigger_list(&self, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (_, p) = caller(&self.0, &ctx).await?;
        let v = self.0.store.triggers(Some(&p.slug)).await.map_err(|e| e.to_string())?;
        Ok(if v.is_empty() { "no triggers".into() } else { v.iter().map(triggers::line).collect::<Vec<_>>().join("\n") })
    }

    #[tool(description = "Add (or replace) a trigger in this project: a script that watches something (a CI pipeline, a queue) and prints an event per line; each new event starts a task from the title and prompt templates, or messages a running one. Whether its script runs is the project's policy's call (triggers.run), which may ask the person.")]
    async fn trigger_add(&self, Parameters(d): Parameters<Def>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let tr = triggers::save(&self.0.store, &p, d.into_trigger(&p.slug, &format!("task:{}", t.id))?, false).await?;
        self.0.emit_trigger(&p.slug, &tr.name).await;
        let hook = if tr.mode == "webhook" { format!("; its URL: <reagent>/hook/{}/{}", p.slug, tr.name) } else { String::new() };
        Ok(format!("added: {}{hook}", triggers::line(&tr)))
    }

    #[tool(description = "Remove one of this project's triggers (a repo trigger's files go too).")]
    async fn trigger_remove(&self, Parameters(a): Parameters<Name>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (_, p) = caller(&self.0, &ctx).await?;
        triggers::remove(&self.0, &p.slug, &a.name).await?;
        Ok(format!("removed {}", a.name))
    }

    #[tool(description = "Move a trigger into the repo (its TRIGGER.md and script written into the project folder, for committing) or back into reagent (the files removed). Its state goes with it.")]
    async fn trigger_move(&self, Parameters(a): Parameters<Move>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (_, p) = caller(&self.0, &ctx).await?;
        let t = triggers::move_to(&self.0.store, &p, &a.name, &a.to).await?;
        self.0.emit_trigger(&p.slug, &t.name).await;
        Ok(if t.source == "repo" { format!("{} is in the repo now: {}", t.name, triggers::repo_dir(&p, &t.name).display()) } else { format!("{} is kept in reagent now", t.name) })
    }
}
