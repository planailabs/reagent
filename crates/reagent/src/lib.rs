//! `reagent up`: the store, the supervisor, subnet's hub (on SQLite) and a
//! node, reagent's MCP servers, the followers, cron and the web server.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use reagent_store::Store;
use reagent_tools::app::{App, Paths};
use reagent_tools::config::{self, Config};
use subnet::hub::{Hub, http};
use subnet::node::{Node, attach};
use subnet_core::addr::Addr;
use subnet_core::agent::PauseMode;

/// Tasks the last stop paused, resumed at the next start.
const PAUSED_FILE: &str = "paused-at-stop.json";

pub struct Opts {
    pub data: PathBuf,
    /// Where the web interface listens (else reagent.hcl's `listen`).
    pub listen: Option<String>,
    /// Run the supervisor inside this process (tests): jobs end with it.
    pub in_process_supervisor: bool,
    /// The reagent binary (to start the supervisor).
    pub exe: PathBuf,
    pub dist: PathBuf,
}

pub struct Running {
    pub app: Arc<App>,
    pub hub: Arc<Hub>,
    pub hub_url: String,
    pub admin_token: String,
    pub web_url: String,
}

/// The data folder: `REAGENT_DATA`, else the platform's (`~/.local/share/reagent`).
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("REAGENT_DATA") {
        return PathBuf::from(d);
    }
    directories::ProjectDirs::from("", "", "reagent").map(|d| d.data_dir().to_path_buf()).unwrap_or_else(|| PathBuf::from(".reagent"))
}

/// reagent.hcl (an example one is written the first time).
pub fn load_config(data: &Path) -> anyhow::Result<Config> {
    let file = data.join("reagent.hcl");
    if !file.exists() {
        std::fs::create_dir_all(data)?;
        std::fs::write(&file, config::EXAMPLE)?;
        tracing::info!(file = %file.display(), "wrote an example reagent.hcl: set your providers and profiles there");
    }
    Config::parse(&std::fs::read_to_string(&file)?).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))
}

pub async fn up(o: Opts) -> anyhow::Result<Running> {
    std::fs::create_dir_all(&o.data)?;
    let config = load_config(&o.data)?;
    let paths = Paths::new(&o.data);
    let store = Store::open(&o.data.join("reagent.db")).await?;
    let sup = if o.in_process_supervisor {
        let s = reagent_supervisor::server::Supervisor::new(&o.data)?;
        let socket = paths.socket.clone();
        tokio::spawn(async move {
            if let Err(e) = s.serve(&socket).await {
                tracing::error!(error = %e, "supervisor stopped");
            }
        });
        let c = reagent_supervisor::Client::new(&paths.socket);
        for _ in 0..100 {
            if c.ping().await.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        c
    } else {
        reagent_supervisor::ensure_running(&o.exe, &o.data, &paths.socket).await?
    };
    let listen = o.listen.clone().unwrap_or_else(|| config.listen.clone());
    let app = App::new(paths, config, store, sup);

    // reagent's MCP servers, for the node only.
    let mcp_l = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let mcp_base = format!("http://{}", mcp_l.local_addr()?);
    let mcp = reagent_tools::mcp::router(app.clone());
    tokio::spawn(async move { axum::serve(mcp_l, mcp).await });

    // The hub, on SQLite next to reagent's own database.
    let admin_token = uuid::Uuid::new_v4().simple().to_string();
    let hub_l = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let hub_url = format!("http://{}", hub_l.local_addr()?);
    let hub = Hub::start(&format!("sqlite://{}", o.data.join("hub.db").display()), Some(admin_token.clone()), &hub_url).await?;
    let router = http::router(hub.clone());
    tokio::spawn(async move { axum::serve(hub_l, router).await });
    hub.wait_leader().await;
    attach(hub.clone(), Arc::new(Node::new("local", None))).await?;
    app.set_hub(hub.clone());
    let _ = app.mcp_base.set(mcp_base.clone());
    app.apply_cluster().await.map_err(anyhow::Error::msg)?;
    tracing::info!("cluster applied");
    resume_paused(&app, &hub).await;
    app.follow();
    app.watch_mcp();
    reagent_tools::cron::schedule(app.clone());

    let web = Arc::new(reagent_web::Web { app: app.clone(), hub_url: hub_url.clone(), hub_token: admin_token.clone(), dist: o.dist.clone() });
    let web_l = tokio::net::TcpListener::bind(&listen).await.map_err(|e| anyhow::anyhow!("listening on {listen}: {e}"))?;
    let web_url = format!("http://{}", web_l.local_addr()?);
    let r = reagent_web::router(web);
    tokio::spawn(async move { axum::serve(web_l, r.into_make_service_with_connect_info::<std::net::SocketAddr>()).await });
    Ok(Running { app, hub, hub_url, admin_token, web_url })
}

async fn resume_paused(app: &App, hub: &Hub) {
    let file = app.paths.data.join(PAUSED_FILE);
    let Ok(text) = std::fs::read(&file) else { return };
    let ids: Vec<String> = serde_json::from_slice(&text).unwrap_or_default();
    for id in ids {
        let Ok(Some(t)) = app.store.task(&id).await else { continue };
        let Some(agent) = t.agent.as_deref().and_then(|a| a.parse::<uuid::Uuid>().ok()) else { continue };
        match hub.resume(&Addr::root(), agent, true).await {
            Ok(_) => {
                let _ = app.store.set_state(&t.id, "running", None).await;
                tracing::info!(task = %t.id, "task resumed");
            }
            Err(e) => tracing::warn!(task = %t.id, error = %e, "couldn't resume a task paused at the last stop"),
        }
    }
    let _ = std::fs::remove_file(&file);
}

/// Before stopping: running tasks are paused (quick: running tool calls
/// finish, nothing new starts), and resumed at the next start.
pub async fn stop(r: &Running) {
    let mut paused = vec![];
    for t in r.app.store.tasks(None, None, true, 10_000).await.unwrap_or_default().into_iter().filter(|t| t.state == "running" || t.state == "waiting") {
        let Some(agent) = t.agent.as_deref().and_then(|a| a.parse::<uuid::Uuid>().ok()) else { continue };
        if r.hub.pause(&Addr::root(), agent, PauseMode::Quick, true).await.is_ok() {
            paused.push((t.id.clone(), agent));
        }
    }
    let _ = std::fs::write(r.app.paths.data.join(PAUSED_FILE), serde_json::to_vec(&paused.iter().map(|(t, _)| t).collect::<Vec<_>>()).unwrap_or_default());
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let agents = r.hub.list_agents().await;
        let busy = paused.iter().filter(|(_, id)| agents.iter().any(|a| a.id == *id && !a.paused && !matches!(a.phase.as_str(), "failed" | "cancelled"))).count();
        if busy == 0 || tokio::time::Instant::now() > deadline {
            tracing::info!(tasks = paused.len(), still_busy = busy, "tasks paused for the stop");
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    r.hub.shutdown();
}
