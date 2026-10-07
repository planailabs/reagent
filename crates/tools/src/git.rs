//! Worktrees: a task's own checkout on its own branch, and merging it back.
//! Run as the `git` CLI: worktrees, merges and rebases aren't covered by a
//! library (gitoxide), and git is the tool here.

use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).env("GIT_TERMINAL_PROMPT", "0").output().map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let msg = if err.is_empty() { String::from_utf8_lossy(&out.stdout).trim().to_string() } else { err };
        return Err(format!("git {}: {msg}", args.first().unwrap_or(&"")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn is_repo(dir: &Path) -> bool {
    git(dir, &["rev-parse", "--git-dir"]).is_ok()
}

/// The repository's top folder.
pub fn top(dir: &Path) -> Result<PathBuf, String> {
    Ok(PathBuf::from(git(dir, &["rev-parse", "--show-toplevel"])?.trim()))
}

pub fn current_branch(dir: &Path) -> Result<String, String> {
    let b = git(dir, &["symbolic-ref", "--short", "-q", "HEAD"]).map(|s| s.trim().to_string()).unwrap_or_default();
    if b.is_empty() { Err("HEAD isn't on a branch (detached): give a base".into()) } else { Ok(b) }
}

/// A branch name made from a title: `reagent/fix-the-login`.
pub fn branch_for(title: &str, id: &str) -> String {
    let mut s: String = title.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-');
    let s: String = s.chars().take(40).collect();
    let short = &id[..id.len().min(6)];
    format!("reagent/{}{}{short}", s.trim_end_matches('-'), if s.is_empty() { "" } else { "-" })
}

/// Makes a worktree at `path` on a new `branch` from `base`.
pub fn add(repo: &Path, path: &Path, branch: &str, base: &str) -> Result<(), String> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    git(repo, &["worktree", "add", "-b", branch, &path.display().to_string(), base]).map(|_| ())
}

/// `git status --short` in a worktree.
pub fn status(dir: &Path) -> Result<String, String> {
    git(dir, &["status", "--short", "--branch"])
}

/// What the worktree's branch (and its uncommitted changes) changed against `base`.
pub fn diff(dir: &Path, base: &str, stat: bool) -> Result<String, String> {
    let fork = git(dir, &["merge-base", base, "HEAD"])?.trim().to_string();
    let mut args = vec!["diff", "--no-color"];
    if stat {
        args.push("--stat");
    }
    args.push(&fork);
    git(dir, &args)
}

/// Commits the worktree's branch has that `base` hasn't.
pub fn ahead(dir: &Path, base: &str) -> Result<usize, String> {
    Ok(git(dir, &["rev-list", "--count", &format!("{base}..HEAD")])?.trim().parse().unwrap_or(0))
}

pub fn clean(dir: &Path) -> Result<bool, String> {
    Ok(git(dir, &["status", "--porcelain"])?.trim().is_empty())
}

/// Where `branch` is checked out, if anywhere.
pub fn checkout_of(repo: &Path, branch: &str) -> Result<Option<PathBuf>, String> {
    let list = git(repo, &["worktree", "list", "--porcelain"])?;
    let mut path = None;
    for l in list.lines() {
        if let Some(p) = l.strip_prefix("worktree ") {
            path = Some(PathBuf::from(p));
        } else if l.strip_prefix("branch refs/heads/") == Some(branch) {
            return Ok(path);
        }
    }
    Ok(None)
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    Merge,
    Squash,
    Rebase,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Merged {
    Done { commit: String },
    /// Nothing to merge.
    Nothing,
    Conflicts { files: Vec<String> },
}

fn conflicted(dir: &Path) -> Vec<String> {
    git(dir, &["diff", "--name-only", "--diff-filter=U"]).map(|s| s.lines().map(String::from).collect()).unwrap_or_default()
}

/// Merges the worktree `wt`'s branch into `base`. The worktree must be clean
/// (commit first). Conflicts are aborted and reported: they're resolved in
/// the worktree (merge `base` into it there) and merged again.
pub fn merge(repo: &Path, wt: &Path, branch: &str, base: &str, strategy: Strategy, message: &str) -> Result<Merged, String> {
    if !clean(wt)? {
        return Err("the worktree has uncommitted changes: commit them (or drop them) first".into());
    }
    if ahead(wt, base)? == 0 {
        return Ok(Merged::Nothing);
    }
    if strategy == Strategy::Rebase && let Err(e) = git(wt, &["rebase", base]) {
        let files = conflicted(wt);
        let _ = git(wt, &["rebase", "--abort"]);
        return if files.is_empty() { Err(e) } else { Ok(Merged::Conflicts { files }) };
    }
    // Into a checkout of the base: the one there is, else a temporary one.
    let (target, temp) = match checkout_of(repo, base)? {
        Some(p) => (p, false),
        None => {
            let t = std::env::temp_dir().join(format!("reagent-merge-{}", uuid::Uuid::new_v4().simple()));
            git(repo, &["worktree", "add", &t.display().to_string(), base])?;
            (t, true)
        }
    };
    let result = (|| {
        // Untracked files don't stop a merge (git refuses itself if one would be overwritten).
        if !git(&target, &["status", "--porcelain", "--untracked-files=no"])?.trim().is_empty() {
            return Err(format!("{} (where {base} is checked out) has uncommitted changes: it can't take a merge now", target.display()));
        }
        let r = match strategy {
            Strategy::Merge => git(&target, &["merge", "--no-ff", "-m", message, branch]),
            Strategy::Rebase => git(&target, &["merge", "--ff-only", branch]),
            Strategy::Squash => git(&target, &["merge", "--squash", branch]).and_then(|_| git(&target, &["commit", "-m", message])),
        };
        if let Err(e) = r {
            let files = conflicted(&target);
            let _ = git(&target, &["merge", "--abort"]);
            let _ = git(&target, &["reset", "--merge"]);
            return if files.is_empty() { Err(e) } else { Ok(Merged::Conflicts { files }) };
        }
        Ok(Merged::Done { commit: git(&target, &["rev-parse", "--short", "HEAD"])?.trim().to_string() })
    })();
    if temp {
        let _ = git(repo, &["worktree", "remove", "--force", &target.display().to_string()]);
    }
    result
}

/// Removes a worktree and its branch; `force` also when the branch has
/// commits not merged into `base`.
pub fn remove(repo: &Path, wt: &Path, branch: &str, base: &str, force: bool) -> Result<(), String> {
    if !force {
        let unmerged = git(repo, &["rev-list", "--count", &format!("{base}..{branch}")]).ok().and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(0);
        if unmerged > 0 {
            return Err(format!("{branch} has {unmerged} commit(s) not in {base}: merge first, or drop with force = true"));
        }
    }
    git(repo, &["worktree", "remove", "--force", &wt.display().to_string()])?;
    git(repo, &["branch", "-D", branch]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        git(r, &["init", "-q", "-b", "main"]).unwrap();
        git(r, &["config", "user.email", "t@t"]).unwrap();
        git(r, &["config", "user.name", "t"]).unwrap();
        std::fs::write(r.join("a.txt"), "one\n").unwrap();
        git(r, &["add", "."]).unwrap();
        git(r, &["commit", "-q", "-m", "init"]).unwrap();
        d
    }

    fn commit(dir: &Path, file: &str, text: &str) {
        std::fs::write(dir.join(file), text).unwrap();
        git(dir, &["add", "."]).unwrap();
        git(dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", file]).unwrap();
    }

    #[test]
    fn a_worktree_is_made_merged_and_dropped() {
        let r = repo();
        let wt = r.path().join("wt/fix");
        let branch = branch_for("Fix the Login!", "abcdef123");
        assert_eq!(branch, "reagent/fix-the-login-abcdef");
        add(r.path(), &wt, &branch, "main").unwrap();
        assert_eq!(current_branch(&wt).unwrap(), branch);
        assert_eq!(merge(r.path(), &wt, &branch, "main", Strategy::Merge, "m").unwrap(), Merged::Nothing);
        std::fs::write(wt.join("b.txt"), "new\n").unwrap();
        assert!(merge(r.path(), &wt, &branch, "main", Strategy::Merge, "m").unwrap_err().contains("uncommitted"));
        git(&wt, &["add", "."]).unwrap();
        git(&wt, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "b"]).unwrap();
        assert!(diff(&wt, "main", true).unwrap().contains("b.txt"));
        assert_eq!(ahead(&wt, "main").unwrap(), 1);
        assert!(remove(r.path(), &wt, &branch, "main", false).unwrap_err().contains("not in main"));
        let Merged::Done { .. } = merge(r.path(), &wt, &branch, "main", Strategy::Merge, "merge fix").unwrap() else { panic!() };
        assert_eq!(std::fs::read_to_string(r.path().join("b.txt")).unwrap(), "new\n", "merged into the main checkout");
        remove(r.path(), &wt, &branch, "main", false).unwrap();
        assert!(!wt.exists() && checkout_of(r.path(), &branch).unwrap().is_none());
    }

    #[test]
    fn conflicts_are_aborted_and_named() {
        let r = repo();
        let wt = r.path().join("wt/c");
        add(r.path(), &wt, "reagent/c", "main").unwrap();
        commit(&wt, "a.txt", "theirs\n");
        commit(r.path(), "a.txt", "ours\n");
        assert_eq!(merge(r.path(), &wt, "reagent/c", "main", Strategy::Merge, "m").unwrap(), Merged::Conflicts { files: vec!["a.txt".into()] });
        assert!(clean(r.path()).unwrap(), "the main checkout is left as it was");
        assert_eq!(merge(r.path(), &wt, "reagent/c", "main", Strategy::Rebase, "m").unwrap(), Merged::Conflicts { files: vec!["a.txt".into()] });
        assert!(clean(&wt).unwrap());
    }

    #[test]
    fn squash_and_a_base_checked_out_nowhere() {
        let r = repo();
        git(r.path(), &["branch", "dev"]).unwrap();
        let wt = r.path().join("wt/s");
        add(r.path(), &wt, "reagent/s", "dev").unwrap();
        commit(&wt, "x.txt", "1\n");
        commit(&wt, "y.txt", "2\n");
        let Merged::Done { .. } = merge(r.path(), &wt, "reagent/s", "dev", Strategy::Squash, "both").unwrap() else { panic!() };
        let log = git(r.path(), &["log", "--format=%s", "dev"]).unwrap();
        assert_eq!(log.lines().collect::<Vec<_>>(), ["both", "init"], "one commit, into dev (checked out nowhere)");
        assert_eq!(current_branch(r.path()).unwrap(), "main");
    }
}
