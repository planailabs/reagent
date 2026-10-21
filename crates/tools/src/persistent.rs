//! Persistent worktrees: named, long-lived worktrees of a project that tasks
//! start in (their `home`). Made by reagent or adopted from git; a task's own
//! worktree branches from its home's branch and merges back into it.

use std::path::{Path, PathBuf};

use reagent_store::{PersistentWorktree, Project, Store, Task};
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::git::{self, Merged, Strategy};

/// A new persistent worktree: made (`branch`, `base`), or adopted (`path`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AddWorktree {
    pub name: String,
    /// Its branch (default: the name): checked out if there is one, else made from `base`.
    #[serde(default)]
    pub branch: Option<String>,
    /// What it merges into (default: the branch the project folder is on).
    #[serde(default)]
    pub base: Option<String>,
    /// A worktree git already has: recorded as it is.
    #[serde(default)]
    pub path: Option<String>,
    /// shared, exclusive, or none: the project's.
    #[serde(default)]
    pub sharing: Option<String>,
}

/// A persistent worktree as shown: its record and how it is now.
#[derive(Debug, Clone, Serialize)]
pub struct WorktreeInfo {
    #[serde(flatten)]
    pub worktree: PersistentWorktree,
    /// What decides: its own sharing, else the project's.
    pub sharing_now: String,
    /// Commits beyond its base (none: it couldn't be read).
    pub ahead: Option<usize>,
    /// Files with uncommitted changes.
    pub changes: Option<usize>,
    /// Top-level tasks working there now.
    pub active: i64,
    /// Its folder is gone.
    pub missing: bool,
}

/// A worktree git has that could be adopted.
#[derive(Debug, Clone, Serialize)]
pub struct Adoptable {
    pub path: String,
    pub branch: String,
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && !name.starts_with(['.', '-']) && name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

fn check_sharing(s: Option<&str>) -> Result<(), String> {
    match s {
        None | Some("shared" | "exclusive") => Ok(()),
        Some(x) => Err(format!("sharing {x:?}: shared or exclusive")),
    }
}

fn same(a: &Path, b: &Path) -> bool {
    a == b || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

/// Whether tasks take this worktree one at a time.
pub fn exclusive(p: &Project, w: &PersistentWorktree) -> bool {
    w.sharing.as_deref().unwrap_or(&p.worktree_sharing) == "exclusive"
}

/// The persistent worktrees, over reagent's store and data folder (the CLI has no App).
pub struct Worktrees<'a> {
    pub store: &'a Store,
    pub data: &'a Path,
}

impl App {
    pub fn worktrees(&self) -> Worktrees<'_> {
        Worktrees { store: &self.store, data: &self.paths.data }
    }
}

impl Worktrees<'_> {
    async fn project(&self, slug: &str) -> Result<Project, String> {
        self.store.project(slug).await.map_err(|e| e.to_string())?.ok_or_else(|| format!("no project {slug:?}"))
    }

    /// Where reagent makes a project's persistent worktree.
    pub fn dir(&self, p: &Project, name: &str) -> PathBuf {
        match p.worktrees.as_str() {
            "repo" => PathBuf::from(&p.path).join(".worktrees/persistent").join(name),
            _ => self.data.join("worktrees").join(&p.slug).join("persistent").join(name),
        }
    }

    pub async fn get(&self, project: &str, name: &str) -> Result<PersistentWorktree, String> {
        self.store.persistent_worktree(project, name).await.map_err(|e| e.to_string())?.ok_or_else(|| format!("{project} has no persistent worktree {name:?}"))
    }

    async fn info(&self, p: &Project, w: PersistentWorktree) -> WorktreeInfo {
        let dir = PathBuf::from(&w.path);
        let missing = !dir.is_dir();
        let (ahead, changes) = if missing {
            (None, None)
        } else {
            (git::ahead(&dir, &w.base).ok(), git::git(&dir, &["status", "--porcelain"]).ok().map(|s| s.lines().count()))
        };
        let active = self.store.tasks_in_home(&p.slug, &w.name).await.unwrap_or(0);
        WorktreeInfo { sharing_now: w.sharing.clone().unwrap_or_else(|| p.worktree_sharing.clone()), ahead, changes, active, missing, worktree: w }
    }

    /// A project's persistent worktrees, and the worktrees git has that could be adopted
    /// (not the project folder, not a task's own, on a branch).
    pub async fn list(&self, project: &str) -> Result<(Vec<WorktreeInfo>, Vec<Adoptable>), String> {
        let p = self.project(project).await?;
        let kept = self.store.persistent_worktrees(&p.slug).await.map_err(|e| e.to_string())?;
        let mut adoptable = vec![];
        if let Ok(repo) = git::top(Path::new(&p.path)) {
            let tasks = self.store.tasks(Some(&p.slug), None, false, 100_000).await.map_err(|e| e.to_string())?;
            let own: Vec<PathBuf> = tasks.iter().filter_map(|t| t.worktree.as_ref().map(|w| PathBuf::from(&w.0.path))).collect();
            for (path, branch) in git::worktrees(&repo).unwrap_or_default().into_iter().skip(1) {
                let Some(branch) = branch else { continue };
                if kept.iter().any(|w| same(Path::new(&w.path), &path)) || own.iter().any(|o| same(o, &path)) || !path.is_dir() {
                    continue;
                }
                adoptable.push(Adoptable { path: path.display().to_string(), branch });
            }
        }
        let mut out = vec![];
        for w in kept {
            out.push(self.info(&p, w).await);
        }
        Ok((out, adoptable))
    }

    pub async fn info_of(&self, project: &str, name: &str) -> Result<WorktreeInfo, String> {
        let p = self.project(project).await?;
        let w = self.get(project, name).await?;
        Ok(self.info(&p, w).await)
    }

    /// Makes a persistent worktree, or adopts one git has.
    pub async fn add(&self, project: &str, a: AddWorktree) -> Result<WorktreeInfo, String> {
        let p = self.project(project).await?;
        let name = a.name.trim().to_string();
        if !valid_name(&name) {
            return Err(format!("{name:?} can't be a worktree's name (letters, digits, . _ -)"));
        }
        check_sharing(a.sharing.as_deref())?;
        if self.store.persistent_worktree(&p.slug, &name).await.map_err(|e| e.to_string())?.is_some() {
            return Err(format!("{} has a persistent worktree {name} already", p.slug));
        }
        let repo = git::top(Path::new(&p.path)).map_err(|_| format!("{} isn't a git repository", p.path))?;
        let base = match a.base.filter(|b| !b.trim().is_empty()) {
            Some(b) => b,
            None => git::current_branch(&repo)?,
        };
        let (path, branch, adopted) = match a.path.filter(|x| !x.trim().is_empty()) {
            Some(path) => {
                let found = git::worktrees(&repo)?.into_iter().skip(1).find(|(w, _)| same(w, Path::new(&path)));
                let Some((w, branch)) = found else { return Err(format!("{path} isn't a worktree of {}", p.path)) };
                let branch = branch.ok_or_else(|| format!("{path} isn't on a branch (detached)"))?;
                (w, branch, true)
            }
            None => {
                let branch = a.branch.filter(|b| !b.trim().is_empty()).unwrap_or_else(|| name.clone());
                if git::checkout_of(&repo, &branch)?.is_some() {
                    return Err(format!("{branch} is checked out already: adopt that worktree instead"));
                }
                let path = self.dir(&p, &name);
                if p.worktrees == "repo" {
                    git::exclude_worktrees(&repo);
                }
                let (r, d, b, bs) = (repo.clone(), path.clone(), branch.clone(), base.clone());
                tokio::task::spawn_blocking(move || git::add_on(&r, &d, &b, &bs)).await.map_err(|e| e.to_string())??;
                (path, branch, false)
            }
        };
        if branch == base {
            return Err(format!("its branch and its base are both {base}: give another base"));
        }
        let w = PersistentWorktree { project: p.slug.clone(), name, path: path.display().to_string(), branch, base, sharing: a.sharing, adopted, created: 0 };
        self.store.add_persistent_worktree(&w).await.map_err(|e| e.to_string())?;
        tracing::info!(project = %p.slug, worktree = %w.name, branch = %w.branch, adopted, "persistent worktree added");
        self.info_of(&p.slug, &w.name).await
    }

    /// Its sharing (none: the project's).
    pub async fn set_sharing(&self, project: &str, name: &str, sharing: Option<&str>) -> Result<WorktreeInfo, String> {
        check_sharing(sharing)?;
        self.get(project, name).await?;
        self.store.set_worktree_sharing(project, name, sharing).await.map_err(|e| e.to_string())?;
        self.info_of(project, name).await
    }

    /// Its branch's changes against its base (committed and not), or only the files.
    pub async fn diff(&self, project: &str, name: &str, stat: bool) -> Result<String, String> {
        let w = self.get(project, name).await?;
        git::diff(Path::new(&w.path), &w.base, stat)
    }

    pub async fn status(&self, project: &str, name: &str) -> Result<String, String> {
        let w = self.get(project, name).await?;
        git::status(Path::new(&w.path))
    }

    pub async fn tasks(&self, project: &str, name: &str) -> Result<Vec<Task>, String> {
        self.store.home_tasks(project, name).await.map_err(|e| e.to_string())
    }

    /// Merges the worktree's branch into its base (the person's doing: no approval).
    pub async fn merge(&self, project: &str, name: &str, strategy: Strategy, message: Option<String>) -> Result<String, String> {
        let p = self.project(project).await?;
        let w = self.get(project, name).await?;
        let repo = git::top(Path::new(&p.path))?;
        let message = message.filter(|m| !m.trim().is_empty()).unwrap_or_else(|| format!("Merge {} ({name})", w.branch));
        let (dir, branch, base) = (PathBuf::from(&w.path), w.branch.clone(), w.base.clone());
        let r = tokio::task::spawn_blocking(move || git::merge(&repo, &dir, &branch, &base, strategy, &message)).await.map_err(|e| e.to_string())??;
        Ok(match r {
            Merged::Done { commit } => format!("merged {} into {} ({commit})", w.branch, w.base),
            Merged::Nothing => format!("nothing to merge: {} has no commits beyond {}", w.branch, w.base),
            Merged::Conflicts { files } => return Err(format!("conflicts in: {}. Nothing was changed in {}; resolve them in the worktree (merge {} into it there), then merge again", files.join(", "), w.base, w.base)),
        })
    }

    /// Removes a persistent worktree: refused while a task works there, and
    /// while it has work not in its base (unmerged commits, uncommitted files)
    /// unless `force` (the person discards it). An adopted one is only
    /// forgotten unless `delete`.
    pub async fn remove(&self, project: &str, name: &str, force: bool, delete: bool) -> Result<String, String> {
        let p = self.project(project).await?;
        let w = self.get(project, name).await?;
        let busy: Vec<String> = self.store.home_tasks(project, name).await.map_err(|e| e.to_string())?.into_iter().filter(|t| t.is_active()).map(|t| t.name().to_string()).collect();
        if !busy.is_empty() {
            return Err(format!("tasks work in {name}: {} (cancel them or wait)", busy.join(", ")));
        }
        let gone = if w.adopted && !delete {
            format!("forgot {name} (its folder {} and branch {} stay)", w.path, w.branch)
        } else if !Path::new(&w.path).is_dir() {
            format!("forgot {name} (its folder was gone already)")
        } else {
            let repo = git::top(Path::new(&p.path))?;
            let (dir, branch, base) = (PathBuf::from(&w.path), w.branch.clone(), w.base.clone());
            tokio::task::spawn_blocking(move || git::remove(&repo, &dir, &branch, &base, force)).await.map_err(|e| e.to_string())??;
            format!("removed {name}: {} and its branch {}", w.path, w.branch)
        };
        self.store.remove_persistent_worktree(project, name).await.map_err(|e| e.to_string())?;
        self.store.remove_git_sync(project, name).await.map_err(|e| e.to_string())?;
        tracing::info!(project, worktree = name, "persistent worktree removed");
        Ok(gone)
    }
}
