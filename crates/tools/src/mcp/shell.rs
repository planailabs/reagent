//! Commands (foreground, background) and terminals, run by the supervisor.

use std::sync::Arc;

use reagent_store::{Project, Task};
use reagent_supervisor::{self as sup, PtyArgs, SpawnArgs, Waited};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::service::RequestContext;
use rmcp::{RoleServer, schemars, tool, tool_router};
use serde::Deserialize;

use super::{caller, more, resolve};
use crate::app::App;

/// Default time a foreground command may take before it goes on in the background.
const FG_TIMEOUT: u64 = 600;
/// Output an answer carries (characters, the end kept).
const OUTPUT: usize = 60_000;

#[derive(Clone)]
pub struct ShellTools(pub Arc<App>);

#[derive(Clone)]
pub struct PtyTools(pub Arc<App>);

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Exec {
    /// A shell command line (sh -c).
    pub cmd: String,
    /// Where (relative to your working directory; default: it).
    pub cwd: Option<String>,
    /// Seconds before it moves to the background (default 600).
    pub timeout: Option<u64>,
    /// Text for its stdin.
    pub stdin: Option<String>,
    /// false: not in the project's nix dev shell (default: in it, when the project uses one).
    pub devshell: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ExecBg {
    pub cmd: String,
    pub cwd: Option<String>,
    /// A name to tell it by (`dev server`).
    pub name: Option<String>,
    /// false: not in the project's nix dev shell.
    pub devshell: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct JobOutput {
    pub job: String,
    pub from: Option<usize>,
    pub to: Option<usize>,
    /// The last lines (default 100).
    pub tail: Option<usize>,
    /// Only lines matching this regex (case-insensitive).
    pub pattern: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct JobWait {
    pub job: String,
    /// Seconds (default 300, at most 3600).
    pub timeout: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct JobInput {
    pub job: String,
    pub text: String,
    /// Close its stdin after.
    #[serde(default)]
    pub close: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct JobKill {
    pub job: String,
    /// The signal number (default 15, TERM; 9 kills).
    pub signal: Option<i32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct PtyOpen {
    /// A command (default: a shell).
    pub cmd: Option<String>,
    pub cwd: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    /// false: not in the project's nix dev shell.
    pub devshell: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct PtySend {
    pub pty: String,
    /// Text, with keys as <enter>, <tab>, <esc>, <up>, <down>, <left>, <right>, <bs>, <C-c>, <C-d>, …
    pub keys: String,
    /// Wait for this many ms of quiet before showing the screen (default 300).
    pub quiet_ms: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct PtyScreen {
    pub pty: String,
    pub quiet_ms: Option<u64>,
    /// Lines of scrollback above the screen (default 0).
    pub scrollback: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct PtyId {
    pub pty: String,
}

async fn place(app: &App, t: &Task, p: &Project, tool: &str, cwd: Option<&str>) -> Result<String, String> {
    Ok(resolve(app, t, p, tool, cwd.unwrap_or("."), false).await?.display().to_string())
}

/// What a task's commands get: the project's env, its secrets, reagent's PATH, who it is.
pub async fn env(app: &App, t: &Task, p: &Project) -> Vec<(String, String)> {
    let mut e = project_env(app, p).await;
    e.push(("REAGENT_TASK".into(), t.id.clone()));
    e
}

/// What any command of a project gets (a task's, a trigger's).
pub async fn project_env(app: &App, p: &Project) -> Vec<(String, String)> {
    let mut e: Vec<(String, String)> = p.env.0.iter().map(|(k, v)| (k.clone(), v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))).collect();
    match app.store.secrets_for(&p.slug).await {
        Ok(secrets) => e.extend(secrets.into_iter().map(|s| (s.name, s.value))),
        Err(err) => tracing::error!(project = %p.slug, error = %err, "secrets can't be read: commands run without them"),
    }
    // reagent's PATH as it is now: the supervisor may be older (it outlives
    // restarts) and its own PATH stale.
    if let Ok(path) = std::env::var("PATH") {
        e.push(("PATH".into(), path));
    }
    e.push(("REAGENT_PROJECT".into(), p.slug.clone()));
    // Commands are run unattended: nothing should wait for a pager or an editor.
    e.push(("PAGER".into(), "cat".into()));
    e.push(("GIT_PAGER".into(), "cat".into()));
    e.push(("GIT_EDITOR".into(), "true".into()));
    e
}

fn own_job(t: &Task, j: &sup::JobMeta) -> Result<(), String> {
    if j.owner.as_deref() != Some(t.id.as_str()) {
        return Err(format!("{} isn't one of your jobs", j.id));
    }
    Ok(())
}

/// The end of a job's output (at most `OUTPUT` characters), with a note when cut.
fn output_of(app: &App, job: &str) -> String {
    let all = sup::tail(&app.paths.data, job, 100_000);
    let n = all.chars().count();
    if n <= OUTPUT {
        return all;
    }
    let kept: String = all.chars().skip(n - OUTPUT).collect();
    format!("[… the first {} characters are left out; shell.job_output(job: {job:?}, from: 1) reads them]\n{kept}", n - OUTPUT)
}

#[tool_router(server_handler)]
impl ShellTools {
    #[tool(description = "Run a command and wait for it: its exit code and output. If it takes longer than `timeout` seconds (or the person moves it), it goes on in the background as a job (you get a message when it ends).")]
    async fn exec(&self, Parameters(a): Parameters<Exec>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let cwd = place(&self.0, &t, &p, "shell.exec", a.cwd.as_deref()).await?;
        let started = std::time::Instant::now();
        let cmd = crate::devshell::wrap(&p, std::path::Path::new(&cwd), &a.cmd, a.devshell.unwrap_or(true));
        let j = self.0.sup.spawn(SpawnArgs { cmd, cwd, env: env(&self.0, &t, &p).await, owner: Some(t.id.clone()), name: Some(a.cmd.clone()), fg: true, stdin: a.stdin }).await?;
        self.0.fg_waiting.lock().unwrap().insert(j.id.clone());
        let r = self.0.sup.wait(&j.id, a.timeout.unwrap_or(FG_TIMEOUT).clamp(1, 24 * 3600) * 1000).await;
        let (w, m) = match r {
            Ok(x) => x,
            Err(e) => {
                self.0.fg_waiting.lock().unwrap().remove(&j.id);
                return Err(e);
            }
        };
        let secs = started.elapsed().as_secs_f32();
        match w {
            Waited::Exited => {
                // Acknowledged before it stops counting as waited for: its end isn't also a message.
                let _ = self.0.sup.ack(&j.id).await;
                self.0.fg_waiting.lock().unwrap().remove(&j.id);
                let how = match (m.exit, m.signal) {
                    (Some(c), _) => format!("exit {c}"),
                    (_, Some(s)) => format!("killed by signal {s}"),
                    _ => "ended".into(),
                };
                Ok(format!("{how} ({secs:.1}s, job {})\n{}", j.id, output_of(&self.0, &j.id)))
            }
            Waited::Backgrounded | Waited::Timeout => {
                if w == Waited::Timeout {
                    let _ = self.0.sup.background(&j.id).await;
                }
                // In the background now: its end comes as a message.
                self.0.fg_waiting.lock().unwrap().remove(&j.id);
                Ok(format!(
                    "still running after {secs:.0}s: it goes on in the background as job {} (you get a message when it ends; shell.job_output reads it, shell.job_kill stops it)\noutput so far:\n{}",
                    j.id,
                    sup::tail(&self.0.paths.data, &j.id, 40)
                ))
            }
        }
    }

    #[tool(description = "Start a command in the background (a server, a watcher, a long build): its job id at once; you get a message when it ends.")]
    async fn exec_bg(&self, Parameters(a): Parameters<ExecBg>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let cwd = place(&self.0, &t, &p, "shell.exec_bg", a.cwd.as_deref()).await?;
        let cmd = crate::devshell::wrap(&p, std::path::Path::new(&cwd), &a.cmd, a.devshell.unwrap_or(true));
        let name = a.name.or_else(|| (cmd != a.cmd).then(|| a.cmd.clone()));
        let j = self.0.sup.spawn(SpawnArgs { cmd, cwd, env: env(&self.0, &t, &p).await, owner: Some(t.id.clone()), name, fg: false, stdin: None }).await?;
        Ok(format!("started job {} (pid {})", j.id, j.pid.unwrap_or(0)))
    }

    #[tool(description = "Your jobs: running and ended, with their commands and exit codes.")]
    async fn jobs(&self, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, _) = caller(&self.0, &ctx).await?;
        let jobs = self.0.sup.jobs(Some(&t.id)).await?;
        if jobs.is_empty() {
            return Ok("no jobs".into());
        }
        Ok(jobs
            .iter()
            .map(|j| {
                let state = match (j.running(), j.exit, j.signal) {
                    (true, _, _) => if j.fg { "running (foreground)".to_string() } else { "running".to_string() },
                    (false, Some(c), _) => format!("exit {c}"),
                    (false, _, Some(s)) => format!("signal {s}"),
                    _ => "ended".into(),
                };
                format!("{}  {state}  {}", j.id, j.name.as_deref().unwrap_or(&j.cmd))
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    #[tool(description = "A job's output: lines from..to, the last `tail`, or those matching `pattern`.")]
    async fn job_output(&self, Parameters(a): Parameters<JobOutput>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, _) = caller(&self.0, &ctx).await?;
        own_job(&t, &self.0.sup.job(&a.job).await?)?;
        let v = sup::output(&self.0.paths.data, &a.job, a.from, a.to, a.tail, a.pattern.as_deref())?;
        if let Some(m) = v.get("matches") {
            let lines: Vec<String> = m.as_array().into_iter().flatten().map(|l| format!("{}: {}", l["line"], l["text"].as_str().unwrap_or(""))).collect();
            return Ok(format!("{}{}", lines.join("\n"), more(format!("{} matching lines of {}", lines.len(), v["lines"]))));
        }
        let lines: Vec<String> = v["text"].as_array().into_iter().flatten().map(|l| format!("{}│{}", l["line"], l["text"].as_str().unwrap_or(""))).collect();
        let note = match v["next"].as_u64() {
            Some(n) => format!("lines {}-{} of {}; from: {n} reads on", v["from"], v["to"], v["lines"]),
            None => format!("lines {}-{} of {}: the end", v["from"], v["to"], v["lines"]),
        };
        Ok(format!("{}{}", lines.join("\n"), more(note)))
    }

    #[tool(description = "Wait for a job to end (or the timeout): how it ended and its last lines.")]
    async fn job_wait(&self, Parameters(a): Parameters<JobWait>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, _) = caller(&self.0, &ctx).await?;
        own_job(&t, &self.0.sup.job(&a.job).await?)?;
        let (w, m) = self.0.sup.wait(&a.job, a.timeout.unwrap_or(300).clamp(1, 3600) * 1000).await?;
        if w != Waited::Exited {
            return Ok(format!("{} is still running\nlast lines:\n{}", a.job, sup::tail(&self.0.paths.data, &a.job, 20)));
        }
        let _ = self.0.sup.ack(&a.job).await;
        Ok(format!("{} ended: exit {:?}, signal {:?}\nlast lines:\n{}", a.job, m.exit, m.signal, sup::tail(&self.0.paths.data, &a.job, 40)))
    }

    #[tool(description = "Write to a running job's stdin.")]
    async fn job_input(&self, Parameters(a): Parameters<JobInput>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, _) = caller(&self.0, &ctx).await?;
        own_job(&t, &self.0.sup.job(&a.job).await?)?;
        self.0.sup.input(&a.job, &a.text, a.close).await?;
        Ok("sent".into())
    }

    #[tool(description = "Send a job (its whole process group) a signal: TERM by default, 9 to kill.")]
    async fn job_kill(&self, Parameters(a): Parameters<JobKill>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, _) = caller(&self.0, &ctx).await?;
        own_job(&t, &self.0.sup.job(&a.job).await?)?;
        self.0.sup.kill(&a.job, a.signal).await?;
        let (_, m) = self.0.sup.wait(&a.job, 5000).await?;
        Ok(if m.running() { format!("signalled {}; it's still running", a.job) } else { format!("{} ended", a.job) })
    }
}

async fn own_pty(app: &App, t: &Task, id: &str) -> Result<(), String> {
    let ptys = app.sup.ptys(Some(&t.id)).await?;
    if !ptys.iter().any(|p| p.id == id) {
        return Err(format!("{id} isn't one of your terminals"));
    }
    Ok(())
}

fn screen_text(v: &serde_json::Value) -> String {
    let mut out = String::new();
    if let Some(s) = v["scrollback"].as_str().filter(|s| !s.is_empty()) {
        out.push_str(s);
        out.push_str("\n──── screen ────\n");
    }
    out.push_str(v["screen"].as_str().unwrap_or(""));
    out.push_str(&more(format!("cursor at row {}, col {}{}", v["cursor"]["row"], v["cursor"]["col"], if v["alive"] == false { "; the program has ended" } else { "" })));
    out
}

#[tool_router(server_handler)]
impl PtyTools {
    #[tool(description = "Open a terminal (a shell, or a command) for interactive programs: its id. The person can watch and type into it too.")]
    async fn pty_open(&self, Parameters(a): Parameters<PtyOpen>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let cwd = place(&self.0, &t, &p, "pty.pty_open", a.cwd.as_deref()).await?;
        let cmd = crate::devshell::wrap_terminal(&p, std::path::Path::new(&cwd), a.cmd.as_deref(), a.devshell.unwrap_or(true));
        let m = self.0.sup.pty_open(PtyArgs { cmd, cwd, env: env(&self.0, &t, &p).await, owner: Some(t.id.clone()), cols: a.cols.unwrap_or(120), rows: a.rows.unwrap_or(32) }).await?;
        let screen = self.0.sup.pty_screen(&m.id, 500, 0).await?;
        Ok(format!("terminal {} ({}x{})\n{}", m.id, m.cols, m.rows, screen_text(&screen)))
    }

    #[tool(description = "Type into a terminal (keys like <enter>, <C-c>, <up>), then see its screen once it's quiet.")]
    async fn pty_send(&self, Parameters(a): Parameters<PtySend>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, _) = caller(&self.0, &ctx).await?;
        own_pty(&self.0, &t, &a.pty).await?;
        self.0.sup.pty_send(&a.pty, &sup::keys(&a.keys)).await?;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        Ok(screen_text(&self.0.sup.pty_screen(&a.pty, a.quiet_ms.unwrap_or(300), 0).await?))
    }

    #[tool(description = "A terminal's screen (and scrollback lines above it).")]
    async fn pty_screen(&self, Parameters(a): Parameters<PtyScreen>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, _) = caller(&self.0, &ctx).await?;
        own_pty(&self.0, &t, &a.pty).await?;
        Ok(screen_text(&self.0.sup.pty_screen(&a.pty, a.quiet_ms.unwrap_or(0), a.scrollback.unwrap_or(0)).await?))
    }

    #[tool(description = "Close a terminal (its program is ended).")]
    async fn pty_close(&self, Parameters(a): Parameters<PtyId>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, _) = caller(&self.0, &ctx).await?;
        own_pty(&self.0, &t, &a.pty).await?;
        self.0.sup.pty_close(&a.pty).await?;
        Ok(format!("closed {}", a.pty))
    }

    #[tool(description = "Your open terminals.")]
    async fn ptys(&self, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, _) = caller(&self.0, &ctx).await?;
        let v = self.0.sup.ptys(Some(&t.id)).await?;
        Ok(if v.is_empty() { "no terminals".into() } else { v.iter().map(|p| format!("{}  {}  {}", p.id, p.cmd.as_deref().unwrap_or("shell"), if p.alive { "running" } else { "ended" })).collect::<Vec<_>>().join("\n") })
    }
}
