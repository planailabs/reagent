//! The supervisor process: owns jobs and terminals so they outlive the
//! `reagent up` that started them. Each job is `sh -c <cmd>` in its own
//! process group, stdout and stderr appended to `jobs/<id>/output.log`, its
//! state in `jobs/<id>/meta.json`. Terminals keep their raw bytes in
//! `ptys/<id>/output.log` and a rendered screen in memory.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use portable_pty::{CommandBuilder, PtySize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, watch};

use crate::proto::*;

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Scrollback the screen keeps (lines).
const SCROLLBACK: usize = 5000;

struct Job {
    meta: JobMeta,
    stdin: Option<tokio::process::ChildStdin>,
    /// Running, ended, or moved to the background: what waiters watch.
    state: watch::Sender<u64>,
}

struct Pty {
    meta: PtyMeta,
    writer: Box<dyn Write + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
    killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
    screen: Arc<Mutex<vt100::Parser>>,
    last_output: Arc<Mutex<Instant>>,
}

pub struct Supervisor {
    dir: PathBuf,
    jobs: Mutex<HashMap<String, Job>>,
    ptys: Mutex<HashMap<String, Pty>>,
    next: Mutex<u64>,
    events: broadcast::Sender<Event>,
    stop: tokio_util_free::Stop,
}

/// A tiny stop signal (no extra dependency).
mod tokio_util_free {
    #[derive(Default)]
    pub struct Stop(tokio::sync::Notify, std::sync::atomic::AtomicBool);
    impl Stop {
        pub fn fire(&self) {
            self.1.store(true, std::sync::atomic::Ordering::SeqCst);
            self.0.notify_waiters();
        }
        pub async fn wait(&self) {
            loop {
                let n = self.0.notified();
                if self.1.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                n.await;
            }
        }
    }
}

pub fn job_dir(dir: &Path, id: &str) -> PathBuf {
    dir.join("jobs").join(id)
}

pub fn pty_dir(dir: &Path, id: &str) -> PathBuf {
    dir.join("ptys").join(id)
}

impl Supervisor {
    /// Loads what earlier runs left: jobs that were running when the
    /// supervisor died are marked `lost`.
    pub fn new(dir: &Path) -> anyhow::Result<Arc<Self>> {
        std::fs::create_dir_all(dir.join("jobs"))?;
        std::fs::create_dir_all(dir.join("ptys"))?;
        let mut jobs = HashMap::new();
        let mut next = 0;
        for e in std::fs::read_dir(dir.join("jobs"))?.flatten() {
            let Ok(text) = std::fs::read_to_string(e.path().join("meta.json")) else { continue };
            let Ok(mut meta) = serde_json::from_str::<JobMeta>(&text) else { continue };
            if let Some(n) = meta.id.strip_prefix('j').and_then(|n| n.parse::<u64>().ok()) {
                next = next.max(n);
            }
            if meta.ended.is_none() {
                meta.ended = Some(now());
                meta.lost = true;
                let _ = std::fs::write(e.path().join("meta.json"), serde_json::to_vec_pretty(&meta)?);
            }
            jobs.insert(meta.id.clone(), Job { meta, stdin: None, state: watch::channel(1).0 });
        }
        for e in std::fs::read_dir(dir.join("ptys"))?.flatten() {
            if let Some(n) = e.file_name().to_string_lossy().strip_prefix('p').and_then(|n| n.parse::<u64>().ok()) {
                next = next.max(n);
            }
        }
        Ok(Arc::new(Supervisor {
            dir: dir.to_path_buf(),
            jobs: Mutex::new(jobs),
            ptys: Mutex::new(HashMap::new()),
            next: Mutex::new(next),
            events: broadcast::channel(4096).0,
            stop: Default::default(),
        }))
    }

    fn id(&self, prefix: char) -> String {
        let mut n = self.next.lock().unwrap();
        *n += 1;
        format!("{prefix}{n}")
    }

    fn save(&self, meta: &JobMeta) {
        let d = job_dir(&self.dir, &meta.id);
        if let Err(e) = std::fs::write(d.join("meta.json"), serde_json::to_vec_pretty(meta).unwrap()) {
            tracing::error!(job = %meta.id, error = %e, "saving a job's state");
        }
    }

    /// Serves on `socket` until `shutdown`.
    pub async fn serve(self: Arc<Self>, socket: &Path) -> anyhow::Result<()> {
        let _ = std::fs::remove_file(socket);
        let l = UnixListener::bind(socket)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
        }
        tracing::info!(socket = %socket.display(), "supervisor listening");
        loop {
            tokio::select! {
                _ = self.stop.wait() => break,
                c = l.accept() => {
                    let (s, _) = c?;
                    tokio::spawn(self.clone().conn(s));
                }
            }
        }
        let _ = std::fs::remove_file(socket);
        Ok(())
    }

    async fn conn(self: Arc<Self>, s: UnixStream) {
        let (r, mut w) = s.into_split();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let writer = tokio::spawn(async move {
            while let Some(mut line) = rx.recv().await {
                line.push('\n');
                if w.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        let mut lines = BufReader::new(r).lines();
        let mut subs: Vec<tokio::task::JoinHandle<()>> = vec![];
        while let Ok(Some(line)) = lines.next_line().await {
            let v: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx.send(json!({"err": format!("not JSON: {e}")}).to_string());
                    continue;
                }
            };
            let id = v["rid"].clone();
            let req: Request = match serde_json::from_value(v) {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(json!({"rid": id, "err": e.to_string()}).to_string());
                    continue;
                }
            };
            if let Request::Subscribe { job, pty, exits } = &req {
                subs.push(self.clone().subscribe(job.clone(), pty.clone(), *exits, tx.clone()));
                let _ = tx.send(json!({"rid": id, "ok": true}).to_string());
                continue;
            }
            // Waits and screens take a while: answered when done, others go on.
            let (me, tx2) = (self.clone(), tx.clone());
            tokio::spawn(async move {
                let out = match me.handle(req).await {
                    Ok(v) => json!({"rid": id, "ok": v}),
                    Err(e) => json!({"rid": id, "err": e}),
                };
                let _ = tx2.send(out.to_string());
            });
        }
        for s in subs {
            s.abort();
        }
        drop(tx);
        let _ = writer.await;
    }

    fn subscribe(self: Arc<Self>, job: Option<String>, pty: Option<String>, exits: bool, tx: tokio::sync::mpsc::UnboundedSender<String>) -> tokio::task::JoinHandle<()> {
        let mut rx = self.events.subscribe();
        // A terminal's scrollback first, so a viewer sees what's there.
        if let Some(p) = &pty
            && let Ok(bytes) = std::fs::read(pty_dir(&self.dir, p).join("output.log"))
        {
            let tail = &bytes[bytes.len().saturating_sub(256 * 1024)..];
            let _ = tx.send(json!({"event": Event::Pty { pty: p.clone(), data: b64().encode(tail) }}).to_string());
        }
        tokio::spawn(async move {
            loop {
                let e = match rx.recv().await {
                    Ok(e) => e,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => return,
                };
                let wanted = match &e {
                    Event::Output { job: j, .. } | Event::Backgrounded { job: j } => job.as_deref() == Some(j.as_str()),
                    Event::Exit { job: j } => exits || job.as_deref() == Some(j.id.as_str()),
                    Event::Pty { pty: p, .. } | Event::PtyExit { pty: p } => pty.as_deref() == Some(p.as_str()),
                };
                if wanted && tx.send(json!({"event": e}).to_string()).is_err() {
                    return;
                }
            }
        })
    }

    async fn handle(self: Arc<Self>, req: Request) -> Result<Value, String> {
        match req {
            Request::Ping => Ok(json!({"pid": std::process::id()})),
            Request::Spawn(a) => self.spawn(a).await.map(|m| json!(m)),
            Request::Jobs { owner } => {
                let mut v: Vec<JobMeta> = self.jobs.lock().unwrap().values().map(|j| j.meta.clone()).filter(|m| owner.is_none() || m.owner == owner).collect();
                v.sort_by_key(|m| (m.started, m.id[1..].parse::<u64>().unwrap_or(0)));
                Ok(json!(v))
            }
            Request::Job { id } => self.meta(&id).map(|m| json!(m)),
            Request::Wait { id, timeout_ms } => self.wait(&id, timeout_ms).await.map(|(w, m)| json!({"waited": w, "job": m})),
            Request::Background { id } => {
                let meta = {
                    let mut jobs = self.jobs.lock().unwrap();
                    let j = jobs.get_mut(&id).ok_or_else(|| format!("no job {id}"))?;
                    if j.meta.fg && j.meta.running() {
                        j.meta.fg = false;
                        j.meta.backgrounded = true;
                        j.state.send_modify(|n| *n += 1);
                    }
                    j.meta.clone()
                };
                self.save(&meta);
                let _ = self.events.send(Event::Backgrounded { job: id });
                Ok(json!(meta))
            }
            Request::Input { id, text, close } => {
                let stdin = self.jobs.lock().unwrap().get_mut(&id).ok_or_else(|| format!("no job {id}"))?.stdin.take();
                let Some(mut stdin) = stdin else { return Err(format!("{id} takes no more input")) };
                stdin.write_all(text.as_bytes()).await.map_err(|e| e.to_string())?;
                stdin.flush().await.map_err(|e| e.to_string())?;
                if !close && let Some(j) = self.jobs.lock().unwrap().get_mut(&id) {
                    j.stdin = Some(stdin);
                }
                Ok(json!(true))
            }
            Request::Kill { id, signal } => {
                let m = self.meta(&id)?;
                let (Some(pid), true) = (m.pid, m.running()) else { return Err(format!("{id} isn't running")) };
                let sig = nix::sys::signal::Signal::try_from(signal.unwrap_or(15)).map_err(|e| e.to_string())?;
                nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid as i32), sig).map_err(|e| e.to_string())?;
                Ok(json!(true))
            }
            Request::Ack { id } => {
                let meta = {
                    let mut jobs = self.jobs.lock().unwrap();
                    let j = jobs.get_mut(&id).ok_or_else(|| format!("no job {id}"))?;
                    j.meta.acked = true;
                    j.meta.clone()
                };
                self.save(&meta);
                Ok(json!(true))
            }
            Request::PtyOpen(a) => self.pty_open(a).map(|m| json!(m)),
            Request::PtySend { id, data } => {
                let bytes = b64().decode(data).map_err(|e| e.to_string())?;
                let mut ptys = self.ptys.lock().unwrap();
                let p = ptys.get_mut(&id).ok_or_else(|| format!("no terminal {id}"))?;
                p.writer.write_all(&bytes).and_then(|_| p.writer.flush()).map_err(|e| e.to_string())?;
                Ok(json!(true))
            }
            Request::PtyScreen { id, quiet_ms, scrollback } => {
                let (screen, last) = {
                    let ptys = self.ptys.lock().unwrap();
                    let p = ptys.get(&id).ok_or_else(|| format!("no terminal {id}"))?;
                    (p.screen.clone(), p.last_output.clone())
                };
                let deadline = Instant::now() + Duration::from_secs(10);
                let quiet = Duration::from_millis(quiet_ms.min(10_000));
                while Instant::now() < deadline && last.lock().unwrap().elapsed() < quiet {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                let alive = self.ptys.lock().unwrap().get(&id).is_some_and(|p| p.meta.alive);
                Ok(render(&screen, scrollback, alive))
            }
            Request::PtyResize { id, cols, rows } => {
                let mut ptys = self.ptys.lock().unwrap();
                let p = ptys.get_mut(&id).ok_or_else(|| format!("no terminal {id}"))?;
                p.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).map_err(|e| e.to_string())?;
                p.screen.lock().unwrap().screen_mut().set_size(rows, cols);
                p.meta.cols = cols;
                p.meta.rows = rows;
                Ok(json!(p.meta))
            }
            Request::PtyClose { id } => {
                let p = self.ptys.lock().unwrap().remove(&id).ok_or_else(|| format!("no terminal {id}"))?;
                let mut killer = p.killer;
                let _ = killer.kill();
                let _ = self.events.send(Event::PtyExit { pty: id });
                Ok(json!(true))
            }
            Request::Ptys { owner } => {
                let mut v: Vec<PtyMeta> = self.ptys.lock().unwrap().values().map(|p| p.meta.clone()).filter(|m| owner.is_none() || m.owner == owner).collect();
                v.sort_by_key(|m| (m.started, m.id.clone()));
                Ok(json!(v))
            }
            Request::Subscribe { .. } => unreachable!("handled by the connection"),
            Request::Shutdown => {
                self.kill_all();
                let me = self.clone();
                tokio::spawn(async move {
                    // Let the answer go out first.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    me.stop.fire();
                });
                Ok(json!(true))
            }
        }
    }

    fn meta(&self, id: &str) -> Result<JobMeta, String> {
        self.jobs.lock().unwrap().get(id).map(|j| j.meta.clone()).ok_or_else(|| format!("no job {id}"))
    }

    async fn spawn(self: &Arc<Self>, a: SpawnArgs) -> Result<JobMeta, String> {
        let id = self.id('j');
        let d = job_dir(&self.dir, &id);
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        let log = tokio::fs::File::create(d.join("output.log")).await.map_err(|e| e.to_string())?;
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(&a.cmd).current_dir(&a.cwd).envs(a.env.iter().cloned()).process_group(0);
        cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("can't start it: {e}"))?;
        let mut meta = JobMeta {
            id: id.clone(),
            cmd: a.cmd,
            cwd: a.cwd,
            owner: a.owner,
            name: a.name,
            pid: child.id(),
            fg: a.fg,
            backgrounded: false,
            started: now(),
            ended: None,
            exit: None,
            signal: None,
            lost: false,
            acked: false,
        };
        let mut stdin = child.stdin.take();
        if let (Some(text), Some(s)) = (&a.stdin, stdin.as_mut()) {
            let _ = s.write_all(text.as_bytes()).await;
        }
        self.save(&meta);
        let (state, _) = watch::channel(0);
        self.jobs.lock().unwrap().insert(id.clone(), Job { meta: meta.clone(), stdin, state });
        // Both streams into one log, in the order they come.
        let log = Arc::new(tokio::sync::Mutex::new(log));
        let pumps: Vec<_> = [child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>), child.stderr.take().map(|s| Box::new(s) as _)]
            .into_iter()
            .flatten()
            .map(|mut s| {
                let (log, events, id) = (log.clone(), self.events.clone(), id.clone());
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 16 * 1024];
                    loop {
                        match s.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                let _ = log.lock().await.write_all(&buf[..n]).await;
                                let _ = events.send(Event::Output { job: id.clone(), text: String::from_utf8_lossy(&buf[..n]).into_owned() });
                            }
                        }
                    }
                })
            })
            .collect();
        let (me, jid) = (self.clone(), id.clone());
        tokio::spawn(async move {
            let id = jid;
            let status = child.wait().await;
            for p in pumps {
                let _ = p.await;
            }
            let _ = log.lock().await.flush().await;
            let meta = {
                let mut jobs = me.jobs.lock().unwrap();
                let Some(j) = jobs.get_mut(&id) else { return };
                j.meta.ended = Some(now());
                if let Ok(s) = status {
                    use std::os::unix::process::ExitStatusExt;
                    j.meta.exit = s.code();
                    j.meta.signal = s.signal();
                }
                j.stdin = None;
                j.state.send_modify(|n| *n += 1);
                j.meta.clone()
            };
            me.save(&meta);
            let _ = me.events.send(Event::Exit { job: meta });
        });
        meta.pid = self.meta(&id)?.pid;
        Ok(meta)
    }

    async fn wait(&self, id: &str, timeout_ms: u64) -> Result<(Waited, JobMeta), String> {
        let mut rx = self.jobs.lock().unwrap().get(id).ok_or_else(|| format!("no job {id}"))?.state.subscribe();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        // Moving to the background frees those who waited for a foreground job.
        let was_fg = self.meta(id)?.fg;
        loop {
            let m = self.meta(id)?;
            if !m.running() {
                return Ok((Waited::Exited, m));
            }
            if was_fg && m.backgrounded {
                return Ok((Waited::Backgrounded, m));
            }
            match tokio::time::timeout_at(deadline, rx.changed()).await {
                Err(_) => return Ok((Waited::Timeout, self.meta(id)?)),
                Ok(Err(_)) => return Err("the job went away".into()),
                Ok(Ok(())) => {}
            }
        }
    }

    fn pty_open(self: &Arc<Self>, a: PtyArgs) -> Result<PtyMeta, String> {
        let id = self.id('p');
        let d = pty_dir(&self.dir, &id);
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        let sys = portable_pty::native_pty_system();
        let pair = sys.openpty(PtySize { rows: a.rows, cols: a.cols, pixel_width: 0, pixel_height: 0 }).map_err(|e| e.to_string())?;
        let mut cmd = match &a.cmd {
            Some(c) => {
                let mut b = CommandBuilder::new("sh");
                b.args(["-c", c]);
                b
            }
            None => CommandBuilder::new_default_prog(),
        };
        cmd.cwd(&a.cwd);
        cmd.env("TERM", "xterm-256color");
        for (k, v) in &a.env {
            cmd.env(k, v);
        }
        let mut child = pair.slave.spawn_command(cmd).map_err(|e| format!("can't start it: {e}"))?;
        drop(pair.slave);
        let killer = child.clone_killer();
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let screen = Arc::new(Mutex::new(vt100::Parser::new(a.rows, a.cols, SCROLLBACK)));
        let last_output = Arc::new(Mutex::new(Instant::now()));
        let meta = PtyMeta { id: id.clone(), cmd: a.cmd, cwd: a.cwd, owner: a.owner, cols: a.cols, rows: a.rows, started: now(), alive: true };
        let mut log = std::fs::File::create(d.join("output.log")).map_err(|e| e.to_string())?;
        let (scr, last, events, pid) = (screen.clone(), last_output.clone(), self.events.clone(), id.clone());
        let me = self.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let _ = log.write_all(&buf[..n]);
                        scr.lock().unwrap().process(&buf[..n]);
                        *last.lock().unwrap() = Instant::now();
                        let _ = events.send(Event::Pty { pty: pid.clone(), data: b64().encode(&buf[..n]) });
                    }
                }
            }
            let _ = child.wait();
            if let Some(p) = me.ptys.lock().unwrap().get_mut(&pid) {
                p.meta.alive = false;
            }
            let _ = events.send(Event::PtyExit { pty: pid });
        });
        self.ptys.lock().unwrap().insert(id, Pty { meta: meta.clone(), writer, master: pair.master, killer, screen, last_output });
        Ok(meta)
    }

    fn kill_all(&self) {
        for j in self.jobs.lock().unwrap().values() {
            if let (Some(pid), true) = (j.meta.pid, j.meta.running()) {
                let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGTERM);
            }
        }
        for (_, p) in self.ptys.lock().unwrap().drain() {
            let mut k = p.killer;
            let _ = k.kill();
        }
    }
}

/// A terminal's screen as text: `scrollback` lines above it, the cursor.
fn render(screen: &Mutex<vt100::Parser>, scrollback: usize, alive: bool) -> Value {
    let mut p = screen.lock().unwrap();
    let mut above = vec![];
    if scrollback > 0 {
        let rows = p.screen().size().0 as usize;
        // vt100 shows scrollback by offsetting the visible window.
        let mut off = 0;
        while off < scrollback {
            let step = (scrollback - off).min(rows);
            p.screen_mut().set_scrollback(off + step);
            let shown = p.screen().scrollback();
            if shown <= off {
                break;
            }
            let lines: Vec<String> = p.screen().contents().lines().take(shown - off).map(String::from).collect();
            above.splice(0..0, lines);
            off = shown;
        }
        p.screen_mut().set_scrollback(0);
    }
    let s = p.screen();
    let text = s.contents();
    let (row, col) = s.cursor_position();
    json!({"scrollback": above.join("\n"), "screen": text.trim_end_matches('\n'), "cursor": {"row": row, "col": col}, "alive": alive})
}
