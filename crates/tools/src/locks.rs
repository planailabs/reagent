//! Named locks (DESIGN.md, Locks): what may not happen at once in a project.
//! A lock is a project's (`deploy`) or every project's (`global:staging`),
//! held exclusive or shared (`db:shared`). A task's are its task tree's
//! (held as `task:<top-level task>`); a sync round, a merge and a trigger's
//! run hold theirs while they run.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

use reagent_store::{Lock, Project, Task};
use serde_json::{Value, json};

use crate::app::App;

/// A lock asked for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Spec {
    /// '' for a global one.
    pub project: String,
    pub name: String,
    pub shared: bool,
}

impl Spec {
    /// As it's written: `deploy`, `global:staging`, `db:shared`.
    pub fn label(&self) -> String {
        format!("{}{}{}", if self.project.is_empty() { "global:" } else { "" }, self.name, if self.shared { ":shared" } else { "" })
    }

    fn key(&self) -> (String, String, bool) {
        (self.project.clone(), self.name.clone(), self.shared)
    }

    fn is(&self, l: &Lock) -> bool {
        l.project == self.project && l.name == self.name
    }
}

/// `deploy`, `db:shared`, `global:staging`, `global:staging:shared`, in a project.
pub fn parse(project: &str, s: &str) -> Result<Spec, String> {
    let s = s.trim();
    let (global, rest) = match s.strip_prefix("global:") {
        Some(r) => (true, r),
        None => (false, s),
    };
    let (name, shared) = match rest.strip_suffix(":shared") {
        Some(n) => (n, true),
        None => (rest, false),
    };
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c)) {
        return Err(format!("{s:?} isn't a lock: a name (letters, digits, . _ - /), global: before it for every project's, :shared after it to share it"));
    }
    Ok(Spec { project: if global { String::new() } else { project.into() }, name: name.into(), shared })
}

pub fn parse_all(project: &str, list: &[String]) -> Result<Vec<Spec>, String> {
    let mut out: Vec<Spec> = vec![];
    for s in list.iter().filter(|s| !s.trim().is_empty()) {
        let l = parse(project, s)?;
        // The same lock twice: exclusive wins.
        match out.iter_mut().find(|o| o.project == l.project && o.name == l.name) {
            Some(o) => o.shared &= l.shared,
            None => out.push(l),
        }
    }
    Ok(out)
}

/// How a start that skipped (a busy lock, `skip`) begins its error.
pub const SKIPPED: &str = "skipped:";

/// What an action does when a lock it needs is busy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnBusy {
    Wait,
    Skip,
    Fail,
}

impl OnBusy {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim() {
            "wait" => Ok(OnBusy::Wait),
            "skip" => Ok(OnBusy::Skip),
            "fail" => Ok(OnBusy::Fail),
            o => Err(format!("on_busy {o:?}: wait, skip or fail")),
        }
    }
}

/// Which of the project's defaults a task started from this origin gets.
pub fn class(origin: &str, subtask: bool) -> &'static str {
    match origin.split(':').next().unwrap_or_default() {
        _ if subtask => "task",
        "mcp" => "agent",
        "task" => "task",
        "cron" => "cron",
        "trigger" => "trigger",
        _ => "person",
    }
}

/// Asked for, else the project's for the class, else wait (skip for cron and triggers).
pub fn on_busy(p: &Project, asked: Option<&str>, class: &str) -> Result<OnBusy, String> {
    match asked.filter(|s| !s.trim().is_empty()).or(p.locking.on_busy.get(class).map(String::as_str)) {
        Some(s) => OnBusy::parse(s),
        None if matches!(class, "cron" | "trigger") => Ok(OnBusy::Skip),
        None => Ok(OnBusy::Wait),
    }
}

/// Locks waited for and taken.
#[derive(Default)]
pub struct Locks {
    /// One take at a time (SQLite's transactions start deferred).
    take: tokio::sync::Mutex<()>,
    /// Woken whenever a lock is let go.
    freed: tokio::sync::Notify,
    /// Holders waiting in `wait_for`, and for what.
    waiting: Mutex<HashMap<String, Vec<Spec>>>,
}

pub fn task_holder(root: &str) -> String {
    format!("task:{root}")
}

/// The holder of a task's locks: its top-level task.
pub async fn holder_of(app: &App, t: &Task) -> String {
    let mut id = t.id.clone();
    let mut parent = t.parent.clone();
    // ponytail: one lookup per level; trees are shallow.
    while let Some(p) = parent {
        match app.store.task(&p).await.ok().flatten() {
            Some(pt) => {
                id = pt.id;
                parent = pt.parent;
            }
            None => break,
        }
    }
    task_holder(&id)
}

/// Who a holder is, for people: a task's title, a sync round, …
pub async fn describe(app: &App, holder: &str) -> String {
    match holder.split_once(':') {
        Some(("task", id)) => match app.store.task(id).await.ok().flatten() {
            Some(t) => format!("task {} ({})", t.name(), &id[..8.min(id.len())]),
            None => format!("task {id}"),
        },
        Some(("sync", w)) => format!("the git sync of {w}"),
        Some(("merge", w)) => format!("a merge in {w}"),
        Some(("trigger", w)) => format!("the trigger {w}'s run"),
        _ => holder.into(),
    }
}

/// "deploy, held by task Release (1a2b3c4d)": what's in the way.
async fn busy_text(app: &App, busy: &[Lock]) -> String {
    let mut out = vec![];
    for b in busy {
        let name = if b.project.is_empty() { format!("global:{}", b.name) } else { b.name.clone() };
        out.push(format!("{name}, held by {}", describe(app, &b.holder).await));
    }
    out.dedup();
    out.join("; ")
}

/// What another holder has in the way of these, without taking anything.
pub async fn in_the_way(app: &App, holder: &str, specs: &[Spec]) -> Result<Option<String>, String> {
    let mut busy = vec![];
    for s in specs {
        let rows = app.store.locks(Some(if s.project.is_empty() { "" } else { &s.project })).await.map_err(|e| e.to_string())?;
        busy.extend(rows.into_iter().filter(|l| s.is(l) && l.holder != holder && (!s.shared || !l.shared)));
    }
    Ok(if busy.is_empty() { None } else { Some(busy_text(app, &busy).await) })
}

/// Takes them all, or none and says what's in the way. The ones newly taken
/// (what the holder had already isn't) come back.
pub async fn take(app: &App, holder: &str, specs: &[Spec]) -> Result<Vec<Spec>, String> {
    match take_rows(app, holder, specs).await? {
        Ok(taken) => Ok(taken),
        Err(busy) => Err(busy_text(app, &busy).await),
    }
}

async fn take_rows(app: &App, holder: &str, specs: &[Spec]) -> Result<Result<Vec<Spec>, Vec<Lock>>, String> {
    if specs.is_empty() {
        return Ok(Ok(vec![]));
    }
    let _one = app.locks.take.lock().await;
    let want: Vec<_> = specs.iter().map(Spec::key).collect();
    Ok(match app.store.take_locks(holder, &want).await.map_err(|e| e.to_string())? {
        Ok(taken) => {
            if !taken.is_empty() {
                app.emit_locks();
            }
            Ok(taken.into_iter().map(|(project, name, shared)| Spec { project, name, shared }).collect())
        }
        Err(busy) => Err(busy),
    })
}

/// Takes them, waiting up to `timeout` for them to be free. A wait that would
/// close a cycle (a holder in the way waits, in the end, for this one) is refused.
pub async fn wait_for(app: &App, holder: &str, specs: &[Spec], timeout: Duration) -> Result<Vec<Spec>, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let result = loop {
        // Listening before trying: a release between the two isn't missed.
        let freed = app.locks.freed.notified();
        tokio::pin!(freed);
        freed.as_mut().enable();
        let busy = match take_rows(app, holder, specs).await {
            Ok(Ok(taken)) => break Ok(taken),
            Ok(Err(busy)) => busy,
            Err(e) => break Err(e),
        };
        if let Some(cycle) = cycle(app, holder, &busy).await {
            break Err(format!("waiting would deadlock: {cycle} waits for a lock this task holds; release one first"));
        }
        app.locks.waiting.lock().unwrap().insert(holder.into(), specs.to_vec());
        app.emit_locks();
        if tokio::time::timeout_at(deadline, freed).await.is_err() {
            break Err(format!("still busy after {} s: {}", timeout.as_secs(), busy_text(app, &busy).await));
        }
    };
    if app.locks.waiting.lock().unwrap().remove(holder).is_some() {
        app.emit_locks();
    }
    result
}

/// What each holder waits for: the ones in `wait_for`, and queued tasks with locks.
async fn waits(app: &App) -> HashMap<String, Vec<Spec>> {
    let mut w = app.locks.waiting.lock().unwrap().clone();
    for t in app.store.queued_tasks().await.unwrap_or_default() {
        if let Ok(specs) = parse_all(&t.project, &t.locks) {
            if !specs.is_empty() {
                w.entry(holder_of(app, &t).await).or_default().extend(specs);
            }
        }
    }
    w
}

/// A holder in the way that waits, through others, for `holder`: who.
async fn cycle(app: &App, holder: &str, busy: &[Lock]) -> Option<String> {
    let waits = waits(app).await;
    let all = app.store.locks(None).await.unwrap_or_default();
    let mut seen = HashSet::new();
    let mut stack: Vec<String> = busy.iter().map(|l| l.holder.clone()).collect();
    while let Some(h) = stack.pop() {
        if !seen.insert(h.clone()) {
            continue;
        }
        for s in waits.get(&h).into_iter().flatten() {
            for l in all.iter().filter(|l| s.is(l) && l.holder != h && (!s.shared || !l.shared)) {
                if l.holder == holder {
                    return Some(describe(app, &h).await);
                }
                stack.push(l.holder.clone());
            }
        }
    }
    None
}

pub async fn release(app: &App, holder: &str, specs: &[Spec]) {
    let mut any = false;
    for s in specs {
        any |= app.store.release_lock(holder, &s.project, &s.name).await.unwrap_or(false);
    }
    if any {
        freed(app);
    }
}

pub async fn release_holder(app: &App, holder: &str) {
    if app.store.release_holder(holder).await.unwrap_or(0) > 0 {
        freed(app);
    }
}

/// Wakes those waiting for a lock (one was let go).
pub fn freed(app: &App) {
    app.locks.freed.notify_waiters();
    app.emit_locks();
}

/// Takes the project's merge locks for a merge: waits for them (up to an
/// hour), or with `merge_on_busy = fail` refuses at once. Those newly taken.
pub async fn for_merge(app: &App, p: &Project, holder: &str) -> Result<Vec<Spec>, String> {
    let specs = parse_all(&p.slug, &p.locking.merge_locks)?;
    match p.locking.merge_on_busy.as_deref() {
        Some("fail") => take(app, holder, &specs).await.map_err(|busy| format!("not merged: a merge lock is busy: {busy}")),
        _ => wait_for(app, holder, &specs, Duration::from_secs(3600)).await.map_err(|e| format!("not merged: a merge lock: {e}")),
    }
}

/// Lets go of the locks of task trees that are over (whichever way they ended).
pub async fn sweep(app: &App) {
    let holders: HashSet<String> = app.store.locks(None).await.unwrap_or_default().into_iter().map(|l| l.holder).filter(|h| h.starts_with("task:")).collect();
    for h in holders {
        if !app.store.tree_active(&h["task:".len()..]).await.unwrap_or(true) {
            tracing::info!(holder = %h, "a task tree is over: its locks are let go");
            release_holder(app, &h).await;
        }
    }
}

/// The person breaks a lock: its holders lose it (a task is told).
pub async fn break_lock(app: &App, project: &str, name: &str) -> Result<Vec<String>, String> {
    let holders = app.store.break_lock(project, name).await.map_err(|e| e.to_string())?;
    if holders.is_empty() {
        return Err(format!("no one holds {}{name}", if project.is_empty() { "global:" } else { "" }));
    }
    freed(app);
    for h in &holders {
        if let Some(id) = h.strip_prefix("task:") {
            let _ = app.message(id, &format!("[reagent] the person broke your lock {}{name}: you don't hold it now (take it again with locks.lock_acquire if you need it).", if project.is_empty() { "global:" } else { "" })).await;
        }
    }
    Ok(holders)
}

/// A project's locks and the global ones: each with its holders and who waits.
pub async fn list(app: &App, project: &str) -> Result<Value, String> {
    let p = app.project(project).await?;
    let rows = app.store.locks(Some(&p.slug)).await.map_err(|e| e.to_string())?;
    let waits = waits(app).await;
    let mut names: Vec<(String, String)> = rows.iter().map(|l| (l.project.clone(), l.name.clone())).collect();
    names.extend(p.locking.locks.keys().map(|n| (p.slug.clone(), n.clone())));
    for s in waits.values().flatten() {
        if s.project == p.slug || s.project.is_empty() {
            names.push((s.project.clone(), s.name.clone()));
        }
    }
    names.sort();
    names.dedup();
    let mut out = vec![];
    for (proj, name) in names {
        let mut holders = vec![];
        for l in rows.iter().filter(|l| l.project == proj && l.name == name) {
            holders.push(json!({"holder": l.holder, "who": describe(app, &l.holder).await, "shared": l.shared, "since": l.since}));
        }
        let mut waiting = vec![];
        for (h, specs) in &waits {
            if let Some(s) = specs.iter().find(|s| s.project == proj && s.name == name) {
                waiting.push(json!({"holder": h, "who": describe(app, h).await, "shared": s.shared}));
            }
        }
        let label = if proj.is_empty() { format!("global:{name}") } else { name.clone() };
        out.push(json!({"lock": label, "global": proj.is_empty(), "name": name, "about": p.locking.locks.get(&name), "holders": holders, "waiting": waiting}));
    }
    Ok(json!(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_are_written_as_names_global_and_shared() {
        assert_eq!(parse("site", "deploy").unwrap(), Spec { project: "site".into(), name: "deploy".into(), shared: false });
        assert_eq!(parse("site", "global:staging:shared").unwrap(), Spec { project: String::new(), name: "staging".into(), shared: true });
        assert_eq!(parse("site", "db:shared").unwrap().label(), "db:shared");
        assert!(parse("site", "two words").is_err() && parse("site", ":shared").is_err());
        let all = parse_all("site", &["db:shared".into(), "db".into(), "".into()]).unwrap();
        assert_eq!(all.len(), 1);
        assert!(!all[0].shared, "asked for both: exclusive");
        assert_eq!((class("cron:4", false), class("mcp", false), class("ui", true), class("trigger:site/x", false)), ("cron", "agent", "task", "trigger"));
        let p = Project::new("site", "Site", "/s");
        assert_eq!((on_busy(&p, None, "cron").unwrap(), on_busy(&p, None, "person").unwrap(), on_busy(&p, Some("fail"), "cron").unwrap()), (OnBusy::Skip, OnBusy::Wait, OnBusy::Fail));
    }
}
