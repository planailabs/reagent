//! Worktrees: start one (the task moves into it), see what changed, merge
//! it back (now, or once the person approves, per project), drop it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use reagent_store::Worktree;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::service::RequestContext;
use rmcp::{RoleServer, schemars, tool, tool_router};
use serde::Deserialize;
use serde_json::json;

use super::caller;
use crate::app::{App, MergeAnswer};
use crate::git::{self, Merged, Strategy};

#[derive(Clone)]
pub struct GitTools(pub Arc<App>);

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Start {
    /// The new branch (default reagent/<task>).
    pub branch: Option<String>,
    /// What it starts from (default: the branch the project is on).
    pub base: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Diff {
    /// Only which files changed and how much.
    #[serde(default)]
    pub stat: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Merge {
    /// merge (default), squash or rebase.
    pub strategy: Option<Strategy>,
    /// The merge (or squashed) commit's message.
    pub message: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Drop {
    /// Also when its branch has commits not merged.
    #[serde(default)]
    pub force: bool,
}

fn worktree(t: &reagent_store::Task) -> Result<Worktree, String> {
    t.worktree.as_ref().map(|w| w.0.clone()).ok_or_else(|| "you aren't in a worktree: git.worktree_start makes one".into())
}

/// Keeps `.worktrees/` out of the repository's status (worktrees in the repo).
fn exclude(repo: &Path) {
    let Ok(dir) = std::process::Command::new("git").arg("-C").arg(repo).args(["rev-parse", "--git-common-dir"]).output() else { return };
    let gd = PathBuf::from(String::from_utf8_lossy(&dir.stdout).trim());
    let gd = if gd.is_absolute() { gd } else { repo.join(gd) };
    let f = gd.join("info/exclude");
    let text = std::fs::read_to_string(&f).unwrap_or_default();
    if !text.lines().any(|l| l.trim() == "/.worktrees/") {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(&f, format!("{text}{}/.worktrees/\n", if text.is_empty() || text.ends_with('\n') { "" } else { "\n" }));
    }
}

#[tool_router(server_handler)]
impl GitTools {
    #[tool(description = "Start working in your own git worktree: a new branch from a base, checked out apart; your working directory moves there.")]
    async fn worktree_start(&self, Parameters(a): Parameters<Start>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        if let Some(w) = &t.worktree {
            return Err(format!("you're already in a worktree ({} on {})", w.0.path, w.0.branch));
        }
        let repo = git::top(Path::new(&p.path)).map_err(|_| format!("{} isn't a git repository", p.path))?;
        let base = match a.base {
            Some(b) => b,
            None => git::current_branch(&repo)?,
        };
        let branch = a.branch.unwrap_or_else(|| git::branch_for(&t.title, &t.id));
        let path = self.0.worktree_dir(&p, &branch);
        if p.worktrees == "repo" {
            exclude(&repo);
        }
        git::add(&repo, &path, &branch, &base)?;
        let w = Worktree { path: path.display().to_string(), branch: branch.clone(), base: base.clone() };
        self.0.store.set_cwd(&t.id, &w.path, Some(&w)).await.map_err(|e| e.to_string())?;
        Ok(format!("worktree {} on branch {branch} (from {base}); your working directory is there now. Commit as you go; git.worktree_merge brings it back.", w.path))
    }

    #[tool(description = "Your worktree's status: branch, commits ahead of the base, uncommitted changes.")]
    async fn worktree_status(&self, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, _) = caller(&self.0, &ctx).await?;
        let w = worktree(&t)?;
        let dir = Path::new(&w.path);
        Ok(format!("{} (from {}), {} commit(s) ahead\n{}", w.branch, w.base, git::ahead(dir, &w.base)?, git::status(dir)?))
    }

    #[tool(description = "What your worktree changed against its base (committed and not), or only the files (stat).")]
    async fn worktree_diff(&self, Parameters(a): Parameters<Diff>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, _) = caller(&self.0, &ctx).await?;
        let w = worktree(&t)?;
        let d = git::diff(Path::new(&w.path), &w.base, a.stat)?;
        Ok(if d.trim().is_empty() { "no changes".into() } else { d })
    }

    #[tool(description = "Merge your worktree's branch into its base (commit everything first). Per project it merges at once or waits for the person's approval; conflicts are named for you to resolve in the worktree (merge the base into it there), then merge again.")]
    async fn worktree_merge(&self, Parameters(a): Parameters<Merge>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let w = worktree(&t)?;
        let (dir, repo) = (Path::new(&w.path), git::top(Path::new(&p.path))?);
        let strategy = a.strategy.unwrap_or(Strategy::Merge);
        let message = a.message.unwrap_or_else(|| format!("{} (reagent task {})", t.title, &t.id[..8]));
        if !git::clean(dir)? {
            return Err("the worktree has uncommitted changes: commit them first".into());
        }
        let ahead = git::ahead(dir, &w.base)?;
        if ahead == 0 {
            return Ok(format!("nothing to merge: {} has no commits beyond {}", w.branch, w.base));
        }
        if p.merge == "approve" {
            let summary = json!({"branch": w.branch, "base": w.base, "strategy": strategy, "message": message, "ahead": ahead, "stat": git::diff(dir, &w.base, true).unwrap_or_default()});
            match self.0.wait_merge(&t.id, summary).await? {
                MergeAnswer::Merge => {}
                MergeAnswer::Reject(why) => return Ok(format!("not merged: the person sent it back: {why}")),
            }
        }
        let (dir, base, branch) = (dir.to_path_buf(), w.base.clone(), w.branch.clone());
        let r = tokio::task::spawn_blocking(move || git::merge(&repo, &dir, &branch, &base, strategy, &message)).await.map_err(|e| e.to_string())??;
        Ok(match r {
            Merged::Done { commit } => format!("merged {} into {} ({commit}). The worktree stays until git.worktree_drop.", w.branch, w.base),
            Merged::Nothing => "nothing to merge".into(),
            Merged::Conflicts { files } => format!("conflicts in: {}. Nothing was changed in {}. Resolve them in your worktree (git merge {} there, fix, commit), then merge again.", files.join(", "), w.base, w.base),
        })
    }

    #[tool(description = "Remove your worktree and its branch (refused while it has unmerged commits, unless force); your working directory goes back to the project.")]
    async fn worktree_drop(&self, Parameters(a): Parameters<Drop>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let w = worktree(&t)?;
        let repo = git::top(Path::new(&p.path))?;
        git::remove(&repo, Path::new(&w.path), &w.branch, &w.base, a.force)?;
        self.0.store.set_cwd(&t.id, &p.path, None).await.map_err(|e| e.to_string())?;
        Ok(format!("removed {} and {}; your working directory is {} again", w.path, w.branch, p.path))
    }
}
