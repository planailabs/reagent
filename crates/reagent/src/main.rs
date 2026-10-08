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
                TriggerCmd::Add { name, project: slug, mode, every, cron, tz, script, script_file, title, prompt, prompt_file, overlap, timeout, profile, kind, skills, secret, no_devshell, description } => {
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
                    let def = reagent_tools::triggers::Def { name, mode, every, cron, tz: Some(tz), script: Some(script), timeout: Some(timeout), overlap: Some(overlap), title, prompt, description: Some(description), profile, kind, skills, secret, devshell: Some(!no_devshell) };
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
