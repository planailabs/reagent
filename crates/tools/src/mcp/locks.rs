//! A task's locks: taken and let go while it runs, held by its task tree.

use std::sync::Arc;
use std::time::Duration;

use rmcp::RoleServer;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::service::RequestContext;
use rmcp::{schemars, tool, tool_router};
use serde::Deserialize;

use super::caller;
use crate::app::App;

#[derive(Clone)]
pub struct LockTools(pub Arc<App>);

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Acquire {
    /// The lock: a name of this project's (`deploy`), or `global:<name>` for one every project shares.
    pub name: String,
    /// Shared with other shared holders (readers); default exclusive.
    #[serde(default)]
    pub shared: bool,
    /// Wait until it's free (default true); false answers at once.
    pub wait: Option<bool>,
    /// Seconds to wait (default 600, at most 3600).
    pub timeout: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Release {
    pub name: String,
}

#[tool_router(server_handler)]
impl LockTools {
    #[tool(description = "Take a lock before doing what it guards (a deploy, a migration, …): held by your task tree (subtasks share it) until your task ends or you release it. Waits until it's free (up to timeout), or with wait = false says at once who holds it. A wait that would deadlock is refused.")]
    async fn lock_acquire(&self, Parameters(a): Parameters<Acquire>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let mut spec = crate::locks::parse(&p.slug, &a.name)?;
        spec.shared |= a.shared;
        let holder = crate::locks::holder_of(&self.0, &t).await;
        let specs = [spec.clone()];
        let r = if a.wait.unwrap_or(true) { crate::locks::wait_for(&self.0, &holder, &specs, Duration::from_secs(a.timeout.unwrap_or(600).clamp(1, 3600))).await } else { crate::locks::take(&self.0, &holder, &specs).await };
        match r {
            Ok(taken) if taken.is_empty() => Ok(format!("your task tree holds {} already", spec.label())),
            Ok(_) => Ok(format!("you hold {} now (until your task ends, or lock_release)", spec.label())),
            Err(busy) => Err(format!("not taken: {busy}")),
        }
    }

    #[tool(description = "Let go of a lock your task tree holds.")]
    async fn lock_release(&self, Parameters(a): Parameters<Release>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let spec = crate::locks::parse(&p.slug, &a.name)?;
        let holder = crate::locks::holder_of(&self.0, &t).await;
        if !self.0.store.release_lock(&holder, &spec.project, &spec.name).await.map_err(|e| e.to_string())? {
            return Err(format!("your task tree doesn't hold {}", spec.label()));
        }
        crate::locks::freed(&self.0);
        Ok(format!("released {}", spec.label()))
    }

    #[tool(description = "This project's locks and the global ones: what each is for, who holds it (shared or not, since when), who waits.")]
    async fn lock_list(&self, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (_, p) = caller(&self.0, &ctx).await?;
        let v = crate::locks::list(&self.0, &p.slug).await?;
        Ok(if v.as_array().is_some_and(|a| a.is_empty()) { "no locks held, none known".into() } else { serde_json::to_string_pretty(&v).unwrap_or_default() })
    }
}
