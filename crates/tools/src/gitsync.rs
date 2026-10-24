//! Git sync: a project's branches (its folder's, each persistent worktree's) pulled
//! with their strategy and pushed on an interval. Settings are kept in reagent.db;
//! the checkout's `.agents/sync.md` overrides any of them. A conflict, or
//! uncommitted changes nobody works on, go to a task; reagent pushes after it.

use crate::app::{App, StartTask};
use crate::git::{git, git_commit};
use reagent_store::{GitSync, Project, SyncState, Task};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// The checkout's own settings (the first found).
pub const FILES: [&str; 2] = [".agents/sync.md", ".agent/sync.md"];
pub const STRATEGIES: [&str; 3] = ["rebase", "merge", "ff-only"];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Front {
    enabled: Option<bool>,
    every: Option<String>,
    strategy: Option<String>,
    remote: Option<String>,
    branch: Option<String>,
    locks: Option<Vec<String>>,
    on_busy: Option<String>,
}

/// A branch's sync as it applies: reagent.db's settings with the repo file's over them.
#[derive(Debug, Clone, Serialize)]
pub struct Sync {
    #[serde(flatten)]
    pub settings: GitSync,
    /// The settings kept in reagent.db (what the person edits).
    pub kept: GitSync,
    /// The checkout, and its branch now.
    pub path: PathBuf,
    pub local_branch: Option<String>,
    /// The repo file, the fields it sets, or why it can't be read (then it's ignored).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_file: Option<PathBuf>,
    pub from_repo: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_error: Option<String>,
}

pub fn check(s: &GitSync) -> Result<(), String> {
    if !STRATEGIES.contains(&s.strategy.as_str()) {
        return Err(format!("strategy {:?}: one of {}", s.strategy, STRATEGIES.join(", ")));
    }
    if s.every < 60 {
        return Err("every: at least a minute".into());
    }
    crate::locks::parse_all(&s.project, &s.locks)?;
    if let Some(b) = &s.on_busy {
        crate::locks::OnBusy::parse(b)?;
    }
    for v in [&s.remote, &s.branch].into_iter().flatten() {
        if v.is_empty() || v.starts_with('-') || v.chars().any(|c| c.is_whitespace() || c == ':') {
            return Err(format!("{v:?} isn't a remote or branch name"));
        }
    }
    Ok(())
}

/// Applies a repo file's text over `s`; the fields it set.
fn apply_repo(s: &mut GitSync, text: &str) -> Result<Vec<&'static str>, String> {
    let t = text.trim_start_matches('\u{feff}');
    let (f, body) = match t.strip_prefix("---") {
        Some(rest) => {
            let rest = rest.trim_start_matches(['\r', '\n']);
            let end = rest.find("\n---").ok_or("the frontmatter isn't closed (---)")?;
            let f = serde_yaml::from_str::<Option<Front>>(&rest[..end]).map_err(|e| format!("frontmatter: {e}"))?.unwrap_or_default();
            (f, rest[end + 4..].trim_start_matches('-').trim().to_string())
        }
        None => (Front::default(), t.trim().to_string()),
    };
    let mut set = vec![];
    if let Some(v) = f.enabled {
        s.enabled = v;
        set.push("enabled");
    }
    if let Some(v) = f.every {
        s.every = crate::triggers::parse_every(&v)?;
        set.push("every");
    }
    if let Some(v) = f.strategy {
        s.strategy = v;
        set.push("strategy");
    }
    if f.remote.is_some() {
        s.remote = f.remote;
        set.push("remote");
    }
    if f.branch.is_some() {
        s.branch = f.branch;
        set.push("branch");
    }
    if let Some(v) = f.locks {
        s.locks = sqlx::types::Json(v);
        set.push("locks");
    }
    if f.on_busy.is_some() {
        s.on_busy = f.on_busy;
        set.push("on_busy");
    }
    if !body.is_empty() {
        s.notes = body;
        set.push("notes");
    }
    check(s)?;
    Ok(set)
}

/// A project's places: the folder (""), then each persistent worktree.
async fn places(store: &reagent_store::Store, p: &Project) -> Vec<(String, PathBuf)> {
    let mut all = vec![(String::new(), PathBuf::from(&p.path))];
    for w in store.persistent_worktrees(&p.slug).await.unwrap_or_default() {
        all.push((w.name, PathBuf::from(w.path)));
    }
    all
}

/// One place's sync as it applies.
pub fn resolve(kept: GitSync, path: PathBuf) -> Sync {
    let mut settings = kept.clone();
    let (mut repo_file, mut from_repo, mut repo_error) = (None, vec![], None);
    if let Some(f) = FILES.iter().map(|f| path.join(f)).find(|f| f.is_file()) {
        match std::fs::read_to_string(&f).map_err(|e| e.to_string()).and_then(|t| apply_repo(&mut settings, &t)) {
            Ok(set) => from_repo = set,
            Err(e) => {
                settings = kept.clone();
                repo_error = Some(e);
            }
        }
        repo_file = Some(f);
    }
    let local_branch = branch_of(&path).ok();
    Sync { settings, kept, path, local_branch, repo_file, from_repo, repo_error }
}

/// A project's syncs, a place each (the folder first).
pub async fn list(store: &reagent_store::Store, p: &Project) -> Vec<Sync> {
    let kept = store.git_syncs(Some(&p.slug)).await.unwrap_or_default();
    let mut out = vec![];
    for (place, path) in places(store, p).await {
        let k = kept.iter().find(|k| k.place == place).cloned().unwrap_or_else(|| GitSync::new(&p.slug, &place));
        out.push(tokio::task::spawn_blocking(move || resolve(k, path)).await.expect("resolve"));
    }
    out
}

/// What a round came to.
#[derive(Debug, PartialEq)]
enum Outcome {
    Ok(String),
    Error(String),
    /// A rebase or merge stopped on conflicts: (how, `git status`, conflicted files).
    Conflict(String, String, String),
    /// Uncommitted changes: `git status`.
    Dirty(String),
}

fn in_progress(dir: &Path) -> Option<&'static str> {
    let exists = |p: &str| git(dir, &["rev-parse", "--git-path", p]).is_ok_and(|g| dir.join(g.trim()).exists());
    if exists("rebase-merge") || exists("rebase-apply") {
        Some("rebase")
    } else if exists("MERGE_HEAD") {
        Some("merge")
    } else {
        None
    }
}

/// The checkout's branch; during a rebase (HEAD detached) the one being rebased.
fn branch_of(dir: &Path) -> Result<String, String> {
    crate::git::current_branch(dir).or_else(|e| {
        ["rebase-merge/head-name", "rebase-apply/head-name"]
            .iter()
            .filter_map(|f| git(dir, &["rev-parse", "--git-path", f]).ok())
            .find_map(|p| std::fs::read_to_string(dir.join(p.trim())).ok())
            .map(|r| r.trim().trim_start_matches("refs/heads/").to_string())
            .ok_or(e)
    })
}

fn conflict(dir: &Path, how: &str) -> Outcome {
    let status = git(dir, &["status"]).unwrap_or_default();
    let files = git(dir, &["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default();
    Outcome::Conflict(how.into(), status, files.trim().to_string())
}

/// The remote and its branch the local `branch` syncs with.
fn target(dir: &Path, s: &GitSync, branch: &str) -> (String, String) {
    let cfg = |k: &str| git(dir, &["config", "--get", &format!("branch.{branch}.{k}")]).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty() && v != ".");
    let remote = s.remote.clone().or_else(|| cfg("remote")).unwrap_or_else(|| "origin".into());
    let rbranch = s.branch.clone().or_else(|| cfg("merge").map(|m| m.trim_start_matches("refs/heads/").to_string())).unwrap_or_else(|| branch.into());
    (remote, rbranch)
}

/// One round in a checkout: pull with the strategy, then push what the remote hasn't.
fn round(dir: &Path, s: &GitSync) -> Outcome {
    if let Some(how) = in_progress(dir) {
        return conflict(dir, how);
    }
    let branch = match crate::git::current_branch(dir) {
        Ok(b) => b,
        Err(_) => return Outcome::Error("HEAD isn't on a branch (detached)".into()),
    };
    match git(dir, &["status", "--porcelain"]) {
        Ok(st) if !st.trim().is_empty() => return Outcome::Dirty(git(dir, &["status"]).unwrap_or(st)),
        Ok(_) => {}
        Err(e) => return Outcome::Error(e),
    }
    let (remote, rbranch) = target(dir, s, &branch);
    let tracking = format!("refs/remotes/{remote}/{rbranch}");
    let there = match git(dir, &["fetch", "--quiet", &remote, &format!("+refs/heads/{rbranch}:{tracking}")]) {
        Ok(_) => true,
        // A branch the remote hasn't yet is pushed; any other failure is one.
        Err(e) => match git(dir, &["ls-remote", "--heads", &remote, &format!("refs/heads/{rbranch}")]) {
            Ok(out) if out.trim().is_empty() => false,
            _ => return Outcome::Error(e),
        },
    };
    let mut took = 0;
    if there {
        took = git(dir, &["rev-list", "--count", &format!("HEAD..{tracking}")]).ok().and_then(|n| n.trim().parse().ok()).unwrap_or(0);
        if took > 0 {
            let r = match s.strategy.as_str() {
                "merge" => git_commit(dir, &["merge", "--no-edit", &tracking]),
                "ff-only" => git(dir, &["merge", "--ff-only", &tracking]),
                _ => git_commit(dir, &["rebase", &tracking]),
            };
            if let Err(e) = r {
                return match in_progress(dir) {
                    Some(how) => conflict(dir, how),
                    None if s.strategy == "ff-only" => Outcome::Error(format!("{branch} and {remote}/{rbranch} went apart: ff-only can't take it ({e})")),
                    None => Outcome::Error(e),
                };
            }
        }
    }
    let ahead: usize = if there { git(dir, &["rev-list", "--count", &format!("{tracking}..HEAD")]).ok().and_then(|n| n.trim().parse().ok()).unwrap_or(0) } else { 1 };
    if ahead > 0
        && let Err(e) = git(dir, &["push", "--quiet", &remote, &format!("HEAD:refs/heads/{rbranch}")])
    {
        return Outcome::Error(e);
    }
    let mut said = vec![];
    if took > 0 {
        said.push(format!("took {took} commit(s) from {remote}/{rbranch} ({})", s.strategy));
    }
    if ahead > 0 {
        said.push(if there { format!("pushed {ahead} commit(s)") } else { format!("pushed {branch} as {remote}/{rbranch}") });
    }
    Outcome::Ok(if said.is_empty() { format!("in sync with {remote}/{rbranch}") } else { said.join("; ") })
}

fn origin(project: &str, place: &str) -> String {
    format!("git-sync:{project}/{place}")
}

fn place_name(place: &str) -> String {
    if place.is_empty() { "the project folder".into() } else { format!("persistent worktree {place}") }
}

/// The task that resolves what a round couldn't.
async fn start_fixer(app: &App, p: &Project, sy: &Sync, o: &Outcome) -> Result<Task, String> {
    let s = &sy.settings;
    let branch = sy.local_branch.clone().unwrap_or_default();
    let (remote, rbranch) = {
        let (dir, s, b) = (sy.path.clone(), s.clone(), branch.clone());
        tokio::task::spawn_blocking(move || target(&dir, &s, &b)).await.map_err(|e| e.to_string())?
    };
    let intro = format!(
        "reagent keeps branch {branch} in {} ({}) in sync with {remote}/{rbranch}: every {} min it pulls ({}) and pushes.",
        place_name(&s.place),
        sy.path.display(),
        s.every / 60,
        s.strategy
    );
    let (title, body) = match o {
        Outcome::Conflict(how, status, files) => (
            format!("Sync {branch}: resolve the {how}"),
            format!(
                "The {how} stopped on conflicts in:\n{files}\n\n`git status`:\n```\n{}\n```\n\nResolve them keeping what both sides meant, then finish it (git add, then `GIT_EDITOR=true git rebase --continue` for a rebase, `git commit --no-edit` for a merge); build and test as the project does. If it can't be resolved sensibly, abort it (`git {how} --abort`) and say why. Don't push: when you end, reagent checks that the checkout is clean and pushes.",
                status.trim()
            ),
        ),
        Outcome::Dirty(status) => (
            format!("Sync {branch}: uncommitted changes"),
            format!(
                "The checkout has uncommitted changes, and no task works there:\n```\n{}\n```\n\nDecide what to do with them: commit what belongs in the branch (with a clear message), remove what's junk (build outputs, temporary files; maybe a .gitignore entry), and stash what you can't tell (`git stash push -u -m \"…\"`), saying so. Leave the checkout clean. Don't push: when you end, reagent syncs and pushes.",
                status.trim()
            ),
        ),
        _ => unreachable!("only a conflict or uncommitted changes need a task"),
    };
    let notes = if s.notes.trim().is_empty() { String::new() } else { format!("\n\nNotes for this sync:\n{}", s.notes.trim()) };
    app.start_task(StartTask {
        project: p.slug.clone(),
        title,
        prompt: format!("{intro}\n\n{body}{notes}"),
        origin: Some(origin(&p.slug, &s.place)),
        kind: Some("code".into()),
        worktree: Some(s.place.clone()),
        ..Default::default()
    })
    .await
}

/// Top-level tasks working in a place now.
async fn busy(app: &App, project: &str, place: &str) -> i64 {
    if place.is_empty() { app.store.tasks_in_folder(project).await } else { app.store.tasks_in_home(project, place).await }.unwrap_or(1)
}

async fn save(app: &App, s: &GitSync, st: &SyncState) {
    if let Err(e) = app.store.set_git_sync_state(&s.project, &s.place, st).await {
        tracing::warn!(project = %s.project, place = %s.place, error = %e, "saving a sync's state");
    }
    let _ = app.events.send(serde_json::json!({"kind": "sync", "project": s.project, "place": s.place}));
}

/// Takes a round's locks; else its status (waiting, skipped, error) and why.
async fn sync_locks(app: &App, holder: &str, s: &GitSync) -> Result<Vec<crate::locks::Spec>, (&'static str, String)> {
    use crate::locks::OnBusy;
    let specs = crate::locks::parse_all(&s.project, &s.locks).map_err(|e| ("error", e))?;
    let busy = match crate::locks::take(app, holder, &specs).await {
        Ok(taken) => return Ok(taken),
        Err(busy) => busy,
    };
    let on_busy = s.on_busy.as_deref().map(OnBusy::parse).transpose().map_err(|e| ("error", e))?.unwrap_or(OnBusy::Skip);
    Err(match on_busy {
        OnBusy::Wait => ("waiting", format!("waits for a lock: {busy}")),
        OnBusy::Skip => ("skipped", format!("a lock it needs is busy: {busy}; next round in {} min", s.every / 60)),
        OnBusy::Fail => ("error", format!("a lock it needs is busy: {busy}")),
    })
}

/// A round for one place, if it's due.
async fn tick_one(app: &App, p: &Project, sy: Sync, now: i64) {
    let s = &sy.settings;
    let mut st = s.state.0.clone();
    if !st.run_now && (!s.enabled || st.status == "stuck" || now - st.last_run < s.every) {
        return;
    }
    // Its fixer still works on it.
    if let Some(f) = &st.fixer
        && app.task(f).await.is_ok_and(|t| t.is_active())
    {
        return;
    }
    let after_fix = st.fixer.take().is_some();
    st.run_now = false;
    st.last_run = now;
    let n = busy(app, &p.slug, &s.place).await;
    if n > 0 {
        st.status = "skipped".into();
        st.message = Some(format!("{n} task(s) work there; next round in {} min", s.every / 60));
        return save(app, s, &st).await;
    }
    let was = st.status.clone();
    let holder = format!("sync:{}/{}", p.slug, s.place);
    let taken = match sync_locks(app, &holder, s).await {
        Ok(taken) => taken,
        Err((status, m)) => {
            if status == "error" && was != "error" {
                app.notify("sync", None, &format!("Sync failed: {} of {}", place_name(&s.place), p.name), &m).await;
            }
            // Waiting: tried again at the next tick.
            st.run_now = status == "waiting";
            st.status = status.into();
            st.message = Some(m);
            return save(app, s, &st).await;
        }
    };
    let (dir, set) = (sy.path.clone(), s.clone());
    let o = tokio::task::spawn_blocking(move || round(&dir, &set)).await.unwrap_or_else(|e| Outcome::Error(e.to_string()));
    crate::locks::release(app, &holder, &taken).await;
    let what = format!("{} of {}", place_name(&s.place), p.name);
    match o {
        Outcome::Ok(m) => {
            st.status = "ok".into();
            st.message = Some(m);
            st.last_ok = now;
        }
        Outcome::Error(m) => {
            if was != "error" {
                app.notify("sync", None, &format!("Sync failed: {what}"), &m).await;
            }
            st.status = "error".into();
            st.message = Some(m);
        }
        o if after_fix => {
            let m = match &o {
                Outcome::Conflict(how, _, files) => format!("its task ended with the {how} unfinished: {files}"),
                _ => "its task ended with uncommitted changes left".into(),
            };
            app.notify("sync", None, &format!("Sync stuck: {what}"), &format!("{m}; sync again once it's sorted")).await;
            st.status = "stuck".into();
            st.message = Some(m);
        }
        o => match start_fixer(app, p, &sy, &o).await {
            Ok(t) => {
                app.notify("sync", Some(&t), &format!("Sync needs a task: {what}"), &t.title).await;
                st.status = "fixing".into();
                st.message = Some(t.title.clone());
                st.fixer = Some(t.id);
            }
            Err(e) => {
                st.status = "error".into();
                st.message = Some(format!("couldn't start a task to fix it: {e}"));
            }
        },
    }
    save(app, s, &st).await;
}

pub async fn tick(app: &Arc<App>, now: i64) {
    for p in app.store.projects().await.unwrap_or_default() {
        for sy in list(&app.store, &p).await {
            if sy.settings.enabled || sy.settings.state.run_now {
                tick_one(app, &p, sy, now).await;
            }
        }
    }
}

/// Every few seconds: the rounds that are due.
pub fn schedule(app: Arc<App>) {
    tokio::spawn(async move {
        loop {
            tick(&app, chrono::Utc::now().timestamp()).await;
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

/// A place as the API and the CLI name it: "." (in a URL "-", as browsers drop a "." segment) is the project folder.
pub fn place_of(s: &str) -> &str {
    if s == "." || s == "-" { "" } else { s }
}

/// Changes a place's kept settings (its state stays).
pub async fn put(store: &reagent_store::Store, p: &Project, mut v: GitSync) -> Result<GitSync, String> {
    v.project = p.slug.clone();
    v.place = place_of(&v.place).to_string();
    if !v.place.is_empty() && store.persistent_worktree(&p.slug, &v.place).await.map_err(|e| e.to_string())?.is_none() {
        return Err(format!("{} has no persistent worktree {:?}", p.slug, v.place));
    }
    v.remote = v.remote.filter(|r| !r.trim().is_empty());
    v.branch = v.branch.filter(|b| !b.trim().is_empty());
    check(&v)?;
    store.put_git_sync(&v).await.map_err(|e| e.to_string())?;
    Ok(store.git_sync(&p.slug, &v.place).await.map_err(|e| e.to_string())?.unwrap_or(v))
}

/// A round now (a stuck one goes on).
pub async fn run_now(store: &reagent_store::Store, project: &str, place: &str) -> Result<(), String> {
    let mut st = store.git_sync(project, place).await.map_err(|e| e.to_string())?.map(|s| s.state.0).unwrap_or_default();
    st.run_now = true;
    store.set_git_sync_state(project, place, &st).await.map_err(|e| e.to_string())
}

/// A fixer ended: the next round checks and pushes.
pub async fn task_ended(app: &App, t: &Task) {
    let Some((project, place)) = t.origin.strip_prefix("git-sync:").and_then(|r| r.split_once('/')) else {
        return;
    };
    if let Err(e) = run_now(&app.store, project, place).await {
        tracing::warn!(project, place, error = %e, "a sync's task ended");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str]) {
        git(dir, args).unwrap();
    }

    fn commit(dir: &Path, file: &str, text: &str) {
        std::fs::write(dir.join(file), text).unwrap();
        sh(dir, &["add", "."]);
        sh(dir, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", file]);
    }

    /// A bare remote, and two clones of it (`a`, `b`) on main.
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let (remote, a, b) = (d.path().join("remote.git"), d.path().join("a"), d.path().join("b"));
        sh(d.path(), &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        sh(d.path(), &["clone", "-q", remote.to_str().unwrap(), a.to_str().unwrap()]);
        sh(&a, &["checkout", "-qb", "main"]);
        commit(&a, "f", "1\n");
        sh(&a, &["push", "-qu", "origin", "main"]);
        sh(d.path(), &["clone", "-q", remote.to_str().unwrap(), b.to_str().unwrap()]);
        for c in [&a, &b] {
            sh(c, &["config", "user.name", "t"]);
            sh(c, &["config", "user.email", "t@t"]);
        }
        (d, a, b)
    }

    #[test]
    fn rounds_pull_push_and_stop_on_conflicts_and_dirt() {
        let (_d, a, b) = setup();
        let s = GitSync::new("p", "");
        assert_eq!(round(&a, &s), Outcome::Ok("in sync with origin/main".into()));

        // Both sides commit: b rebases onto a's and pushes.
        commit(&a, "g", "a\n");
        sh(&a, &["push", "-q"]);
        commit(&b, "h", "b\n");
        assert_eq!(round(&b, &s), Outcome::Ok("took 1 commit(s) from origin/main (rebase); pushed 1 commit(s)".into()));
        assert_eq!(git(&b, &["rev-list", "--count", "HEAD"]).unwrap().trim(), "3");
        assert!(git(&b, &["rev-list", "--merges", "HEAD"]).unwrap().trim().is_empty(), "rebased, no merge commit");

        // Uncommitted changes go to a task.
        std::fs::write(b.join("junk"), "x").unwrap();
        assert!(matches!(round(&b, &s), Outcome::Dirty(st) if st.contains("junk")));
        std::fs::remove_file(b.join("junk")).unwrap();

        // The same line changed on both sides: the rebase stops and stays for a task.
        sh(&a, &["pull", "-q", "--rebase"]);
        commit(&a, "f", "from a\n");
        sh(&a, &["push", "-q"]);
        commit(&b, "f", "from b\n");
        assert!(matches!(round(&b, &s), Outcome::Conflict(how, _, files) if how == "rebase" && files == "f"));
        assert_eq!(in_progress(&b), Some("rebase"));
        assert!(matches!(round(&b, &s), Outcome::Conflict(..)), "still there next round");
        assert_eq!(branch_of(&b).unwrap(), "main", "the branch being rebased");

        // ff-only refuses to go on when they went apart.
        sh(&b, &["rebase", "--abort"]);
        let ff = GitSync { strategy: "ff-only".into(), ..GitSync::new("p", "") };
        assert!(matches!(round(&b, &ff), Outcome::Error(e) if e.contains("went apart")));
    }

    #[test]
    fn a_branch_the_remote_hasnt_is_pushed() {
        let (_d, a, _b) = setup();
        sh(&a, &["checkout", "-qb", "feature"]);
        commit(&a, "x", "x\n");
        let s = GitSync::new("p", "");
        assert_eq!(round(&a, &s), Outcome::Ok("pushed feature as origin/feature".into()));
        assert_eq!(round(&a, &s), Outcome::Ok("in sync with origin/feature".into()));
    }

    #[test]
    fn the_repo_file_wins_field_by_field() {
        let mut s = GitSync { notes: "kept".into(), ..GitSync::new("p", "") };
        let set = apply_repo(&mut s, "---\nenabled: true\nevery: 5m\n---\nfrom the repo\n").unwrap();
        assert_eq!(set, ["enabled", "every", "notes"]);
        assert_eq!((s.enabled, s.every, s.strategy.as_str(), s.notes.as_str()), (true, 300, "rebase", "from the repo"));
        assert!(apply_repo(&mut GitSync::new("p", ""), "---\nstrategy: squash\n---\n").is_err());
        assert!(apply_repo(&mut GitSync::new("p", ""), "---\ninterval: 5m\n---\n").is_err(), "unknown fields are errors");
    }
}
