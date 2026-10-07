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
    Passwd,
    /// Whether the supervisor runs, and its jobs.
    Status,
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
        Cmd::Passwd => {
            let a = rpassword::prompt_password("new password: ")?;
            anyhow::ensure!(a.len() >= 8, "at least 8 characters");
            let b = rpassword::prompt_password("again: ")?;
            anyhow::ensure!(a == b, "they differ");
            std::fs::create_dir_all(&data)?;
            let store = reagent_store::Store::open(&data.join("reagent.db")).await?;
            store.set_setting("password", &reagent_web::auth::hash(&a)?).await?;
            store.end_all_sessions().await?;
            println!("password set; every session ended");
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
