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
            let dist = std::env::var_os("REAGENT_WEBUI").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../webui/dist")));
            let r = reagent::up(reagent::Opts { data: data.clone(), listen, in_process_supervisor: false, exe: std::env::current_exe()?, dist }).await?;
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
