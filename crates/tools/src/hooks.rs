//! The hooks subnet asks (served at `/mcp/hooks`; no mixture lists it, so
//! tasks never see these tools): `policy` before each tool call, `context`
//! before each model call, `checkpoint` before a compaction.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{schemars, tool, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::App;
use crate::policy::{self, Action, Call};

#[derive(Clone)]
pub struct HookTools(pub Arc<App>);

/// A hook's question, as subnet asks it.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct Question {
    pub agent: HookAgent,
    #[serde(default)]
    pub input: Value,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct HookAgent {
    pub id: String,
}

/// The command line a call runs, for the rules (shell and terminal tools).
pub fn command_of<'a>(tool: &str, args: &'a Value) -> Option<&'a str> {
    let key = match tool {
        "shell.exec" | "shell.exec_bg" => "cmd",
        "shell.job_input" => "text",
        "pty.pty_open" => "cmd",
        "pty.pty_send" => "keys",
        _ => return None,
    };
    args[key].as_str()
}

/// What a call is aimed at, for rules with a `target`: the project a
/// subtask goes to, the path a file tool touches.
pub fn target_of<'a>(tool: &str, args: &'a Value) -> Option<&'a str> {
    match tool {
        "tasks.task_spawn" => args["project"].as_str(),
        t if t.starts_with("fs.") => args["path"].as_str().or(args["pattern"].as_str()),
        _ => None,
    }
}

fn hash(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// What a task should have in view: the memory indexes, `AGENTS.md`, its
/// skills (by kind, so only what changed is shown again).
pub async fn context_parts(app: &App, task: &reagent_store::Task) -> Vec<(String, String)> {
    let mut parts = vec![];
    let Ok(p) = app.project(&task.project).await else { return parts };
    parts.push(("global".into(), format!("Global memory index ({}):\n\n{}", app.global_memory().dir.display(), app.global_memory().index())));
    let pm = app.project_memory(&p);
    parts.push(("project".into(), format!("Project memory index ({}):\n\n{}", pm.dir.display(), pm.index())));
    for dir in [Path::new(&task.cwd), Path::new(&p.path)] {
        if let Ok(text) = std::fs::read_to_string(dir.join("AGENTS.md")) {
            parts.push(("agents".into(), format!("The project's AGENTS.md ({}):\n\n{}", dir.join("AGENTS.md").display(), text.chars().take(20_000).collect::<String>())));
            break;
        }
    }
    // The secrets its commands get (names only).
    if let Ok(s) = app.store.secrets_for(&p.slug).await
        && !s.is_empty()
    {
        parts.push(("secrets".into(), format!("Environment variables your commands get (secrets; secrets.secrets_get reads one; values in tool results show as ***): {}", s.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "))));
    }
    // The todo list, so a summary or a restart never loses the plan.
    let todos = app.store.todos(&task.id).await.unwrap_or_default();
    if !todos.is_empty() {
        parts.push(("todos".into(), format!("Your todo list (todo.todo_update keeps it current):\n{}", crate::mcp::misc::todo_text(&todos))));
    }
    let skills = app.skills_for(Path::new(&task.cwd), &p);
    if !skills.is_empty() {
        parts.push(("skills".into(), format!("Skills you have (load one with skills.skill_load):\n{}", crate::skills::listing(&skills))));
    }
    parts
}

#[tool_router(server_handler)]
impl HookTools {
    #[tool(description = "pre_tool: the project's policy for this call (allow, ask, deny).")]
    async fn policy(&self, Parameters(q): Parameters<Question>) -> Result<String, String> {
        let t = self.0.task_of_agent(&q.agent.id).await?;
        let p = self.0.project(&t.project).await?;
        let rules = self.0.store.rules(&p.slug).await.map_err(|e| e.to_string())?;
        let tool = q.input["tool"].as_str().unwrap_or_default().replace("__", ".");
        let args = &q.input["args"];
        let default = Action::parse(&p.default_action).unwrap_or(Action::Ask);
        // Subnet's own tools (search_history, grep_result, …) read the task's own history.
        if !tool.contains('.') {
            return Ok(json!({"decision": "allow"}).to_string());
        }
        // A subtask without a project is in this one.
        let target = target_of(&tool, args).or((tool == "tasks.task_spawn").then_some(p.slug.as_str()));
        let d = policy::decide(&rules, default, &Call { tool: &tool, command: command_of(&tool, args), target });
        let why = match (&d.rule, &d.piece) {
            (Some(r), Some(piece)) => format!("the rule {} {}{} (for `{piece}`)", r.tool, r.command.as_deref().map(|c| format!("`{c}` ")).unwrap_or_default(), r.action),
            (Some(r), None) => format!("the rule {} {}{}", r.tool, r.command.as_deref().map(|c| format!("`{c}` ")).unwrap_or_default(), r.action),
            (None, _) => format!("the project's default ({})", p.default_action),
        };
        Ok(match d.action {
            Action::Allow => json!({"decision": "allow"}),
            Action::Ask => json!({"decision": "ask", "reason": format!("needs approval: {why}")}),
            Action::Deny => json!({"decision": "deny", "reason": format!("not allowed in {}: {why}", p.name)}),
        }
        .to_string())
    }

    #[tool(description = "post_tool: secret values in a result become *** (the secrets tools' own results stay).")]
    async fn mask(&self, Parameters(q): Parameters<Question>) -> Result<String, String> {
        let tool = q.input["tool"].as_str().unwrap_or_default().replace("__", ".");
        let Ok(t) = self.0.task_of_agent(&q.agent.id).await else {
            return Ok(json!({"decision": "allow"}).to_string());
        };
        let result = q.input["result"].as_str().unwrap_or_default();
        if tool.starts_with("secrets.") || result.is_empty() {
            return Ok(json!({"decision": "allow"}).to_string());
        }
        let masked = self.0.mask(&t.project, result).await;
        Ok(if masked == result { json!({"decision": "allow"}) } else { json!({"decision": "rewrite", "text": masked}) }.to_string())
    }

    #[tool(description = "pre_model: the memory indexes, AGENTS.md and the skills list, when they're new to the task.")]
    async fn context(&self, Parameters(q): Parameters<Question>) -> Result<String, String> {
        let Ok(t) = self.0.task_of_agent(&q.agent.id).await else {
            return Ok(json!({"decision": "allow"}).to_string());
        };
        // Over budget: paused before this call (and the person hears it).
        self.0.check_budget(&t).await;
        let parts = context_parts(&self.0, &t).await;
        let mut inject = vec![];
        {
            let mut shown = self.0.shown.lock().unwrap();
            let seen: &mut HashMap<String, u64> = shown.entry(t.id.clone()).or_default();
            for (kind, text) in parts {
                let h = hash(&text);
                if seen.get(&kind) != Some(&h) {
                    seen.insert(kind, h);
                    inject.push(text);
                }
            }
        }
        Ok(json!({"decision": "allow", "inject": inject}).to_string())
    }

    #[tool(description = "pre_compact: how to write the summary (it becomes a checkpoint in the project's memory).")]
    async fn checkpoint(&self, Parameters(_q): Parameters<Question>) -> Result<String, String> {
        Ok(json!({"decision": "allow", "text": "Write the summary as a checkpoint someone could pick the work up from: the goal, what's done (with branches, commits, files), what's next, decisions made and why, open questions, and commands that matter (how to build, test, run). It is also kept in the project's memory."}).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_and_targets_of_calls() {
        let a = json!({"cmd": "cargo test", "keys": "ls<enter>", "project": "site", "path": "src/a.rs"});
        assert_eq!(command_of("shell.exec", &a), Some("cargo test"));
        assert_eq!(command_of("pty.pty_send", &a), Some("ls<enter>"));
        assert_eq!(command_of("fs.read", &a), None);
        assert_eq!(target_of("tasks.task_spawn", &a), Some("site"));
        assert_eq!(target_of("fs.write", &a), Some("src/a.rs"));
    }
}
