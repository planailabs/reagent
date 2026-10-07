//! Talking to the supervisor: one connection for requests (it reconnects
//! when the supervisor restarted), and separate ones for event streams.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};

use crate::proto::*;

type Pending = Arc<std::sync::Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

struct Conn {
    tx: mpsc::UnboundedSender<String>,
    pending: Pending,
    alive: Arc<std::sync::atomic::AtomicBool>,
}

pub struct Client {
    socket: PathBuf,
    conn: Mutex<Option<Conn>>,
    next: AtomicU64,
    /// Every job's end (`Event::Exit`), from the request connection.
    exits: broadcast::Sender<Event>,
}

async fn open(socket: &Path, events: Option<broadcast::Sender<Event>>) -> std::io::Result<Conn> {
    let s = UnixStream::connect(socket).await?;
    let (r, mut w) = s.into_split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let pending: Pending = Default::default();
    let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    tokio::spawn(async move {
        while let Some(mut l) = rx.recv().await {
            l.push('\n');
            if w.write_all(l.as_bytes()).await.is_err() {
                break;
            }
        }
    });
    let (p, a) = (pending.clone(), alive.clone());
    tokio::spawn(async move {
        let mut lines = BufReader::new(r).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            if let Some(e) = v.get("event") {
                if let (Some(tx), Ok(e)) = (&events, serde_json::from_value::<Event>(e.clone())) {
                    let _ = tx.send(e);
                }
                continue;
            }
            let Some(id) = v["rid"].as_u64() else { continue };
            if let Some(tx) = p.lock().unwrap().remove(&id) {
                let r = match v.get("err") {
                    Some(e) => Err(e.as_str().unwrap_or("error").to_string()),
                    None => Ok(v["ok"].clone()),
                };
                let _ = tx.send(r);
            }
        }
        a.store(false, Ordering::SeqCst);
        // Whoever still waits learns the supervisor went away.
        for (_, tx) in p.lock().unwrap().drain() {
            let _ = tx.send(Err("the supervisor went away".into()));
        }
    });
    Ok(Conn { tx, pending, alive })
}

impl Client {
    /// A client for the supervisor on `socket` (connects on first use).
    pub fn new(socket: &Path) -> Arc<Self> {
        Arc::new(Client { socket: socket.into(), conn: Mutex::new(None), next: AtomicU64::new(1), exits: broadcast::channel(1024).0 })
    }

    /// Every job's end, as the supervisor reports it.
    pub fn exits(&self) -> broadcast::Receiver<Event> {
        self.exits.subscribe()
    }

    pub async fn call(&self, req: Request) -> Result<Value, String> {
        let mut req = serde_json::to_value(&req).unwrap();
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        req["rid"] = json!(id);
        let rx = {
            let mut c = self.conn.lock().await;
            if c.as_ref().is_none_or(|c| !c.alive.load(Ordering::SeqCst)) {
                let conn = open(&self.socket, Some(self.exits.clone())).await.map_err(|e| format!("the supervisor can't be reached: {e}"))?;
                // This connection hears every job's end.
                let mut sub = serde_json::to_value(Request::Subscribe { job: None, pty: None, exits: true }).unwrap();
                sub["rid"] = json!(0);
                let _ = conn.tx.send(sub.to_string());
                *c = Some(conn);
            }
            let c = c.as_ref().unwrap();
            let (tx, rx) = oneshot::channel();
            c.pending.lock().unwrap().insert(id, tx);
            c.tx.send(req.to_string()).map_err(|_| "the supervisor went away".to_string())?;
            rx
        };
        rx.await.map_err(|_| "the supervisor went away".to_string())?
    }

    async fn typed<T: serde::de::DeserializeOwned>(&self, req: Request) -> Result<T, String> {
        serde_json::from_value(self.call(req).await?).map_err(|e| e.to_string())
    }

    pub async fn ping(&self) -> Result<Value, String> {
        self.call(Request::Ping).await
    }

    pub async fn spawn(&self, a: SpawnArgs) -> Result<JobMeta, String> {
        self.typed(Request::Spawn(a)).await
    }

    pub async fn jobs(&self, owner: Option<&str>) -> Result<Vec<JobMeta>, String> {
        self.typed(Request::Jobs { owner: owner.map(Into::into) }).await
    }

    pub async fn job(&self, id: &str) -> Result<JobMeta, String> {
        self.typed(Request::Job { id: id.into() }).await
    }

    pub async fn wait(&self, id: &str, timeout_ms: u64) -> Result<(Waited, JobMeta), String> {
        let v = self.call(Request::Wait { id: id.into(), timeout_ms }).await?;
        Ok((serde_json::from_value(v["waited"].clone()).map_err(|e| e.to_string())?, serde_json::from_value(v["job"].clone()).map_err(|e| e.to_string())?))
    }

    pub async fn background(&self, id: &str) -> Result<JobMeta, String> {
        self.typed(Request::Background { id: id.into() }).await
    }

    pub async fn input(&self, id: &str, text: &str, close: bool) -> Result<(), String> {
        self.call(Request::Input { id: id.into(), text: text.into(), close }).await.map(|_| ())
    }

    pub async fn kill(&self, id: &str, signal: Option<i32>) -> Result<(), String> {
        self.call(Request::Kill { id: id.into(), signal }).await.map(|_| ())
    }

    pub async fn ack(&self, id: &str) -> Result<(), String> {
        self.call(Request::Ack { id: id.into() }).await.map(|_| ())
    }

    pub async fn pty_open(&self, a: PtyArgs) -> Result<PtyMeta, String> {
        self.typed(Request::PtyOpen(a)).await
    }

    pub async fn pty_send(&self, id: &str, bytes: &[u8]) -> Result<(), String> {
        use base64::Engine;
        self.call(Request::PtySend { id: id.into(), data: base64::engine::general_purpose::STANDARD.encode(bytes) }).await.map(|_| ())
    }

    pub async fn pty_screen(&self, id: &str, quiet_ms: u64, scrollback: usize) -> Result<Value, String> {
        self.call(Request::PtyScreen { id: id.into(), quiet_ms, scrollback }).await
    }

    pub async fn pty_resize(&self, id: &str, cols: u16, rows: u16) -> Result<PtyMeta, String> {
        self.typed(Request::PtyResize { id: id.into(), cols, rows }).await
    }

    pub async fn pty_close(&self, id: &str) -> Result<(), String> {
        self.call(Request::PtyClose { id: id.into() }).await.map(|_| ())
    }

    pub async fn ptys(&self, owner: Option<&str>) -> Result<Vec<PtyMeta>, String> {
        self.typed(Request::Ptys { owner: owner.map(Into::into) }).await
    }

    pub async fn shutdown(&self) -> Result<(), String> {
        self.call(Request::Shutdown).await.map(|_| ())
    }
}

/// A stream of a job's output or a terminal's bytes, on its own connection
/// (dropped: unsubscribed).
pub async fn stream(socket: &Path, job: Option<&str>, pty: Option<&str>) -> Result<broadcast::Receiver<Event>, String> {
    let (tx, rx) = broadcast::channel(4096);
    let c = open(socket, Some(tx.clone())).await.map_err(|e| format!("the supervisor can't be reached: {e}"))?;
    let mut sub = serde_json::to_value(Request::Subscribe { job: job.map(Into::into), pty: pty.map(Into::into), exits: false }).unwrap();
    sub["rid"] = json!(1);
    c.tx.send(sub.to_string()).map_err(|_| "the supervisor went away".to_string())?;
    // The connection lives while someone listens.
    tokio::spawn(async move {
        let _keep = c;
        while tx.receiver_count() > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
    Ok(rx)
}
