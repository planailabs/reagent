//! The merge reviewer (`merge = "review"`): before a task's worktree is
//! merged, a `review` subtask reads the change in the worktree and judges
//! whether it does what the task was asked; its report's first line is
//! APPROVE or REJECT. Its subtasks' origins (`merge-review:<task>@<head>`)
//! are its memory: a call run again after a restart finds the review it
//! started, and the rejections since the last approval are counted there.

use std::path::Path;

use reagent_store::{Project, Task, Worktree};

use crate::app::{App, StartTask};

/// Rejections in a row before the person decides.
pub const MAX_REJECTIONS: usize = 3;

#[derive(Debug, PartialEq)]
pub enum Review {
    Approved(String),
    Rejected(String),
    /// The person decides, with why (the reviewer's last word, or what went wrong).
    Person(String),
}

/// A review's verdict from its report: the first line starts with APPROVE or REJECT.
pub fn verdict(report: &str) -> Option<Review> {
    let report = report.trim();
    let first = report.lines().next()?.trim_start_matches(|c: char| !c.is_alphabetic()).to_ascii_uppercase();
    if first.starts_with("APPROVE") {
        Some(Review::Approved(report.into()))
    } else if first.starts_with("REJECT") {
        Some(Review::Rejected(report.into()))
    } else {
        None
    }
}

fn of(r: &Task) -> Option<Review> {
    (r.state == "done").then(|| verdict(r.report.as_deref().unwrap_or_default())).flatten()
}

/// The profile reviews run on: `reviewer_profile`, else `evaluator_profile`.
pub fn profile(app: &App) -> Option<String> {
    app.config.reviewer_profile.clone().or_else(|| app.config.evaluator_profile.clone())
}

/// Reviews the worktree's branch at `head` for task `t` (waits for it).
pub async fn merge(app: &App, t: &Task, p: &Project, w: &Worktree, head: &str, message: &str) -> Result<Review, String> {
    let Some(profile) = profile(app) else {
        return Ok(Review::Person("no reviewer: reagent.hcl has neither reviewer_profile nor evaluator_profile".into()));
    };
    let prefix = format!("merge-review:{}@", t.id);
    let mut reviews: Vec<Task> = app.store.tasks(None, Some(&t.id), false, 1000).await.map_err(|e| e.to_string())?.into_iter().filter(|r| r.origin.starts_with(&prefix)).collect();
    // Oldest first (ponytail: `created` is in seconds; ties only blur the count of rejections in a row).
    reviews.sort_by_key(|r| r.created);
    let origin = format!("{prefix}{head}");
    // One review per head commit: a new commit gets a new one.
    let sub = match reviews.iter().find(|r| r.origin == origin) {
        Some(r) if r.is_active() => r.clone(),
        Some(r) => {
            return Ok(match of(r) {
                Some(Review::Rejected(why)) => Review::Person(format!("asked again unchanged after the reviewer sent it back:\n\n{why}")),
                Some(v) => v,
                None => Review::Person(format!("the review {} ended {} without a verdict", r.id, r.state)),
            });
        }
        None => {
            let rejected: Vec<&Task> = reviews.iter().rev().take_while(|r| matches!(of(r), Some(Review::Rejected(_)))).collect();
            if rejected.len() >= MAX_REJECTIONS {
                let last = rejected[0].report.clone().unwrap_or_default();
                return Ok(Review::Person(format!("the reviewer sent it back {} times in a row; its last review:\n\n{last}", rejected.len())));
            }
            let earlier = rejected.first().and_then(|r| r.report.clone());
            app.start_task(StartTask {
                project: p.slug.clone(),
                title: format!("Review: {}", t.name()),
                prompt: prompt(t, w, head, message, earlier.as_deref()) + &extensions(app, p).await,
                profile: Some(profile),
                parent: Some(t.id.clone()),
                origin: Some(origin),
                kind: Some("review".into()),
                cwd: Some(w.path.clone()),
                ..Default::default()
            })
            .await?
        }
    };
    let mut rx = app.events.subscribe();
    let r = loop {
        let r = app.task(&sub.id).await?;
        if !r.is_active() {
            break r;
        }
        // ponytail: wakes on every event and on a timer; a per-task channel if that's ever too busy.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await;
    };
    Ok(of(&r).unwrap_or_else(|| Review::Person(format!("the review {} ended {} without a verdict: {}", r.id, r.state, r.report.unwrap_or_default().chars().take(500).collect::<String>()))))
}

/// The instructions added for every project (the settings' `reviewer_prompt`), then the project's.
async fn extensions(app: &App, p: &Project) -> String {
    let mut s = String::new();
    if let Some(g) = app.store.setting("reviewer_prompt").await.ok().flatten().filter(|g| !g.trim().is_empty()) {
        s.push_str(&format!("\n\n# For every project\n\n{}", g.trim()));
    }
    if !p.evaluator.review_prompt.trim().is_empty() {
        s.push_str(&format!("\n\n# For the project {}\n\n{}", p.name, p.evaluator.review_prompt.trim()));
    }
    s
}

fn prompt(t: &Task, w: &Worktree, head: &str, message: &str, earlier: Option<&str>) -> String {
    let dir = Path::new(&w.path);
    let log = crate::git::git(dir, &["log", "--format=%h %s", &format!("{}..HEAD", w.base)]).unwrap_or_default();
    let stat = crate::git::diff(dir, &w.base, true).unwrap_or_default();
    let reports = t.report.as_deref().map(|r| format!("\n\nIts report so far:\n{r}")).unwrap_or_default();
    let earlier = earlier.map(|e| format!("\n\nYour previous review sent it back:\n{e}\nCheck that it was dealt with.")).unwrap_or_default();
    format!(
        "Review a merge. Task {} ({}) asks to merge its branch {} (at {head}) into {}, with the message {message:?}. You're in its worktree ({}).\n\n\
         Judge whether the change does what the task was asked: all of it, nothing unrelated, no obvious bugs or missing tests for what it adds, \
         and the project's conventions (AGENTS.md, its rules, docs the change should update). Read the whole diff (git diff {}...HEAD) and \
         what it touches; run the tests when that settles a question. Don't nitpick style: send it back only for what should change before it's merged.\n\n\
         The first line of your report is APPROVE or REJECT. Then why: for REJECT, each problem with file:line and what to change; the task reads it.\n\n\
         The task's prompt:\n{}{reports}\n\nCommits:\n{log}\nFiles:\n{stat}{earlier}",
        t.name(),
        t.id,
        w.branch,
        w.base,
        w.path,
        w.base,
        t.prompt,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts() {
        assert!(matches!(verdict("APPROVE\nlooks right"), Some(Review::Approved(_))));
        assert!(matches!(verdict("**Reject**: no tests"), Some(Review::Rejected(_))));
        assert_eq!(verdict("I think it's fine"), None);
        assert_eq!(verdict(""), None);
    }
}
