//! The subnet cluster file reagent runs: one node, an agent type and a
//! mixture per model profile (`task-<profile>`), reagent's MCP servers and
//! hooks. Made from `reagent.hcl` at every start.

use crate::config::Config;

pub const PROMPT: &str = include_str!("../../../prompts/task.md");

/// The MCP servers a task has, with their tools that may be run again
/// after a restart (they change nothing).
pub const SERVERS: &[(&str, &[&str])] = &[
    ("fs", &["read", "grep", "glob", "ls"]),
    ("shell", &["jobs", "job_output"]),
    ("pty", &["pty_screen", "ptys"]),
    ("git", &["worktree_status", "worktree_diff"]),
    ("memory", &["memory_read", "memory_search"]),
    ("skills", &["skill_list", "skill_load"]),
    ("tasks", &["task_list", "cron_list"]),
    ("ask", &[]),
];

/// The env var the node reads the MCP servers' token from.
pub const TOKEN_ENV: &str = "REAGENT_MCP_TOKEN";

/// The mixture (what's spawned) for a profile.
pub fn mixture(profile: &str) -> String {
    format!("task-{profile}")
}

fn hcl_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out.replace("${", "$${").replace("%{", "%%{")
}

fn heredoc(s: &str) -> String {
    let body: String = s.replace("${", "$${").replace("%{", "%%{").lines().map(|l| format!("    {l}\n")).collect();
    format!("<<-EOT\n{body}  EOT")
}

/// The cluster file; `mcp_base` is where reagent serves its MCP servers
/// (`http://127.0.0.1:<port>`).
pub fn render(c: &Config, mcp_base: &str) -> String {
    let mut out = String::from("# Made by reagent from reagent.hcl at each start; edits here are lost.\nnode \"local\" {\n  capacity = 256\n}\n\n");
    let names: Vec<&str> = SERVERS.iter().map(|(n, _)| *n).collect();
    for (name, p) in &c.profile {
        let prov = &c.provider[&p.provider];
        let env = prov.key_env.as_deref().map(|e| format!("\n    env      = {}", hcl_str(e))).unwrap_or_default();
        let params = if p.params.is_empty() {
            String::new()
        } else {
            let kv: Vec<String> = p.params.iter().map(|(k, v)| format!("{k} = {}", match v {
                serde_json::Value::String(s) => hcl_str(s),
                other => other.to_string(),
            })).collect();
            format!("  params = {{ {} }}\n", kv.join(", "))
        };
        out.push_str(&format!(
            "agent \"model-{name}\" {{\n  description = {desc}\n  credential {{\n    base_url = {url}{env}\n  }}\n  model = {model}\n{params}  system_prompt = {prompt}\n  executor {{ internal = true }}\n  nodes = [\"local\"]\n  spawns = []\n{history}{grep}  hooks = [\"policy\", \"context\", \"checkpoint\"]\n  compact = {{ at_tokens = {at}, keep = 8 }}\n}}\n\nmixture {mix} {{\n  agent = \"model-{name}\"\n  mcp = [{mcps}]\n}}\n\n",
            desc = hcl_str(&format!("a reagent task on {}", p.model)),
            url = hcl_str(&prov.base_url),
            model = hcl_str(&p.model),
            prompt = heredoc(PROMPT),
            at = p.context * 3 / 4,
            history = if c.search_history { "  search_history = true\n" } else { "" },
            grep = {
                let g = p.grep_results.as_ref().unwrap_or(&c.grep_results);
                if g.over == 0 { String::new() } else { format!("  grep_results = {{ over = {}, except = [{}] }}\n", g.over, g.except.iter().map(|e| hcl_str(e)).collect::<Vec<_>>().join(", ")) }
            },
            mix = hcl_str(&mixture(name)),
            mcps = names.iter().map(|n| hcl_str(n)).collect::<Vec<_>>().join(", "),
        ));
    }
    for (name, idem) in SERVERS.iter().map(|(n, i)| (*n, *i)).chain(std::iter::once(("hooks", &["policy", "context", "checkpoint"][..]))) {
        out.push_str(&format!(
            "mcp {n} {{\n  url = {url}\n  credential = {{ header = \"Authorization\", env = \"{TOKEN_ENV}\", prefix = \"Bearer \" }}\n  nodes = [\"local\"]\n  idempotent = [{idem}]\n  lazy = false\n}}\n\n",
            n = hcl_str(name),
            url = hcl_str(&format!("{mcp_base}/mcp/{name}")),
            idem = idem.iter().map(|t| hcl_str(t)).collect::<Vec<_>>().join(", "),
        ));
    }
    out.push_str(
        "hook \"policy\" {\n  on = \"pre_tool\"\n  run { mcp = { server = \"hooks\", tool = \"policy\" } }\n  timeout = \"30s\"\n  on_lost = \"deny\"\n  idempotent = true\n}\n\n\
         hook \"context\" {\n  on = \"pre_model\"\n  run { mcp = { server = \"hooks\", tool = \"context\" } }\n  timeout = \"15s\"\n  on_lost = \"allow\"\n  idempotent = true\n}\n\n\
         hook \"checkpoint\" {\n  on = \"pre_compact\"\n  run { mcp = { server = \"hooks\", tool = \"checkpoint\" } }\n  timeout = \"15s\"\n  on_lost = \"allow\"\n  idempotent = true\n}\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_a_valid_cluster() {
        let mut c = Config::parse(crate::config::EXAMPLE).unwrap();
        c.profile.get_mut("default").unwrap().params.insert("temperature".into(), serde_json::json!(0.2));
        let text = render(&c, "http://127.0.0.1:9999");
        let spec = subnet_cluster::Cluster::parse(&[("cluster.hcl", &text)]).unwrap_or_else(|e| panic!("{e}\n{text}"));
        let a = &spec.agents["model-default"];
        assert_eq!(a.model, "deepseek-chat");
        assert_eq!(a.hooks, ["policy", "context", "checkpoint"]);
        assert!(a.search_history && a.system_prompt.contains("You are a task in reagent"));
        assert_eq!(spec.mixtures["task-default"].mcp.len(), SERVERS.len());
        assert_eq!(spec.mcps["fs"].url.as_deref(), Some("http://127.0.0.1:9999/mcp/fs"));
        assert!(spec.hooks.contains_key("checkpoint"));
        assert_eq!(a.grep_results.as_ref().map(|g| g.over()), Some(12000));
        // Configurable: globally, per profile, or off.
        c.grep_results.over = 0;
        c.profile.get_mut("default").unwrap().grep_results = Some(crate::config::GrepResults { over: 4000, except: vec!["fs.*".into()] });
        let spec = subnet_cluster::Cluster::parse(&[("cluster.hcl", &render(&c, "http://x"))]).unwrap();
        let g = spec.agents["model-default"].grep_results.clone().unwrap();
        assert_eq!((g.over(), g.except()), (4000, &["fs.*".to_string()][..]));
        c.profile.get_mut("default").unwrap().grep_results = None;
        c.search_history = false;
        let spec = subnet_cluster::Cluster::parse(&[("cluster.hcl", &render(&c, "http://x"))]).unwrap();
        assert!(spec.agents["model-default"].grep_results.is_none() && !spec.agents["model-default"].search_history);
    }
}
