//! A task's services tools: commands of its project that should always run
//! (started, checked, restarted; repaired by a task when that doesn't help).

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::service::RequestContext;
use rmcp::{RoleServer, schemars, tool, tool_router};
use serde::Deserialize;

use super::caller;
use crate::app::App;
use crate::services::{self, Def};

#[derive(Clone)]
pub struct ServiceTools(pub Arc<App>);

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Name {
    pub name: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Logs {
    pub name: String,
    /// How many of its last lines (default 100).
    pub lines: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Move {
    pub name: String,
    /// repo (into .agents/services/<name>/ in the project folder) or db (into reagent).
    pub to: String,
}

#[tool_router(server_handler)]
impl ServiceTools {
    #[tool(description = "This project's services: each one's state (starting, up, stopping, backoff, waiting for approval, stopped), since when, failures and the last error, its job.")]
    async fn service_list(&self, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (_, p) = caller(&self.0, &ctx).await?;
        let v = self.0.store.services(Some(&p.slug)).await.map_err(|e| e.to_string())?;
        Ok(if v.is_empty() { "no services".into() } else { v.iter().map(services::line).collect::<Vec<_>>().join("\n") })
    }

    #[tool(description = "Add (or replace) a service of this project: a command that should always run (a dev server, a database, a worker). reagent starts it, checks it with its health command, restarts it when it ends or stops answering (backing off), and after 5 failures in a row starts a task to repair it. A replaced running one restarts. Whether its command runs is the project's policy's call (services.run), which may ask the person.")]
    async fn service_add(&self, Parameters(d): Parameters<Def>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let v = services::save(&self.0.store, &p, d.into_service(&p.slug, &format!("task:{}", t.id))?, false).await?;
        self.0.emit_service(&p.slug, &v.name).await;
        Ok(format!("added: {} (it starts within seconds; service_list shows how it's doing)", services::line(&v)))
    }

    #[tool(description = "Remove one of this project's services: it's stopped (a repo service's files go too).")]
    async fn service_remove(&self, Parameters(a): Parameters<Name>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (_, p) = caller(&self.0, &ctx).await?;
        services::remove(&self.0, &p.slug, &a.name).await?;
        Ok(format!("removed {}", a.name))
    }

    #[tool(description = "Switch a service on: started at once, its failure streak cleared.")]
    async fn service_start(&self, Parameters(a): Parameters<Name>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (_, p) = caller(&self.0, &ctx).await?;
        let v = services::set_enabled(&self.0.store, &p.slug, &a.name, true).await?;
        self.0.emit_service(&p.slug, &v.name).await;
        Ok(format!("{} switched on: it starts within seconds", v.name))
    }

    #[tool(description = "Switch a service off: stopped (SIGTERM, then SIGKILL after its stop_timeout); it stays off until started.")]
    async fn service_stop(&self, Parameters(a): Parameters<Name>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (_, p) = caller(&self.0, &ctx).await?;
        let v = services::set_enabled(&self.0.store, &p.slug, &a.name, false).await?;
        self.0.emit_service(&p.slug, &v.name).await;
        Ok(format!("{} switched off: it stops within seconds", v.name))
    }

    #[tool(description = "Restart a service: stopped, then started again at once (a stopped one is switched on).")]
    async fn service_restart(&self, Parameters(a): Parameters<Name>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (_, p) = caller(&self.0, &ctx).await?;
        let v = services::restart(&self.0.store, &p.slug, &a.name).await?;
        self.0.emit_service(&p.slug, &v.name).await;
        Ok(format!("{} restarts within seconds", v.name))
    }

    #[tool(description = "The end of a service's output (its running job's, else its last start's).")]
    async fn service_logs(&self, Parameters(a): Parameters<Logs>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (_, p) = caller(&self.0, &ctx).await?;
        services::logs(&self.0, &p.slug, &a.name, a.lines.unwrap_or(100)).await
    }

    #[tool(description = "Move a service into the repo (its SERVICE.md and script written into the project folder, for committing) or back into reagent (the files removed). Its state goes with it.")]
    async fn service_move(&self, Parameters(a): Parameters<Move>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (_, p) = caller(&self.0, &ctx).await?;
        let v = services::move_to(&self.0.store, &p, &a.name, &a.to).await?;
        self.0.emit_service(&p.slug, &v.name).await;
        Ok(if v.source == "repo" { format!("{} is in the repo now: {}", v.name, services::repo_dir(&p, &v.name).display()) } else { format!("{} is kept in reagent now", v.name) })
    }
}
