use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "reagent", about = "A resumable coding agent on subagent-net")]
struct Cli {
    /// The data folder (default: REAGENT_DATA, else the platform's).
    #[arg(long, global = true)]
    data: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run reagent: the web interface, the tasks, cron.
    Up {
        /// Where the web interface listens (default: reagent.hcl's listen).
        #[arg(long)]
        listen: Option<String>,
    },
    /// The process owning commands and terminals (started by `up` when needed).
    Supervisor {
        /// Stop the running supervisor and everything it runs.
        #[arg(long)]
        stop: bool,
    },
    /// Set the web interface's password (ends every session).
    Passwd {
        /// Read it from stdin (one line) instead of asking.
        #[arg(long)]
        password_stdin: bool,
    },
    /// Whether the supervisor runs, and its jobs.
    Status,
    /// MCP servers every task gets (lazily, unless --eager); a running reagent applies changes within seconds.
    Mcp {
        #[command(subcommand)]
        cmd: McpCmd,
    },
    /// Secrets tasks' commands get as environment variables (every project's, or one project's with --project).
    Secret {
        #[command(subcommand)]
        cmd: SecretCmd,
    },
    /// Triggers: scripts that watch something and start tasks (a running reagent picks changes up within a second).
    Trigger {
        #[command(subcommand)]
        cmd: TriggerCmd,
    },
    /// Tasks: design a task's prompt with the prompt designer.
    Task {
        #[command(subcommand)]
        cmd: TaskCmd,
    },
    /// Prompt templates: the global ones, or a project's (its repo's, kept in reagent, global, plugins').
    Template {
        #[command(subcommand)]
        cmd: TemplateCmd,
    },
    /// Plugins: git repositories (in <data>/plugins/<id>/) whose skills and rules every task gets.
    Plugin {
        #[command(subcommand)]
        cmd: PluginCmd,
    },
    /// Services: commands of a project that should always run (started, checked, restarted, repaired).
    Service {
        #[command(subcommand)]
        cmd: ServiceCmd,
    },
    /// Git sync: a project's branches pulled (with their strategy) and pushed on an interval.
    Sync {
        #[command(subcommand)]
        cmd: SyncCmd,
    },
    /// Persistent worktrees: a project's named, long-lived checkouts that tasks start in.
    Worktree {
        #[command(subcommand)]
        cmd: WorktreeCmd,
    },
    /// API tokens for reagent's MCP API (`/mcp`), which other agents use.
    Token {
        #[command(subcommand)]
        cmd: TokenCmd,
    },
}

#[derive(Subcommand)]
enum McpCmd {
    /// Add (or change) a server: a URL (streamable HTTP) or a command (stdio, after --).
    Add {
        name: String,
        #[arg(long)]
        url: Option<String>,
        /// The environment variable holding a header's value (a token), for a URL server.
        #[arg(long)]
        header_env: Option<String>,
        #[arg(long, default_value = "Authorization")]
        header: String,
        /// What goes before the value (e.g. "Bearer ").
        #[arg(long, default_value = "")]
        prefix: String,
        /// KEY=VALUE for a command's environment (`$VAR` takes reagent's).
        #[arg(long = "env")]
        env: Vec<String>,
        /// Offer its tools' schemas from the start (lazy = false).
        #[arg(long)]
        eager: bool,
        /// Tools that may run again after a restart (they change nothing).
        #[arg(long, value_delimiter = ',')]
        idempotent: Vec<String>,
        #[arg(long, default_value = "")]
        description: String,
        /// Added but not given to tasks.
        #[arg(long)]
        disabled: bool,
        /// Only this project's tasks get it (default: every task).
        #[arg(long)]
        project: Option<String>,
        /// The command and its arguments.
        #[arg(last = true)]
        command: Vec<String>,
    },
    List,
    Remove { name: String },
}

#[derive(Subcommand)]
enum SecretCmd {
    /// Set one: its value from stdin (one line, or all of it with --multiline).
    Set {
        name: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        multiline: bool,
    },
    List {
        #[arg(long)]
        project: Option<String>,
    },
    /// Print one's value.
    Get {
        name: String,
        #[arg(long)]
        project: Option<String>,
    },
    Remove {
        name: String,
        #[arg(long)]
        project: Option<String>,
    },
}

#[derive(Subcommand)]
enum TriggerCmd {
    /// Add (or change) a trigger; the script from --script or --script-file.
    Add {
        name: String,
        #[arg(long)]
        project: String,
        /// poll, watch or webhook.
        #[arg(long, default_value = "poll")]
        mode: String,
        /// poll: how often (90s, 2m, 1h).
        #[arg(long)]
        every: Option<String>,
        /// poll: a cron expression instead.
        #[arg(long)]
        cron: Option<String>,
        #[arg(long, default_value = "UTC")]
        tz: String,
        #[arg(long)]
        script: Option<String>,
        #[arg(long)]
        script_file: Option<PathBuf>,
        /// The tasks' title and prompt (templates: {{key}}, {{vars.x}}, {{message}}).
        #[arg(long)]
        title: String,
        #[arg(long)]
        prompt: Option<String>,
        #[arg(long)]
        prompt_file: Option<PathBuf>,
        #[arg(long, default_value = "skip")]
        overlap: String,
        #[arg(long, default_value_t = 60)]
        timeout: i64,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long = "skill")]
        skills: Vec<String>,
        /// A webhook's secret: the name of one of the project's secrets.
        #[arg(long)]
        secret: Option<String>,
        /// Not in the project's nix dev shell.
        #[arg(long)]
        no_devshell: bool,
        /// A lock its script run and its tasks take (deploy, db:shared, global:staging; again for more).
        #[arg(long = "lock")]
        locks: Vec<String>,
        /// When one is busy: wait, skip or fail (default: the project's, else skip).
        #[arg(long)]
        on_busy: Option<String>,
        #[arg(long, default_value = "")]
        description: String,
    },
    List {
        #[arg(long)]
        project: Option<String>,
    },
    Remove {
        name: String,
        #[arg(long)]
        project: String,
    },
    Enable {
        name: String,
        #[arg(long)]
        project: String,
    },
    Disable {
        name: String,
        #[arg(long)]
        project: String,
    },
    /// Run a poll now (or restart a watcher).
    Run {
        name: String,
        #[arg(long)]
        project: String,
    },
    /// Into the repo (its files written into the project folder) or back into reagent (db).
    Move {
        name: String,
        to: String,
        #[arg(long)]
        project: String,
    },
    /// Allow (or --deny) a trigger's script that waits for approval; --always adds a rule.
    Approve {
        name: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        deny: bool,
        #[arg(long)]
        always: bool,
    },
}

#[derive(Subcommand)]
enum TaskCmd {
    /// Start a task (queued: the running reagent starts it): from a prompt, or a template
    /// (`reagent template list`) with --set field=value.
    Start {
        /// The project (may be left out with a template pinned to one).
        #[arg(long, default_value = "")]
        project: String,
        #[arg(long)]
        template: Option<String>,
        /// A template field's value (field=value; again for more).
        #[arg(long = "set")]
        set: Vec<String>,
        /// Default: the template's, else the task names itself.
        #[arg(long, default_value = "")]
        title: String,
        /// What to do (default: the template's prompt).
        #[arg(long, default_value = "")]
        prompt: String,
        #[arg(long = "skill")]
        skills: Vec<String>,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        /// A persistent worktree of the project to work in (`reagent worktree list`).
        #[arg(long)]
        worktree: Option<String>,
        /// A lock it takes when it starts (deploy, db:shared, global:staging; again for more): it waits for it.
        #[arg(long = "lock")]
        locks: Vec<String>,
    },
    /// Start the designer for a rough goal: a design task that reads the project, asks you
    /// (in the web UI, or by notification), and proposes tasks, cron entries, triggers, skills.
    /// The running reagent starts it.
    Design {
        goal: String,
        #[arg(long)]
        project: String,
        /// What it's for: task, cron or trigger.
        #[arg(long, default_value = "task")]
        target: String,
    },
    /// A design task's proposal; --make all|1,3 makes those items (tasks are queued for the running reagent).
    Proposal {
        task: String,
        #[arg(long)]
        make: Option<String>,
        /// Print it as JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ServiceCmd {
    /// Add (or change) a service; its command from --script or --script-file. The running reagent starts it.
    Add {
        name: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        script: Option<String>,
        #[arg(long)]
        script_file: Option<PathBuf>,
        /// A command whose exit 0 says it's healthy.
        #[arg(long)]
        health: Option<String>,
        /// Between checks while it's up (default 30s).
        #[arg(long)]
        interval: Option<String>,
        /// How long it may take to come up (default 60s).
        #[arg(long)]
        ready_timeout: Option<String>,
        /// SIGTERM, then SIGKILL after this (default 10s).
        #[arg(long)]
        stop_timeout: Option<String>,
        /// A persistent worktree of the project to run in.
        #[arg(long)]
        worktree: Option<String>,
        #[arg(long)]
        no_devshell: bool,
        #[arg(long, default_value = "")]
        description: String,
        /// For whoever repairs it.
        #[arg(long, default_value = "")]
        notes: String,
    },
    List {
        #[arg(long)]
        project: Option<String>,
    },
    Remove {
        name: String,
        #[arg(long)]
        project: String,
    },
    /// Switch it on (started at once).
    Start {
        name: String,
        #[arg(long)]
        project: String,
    },
    /// Switch it off (stopped; it stays off).
    Stop {
        name: String,
        #[arg(long)]
        project: String,
    },
    Restart {
        name: String,
        #[arg(long)]
        project: String,
    },
    /// The end of its output.
    Logs {
        name: String,
        #[arg(long)]
        project: String,
        #[arg(long, default_value_t = 100)]
        lines: usize,
    },
    /// Into the repo (.agents/services/<name>/) or back into reagent (db).
    Move {
        name: String,
        #[arg(long)]
        project: String,
        to: String,
    },
    /// Allow (or --deny) its command, which waits for the person.
    Approve {
        name: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        deny: bool,
    },
}

#[derive(Subcommand)]
enum SyncCmd {
    /// The project's branches (its folder's, each persistent worktree's) and how their sync is doing.
    List {
        #[arg(long)]
        project: String,
    },
    /// Change a branch's sync, kept in reagent.db (the checkout's .agents/sync.md overrides what it sets).
    Set {
        #[arg(long)]
        project: String,
        /// A persistent worktree (none: the project folder).
        #[arg(long)]
        worktree: Option<String>,
        #[arg(long)]
        on: bool,
        #[arg(long, conflicts_with = "on")]
        off: bool,
        /// Between rounds, like 15m (default 15m).
        #[arg(long)]
        every: Option<String>,
        /// rebase (default), merge or ff-only.
        #[arg(long)]
        strategy: Option<String>,
        /// Default: the branch's upstream, else origin ("" goes back to it).
        #[arg(long)]
        remote: Option<String>,
        /// The remote's branch. Default: the upstream's, else the same name ("" goes back to it).
        #[arg(long)]
        branch: Option<String>,
        /// For the task that resolves a conflict or decides about uncommitted changes.
        #[arg(long)]
        notes: Option<String>,
    },
    /// A round now (the running reagent does it within seconds; a stuck one goes on).
    Run {
        #[arg(long)]
        project: String,
        #[arg(long)]
        worktree: Option<String>,
    },
}

#[derive(Subcommand)]
enum WorktreeCmd {
    /// A project's persistent worktrees, and those git has that could be adopted.
    List { project: String },
    /// Make one: its branch (default: the name; checked out if there is one, else made from --base).
    Add {
        project: String,
        name: String,
        #[arg(long)]
        branch: Option<String>,
        /// What it merges into (default: the branch the project folder is on).
        #[arg(long)]
        base: Option<String>,
        /// shared or exclusive (default: the project's worktree_sharing).
        #[arg(long)]
        sharing: Option<String>,
    },
    /// Record a worktree git already has.
    Adopt {
        project: String,
        name: String,
        path: String,
        #[arg(long)]
        base: Option<String>,
        #[arg(long)]
        sharing: Option<String>,
    },
    /// Its sharing: shared, exclusive, or project (the project's).
    Set { project: String, name: String, sharing: String },
    /// Branch, base, commits ahead, uncommitted changes, and the tasks that worked there.
    Status { project: String, name: String },
    /// Its changes against its base.
    Diff {
        project: String,
        name: String,
        #[arg(long)]
        stat: bool,
    },
    /// Merge its branch into its base.
    Merge {
        project: String,
        name: String,
        /// merge, squash or rebase.
        #[arg(long, default_value = "merge")]
        strategy: String,
        #[arg(long)]
        message: Option<String>,
    },
    /// Remove it and its branch (an adopted one is only forgotten unless --delete).
    Remove {
        project: String,
        name: String,
        /// Discard its work not in its base (unmerged commits, uncommitted files).
        #[arg(long)]
        force: bool,
        #[arg(long)]
        delete: bool,
    },
}

#[derive(Subcommand)]
enum TemplateCmd {
    List {
        #[arg(long)]
        project: Option<String>,
    },
    /// One template's file.
    Show {
        name: String,
        #[arg(long)]
        project: Option<String>,
    },
}

#[derive(Subcommand)]
enum PluginCmd {
    /// Clone a plugin from a git URL.
    Add {
        url: String,
        /// Its folder's name (default: the URL's last part).
        #[arg(long)]
        id: Option<String>,
        /// A branch or tag to follow (default: the remote's default branch).
        #[arg(long = "ref")]
        reference: Option<String>,
    },
    List,
    /// Fetch what each (or the named) plugin follows and move onto it.
    Update { ids: Vec<String> },
    /// Switch these of a plugin's rules off, the others on (none: all on).
    RulesOff { id: String, rules: Vec<String> },
    Remove { id: String },
}

#[derive(Subcommand)]
enum TokenCmd {
    /// Make a token (printed once).
    Add { name: String },
    List,
    Revoke { name: String },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let data = cli.data.clone().unwrap_or_else(reagent::data_dir);
    let _ = dotenvy::from_path(data.join(".env"));
    // The node reads it to reach reagent's MCP servers; set before any thread starts.
    if std::env::var_os(reagent_tools::cluster::TOKEN_ENV).is_none() {
        unsafe { std::env::set_var(reagent_tools::cluster::TOKEN_ENV, uuid::Uuid::new_v4().simple().to_string()) };
    }
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,sqlx=warn,rmcp=warn".into())).init();
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run(cli.cmd, data))
}

async fn run(cmd: Cmd, data: PathBuf) -> anyhow::Result<()> {
    match cmd {
        Cmd::Up { listen } => {
            let r = reagent::up(reagent::Opts { data: data.clone(), listen, in_process_supervisor: false, exe: std::env::current_exe()? }).await?;
            if r.app.store.setting("password").await?.is_none() {
                tracing::warn!("no password set: run `reagent passwd` before using the web interface");
            }
            println!("reagent:        {}", r.web_url);
            println!("subnet hub:     {}  (admin token {})", r.hub_url, r.admin_token);
            println!("data:           {}", data.display());
            tokio::signal::ctrl_c().await?;
            tracing::info!("stopping: pausing the tasks first (quick)");
            reagent::stop(&r).await;
            Ok(())
        }
        Cmd::Supervisor { stop } => {
            let socket = data.join("supervisor.sock");
            if stop {
                reagent_supervisor::Client::new(&socket).shutdown().await.map_err(anyhow::Error::msg)?;
                println!("supervisor stopped");
                return Ok(());
            }
            std::fs::create_dir_all(&data)?;
            let s = reagent_supervisor::server::Supervisor::new(&data)?;
            s.serve(&socket).await
        }
        Cmd::Passwd { password_stdin } => {
            let a = if password_stdin {
                let mut line = String::new();
                std::io::stdin().read_line(&mut line)?;
                line.trim_end_matches(['\r', '\n']).to_string()
            } else {
                let a = rpassword::prompt_password("new password: ")?;
                let b = rpassword::prompt_password("again: ")?;
                anyhow::ensure!(a == b, "they differ");
                a
            };
            anyhow::ensure!(a.len() >= 8, "at least 8 characters");
            std::fs::create_dir_all(&data)?;
            let store = reagent_store::Store::open(&data.join("reagent.db")).await?;
            store.set_setting("password", &reagent_web::auth::hash(&a)?).await?;
            store.end_all_sessions().await?;
            println!("password set; every session ended");
            Ok(())
        }
        Cmd::Mcp { cmd } => {
            std::fs::create_dir_all(&data)?;
            let store = reagent_store::Store::open(&data.join("reagent.db")).await?;
            match cmd {
                McpCmd::Add { name, url, header_env, header, prefix, env, eager, idempotent, description, disabled, project, command } => {
                    if let Some(p) = &project {
                        anyhow::ensure!(store.project(p).await?.is_some(), "no project {p:?}");
                    }
                    let env = env
                        .iter()
                        .map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())).ok_or_else(|| anyhow::anyhow!("--env {kv:?}: KEY=VALUE")))
                        .collect::<anyhow::Result<std::collections::BTreeMap<_, _>>>()?;
                    let m = reagent_store::McpServer {
                        name,
                        description,
                        url,
                        command: (!command.is_empty()).then(|| sqlx_json(command)),
                        env: sqlx_json(env),
                        credential: header_env.map(|e| sqlx_json(reagent_store::McpCredential { header, env: e, prefix })),
                        lazy: !eager,
                        idempotent: sqlx_json(idempotent),
                        enabled: !disabled,
                        created: 0,
                        project,
                    };
                    reagent_tools::cluster::check_mcp(&m).map_err(anyhow::Error::msg)?;
                    store.put_mcp_server(&m).await?;
                    println!("added {} ({}); a running reagent applies it within seconds (the web UI's settings show whether it runs)", m.name, if m.lazy { "lazy" } else { "eager" });
                }
                McpCmd::List => {
                    for m in store.mcp_servers().await? {
                        let what = m.url.clone().unwrap_or_else(|| m.command.as_ref().map(|c| c.0.join(" ")).unwrap_or_default());
                        println!("{}\t{}\t{}\t{}{}", m.name, m.project.as_deref().unwrap_or("(every task)"), what, if m.lazy { "lazy" } else { "eager" }, if m.enabled { "" } else { "\toff" });
                    }
                }
                McpCmd::Remove { name } => {
                    anyhow::ensure!(store.remove_mcp_server(&name).await?, "no server called {name:?}");
                    println!("removed {name}");
                }
            }
            Ok(())
        }
        Cmd::Secret { cmd } => {
            std::fs::create_dir_all(&data)?;
            let store = reagent_store::Store::open(&data.join("reagent.db")).await?;
            let check = |p: &Option<String>| p.clone();
            match cmd {
                SecretCmd::Set { name, project, multiline } => {
                    if let Some(p) = &project {
                        anyhow::ensure!(store.project(p).await?.is_some(), "no project {p:?}");
                    }
                    let mut value = String::new();
                    if multiline {
                        std::io::Read::read_to_string(&mut std::io::stdin(), &mut value)?;
                    } else {
                        std::io::stdin().read_line(&mut value)?;
                        value = value.trim_end_matches(['\r', '\n']).to_string();
                    }
                    store.set_secret(check(&project).as_deref(), &name, &value).await.map_err(anyhow::Error::msg)?;
                    println!("set {name}{}", project.map(|p| format!(" for {p}")).unwrap_or_default());
                }
                SecretCmd::List { project } => {
                    for s in store.secrets(project.as_deref()).await.map_err(anyhow::Error::msg)? {
                        println!("{}\t{}", s.name, s.project.as_deref().unwrap_or("(every project)"));
                    }
                }
                SecretCmd::Get { name, project } => {
                    let s = store.secrets(project.as_deref()).await.map_err(anyhow::Error::msg)?.into_iter().find(|s| s.name == name).ok_or_else(|| anyhow::anyhow!("no secret {name:?}"))?;
                    println!("{}", s.value);
                }
                SecretCmd::Remove { name, project } => {
                    anyhow::ensure!(store.remove_secret(project.as_deref(), &name).await?, "no secret {name:?}");
                    println!("removed {name}");
                }
            }
            Ok(())
        }
        Cmd::Trigger { cmd } => {
            std::fs::create_dir_all(&data)?;
            let store = reagent_store::Store::open(&data.join("reagent.db")).await?;
            let project = |store: reagent_store::Store, slug: String| async move { store.project(&slug).await?.ok_or_else(|| anyhow::anyhow!("no project {slug:?}")) };
            let trigger = |store: reagent_store::Store, slug: String, name: String| async move { store.trigger(&slug, &name).await?.ok_or_else(|| anyhow::anyhow!("no trigger {name:?} in {slug}")) };
            match cmd {
                TriggerCmd::Add { name, project: slug, mode, every, cron, tz, script, script_file, title, prompt, prompt_file, overlap, timeout, profile, kind, skills, secret, no_devshell, description, locks, on_busy } => {
                    let p = project(store.clone(), slug).await?;
                    let script = match (script, script_file) {
                        (Some(s), None) => s,
                        (None, Some(f)) => std::fs::read_to_string(&f).map_err(|e| anyhow::anyhow!("{}: {e}", f.display()))?,
                        (None, None) => String::new(),
                        _ => anyhow::bail!("--script or --script-file, not both"),
                    };
                    let prompt = match (prompt, prompt_file) {
                        (Some(s), None) => s,
                        (None, Some(f)) => std::fs::read_to_string(&f)?,
                        _ => anyhow::bail!("--prompt or --prompt-file"),
                    };
                    let def = reagent_tools::triggers::Def { name, mode, every, cron, tz: Some(tz), script: Some(script), timeout: Some(timeout), overlap: Some(overlap), title, prompt, description: Some(description), profile, kind, skills, secret, devshell: Some(!no_devshell), locks, on_busy };
                    let t = reagent_tools::triggers::save(&store, &p, def.into_trigger(&p.slug, "person").map_err(anyhow::Error::msg)?, true).await.map_err(anyhow::Error::msg)?;
                    println!("{}", reagent_tools::triggers::line(&t));
                    if t.mode == "webhook" {
                        println!("its URL: <reagent>/hook/{}/{}", t.project, t.name);
                    }
                }
                TriggerCmd::List { project } => {
                    for t in store.triggers(project.as_deref()).await? {
                        println!("{}\t{}", t.project, reagent_tools::triggers::line(&t));
                    }
                }
                TriggerCmd::Remove { name, project: slug } => {
                    let p = project(store.clone(), slug.clone()).await?;
                    let t = trigger(store.clone(), slug.clone(), name.clone()).await?;
                    if let Some(j) = &t.state.0.job {
                        let _ = reagent_supervisor::Client::new(&data.join("supervisor.sock")).kill(j, None).await;
                    }
                    if t.source == "repo" {
                        store.put_trigger(&reagent_store::Trigger { source: "db".into(), ..t.clone() }).await?;
                        reagent_tools::triggers::remove_repo(&p, &t).map_err(anyhow::Error::msg)?;
                    }
                    store.remove_trigger(&slug, &name).await?;
                    println!("removed {name}");
                }
                TriggerCmd::Enable { name, project: slug } => {
                    anyhow::ensure!(store.set_trigger_enabled(&slug, &name, true).await?, "no trigger {name:?} in {slug}");
                    println!("{name} is on");
                }
                TriggerCmd::Disable { name, project: slug } => {
                    anyhow::ensure!(store.set_trigger_enabled(&slug, &name, false).await?, "no trigger {name:?} in {slug}");
                    println!("{name} is off");
                }
                TriggerCmd::Run { name, project: slug } => {
                    let t = trigger(store.clone(), slug.clone(), name.clone()).await?;
                    let mut st = t.state.0.clone();
                    match t.mode.as_str() {
                        "webhook" => anyhow::bail!("a webhook runs when it's called"),
                        "watch" => {
                            if let Some(j) = &st.job {
                                reagent_supervisor::Client::new(&data.join("supervisor.sock")).kill(j, None).await.map_err(anyhow::Error::msg)?;
                            }
                            st.next_run = None;
                        }
                        _ => st.next_run = Some(0),
                    }
                    store.set_trigger_state(&slug, &name, &st).await?;
                    println!("a running reagent runs {name} within a second");
                }
                TriggerCmd::Move { name, to, project: slug } => {
                    let p = project(store.clone(), slug).await?;
                    let t = reagent_tools::triggers::move_to(&store, &p, &name, &to).await.map_err(anyhow::Error::msg)?;
                    println!("{} is {}", t.name, if t.source == "repo" { format!("in the repo: {}", reagent_tools::triggers::repo_dir(&p, &t.name).display()) } else { "kept in reagent".into() });
                }
                TriggerCmd::Approve { name, project: slug, deny, always } => {
                    let t = trigger(store.clone(), slug.clone(), name.clone()).await?;
                    let hash = reagent_tools::triggers::script_hash(&t);
                    if always && !deny {
                        let rule = reagent_store::Rule { id: 0, project: slug.clone(), pos: 0, tool: "triggers.run".into(), command: Some(reagent_tools::triggers::command_line(&t)), target: None, action: "allow".into() };
                        store.prepend_rule(&slug, &rule).await?;
                    }
                    let mut st = t.state.0.clone();
                    st.asking = None;
                    if deny {
                        st.denied = Some(hash);
                    } else {
                        st.approved = Some(hash);
                        st.denied = None;
                        st.next_run = None;
                    }
                    store.set_trigger_state(&slug, &name, &st).await?;
                    println!("{name}: {}", if deny { "denied" } else { "allowed" });
                }
            }
            Ok(())
        }
        Cmd::Task { cmd } => {
            let config = reagent::load_config(&data)?;
            let store = reagent_store::Store::open(&data.join("reagent.db")).await?;
            // A task the running reagent starts (it starts queued ones as places free).
            let queue = |p: &reagent_store::Project, title: String, prompt: String, origin: &str, skills: Vec<String>, profile: Option<String>, kind: Option<String>, budget: Option<reagent_store::Budget>, home: Option<reagent_store::PersistentWorktree>, locks: Vec<String>| {
                let store = store.clone();
                let cwd = home.as_ref().map(|w| w.path.clone()).unwrap_or_else(|| p.path.clone());
                let new = reagent_store::NewTask { project: p.slug.clone(), parent: None, title, prompt, origin: origin.into(), cwd, profile: profile.or(p.profile.clone()).unwrap_or(config.default_profile.clone()), budget: budget.unwrap_or_default(), skills, kind, home: home.map(|w| w.name), locks };
                async move {
                    let t = store.add_task(&new).await?;
                    store.set_state(&t.id, "queued", None).await?;
                    anyhow::Ok(t)
                }
            };
            match cmd {
                TaskCmd::Start { project, template, set, title, prompt, skills, profile, kind, worktree, locks } => {
                    let mut req = reagent_tools::app::StartTask { project, title, prompt, skills, profile, kind, worktree, locks, ..Default::default() };
                    if let Some(name) = template {
                        let p = if req.project.is_empty() { None } else { Some(store.project(&req.project).await?.ok_or_else(|| anyhow::anyhow!("no project {:?}", req.project))?) };
                        let t = reagent_tools::templates::find(&store, &data, p.as_ref(), &name).await.map_err(anyhow::Error::msg)?;
                        let mut values = serde_json::Map::new();
                        for kv in set {
                            let (k, v) = kv.split_once('=').ok_or_else(|| anyhow::anyhow!("--set field=value, not {kv:?}"))?;
                            values.insert(k.trim().into(), serde_json::Value::String(v.into()));
                        }
                        reagent_tools::templates::apply(&t, &values, &mut req).map_err(anyhow::Error::msg)?;
                    }
                    anyhow::ensure!(!req.project.is_empty(), "--project: which project?");
                    anyhow::ensure!(!req.prompt.trim().is_empty(), "--prompt or --template: what to do?");
                    let p = store.project(&req.project).await?.ok_or_else(|| anyhow::anyhow!("no project {:?}", req.project))?;
                    let locks = reagent_tools::locks::parse_all(&p.slug, &req.locks).map_err(anyhow::Error::msg)?.iter().map(reagent_tools::locks::Spec::label).collect();
                    let mut budget = req.budget.unwrap_or_default();
                    budget.tokens = budget.tokens.or(p.budget.tokens);
                    budget.cost = budget.cost.or(p.budget.cost);
                    budget.minutes = budget.minutes.or(p.budget.minutes);
                    let kind_profile = req.kind.as_deref().and_then(|k| config.kind.get(k)).and_then(|k| k.profile.clone());
                    let home = match req.worktree.as_deref().filter(|w| !w.trim().is_empty()) {
                        Some(n) => Some(reagent_tools::persistent::Worktrees { store: &store, data: &data }.get(&p.slug, n).await.map_err(anyhow::Error::msg)?),
                        None => None,
                    };
                    let t = queue(&p, req.title.trim().to_string(), req.prompt, "cli", req.skills, req.profile.or(kind_profile), req.kind, Some(budget), home, locks).await?;
                    println!("{}", t.id);
                    eprintln!("queued: the running reagent starts it");
                }
                TaskCmd::Design { goal, project, target } => {
                    anyhow::ensure!(matches!(target.as_str(), "task" | "cron" | "trigger"), "--target: task, cron or trigger");
                    anyhow::ensure!(!goal.trim().is_empty(), "what do you want done?");
                    let p = store.project(&project).await?.ok_or_else(|| anyhow::anyhow!("no project {project:?}"))?;
                    let t = queue(&p, reagent_tools::design::title(&goal), reagent_tools::design::task_prompt(&target, &goal), "design", vec!["reagent-prompt-design".into()], config.design_profile.clone(), None, None, None, vec![]).await?;
                    println!("{}", t.id);
                    eprintln!("design task queued: the running reagent starts it; answer its questions in the web UI (or a notification), then `reagent task proposal {}`", t.id);
                }
                TaskCmd::Proposal { task, make, json } => {
                    let t = store.task(&task).await?.ok_or_else(|| anyhow::anyhow!("no task {task:?}"))?;
                    anyhow::ensure!(t.origin == "design", "{} isn't a design task", t.title);
                    let p = store.project(&t.project).await?.ok_or_else(|| anyhow::anyhow!("its project is gone"))?;
                    let paths = reagent_tools::app::Paths::new(&data);
                    let skills: Vec<String> = reagent_tools::skills::discover(std::path::Path::new(&p.path), std::path::Path::new(&p.path), &paths.skills, Some(&paths.system_skills)).into_iter().map(|s| s.name).collect();
                    let Some(prop) = t.report.as_deref().and_then(|r| reagent_tools::design::parse_report(r, &config, &skills)) else {
                        anyhow::bail!("no proposal yet ({} is {})", t.title, t.state);
                    };
                    if json {
                        println!("{}", serde_json::to_string_pretty(&prop)?);
                    } else {
                        println!("{}\n", prop.note);
                        for (i, item) in prop.items.iter().enumerate() {
                            println!("{}. {}", i + 1, item.label());
                        }
                    }
                    if let Some(which) = make {
                        let picked: Vec<usize> = if which == "all" { (0..prop.items.len()).collect() } else { which.split(',').map(|n| n.trim().parse::<usize>().map(|n| n - 1)).collect::<Result<_, _>>()? };
                        for i in picked {
                            let item = prop.items.get(i).ok_or_else(|| anyhow::anyhow!("there's no item {}", i + 1))?;
                            let done = match item {
                                reagent_tools::design::Item::Task { title, prompt, skills, kind, profile, budget, .. } => queue(&p, title.clone(), prompt.clone(), "ui", skills.clone(), profile.clone(), kind.clone(), budget.clone(), None, vec![]).await.map(|t| format!("task {} queued ({})", t.title, t.id)).map_err(|e| e.to_string()),
                                other => reagent_tools::design::make(&store, &p, other).await,
                            };
                            eprintln!("{}. {}", i + 1, done.unwrap_or_else(|e| format!("not made: {e}")));
                        }
                    }
                }
            }
            Ok(())
        }
        Cmd::Template { cmd } => {
            let store = reagent_store::Store::open(&data.join("reagent.db")).await?;
            let project = match &cmd {
                TemplateCmd::List { project } | TemplateCmd::Show { project, .. } => match project {
                    Some(slug) => Some(store.project(slug).await?.ok_or_else(|| anyhow::anyhow!("no project {slug:?}"))?),
                    None => None,
                },
            };
            match cmd {
                TemplateCmd::List { .. } => {
                    for t in reagent_tools::templates::discover(&store, &data, project.as_ref()).await {
                        let form = if t.fields.is_empty() { "prompt".to_string() } else { format!("form: {}", t.fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", ")) };
                        let pin = t.project.as_deref().map(|p| format!(" [for {p}]")).unwrap_or_default();
                        match &t.error {
                            Some(e) => println!("{}\t{}{pin}\tbroken: {e}", t.name, t.source),
                            None => println!("{}\t{}{pin}\t{form}\t{}", t.name, t.source, t.description),
                        }
                    }
                }
                TemplateCmd::Show { name, .. } => {
                    let all = reagent_tools::templates::discover(&store, &data, project.as_ref()).await;
                    let t = all.iter().find(|t| t.name == name).ok_or_else(|| anyhow::anyhow!("no template {name:?}"))?;
                    print!("{}", t.text);
                }
            }
            Ok(())
        }
        Cmd::Service { cmd } => {
            use reagent_tools::services;
            std::fs::create_dir_all(&data)?;
            let store = reagent_store::Store::open(&data.join("reagent.db")).await?;
            let m = anyhow::Error::msg;
            let project = |slug: String| {
                let store = store.clone();
                async move { store.project(&slug).await?.ok_or_else(|| anyhow::anyhow!("no project {slug:?}")) }
            };
            let service = |slug: String, name: String| {
                let store = store.clone();
                async move { store.service(&slug, &name).await?.ok_or_else(|| anyhow::anyhow!("no service {name:?} in {slug}")) }
            };
            match cmd {
                ServiceCmd::Add { name, project: slug, script, script_file, health, interval, ready_timeout, stop_timeout, worktree, no_devshell, description, notes } => {
                    let p = project(slug).await?;
                    let script = match (script, script_file) {
                        (Some(s), None) => s,
                        (None, Some(f)) => std::fs::read_to_string(&f).map_err(|e| anyhow::anyhow!("{}: {e}", f.display()))?,
                        _ => anyhow::bail!("--script or --script-file"),
                    };
                    let def = services::Def { name, script, health, interval, ready_timeout, stop_timeout, worktree, devshell: Some(!no_devshell), description: Some(description), notes: Some(notes) };
                    let v = services::save(&store, &p, def.into_service(&p.slug, "person").map_err(m)?, true).await.map_err(m)?;
                    println!("{}", services::line(&v));
                    eprintln!("the running reagent starts it within seconds");
                }
                ServiceCmd::List { project } => {
                    for v in store.services(project.as_deref()).await? {
                        println!("{}\t{}", v.project, services::line(&v));
                    }
                }
                ServiceCmd::Remove { name, project: slug } => {
                    let p = project(slug.clone()).await?;
                    let v = service(slug.clone(), name.clone()).await?;
                    if let Some(j) = &v.state.0.job {
                        let _ = reagent_supervisor::Client::new(&data.join("supervisor.sock")).kill(j, None).await;
                    }
                    if v.source == "repo" {
                        store.put_service(&reagent_store::Service { source: "db".into(), ..v.clone() }).await?;
                        services::remove_repo(&p, &v).map_err(m)?;
                    }
                    store.remove_service(&slug, &name).await?;
                    println!("removed {name}");
                }
                ServiceCmd::Start { name, project } => println!("{}", services::line(&services::set_enabled(&store, &project, &name, true).await.map_err(m)?)),
                ServiceCmd::Stop { name, project } => println!("{}", services::line(&services::set_enabled(&store, &project, &name, false).await.map_err(m)?)),
                ServiceCmd::Restart { name, project } => println!("{}", services::line(&services::restart(&store, &project, &name).await.map_err(m)?)),
                ServiceCmd::Logs { name, project, lines } => {
                    let v = service(project.clone(), name.clone()).await?;
                    let job = match v.state.0.job.clone() {
                        Some(j) => Some(j),
                        None => store.service_runs(&project, &name).await?.into_iter().find_map(|r| r.job),
                    };
                    match job {
                        Some(j) => print!("{}", reagent_supervisor::tail(&data, &j, lines)),
                        None => eprintln!("{name} hasn't run yet"),
                    }
                }
                ServiceCmd::Move { name, project: slug, to } => {
                    let p = project(slug).await?;
                    println!("{}", services::line(&services::move_to(&store, &p, &name, &to).await.map_err(m)?));
                }
                ServiceCmd::Approve { name, project, deny } => {
                    let v = service(project.clone(), name.clone()).await?;
                    let mut st = v.state.0.clone();
                    let hash = services::script_hash(&v);
                    st.asking = None;
                    if deny {
                        st.denied = Some(hash);
                    } else {
                        st.approved = Some(hash);
                        st.denied = None;
                        st.next_start = None;
                    }
                    store.set_service_state(&project, &name, &st).await?;
                    println!("{name}: {}", if deny { "denied" } else { "allowed" });
                }
            }
            Ok(())
        }
        Cmd::Sync { cmd } => {
            let store = reagent_store::Store::open(&data.join("reagent.db")).await?;
            let project = |slug: String| {
                let store = store.clone();
                async move { store.project(&slug).await?.ok_or_else(|| anyhow::anyhow!("no project {slug:?}")) }
            };
            match cmd {
                SyncCmd::List { project: slug } => {
                    let p = project(slug).await?;
                    for s in reagent_tools::gitsync::list(&store, &p).await {
                        let v = &s.settings;
                        let place = if v.place.is_empty() { "." } else { &v.place };
                        let target = format!("{}/{}", v.remote.as_deref().unwrap_or("(upstream)"), v.branch.as_deref().unwrap_or("(same)"));
                        let repo = if s.from_repo.is_empty() { String::new() } else { format!(", repo sets {}", s.from_repo.join(" ")) };
                        let st = &v.state.0;
                        println!("{place}	{}	{}	every {}m, {}, {target}{repo}	{} {}", s.local_branch.as_deref().unwrap_or("(detached)"), if v.enabled { "on" } else { "off" }, v.every / 60, v.strategy, st.status, st.message.as_deref().unwrap_or(""));
                        if let Some(e) = &s.repo_error {
                            println!("\t{}: {e} (ignored)", s.repo_file.as_ref().map(|f| f.display().to_string()).unwrap_or_default());
                        }
                    }
                }
                SyncCmd::Set { project: slug, worktree, on, off, every, strategy, remote, branch, notes } => {
                    let p = project(slug).await?;
                    let place = worktree.unwrap_or_default();
                    let mut v = store.git_sync(&p.slug, &place).await?.unwrap_or_else(|| reagent_store::GitSync::new(&p.slug, &place));
                    if on || off {
                        v.enabled = on;
                    }
                    if let Some(e) = every {
                        v.every = reagent_tools::triggers::parse_every(&e).map_err(anyhow::Error::msg)?;
                    }
                    if let Some(x) = strategy {
                        v.strategy = x;
                    }
                    if let Some(x) = remote {
                        v.remote = Some(x);
                    }
                    if let Some(x) = branch {
                        v.branch = Some(x);
                    }
                    if let Some(x) = notes {
                        v.notes = x;
                    }
                    let v = reagent_tools::gitsync::put(&store, &p, v).await.map_err(anyhow::Error::msg)?;
                    println!("{}: {}, every {}m, {}", if place.is_empty() { "the project folder" } else { &place }, if v.enabled { "on" } else { "off" }, v.every / 60, v.strategy);
                }
                SyncCmd::Run { project: slug, worktree } => {
                    let p = project(slug).await?;
                    reagent_tools::gitsync::run_now(&store, &p.slug, &worktree.unwrap_or_default()).await.map_err(anyhow::Error::msg)?;
                    println!("a running reagent syncs it within seconds");
                }
            }
            Ok(())
        }
        Cmd::Worktree { cmd } => {
            let store = reagent_store::Store::open(&data.join("reagent.db")).await?;
            let w = reagent_tools::persistent::Worktrees { store: &store, data: &data };
            let m = anyhow::Error::msg;
            let line = |i: &reagent_tools::persistent::WorktreeInfo| {
                let ahead = i.ahead.map(|n| format!("{n} ahead")).unwrap_or_else(|| "?".into());
                let changes = match i.changes {
                    Some(0) | None => String::new(),
                    Some(n) => format!(", {n} changed"),
                };
                format!("{}\t{} -> {}\t{ahead}{changes}\t{}{}{}\t{}", i.worktree.name, i.worktree.branch, i.worktree.base, i.sharing_now, if i.worktree.adopted { ", adopted" } else { "" }, if i.missing { ", MISSING" } else { "" }, i.worktree.path)
            };
            match cmd {
                WorktreeCmd::List { project } => {
                    let (kept, adoptable) = w.list(&project).await.map_err(m)?;
                    for i in &kept {
                        println!("{}", line(i));
                    }
                    for a in &adoptable {
                        println!("(adoptable)\t{}\t{}", a.branch, a.path);
                    }
                }
                WorktreeCmd::Add { project, name, branch, base, sharing } => println!("{}", line(&w.add(&project, reagent_tools::persistent::AddWorktree { name, branch, base, path: None, sharing }).await.map_err(m)?)),
                WorktreeCmd::Adopt { project, name, path, base, sharing } => println!("{}", line(&w.add(&project, reagent_tools::persistent::AddWorktree { name, branch: None, base, path: Some(path), sharing }).await.map_err(m)?)),
                WorktreeCmd::Set { project, name, sharing } => println!("{}", line(&w.set_sharing(&project, &name, Some(sharing.as_str()).filter(|s| *s != "project")).await.map_err(m)?)),
                WorktreeCmd::Status { project, name } => {
                    println!("{}", line(&w.info_of(&project, &name).await.map_err(m)?));
                    print!("{}", w.status(&project, &name).await.map_err(m)?);
                    for t in w.tasks(&project, &name).await.map_err(m)? {
                        println!("task {}\t{}\t{}", t.id, t.state, t.name());
                    }
                }
                WorktreeCmd::Diff { project, name, stat } => print!("{}", w.diff(&project, &name, stat).await.map_err(m)?),
                WorktreeCmd::Merge { project, name, strategy, message } => {
                    let strategy: reagent_tools::git::Strategy = serde_json::from_value(serde_json::Value::String(strategy)).map_err(|_| anyhow::anyhow!("--strategy: merge, squash or rebase"))?;
                    println!("{}", w.merge(&project, &name, strategy, message).await.map_err(m)?);
                }
                WorktreeCmd::Remove { project, name, force, delete } => println!("{}", w.remove(&project, &name, force, delete).await.map_err(m)?),
            }
            Ok(())
        }
        Cmd::Plugin { cmd } => {
            use reagent_tools::plugins;
            // A marketplace's plugins, as a tree under it.
            fn print_parts(p: &plugins::Plugin) {
                for (i, x) in p.plugins.iter().enumerate() {
                    let branch = if i + 1 == p.plugins.len() { "└─" } else { "├─" };
                    match &x.error {
                        Some(e) => println!("  {branch} {}\t{e}", x.name),
                        None => println!("  {branch} {}\t{}\tskills: {}\trules: {}", x.name, x.path.as_deref().unwrap_or_default(), x.skills.join(","), x.rules.join(",")),
                    }
                }
            }
            match cmd {
                PluginCmd::Add { url, id, reference } => {
                    let p = plugins::add(&data, &url, id.as_deref(), reference.as_deref()).map_err(anyhow::Error::msg)?;
                    println!("added {} at {} ({}): skills {}; rules {}", p.id, p.commit, p.subject, p.skills.join(", "), p.rules.join(", "));
                    print_parts(&p);
                }
                PluginCmd::List => {
                    for p in plugins::list(&data) {
                        println!("{}\t{}{}\t{} {}\tskills: {}\trules: {}\toff: {}", p.id, p.url, p.reference.as_ref().map(|r| format!("@{r}")).unwrap_or_default(), p.commit, p.subject, p.skills.join(","), p.rules.join(","), p.rules_off.join(","));
                        print_parts(&p);
                    }
                }
                PluginCmd::Update { ids } => {
                    let ids = if ids.is_empty() { plugins::dirs(&data).into_iter().map(|(id, _)| id).collect() } else { ids };
                    let mut failed = false;
                    for id in ids {
                        match plugins::update(&data, &id) {
                            Ok((before, p)) if before == p.commit => println!("{id}: up to date ({before})"),
                            Ok((before, p)) => println!("{id}: {before} → {} ({})", p.commit, p.subject),
                            Err(e) => {
                                failed = true;
                                eprintln!("{id}: {e}");
                            }
                        }
                    }
                    anyhow::ensure!(!failed, "not every plugin was updated");
                }
                PluginCmd::RulesOff { id, rules } => {
                    let p = plugins::set_rules_off(&data, &id, &rules).map_err(anyhow::Error::msg)?;
                    println!("{id}: rules off: {}", if p.rules_off.is_empty() { "none".into() } else { p.rules_off.join(", ") });
                }
                PluginCmd::Remove { id } => {
                    plugins::remove(&data, &id).map_err(anyhow::Error::msg)?;
                    println!("removed {id}");
                }
            }
            Ok(())
        }
        Cmd::Token { cmd } => {
            std::fs::create_dir_all(&data)?;
            let store = reagent_store::Store::open(&data.join("reagent.db")).await?;
            match cmd {
                TokenCmd::Add { name } => {
                    let token = format!("rgt_{}", reagent_web::auth::new_token());
                    store.add_api_token(&name, &reagent_web::auth::token_hash(&token)).await.map_err(|_| anyhow::anyhow!("there's a token called {name:?}"))?;
                    println!("{token}");
                    eprintln!("an MCP client reaches reagent at <reagent's URL>/mcp with the header `Authorization: Bearer <this token>`; it isn't shown again");
                }
                TokenCmd::List => {
                    for (n, created, used) in store.api_tokens().await? {
                        println!("{n}\tmade {created}\tlast used {}", used.map(|u| u.to_string()).unwrap_or("never".into()));
                    }
                }
                TokenCmd::Revoke { name } => {
                    anyhow::ensure!(store.revoke_api_token(&name).await?, "no token called {name:?}");
                    println!("revoked {name}");
                }
            }
            Ok(())
        }
        Cmd::Status => {
            let c = reagent_supervisor::Client::new(&data.join("supervisor.sock"));
            match c.ping().await {
                Ok(v) => {
                    let jobs = c.jobs(None).await.map_err(anyhow::Error::msg)?;
                    println!("supervisor: running (pid {}), {} jobs running, {} terminals", v["pid"], jobs.iter().filter(|j| j.running()).count(), c.ptys(None).await.map_err(anyhow::Error::msg)?.len());
                }
                Err(e) => println!("supervisor: not running ({e})"),
            }
            Ok(())
        }
    }
}

fn sqlx_json<T>(v: T) -> reagent_store::Json<T> {
    reagent_store::Json(v)
}
