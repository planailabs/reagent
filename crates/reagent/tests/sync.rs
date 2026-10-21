//! Git sync: a branch pulled (rebase) and pushed on an interval; a conflict or
//! uncommitted changes go to a task, after which reagent pushes, or the sync is stuck.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::*;
use reagent_store::GitSync;
use reagent_tools::gitsync;
use serde_json::json;

impl R {
    /// A bare remote the project folder's main tracks, and another clone of it.
    fn remote(&self) -> (PathBuf, PathBuf) {
        let (remote, other) = (self.data.path().join("remote.git"), self.data.path().join("other"));
        git(self.data.path(), &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        git(self.project.path(), &["remote", "add", "origin", remote.to_str().unwrap()]);
        git(self.project.path(), &["push", "-qu", "origin", "main"]);
        git(self.data.path(), &["clone", "-q", remote.to_str().unwrap(), other.to_str().unwrap()]);
        git(&other, &["config", "user.email", "o@o"]);
        git(&other, &["config", "user.name", "o"]);
        (remote, other)
    }

    async fn sync_on(&self) {
        let p = self.run.app.project("site").await.unwrap();
        gitsync::put(&self.run.app.store, &p, GitSync { enabled: true, notes: "Keep both greetings.".into(), ..GitSync::new("site", "") }).await.unwrap();
    }

    /// Waits (up to 60 s) until `f` holds for the folder's sync.
    async fn sync_until(&self, what: &str, f: impl Fn(&GitSync) -> bool) -> GitSync {
        for _ in 0..600 {
            let s = self.run.app.store.git_sync("site", "").await.unwrap().unwrap();
            if f(&s) {
                return s;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("timed out waiting for {what}: {:?}", self.run.app.store.git_sync("site", "").await.unwrap());
    }
}

fn commit(dir: &Path, file: &str, text: &str, msg: &str) {
    std::fs::write(dir.join(file), text).unwrap();
    git(dir, &["commit", "-qam", msg]);
}

#[tokio::test]
async fn a_conflict_is_resolved_by_a_task_then_pushed() {
    let r = start().await;
    r.allow_all().await;
    let (remote, other) = r.remote();
    commit(&other, "a.txt", "hello there\n", "theirs");
    git(&other, &["push", "-q"]);
    commit(r.project.path(), "a.txt", "hello world!\n", "ours");

    r.push("Sync main: resolve the rebase", |b| {
        let t = all_text(b);
        assert!(t.contains("a.txt") && t.contains("Keep both greetings.") && t.contains("Don't push"), "{t}");
        call("c1", "shell.exec", json!({"cmd": "printf 'hello there\\nhello world!\\n' > a.txt && git add a.txt && GIT_EDITOR=true git rebase --continue"}))
    });
    r.push("Sync main: resolve the rebase", |_| text("resolved: both greetings kept"));
    r.sync_on().await;
    let s = r.sync_until("its task", |s| s.state.0.fixer.is_some()).await;
    assert_eq!(s.state.0.status, "fixing");
    let task = r.task(s.state.0.fixer.as_deref().unwrap()).await;
    assert_eq!((task.origin.as_str(), task.cwd.as_str()), ("git-sync:site/", r.project.path().to_str().unwrap()));
    r.done(&task.id).await;

    // When it ends, reagent pushes the rebased branch.
    let s = r.sync_until("the push", |s| s.state.0.status == "ok").await;
    assert!(s.state.0.message.as_deref().unwrap().contains("pushed 1 commit(s)"), "{s:?}");
    assert_eq!(git(&remote, &["show", "main:a.txt"]), "hello there\nhello world!\n");
    assert!(git(&remote, &["rev-list", "--merges", "main"]).trim().is_empty(), "rebased");
    let notes = r.run.app.store.notifications(10).await.unwrap();
    assert!(notes.iter().any(|n| n.kind == "sync" && n.title.contains("needs a task")));
}

#[tokio::test]
async fn uncommitted_changes_left_by_its_task_make_it_stuck_until_asked() {
    let r = start().await;
    r.allow_all().await;
    r.remote();
    std::fs::write(r.project.path().join("notes.txt"), "draft\n").unwrap();
    r.push("Sync main: uncommitted changes", |b| {
        assert!(all_text(b).contains("notes.txt"), "{}", all_text(b));
        text("not sure: left them")
    });
    r.sync_on().await;
    let s = r.sync_until("its task", |s| s.state.0.fixer.is_some()).await;
    r.done(s.state.0.fixer.as_deref().unwrap()).await;
    let s = r.sync_until("stuck", |s| s.state.0.status == "stuck").await;
    assert!(s.state.0.fixer.is_none() && s.state.0.message.as_deref().unwrap().contains("uncommitted"));

    // Stuck: no rounds until the person says go on; sorted by then, it pushes.
    std::fs::remove_file(r.project.path().join("notes.txt")).unwrap();
    commit(r.project.path(), "a.txt", "hello again\n", "mine");
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(r.run.app.store.git_sync("site", "").await.unwrap().unwrap().state.0.status, "stuck");
    gitsync::run_now(&r.run.app.store, "site", "").await.unwrap();
    let s = r.sync_until("the push", |s| s.state.0.status == "ok").await;
    assert!(s.state.0.message.as_deref().unwrap().contains("pushed 1 commit(s)"), "{s:?}");
}

#[tokio::test]
async fn the_repo_file_overrides_the_kept_settings() {
    let r = start().await;
    std::fs::create_dir_all(r.project.path().join(".agents")).unwrap();
    std::fs::write(r.project.path().join(".agents/sync.md"), "---\nenabled: true\nstrategy: merge\n---\nMerge, never rebase.\n").unwrap();
    let p = r.run.app.project("site").await.unwrap();
    let all = gitsync::list(&r.run.app.store, &p).await;
    assert_eq!(all.len(), 1);
    let s = &all[0];
    assert_eq!((s.settings.enabled, s.settings.strategy.as_str(), s.settings.notes.as_str()), (true, "merge", "Merge, never rebase."));
    assert_eq!((s.kept.enabled, s.kept.strategy.as_str()), (false, "rebase"));
    assert_eq!(s.from_repo, ["enabled", "strategy", "notes"]);
    assert_eq!(s.local_branch.as_deref(), Some("main"));
}
