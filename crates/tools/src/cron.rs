//! Cron: per project, a schedule (with a time zone) that starts a task.
//! While the last run still goes, a new one is skipped (`skip`), waits for
//! it (`queue`) or runs beside it (`parallel`). A run missed while reagent
//! was down runs once at start (`catch_up`).

use std::str::FromStr;
use std::sync::Arc;

use reagent_store::{Cron, Task};

use crate::app::{App, StartTask};

/// The next run after `after` (unix seconds), in the entry's time zone.
pub fn next_run(expr: &str, tz: &str, after: i64) -> Result<i64, String> {
    let cron = croner::Cron::from_str(expr).map_err(|e| format!("{expr:?}: {e}"))?;
    let tz: chrono_tz::Tz = tz.parse().map_err(|_| format!("{tz:?} isn't a time zone (like Europe/Vienna or UTC)"))?;
    let start = chrono::DateTime::from_timestamp(after, 0).ok_or("a bad time")?.with_timezone(&tz);
    let next = cron.find_next_occurrence(&start, false).map_err(|e| e.to_string())?;
    Ok(next.timestamp())
}

/// Checks an entry and sets its next run.
pub fn prepare(c: &mut Cron, now: i64) -> Result<(), String> {
    if !matches!(c.overlap.as_str(), "skip" | "queue" | "parallel") {
        return Err("overlap: skip, queue or parallel".into());
    }
    if c.title.trim().is_empty() || c.prompt.trim().is_empty() {
        return Err("a cron entry needs a title and a prompt".into());
    }
    c.next_run = Some(next_run(&c.expr, &c.tz, now)?);
    Ok(())
}

/// What's due at `now`: run it, or (missed long ago, without catch_up) skip it.
fn due(c: &Cron, now: i64) -> Option<bool> {
    let next = c.next_run?;
    if next > now {
        return None;
    }
    // More than two minutes late: reagent wasn't running then.
    let missed = now - next > 120;
    Some(!missed || c.catch_up)
}

async fn active_run(app: &App, c: &Cron) -> bool {
    let origin = format!("cron:{}", c.id);
    app.store.tasks(Some(&c.project), None, true, 1000).await.unwrap_or_default().iter().any(|t| t.origin == origin)
}

/// Starts a run now.
pub async fn run(app: &App, c: &Cron) -> Result<Task, String> {
    let o = &c.options.0;
    app.start_task(StartTask {
        project: c.project.clone(),
        title: c.title.clone(),
        prompt: c.prompt.clone(),
        profile: o.profile.clone(),
        budget: o.budget.clone(),
        skills: o.skills.clone(),
        parent: None,
        origin: Some(format!("cron:{}", c.id)),
        kind: o.kind.clone(),
    })
    .await
}

/// One pass over the entries.
pub async fn tick(app: &App, now: i64) {
    let Ok(crons) = app.store.crons(None).await else { return };
    for mut c in crons.into_iter().filter(|c| c.enabled) {
        if c.next_run.is_none() {
            if let Err(e) = prepare(&mut c, now) {
                tracing::warn!(cron = c.id, error = %e, "a cron entry that can't run");
                continue;
            }
            let _ = app.store.put_cron(&c).await;
            continue;
        }
        let Some(go) = due(&c, now) else { continue };
        if go {
            let busy = c.overlap != "parallel" && active_run(app, &c).await;
            if busy && c.overlap == "queue" {
                c.queued = true;
            } else if !busy {
                match run(app, &c).await {
                    Ok(t) => tracing::info!(cron = c.id, task = %t.id, "cron run started"),
                    Err(e) => {
                        tracing::warn!(cron = c.id, error = %e, "cron run didn't start");
                        app.notify("cron", None, &format!("{} didn't start", c.title), &e).await;
                    }
                }
                c.last_run = Some(now);
            }
        }
        c.next_run = next_run(&c.expr, &c.tz, now).ok();
        let _ = app.store.put_cron(&c).await;
    }
}

/// A task ended: a run queued behind it goes now.
pub async fn task_ended(app: &App, t: &Task) {
    let Some(id) = t.origin.strip_prefix("cron:").and_then(|i| i.parse::<i64>().ok()) else { return };
    let Ok(Some(mut c)) = app.store.cron(id).await else { return };
    if c.queued && c.enabled {
        c.queued = false;
        c.last_run = Some(chrono::Utc::now().timestamp());
        let _ = app.store.put_cron(&c).await;
        if let Err(e) = run(app, &c).await {
            app.notify("cron", None, &format!("{} didn't start", c.title), &e).await;
        }
    }
}

/// Runs the schedule for as long as reagent runs.
pub fn schedule(app: Arc<App>) {
    tokio::spawn(async move {
        loop {
            tick(&app, chrono::Utc::now().timestamp()).await;
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_runs_in_a_time_zone() {
        // 2026-01-01 00:00 UTC: 03:00 in Vienna is 02:00 UTC.
        let t0 = 1_767_225_600;
        assert_eq!(next_run("0 3 * * *", "Europe/Vienna", t0).unwrap(), t0 + 2 * 3600);
        assert_eq!(next_run("0 3 * * *", "UTC", t0).unwrap(), t0 + 3 * 3600);
        assert_eq!(next_run("*/15 * * * *", "UTC", t0).unwrap(), t0 + 900, "after, not at");
        assert!(next_run("nope", "UTC", t0).is_err());
        assert!(next_run("0 3 * * *", "Mars/Olympus", t0).unwrap_err().contains("time zone"));
    }

    #[test]
    fn missed_runs_catch_up_once_or_not() {
        let mut c: Cron = serde_json::from_value(serde_json::json!({"expr": "0 3 * * *", "title": "t", "prompt": "p"})).unwrap();
        c.next_run = Some(1000);
        assert_eq!(due(&c, 999), None);
        assert_eq!(due(&c, 1030), Some(true), "on time");
        assert_eq!(due(&c, 90_000), Some(true), "missed, caught up");
        c.catch_up = false;
        assert_eq!(due(&c, 90_000), Some(false), "missed, skipped");
        c.overlap = "sometimes".into();
        assert!(prepare(&mut c, 0).is_err());
    }
}
