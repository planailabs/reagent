//! The supervisor's protocol: newline-delimited JSON over a Unix socket.
//! A request is `{"rid": n, "op": …, …}`; its answer `{"rid": n, "ok": …}` or
//! `{"rid": n, "err": "…"}`. A connection that subscribed also gets
//! `{"event": …}` lines.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SpawnArgs {
    /// Run by `sh -c`.
    pub cmd: String,
    pub cwd: String,
    #[serde(default)]
    pub env: Vec<(String, String)>,
    /// Whose it is (a task id).
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// A foreground command: someone waits for it.
    #[serde(default)]
    pub fg: bool,
    /// Written to its stdin first (then stdin stays open for `input`).
    #[serde(default)]
    pub stdin: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PtyArgs {
    /// Run by `sh -c`; none: the user's shell.
    #[serde(default)]
    pub cmd: Option<String>,
    pub cwd: String,
    #[serde(default)]
    pub env: Vec<(String, String)>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default = "cols")]
    pub cols: u16,
    #[serde(default = "rows")]
    pub rows: u16,
}

fn cols() -> u16 {
    120
}
fn rows() -> u16 {
    32
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Ping,
    Spawn(SpawnArgs),
    Jobs {
        #[serde(default)]
        owner: Option<String>,
    },
    Job {
        id: String,
    },
    /// Until it ends, is moved to the background, or `timeout_ms` passes.
    Wait {
        id: String,
        timeout_ms: u64,
    },
    /// A foreground job goes on in the background: its waiters stop waiting.
    Background {
        id: String,
    },
    Input {
        id: String,
        text: String,
        #[serde(default)]
        close: bool,
    },
    /// A signal to its whole process group (default SIGTERM).
    Kill {
        id: String,
        #[serde(default)]
        signal: Option<i32>,
    },
    /// Its end has been passed on (to its task): not reported again.
    Ack {
        id: String,
    },
    PtyOpen(PtyArgs),
    PtySend {
        id: String,
        /// Bytes, base64.
        data: String,
    },
    /// The screen as text, once it's been quiet for `quiet_ms` (at most 10 s).
    PtyScreen {
        id: String,
        #[serde(default)]
        quiet_ms: u64,
        /// Lines of scrollback above the screen.
        #[serde(default)]
        scrollback: usize,
    },
    PtyResize {
        id: String,
        cols: u16,
        rows: u16,
    },
    PtyClose {
        id: String,
    },
    Ptys {
        #[serde(default)]
        owner: Option<String>,
    },
    /// Events on this connection: a job's output, a terminal's bytes (its
    /// scrollback first), and/or every job's end.
    Subscribe {
        #[serde(default)]
        job: Option<String>,
        #[serde(default)]
        pty: Option<String>,
        #[serde(default)]
        exits: bool,
    },
    /// Kills everything and exits.
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobMeta {
    pub id: String,
    pub cmd: String,
    pub cwd: String,
    pub owner: Option<String>,
    pub name: Option<String>,
    pub pid: Option<u32>,
    pub fg: bool,
    /// Moved from the foreground to the background.
    pub backgrounded: bool,
    /// Unix seconds.
    pub started: i64,
    pub ended: Option<i64>,
    pub exit: Option<i32>,
    /// The signal that ended it.
    pub signal: Option<i32>,
    /// The supervisor died while it ran: how it ended isn't known.
    #[serde(default)]
    pub lost: bool,
    /// Its end was passed on.
    #[serde(default)]
    pub acked: bool,
}

impl JobMeta {
    pub fn running(&self) -> bool {
        self.ended.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PtyMeta {
    pub id: String,
    pub cmd: Option<String>,
    pub cwd: String,
    pub owner: Option<String>,
    pub cols: u16,
    pub rows: u16,
    pub started: i64,
    pub alive: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Output { job: String, text: String },
    Exit { job: JobMeta },
    Backgrounded { job: String },
    /// A terminal's output bytes, base64.
    Pty { pty: String, data: String },
    PtyExit { pty: String },
}

/// What `wait` saw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Waited {
    Exited,
    Backgrounded,
    Timeout,
}
