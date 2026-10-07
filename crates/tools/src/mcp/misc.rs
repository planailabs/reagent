//! Memory, skills, tasks (subtasks, other tasks' histories, cron) and asking.

use std::path::Path;
use std::sync::Arc;

use reagent_store::{Budget, Cron, CronOptions};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::service::RequestContext;
use rmcp::{RoleServer, schemars, tool, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::types::Json;

use super::{caller, more};
use crate::app::{App, StartTask};
use crate::edit::Edit;
use crate::memory::Memory;

#[derive(Clone)]
pub struct MemoryTools(pub Arc<App>);
#[derive(Clone)]
pub struct SkillTools(pub Arc<App>);
#[derive(Clone)]
pub struct TaskTools(pub Arc<App>);
#[derive(Clone)]
pub struct AskTools(pub Arc<App>);
#[derive(Clone)]
pub struct TodoTools(pub Arc<App>);
#[derive(Clone)]
pub struct SecretTools(pub Arc<App>);

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SecretName {
    pub name: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct TodoAdd {
    /// The items, in order (each a short line).
    pub items: Vec<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct TodoUpdate {
    pub id: i64,
    /// pending, in_progress, done or cancelled.
    pub status: Option<String>,
    /// A new text for it.
    pub text: Option<String>,
}

/// A todo list as text: `[ ] 1 …`, `[~]` in progress, `[x]` done, `[-]` cancelled.
pub fn todo_text(todos: &[reagent_store::Todo]) -> String {
    if todos.is_empty() {
        return "(the list is empty)".into();
    }
    let open = todos.iter().filter(|t| t.status == "pending" || t.status == "in_progress").count();
    let lines: Vec<String> = todos
        .iter()
        .map(|t| {
            let mark = match t.status.as_str() {
                "in_progress" => "[~]",
                "done" => "[x]",
                "cancelled" => "[-]",
                _ => "[ ]",
            };
            format!("{mark} {} {}", t.id, t.text)
        })
        .collect();
    format!("{}\n({open} open of {})", lines.join("\n"), todos.len())
}

#[derive(Deserialize, schemars::JsonSchema, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// What holds for all projects.
    Global,
    /// This project's.
    Project,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MemRead {
    pub scope: Scope,
    /// A file (topics/build.md); none: the index.
    pub file: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MemWrite {
    pub scope: Scope,
    /// topics/<name>.md, folders/<path>.md, or any .md path.
    pub file: String,
    pub text: String,
    /// One line saying what's in it, for the index.
    pub about: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MemEdit {
    pub scope: Scope,
    pub file: String,
    /// A new index line for it (default: it keeps its own).
    pub about: Option<String>,
    #[serde(flatten)]
    pub edit: Edit,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MemSearch {
    /// A regex (case-insensitive).
    pub pattern: String,
    /// Default: both.
    pub scope: Option<Scope>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MemFile {
    pub scope: Scope,
    pub file: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SkillName {
    pub name: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Spawn {
    pub title: String,
    /// What it should do: everything it needs to know (it doesn't see your conversation).
    pub prompt: String,
    /// Another project (by id; default: this one).
    pub project: Option<String>,
    pub profile: Option<String>,
    /// Skills loaded into its first message.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Limits: {tokens?, cost?, minutes?}.
    pub budget: Option<Value>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct TaskList {
    /// A project (default: this one); "all" for every project.
    pub project: Option<String>,
    /// Finished ones too.
    #[serde(default)]
    pub all: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct TaskMessage {
    pub task: String,
    pub text: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct TaskWait {
    pub task: String,
    /// Seconds (default 600, at most 3600).
    pub timeout: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Search {
    /// A regex (case-insensitive).
    pub pattern: String,
    /// One task (all of it, paged); none: every task of the project, a few hits each.
    pub task: Option<String>,
    /// A project (default: this one); "all" for every project.
    pub project: Option<String>,
    pub page: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CronAdd {
    /// Five fields: minute hour day month weekday (`0 3 * * *`, `*/30 * * * 1-5`).
    pub expr: String,
    pub title: String,
    pub prompt: String,
    /// A time zone (default UTC), like Europe/Vienna.
    pub tz: Option<String>,
    /// While the last run goes: skip (default), queue or parallel.
    pub overlap: Option<String>,
    pub profile: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CronId {
    pub id: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Ask {
    /// What you need to know, with what the person needs to decide.
    pub question: String,
    /// Answers to choose from (they can still write their own).
    #[serde(default)]
    pub options: Vec<String>,
}

async fn memory(app: &App, ctx: &RequestContext<RoleServer>, scope: Scope) -> Result<Memory, String> {
    let (_, p) = caller(app, ctx).await?;
    Ok(match scope {
        Scope::Global => app.global_memory(),
        Scope::Project => app.project_memory(&p),
    })
}

fn regex(p: &str) -> Result<regex::Regex, String> {
    regex::RegexBuilder::new(p).case_insensitive(true).size_limit(1 << 20).build().map_err(|e| format!("bad pattern: {e}"))
}

#[tool_router(server_handler)]
impl MemoryTools {
    #[tool(description = "Read a memory's index, or one of its files.")]
    async fn memory_read(&self, Parameters(a): Parameters<MemRead>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let m = memory(&self.0, &ctx, a.scope).await?;
        match a.file {
            Some(f) => m.read(&f),
            None => Ok(m.index()),
        }
    }

    #[tool(description = "Write a memory file (made or replaced), with a one-line `about` for the index.")]
    async fn memory_write(&self, Parameters(a): Parameters<MemWrite>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        memory(&self.0, &ctx, a.scope).await?.write(&a.file, &a.text, &a.about)
    }

    #[tool(description = "Change part of a memory file: replace (old → new), append, insert, delete (as fs.edit).")]
    async fn memory_edit(&self, Parameters(a): Parameters<MemEdit>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        memory(&self.0, &ctx, a.scope).await?.edit(&a.file, &a.edit, a.about.as_deref())
    }

    #[tool(description = "Search the memories' files for a regex: file, line, text.")]
    async fn memory_search(&self, Parameters(a): Parameters<MemSearch>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let re = regex(&a.pattern)?;
        let mut hits = vec![];
        for s in [Scope::Project, Scope::Global] {
            if a.scope.is_none_or(|x| x == s) {
                let name = if s == Scope::Global { "global" } else { "project" };
                for mut h in memory(&self.0, &ctx, s).await?.search(&re) {
                    h["scope"] = json!(name);
                    hits.push(h);
                }
            }
        }
        if hits.is_empty() {
            return Ok(format!("nothing matches {:?}", a.pattern));
        }
        Ok(hits.iter().map(|h| format!("{} {}:{}: {}", h["scope"].as_str().unwrap_or(""), h["file"].as_str().unwrap_or(""), h["line"], h["text"].as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n"))
    }

    #[tool(description = "Remove a memory file (and its index line).")]
    async fn memory_remove(&self, Parameters(a): Parameters<MemFile>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        memory(&self.0, &ctx, a.scope).await?.remove(&a.file)
    }
}

#[tool_router(server_handler)]
impl SkillTools {
    #[tool(description = "The skills you have: name, description, where it's from.")]
    async fn skill_list(&self, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let s = self.0.skills_for(Path::new(&t.cwd), &p);
        Ok(if s.is_empty() { "no skills (they live in .agents/skills/<name>/SKILL.md)".into() } else { s.iter().map(|s| format!("{} ({}) — {}", s.name, s.source, s.description)).collect::<Vec<_>>().join("\n") })
    }

    #[tool(description = "Load a skill: its instructions, its folder and the files in it (read them with fs.read, run its scripts with shell.exec).")]
    async fn skill_load(&self, Parameters(a): Parameters<SkillName>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let skills = self.0.skills_for(Path::new(&t.cwd), &p);
        let s = skills.iter().find(|s| s.name == a.name).ok_or_else(|| format!("no skill {:?} (skills.skill_list lists them)", a.name))?;
        let (body, files) = crate::skills::load(s)?;
        Ok(format!("# Skill {} (in {})\n\n{body}{}", s.name, s.dir.display(), if files.is_empty() { String::new() } else { format!("\n\nFiles: {}", files.join(", ")) }))
    }
}

fn task_line(t: &reagent_store::Task) -> String {
    format!("{}  {}  [{}]  {}{}", &t.id, t.title, t.state, t.project, t.parent.as_deref().map(|p| format!("  (subtask of {})", &p[..8.min(p.len())])).unwrap_or_default())
}

#[tool_router(server_handler)]
impl TaskTools {
    #[tool(description = "Start a subtask: another task working on its own (in this project, or another if the policy allows). Its report comes to you as a message when it ends.")]
    async fn task_spawn(&self, Parameters(a): Parameters<Spawn>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let budget: Option<Budget> = a.budget.map(serde_json::from_value).transpose().map_err(|e| format!("budget: {e}"))?;
        let sub = self
            .0
            .start_task(StartTask {
                project: a.project.unwrap_or(p.slug),
                title: a.title,
                prompt: a.prompt,
                profile: a.profile,
                budget,
                skills: a.skills,
                parent: Some(t.id.clone()),
                origin: Some(format!("task:{}", t.id)),
            })
            .await?;
        Ok(format!("started subtask {} ({}); its report comes as a message, tasks.task_wait waits for it", sub.id, sub.title))
    }

    #[tool(description = "Tasks: of this project (or another, or all), going on or (all = true) finished too.")]
    async fn task_list(&self, Parameters(a): Parameters<TaskList>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (_, p) = caller(&self.0, &ctx).await?;
        let project = match a.project.as_deref() {
            Some("all") => None,
            Some(x) => Some(x.to_string()),
            None => Some(p.slug),
        };
        let v = self.0.store.tasks(project.as_deref(), None, !a.all, 100).await.map_err(|e| e.to_string())?;
        Ok(if v.is_empty() { "no tasks".into() } else { v.iter().map(task_line).collect::<Vec<_>>().join("\n") })
    }

    #[tool(description = "Send a message to another task (one of your subtasks, say).")]
    async fn task_message(&self, Parameters(a): Parameters<TaskMessage>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, _) = caller(&self.0, &ctx).await?;
        self.0.message(&a.task, &format!("[from task {} ({})] {}", t.title, &t.id[..8], a.text)).await?;
        Ok("sent".into())
    }

    #[tool(description = "Wait until a task ends (or the timeout): its state and report.")]
    async fn task_wait(&self, Parameters(a): Parameters<TaskWait>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        caller(&self.0, &ctx).await?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(a.timeout.unwrap_or(600).clamp(1, 3600));
        let mut rx = self.0.events.subscribe();
        loop {
            let t = self.0.task(&a.task).await?;
            if !t.is_active() {
                return Ok(format!("{} [{}]\n{}", t.title, t.state, t.report.unwrap_or_default()));
            }
            if tokio::time::timeout_at(deadline, rx.recv()).await.is_err() {
                return Ok(format!("{} is still {}", t.title, t.state));
            }
        }
    }

    #[tool(description = "Search other tasks' whole conversations (the parts summarised away too) for a regex: in one task (paged), or across the project's (or all projects') tasks, a few hits each.")]
    async fn search_history(&self, Parameters(a): Parameters<Search>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (me, p) = caller(&self.0, &ctx).await?;
        let project = match a.project.as_deref() {
            Some("all") => None,
            Some(x) => Some(x.to_string()),
            None => Some(p.slug.clone()),
        };
        let v = self.0.search(&a.pattern, a.task.as_deref(), project.as_deref(), Some(&me.id), a.page.unwrap_or(1)).await?;
        let hit = |h: &Value| format!("#{} {}{}: {}", h["n"], h["role"].as_str().unwrap_or(""), if h["summarised"] == true { " (summarised)" } else { "" }, h["text"].as_str().unwrap_or("").chars().take(300).collect::<String>());
        if a.task.is_some() {
            let hits: Vec<String> = v["hits"].as_array().into_iter().flatten().map(hit).collect();
            let note = format!("{} matches in task {} ({}), page {} of {}{}", v["matches"], v["task"]["title"].as_str().unwrap_or(""), v["task"]["id"].as_str().unwrap_or(""), v["page"], v["pages"], v["next_page"].as_u64().map(|n| format!("; page: {n} for more")).unwrap_or_default());
            return Ok(format!("{}{}", hits.join("\n"), more(note)));
        }
        let tasks = v["tasks"].as_array().cloned().unwrap_or_default();
        if tasks.is_empty() {
            return Ok(format!("no other task's conversation matches {:?}", a.pattern));
        }
        let out: Vec<String> = tasks
            .iter()
            .map(|t| {
                let mut s = format!("task {} ({}, {}) — {} matches:", t["task"]["title"].as_str().unwrap_or(""), t["task"]["id"].as_str().unwrap_or(""), t["task"]["state"].as_str().unwrap_or(""), t["matches"]);
                for h in t["hits"].as_array().into_iter().flatten() {
                    s.push_str(&format!("\n  {}", hit(h)));
                }
                s
            })
            .collect();
        Ok(format!("{}{}", out.join("\n\n"), more(format!("{} tasks match; search_history(pattern, task: <id>) shows all of one", out.len()))))
    }

    #[tool(description = "This project's cron entries: schedule, task, next run.")]
    async fn cron_list(&self, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (_, p) = caller(&self.0, &ctx).await?;
        let v = self.0.store.crons(Some(&p.slug)).await.map_err(|e| e.to_string())?;
        Ok(if v.is_empty() {
            "no cron entries".into()
        } else {
            v.iter()
                .map(|c| format!("{}  `{}` {}  {}  {}next: {}", c.id, c.expr, c.tz, c.title, if c.enabled { "" } else { "(disabled) " }, c.next_run.and_then(|n| chrono::DateTime::from_timestamp(n, 0)).map(|d| d.to_rfc3339()).unwrap_or("-".into())))
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    #[tool(description = "Add a cron entry to this project: on its schedule a task starts with this title and prompt.")]
    async fn cron_add(&self, Parameters(a): Parameters<CronAdd>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (_, p) = caller(&self.0, &ctx).await?;
        let mut c = Cron {
            id: 0,
            project: p.slug.clone(),
            expr: a.expr,
            tz: a.tz.unwrap_or_else(|| "UTC".into()),
            title: a.title,
            prompt: a.prompt,
            options: Json(CronOptions { profile: a.profile, budget: None, skills: a.skills }),
            overlap: a.overlap.unwrap_or_else(|| "skip".into()),
            catch_up: true,
            enabled: true,
            last_run: None,
            next_run: None,
            queued: false,
        };
        crate::cron::prepare(&mut c, chrono::Utc::now().timestamp())?;
        let id = self.0.store.put_cron(&c).await.map_err(|e| e.to_string())?;
        Ok(format!("cron entry {id}; first run {}", c.next_run.and_then(|n| chrono::DateTime::from_timestamp(n, 0)).map(|d| d.to_rfc3339()).unwrap_or_default()))
    }

    #[tool(description = "Remove one of this project's cron entries.")]
    async fn cron_remove(&self, Parameters(a): Parameters<CronId>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (_, p) = caller(&self.0, &ctx).await?;
        let c = self.0.store.cron(a.id).await.map_err(|e| e.to_string())?.filter(|c| c.project == p.slug).ok_or_else(|| format!("no cron entry {} in this project", a.id))?;
        self.0.store.remove_cron(c.id).await.map_err(|e| e.to_string())?;
        Ok(format!("removed cron entry {}", c.id))
    }
}

#[tool_router(server_handler)]
impl AskTools {
    #[tool(description = "Ask the person and wait for the answer (they get a notification).")]
    async fn ask(&self, Parameters(a): Parameters<Ask>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, _) = caller(&self.0, &ctx).await?;
        self.0.ask(&t.id, &a.question, &a.options).await
    }
}

#[tool_router(server_handler)]
impl TodoTools {
    #[tool(description = "Your todo list: each item with its id and state ([ ] pending, [~] in progress, [x] done, [-] cancelled).")]
    async fn todo_list(&self, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, _) = caller(&self.0, &ctx).await?;
        Ok(todo_text(&self.0.store.todos(&t.id).await.map_err(|e| e.to_string())?))
    }

    #[tool(description = "Add items to your todo list (at the end): plan multi-step work here and keep it current. It's kept across restarts and summaries, and the person sees it.")]
    async fn todo_add(&self, Parameters(a): Parameters<TodoAdd>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, _) = caller(&self.0, &ctx).await?;
        let items: Vec<String> = a.items.iter().map(|i| i.trim().to_string()).filter(|i| !i.is_empty()).collect();
        if items.is_empty() {
            return Err("nothing to add".into());
        }
        self.0.store.add_todos(&t.id, &items).await.map_err(|e| e.to_string())?;
        self.0.todos_changed(&t.id).await;
        Ok(todo_text(&self.0.store.todos(&t.id).await.map_err(|e| e.to_string())?))
    }

    #[tool(description = "Change an item: its state (pending, in_progress, done, cancelled) and/or its text. Mark one in_progress when you start it, done when it's done.")]
    async fn todo_update(&self, Parameters(a): Parameters<TodoUpdate>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, _) = caller(&self.0, &ctx).await?;
        if let Some(s) = &a.status
            && !reagent_store::TODO_STATES.contains(&s.as_str())
        {
            return Err(format!("{s:?}: pending, in_progress, done or cancelled"));
        }
        if !self.0.store.update_todo(&t.id, a.id, a.status.as_deref(), a.text.as_deref()).await.map_err(|e| e.to_string())? {
            return Err(format!("there's no item {} (todo_list shows them)", a.id));
        }
        self.0.todos_changed(&t.id).await;
        Ok(todo_text(&self.0.store.todos(&t.id).await.map_err(|e| e.to_string())?))
    }

    #[tool(description = "Empty your todo list (to plan afresh).")]
    async fn todo_clear(&self, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, _) = caller(&self.0, &ctx).await?;
        self.0.store.clear_todos(&t.id).await.map_err(|e| e.to_string())?;
        self.0.todos_changed(&t.id).await;
        Ok("emptied".into())
    }
}

#[tool_router(server_handler)]
impl SecretTools {
    #[tool(description = "The secrets your commands get as environment variables (names, and whether they're the project's own or every project's). Values: secrets_get.")]
    async fn secrets_list(&self, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (_, p) = caller(&self.0, &ctx).await?;
        let s = self.0.store.secrets_for(&p.slug).await?;
        Ok(if s.is_empty() { "no secrets (the person sets them in reagent's web UI)".into() } else { s.iter().map(|s| format!("{} ({})", s.name, if s.project.is_some() { "this project's" } else { "every project's" })).collect::<Vec<_>>().join("\n") })
    }

    #[tool(description = "A secret's value. Your commands already get it as an environment variable (use $NAME there); a value appearing in other tool results is shown as ***.")]
    async fn secrets_get(&self, Parameters(a): Parameters<SecretName>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (_, p) = caller(&self.0, &ctx).await?;
        self.0.store.secrets_for(&p.slug).await?.into_iter().find(|s| s.name == a.name).map(|s| s.value).ok_or_else(|| format!("no secret {:?} (secrets_list lists them)", a.name))
    }
}
