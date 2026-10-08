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
    // worktree_merge and ask run again after a restart: a merge already made is
    // "nothing to merge", and a stored question or decision is found again.
    ("git", &["worktree_status", "worktree_diff", "worktree_merge"]),
    ("memory", &["memory_read", "memory_search"]),
    ("skills", &["skill_list", "skill_load"]),
    ("tasks", &["task_list", "cron_list", "prompt_design"]),
    ("ask", &["ask"]),
    ("todo", &["todo_list"]),
    ("secrets", &["secrets_list", "secrets_get"]),
    ("triggers", &["trigger_list"]),
];

/// The env var the node reads the MCP servers' token from.
pub const TOKEN_ENV: &str = "REAGENT_MCP_TOKEN";

/// The mixture (what's spawned) for a profile.
pub fn mixture(profile: &str) -> String {
    format!("task-{profile}")
}

/// The mixture of a project with servers of its own.
pub fn project_mixture(profile: &str, project: &str) -> String {
    format!("task-{profile}--{project}")
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

/// Names reagent's own servers take.
pub fn reserved(name: &str) -> bool {
    name == "hooks" || SERVERS.iter().any(|(n, _)| *n == name)
}

/// Checks a server the person adds.
pub fn check_mcp(m: &reagent_store::McpServer) -> Result<(), String> {
    if m.name.is_empty() || m.name.len() > 40 || !m.name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
        return Err(format!("{:?}: a server's name is lowercase letters, digits, - and _ (its tools are <name>.<tool>)", m.name));
    }
    if reserved(&m.name) {
        return Err(format!("{:?} is one of reagent's own servers", m.name));
    }
    match (&m.url, &m.command) {
        (Some(u), None) if u.starts_with("http://") || u.starts_with("https://") => {}
        (Some(_), None) => return Err("url: http:// or https://".into()),
        (None, Some(c)) if !c.0.is_empty() && !c.0[0].trim().is_empty() => {}
        _ => return Err("a server has a url (streamable HTTP) or a command (stdio): one of them".into()),
    }
    if m.credential.is_some() && m.url.is_none() {
        return Err("a credential (a header) is for url servers; a command gets env".into());
    }
    if let Some(c) = &m.credential
        && (c.0.env.trim().is_empty() || c.0.header.trim().is_empty())
    {
        return Err("a credential needs the header and the environment variable holding its value".into());
    }
    Ok(())
}

fn custom_block(m: &reagent_store::McpServer) -> String {
    let mut b = format!("mcp {} {{\n", hcl_str(&m.name));
    if !m.description.is_empty() {
        b.push_str(&format!("  description = {}\n", hcl_str(&m.description)));
    }
    if let Some(u) = &m.url {
        b.push_str(&format!("  url = {}\n", hcl_str(u)));
    }
    if let Some(cmd) = &m.command {
        b.push_str(&format!("  command = [{}]\n", cmd.0.iter().map(|a| hcl_str(a)).collect::<Vec<_>>().join(", ")));
    }
    if !m.env.0.is_empty() {
        // `$VAR` stays as it is: the node resolves it from reagent's environment.
        let kv: Vec<String> = m.env.0.iter().map(|(k, v)| format!("{} = {}", hcl_str(k), hcl_str(v).replace("$${", "${"))).collect();
        b.push_str(&format!("  env = {{ {} }}\n", kv.join(", ")));
    }
    if let Some(c) = &m.credential {
        b.push_str(&format!("  credential = {{ header = {}, env = {}, prefix = {} }}\n", hcl_str(&c.0.header), hcl_str(&c.0.env), hcl_str(&c.0.prefix)));
    }
    b.push_str(&format!("  nodes = [\"local\"]\n  idempotent = [{}]\n  lazy = {}\n}}\n\n", m.idempotent.0.iter().map(|t| hcl_str(t)).collect::<Vec<_>>().join(", "), m.lazy));
    b
}

/// The cluster file; `mcp_base` is where reagent serves its MCP servers
/// (`http://127.0.0.1:<port>`); `custom` are the servers the person added
/// (the enabled ones are declared; those in `join` are in every task's
/// mixture: a server that doesn't start must not block every task).
pub fn render(c: &Config, mcp_base: &str, custom: &[reagent_store::McpServer], join: &[String]) -> String {
    let custom: Vec<&reagent_store::McpServer> = custom.iter().filter(|m| m.enabled && check_mcp(m).is_ok()).collect();
    let mut out = String::from("# Made by reagent from reagent.hcl at each start; edits here are lost.\nnode \"local\" {\n  capacity = 256\n}\n\n");
    let joined = |m: &&&reagent_store::McpServer| join.iter().any(|j| *j == m.name);
    let names: Vec<&str> = SERVERS.iter().map(|(n, _)| *n).chain(custom.iter().filter(joined).filter(|m| m.project.is_none()).map(|m| m.name.as_str())).collect();
    // Projects with servers of their own (running ones): a mixture each, per profile.
    let mut projects: Vec<&str> = custom.iter().filter(joined).filter_map(|m| m.project.as_deref()).collect();
    projects.sort();
    projects.dedup();
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
            "agent \"model-{name}\" {{\n  description = {desc}\n  credential {{\n    base_url = {url}{env}\n  }}\n  model = {model}\n{params}  system_prompt = {prompt}\n  executor {{ internal = true }}\n  nodes = [\"local\"]\n  spawns = []\n{history}{grep}  hooks = [\"policy\", \"mask\", \"context\", \"checkpoint\"]\n  compact = {{ at_tokens = {at}, keep = 8 }}\n}}\n\nmixture {mix} {{\n  agent = \"model-{name}\"\n  mcp = [{mcps}]\n}}\n\n",
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
        for proj in &projects {
            let own = custom.iter().filter(joined).filter(|m| m.project.as_deref() == Some(proj)).map(|m| m.name.as_str());
            let all: Vec<String> = names.iter().copied().chain(own).map(hcl_str).collect();
            out.push_str(&format!("mixture {} {{\n  agent = \"model-{name}\"\n  mcp = [{}]\n}}\n\n", hcl_str(&project_mixture(name, proj)), all.join(", ")));
        }
    }
    for (name, idem) in SERVERS.iter().map(|(n, i)| (*n, *i)).chain(std::iter::once(("hooks", &["policy", "mask", "context", "checkpoint"][..]))) {
        out.push_str(&format!(
            "mcp {n} {{\n  url = {url}\n  credential = {{ header = \"Authorization\", env = \"{TOKEN_ENV}\", prefix = \"Bearer \" }}\n  nodes = [\"local\"]\n  idempotent = [{idem}]\n  lazy = false\n}}\n\n",
            n = hcl_str(name),
            url = hcl_str(&format!("{mcp_base}/mcp/{name}")),
            idem = idem.iter().map(|t| hcl_str(t)).collect::<Vec<_>>().join(", "),
        ));
    }
    for m in &custom {
        out.push_str(&custom_block(m));
    }
    out.push_str(
        "hook \"policy\" {\n  on = \"pre_tool\"\n  run { mcp = { server = \"hooks\", tool = \"policy\" } }\n  timeout = \"30s\"\n  on_lost = \"deny\"\n  idempotent = true\n}\n\n\
         hook \"context\" {\n  on = \"pre_model\"\n  run { mcp = { server = \"hooks\", tool = \"context\" } }\n  timeout = \"15s\"\n  on_lost = \"allow\"\n  idempotent = true\n}\n\n\
         hook \"mask\" {\n  on = \"post_tool\"\n  run { mcp = { server = \"hooks\", tool = \"mask\" } }\n  timeout = \"15s\"\n  on_lost = \"allow\"\n  idempotent = true\n}\n\n\
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
        let text = render(&c, "http://127.0.0.1:9999", &[], &[]);
        let spec = subnet_cluster::Cluster::parse(&[("cluster.hcl", &text)]).unwrap_or_else(|e| panic!("{e}\n{text}"));
        let a = &spec.agents["model-default"];
        assert_eq!(a.model, "deepseek-chat");
        assert_eq!(a.hooks, ["policy", "mask", "context", "checkpoint"]);
        assert!(a.search_history && a.system_prompt.contains("You are a task in reagent"));
        assert_eq!(spec.mixtures["task-default"].mcp.len(), SERVERS.len());
        assert_eq!(spec.mcps["fs"].url.as_deref(), Some("http://127.0.0.1:9999/mcp/fs"));
        assert!(spec.hooks.contains_key("checkpoint"));
        assert_eq!(a.grep_results.as_ref().map(|g| g.over()), Some(12000));
        // Configurable: globally, per profile, or off.
        c.grep_results.over = 0;
        c.profile.get_mut("default").unwrap().grep_results = Some(crate::config::GrepResults { over: 4000, except: vec!["fs.*".into()] });
        let spec = subnet_cluster::Cluster::parse(&[("cluster.hcl", &render(&c, "http://x", &[], &[]))]).unwrap();
        let g = spec.agents["model-default"].grep_results.clone().unwrap();
        assert_eq!((g.over(), g.except()), (4000, &["fs.*".to_string()][..]));
        c.profile.get_mut("default").unwrap().grep_results = None;
        c.search_history = false;
        let spec = subnet_cluster::Cluster::parse(&[("cluster.hcl", &render(&c, "http://x", &[], &[]))]).unwrap();
        assert!(spec.agents["model-default"].grep_results.is_none() && !spec.agents["model-default"].search_history);
    }

    fn server(v: serde_json::Value) -> reagent_store::McpServer {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn added_servers_join_every_task_lazily_or_not() {
        let c = Config::parse(crate::config::EXAMPLE).unwrap();
        let web = server(serde_json::json!({"name": "web", "description": "search the \"web\"", "url": "https://mcp.example/mcp", "credential": {"env": "WEB_TOKEN", "prefix": "Bearer "}, "idempotent": ["search"]}));
        let calc = server(serde_json::json!({"name": "calc", "command": ["calc-mcp", "--precise"], "env": {"KEY": "$CALC_KEY", "MODE": "x"}, "lazy": false}));
        let off = server(serde_json::json!({"name": "off", "url": "https://off/mcp", "enabled": false}));
        let all = [web, calc, off];
        let text = render(&c, "http://127.0.0.1:1", &all, &[]);
        let spec = subnet_cluster::Cluster::parse(&[("cluster.hcl", &text)]).unwrap();
        assert!(spec.mcps.contains_key("web") && !spec.mixtures["task-default"].mcp.contains(&"web".to_string()), "declared, not yet joined");
        let text = render(&c, "http://127.0.0.1:1", &all, &["web".into(), "calc".into(), "off".into()]);
        let spec = subnet_cluster::Cluster::parse(&[("cluster.hcl", &text)]).unwrap_or_else(|e| panic!("{e}\n{text}"));
        let mix = &spec.mixtures["task-default"].mcp;
        assert!(mix.contains(&"web".to_string()) && mix.contains(&"calc".to_string()) && !mix.contains(&"off".to_string()), "{mix:?}");
        let w = &spec.mcps["web"];
        assert!(w.lazy && w.idempotent == ["search"]);
        assert_eq!(w.credential.as_ref().unwrap().env, "WEB_TOKEN");
        let k = &spec.mcps["calc"];
        assert!(!k.lazy);
        assert_eq!(k.command.as_ref().unwrap(), &["calc-mcp", "--precise"]);
        assert_eq!(k.env["KEY"], "$CALC_KEY", "an env reference stays for the node");
        assert!(!spec.mcps.contains_key("off"));
        assert!(!spec.mcps["fs"].lazy, "reagent's own stay eager");
    }

    #[test]
    fn a_projects_own_servers_go_to_its_tasks_only() {
        let c = Config::parse(crate::config::EXAMPLE).unwrap();
        let all = [
            server(serde_json::json!({"name": "web", "url": "https://w/mcp"})),
            server(serde_json::json!({"name": "db", "url": "https://d/mcp", "project": "site"})),
            server(serde_json::json!({"name": "cms", "url": "https://c/mcp", "project": "site"})),
            server(serde_json::json!({"name": "docs-search", "url": "https://s/mcp", "project": "docs"})),
        ];
        let join: Vec<String> = ["web", "db", "cms"].iter().map(|s| s.to_string()).collect();
        let spec = subnet_cluster::Cluster::parse(&[("cluster.hcl", &render(&c, "http://x", &all, &join))]).unwrap();
        let default = &spec.mixtures["task-default"].mcp;
        assert!(default.contains(&"web".into()) && !default.contains(&"db".into()), "{default:?}");
        let site = &spec.mixtures["task-default--site"].mcp;
        assert!(site.contains(&"fs".into()) && site.contains(&"web".into()) && site.contains(&"db".into()) && site.contains(&"cms".into()), "{site:?}");
        assert!(!spec.mixtures.contains_key("task-default--docs"), "its server didn't start: no mixture");
        assert!(spec.mcps.contains_key("docs-search"), "but it's declared (to see whether it runs)");
    }

    #[test]
    fn bad_servers_are_named() {
        let e = |v| check_mcp(&server(v)).unwrap_err();
        assert!(e(serde_json::json!({"name": "fs", "url": "https://x"})).contains("reagent's own"));
        assert!(e(serde_json::json!({"name": "Big Name", "url": "https://x"})).contains("lowercase"));
        assert!(e(serde_json::json!({"name": "a"})).contains("one of them"));
        assert!(e(serde_json::json!({"name": "a", "url": "ftp://x"})).contains("http"));
        assert!(e(serde_json::json!({"name": "a", "command": ["x"], "credential": {"env": "T"}})).contains("url servers"));
        assert!(check_mcp(&server(serde_json::json!({"name": "ok-1", "command": ["x"]}))).is_ok());
    }
}
