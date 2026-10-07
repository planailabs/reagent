use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::server::Supervisor;

struct S {
    dir: tempfile::TempDir,
    socket: std::path::PathBuf,
    task: tokio::task::JoinHandle<()>,
    client: Arc<Client>,
}

async fn start_in(dir: tempfile::TempDir) -> S {
    let socket = dir.path().join("s.sock");
    let sup = Supervisor::new(dir.path()).unwrap();
    let sk = socket.clone();
    let task = tokio::spawn(async move { sup.serve(&sk).await.unwrap() });
    let client = Client::new(&socket);
    for _ in 0..100 {
        if client.ping().await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    S { dir, socket, task, client }
}

async fn start() -> S {
    start_in(tempfile::tempdir().unwrap()).await
}

fn args(cmd: &str, dir: &std::path::Path) -> SpawnArgs {
    SpawnArgs { cmd: cmd.into(), cwd: dir.display().to_string(), owner: Some("t1".into()), fg: true, ..Default::default() }
}

#[tokio::test]
async fn a_foreground_job_runs_and_its_output_is_kept() {
    let s = start().await;
    let j = s.client.spawn(args("echo one; echo two >&2; exit 3", s.dir.path())).await.unwrap();
    let (w, m) = s.client.wait(&j.id, 5000).await.unwrap();
    assert_eq!((w, m.exit, m.running()), (Waited::Exited, Some(3), false));
    let out = output(s.dir.path(), &j.id, None, None, None, None).unwrap();
    assert_eq!(out["lines"], 2);
    let texts: Vec<&str> = out["text"].as_array().unwrap().iter().map(|l| l["text"].as_str().unwrap()).collect();
    assert!(texts.contains(&"one") && texts.contains(&"two"), "{texts:?}");
    assert_eq!(output(s.dir.path(), &j.id, None, None, None, Some("TW")).unwrap()["matches"][0]["text"], "two");
    assert_eq!(s.client.jobs(Some("t1")).await.unwrap().len(), 1);
    assert!(s.client.jobs(Some("other")).await.unwrap().is_empty());
}

#[tokio::test]
async fn waiting_times_out_and_moving_to_the_background_frees_the_waiter() {
    let s = start().await;
    let j = s.client.spawn(args("sleep 30", s.dir.path())).await.unwrap();
    let (w, _) = s.client.wait(&j.id, 100).await.unwrap();
    assert_eq!(w, Waited::Timeout);
    let c = s.client.clone();
    let id = j.id.clone();
    let waiter = tokio::spawn(async move { c.wait(&id, 20_000).await.unwrap() });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let m = s.client.background(&j.id).await.unwrap();
    assert!(m.backgrounded && !m.fg);
    let (w, m) = tokio::time::timeout(Duration::from_secs(5), waiter).await.unwrap().unwrap();
    assert_eq!((w, m.running()), (Waited::Backgrounded, true));
    s.client.kill(&j.id, None).await.unwrap();
    let (w, m) = s.client.wait(&j.id, 5000).await.unwrap();
    assert_eq!((w, m.signal), (Waited::Exited, Some(15)));
}

#[tokio::test]
async fn input_reaches_stdin_and_kill_takes_the_whole_group() {
    let s = start().await;
    let j = s.client.spawn(args("read a; echo got $a; sleep 60 & sleep 60", s.dir.path())).await.unwrap();
    s.client.input(&j.id, "hello\n", false).await.unwrap();
    for _ in 0..100 {
        if tail(s.dir.path(), &j.id, 5).contains("got hello") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(tail(s.dir.path(), &j.id, 5).contains("got hello"));
    let pid = j.pid.unwrap() as i32;
    s.client.kill(&j.id, Some(9)).await.unwrap();
    let (_, m) = s.client.wait(&j.id, 5000).await.unwrap();
    assert_eq!(m.signal, Some(9));
    // The background sleep was in the group too: nothing of it is left.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid), None).is_err(), "the group is gone");
}

#[tokio::test]
async fn ends_are_reported_and_acknowledged() {
    let s = start().await;
    let mut exits = s.client.exits();
    let mut a = args("true", s.dir.path());
    a.fg = false;
    // The first call opens the connection that hears ends.
    s.client.ping().await.unwrap();
    let j = s.client.spawn(a).await.unwrap();
    let e = tokio::time::timeout(Duration::from_secs(5), exits.recv()).await.unwrap().unwrap();
    let Event::Exit { job } = e else { panic!("{e:?}") };
    assert_eq!((job.id.as_str(), job.exit, job.acked), (j.id.as_str(), Some(0), false));
    s.client.ack(&j.id).await.unwrap();
    assert!(s.client.job(&j.id).await.unwrap().acked);
}

#[tokio::test]
async fn jobs_of_a_dead_supervisor_are_lost() {
    let dir = tempfile::tempdir().unwrap();
    let d = server::job_dir(dir.path(), "j7");
    std::fs::create_dir_all(&d).unwrap();
    let running = JobMeta { id: "j7".into(), cmd: "sleep 9".into(), cwd: "/".into(), owner: None, name: None, pid: Some(1), fg: false, backgrounded: false, started: 1, ended: None, exit: None, signal: None, lost: false, acked: false };
    std::fs::write(d.join("meta.json"), serde_json::to_vec(&running).unwrap()).unwrap();
    let s = start_in(dir).await;
    let m = s.client.job("j7").await.unwrap();
    assert!(m.lost && !m.running(), "{m:?}");
    let next = s.client.spawn(args("true", s.dir.path())).await.unwrap();
    assert_eq!(next.id, "j8", "ids go on from the last");
}

#[tokio::test]
async fn the_client_reconnects_to_a_new_supervisor() {
    let s = start().await;
    s.client.ping().await.unwrap();
    s.task.abort();
    let _ = s.task.await;
    let client = s.client.clone();
    let s2 = start_in(s.dir).await;
    let mut ok = false;
    for _ in 0..50 {
        if client.ping().await.is_ok() {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "reconnected");
    drop(s2);
}

#[tokio::test]
async fn a_terminal_takes_keys_and_shows_its_screen() {
    let s = start().await;
    let p = s.client.pty_open(PtyArgs { cmd: Some("cat".into()), cwd: s.dir.path().display().to_string(), owner: Some("t1".into()), cols: 40, rows: 10, ..Default::default() }).await.unwrap();
    let mut stream = client::stream(&s.socket, None, Some(&p.id)).await.unwrap();
    s.client.pty_send(&p.id, &keys("hello there<enter>")).await.unwrap();
    let screen = s.client.pty_screen(&p.id, 300, 0).await.unwrap();
    assert!(screen["screen"].as_str().unwrap().contains("hello there"), "{screen}");
    assert_eq!(screen["alive"], true);
    // Its bytes stream to whoever watches.
    let mut seen = String::new();
    while !seen.contains("hello") {
        let Ok(Ok(Event::Pty { data, .. })) = tokio::time::timeout(Duration::from_secs(5), stream.recv()).await else { panic!("no bytes") };
        use base64::Engine;
        seen.push_str(&String::from_utf8_lossy(&base64::engine::general_purpose::STANDARD.decode(data).unwrap()));
    }
    let m = s.client.pty_resize(&p.id, 80, 24).await.unwrap();
    assert_eq!((m.cols, m.rows), (80, 24));
    assert_eq!(s.client.ptys(Some("t1")).await.unwrap().len(), 1);
    s.client.pty_send(&p.id, &keys("<C-d>")).await.unwrap();
    for _ in 0..100 {
        if s.client.pty_screen(&p.id, 0, 0).await.unwrap()["alive"] == false {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert_eq!(s.client.pty_screen(&p.id, 0, 0).await.unwrap()["alive"], false, "ctrl-d ends cat");
    s.client.pty_close(&p.id).await.unwrap();
    assert!(s.client.ptys(None).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_terminal_keeps_scrollback() {
    let s = start().await;
    let p = s.client.pty_open(PtyArgs { cmd: Some("seq 1 50".into()), cwd: s.dir.path().display().to_string(), cols: 20, rows: 10, ..Default::default() }).await.unwrap();
    let v = s.client.pty_screen(&p.id, 300, 30).await.unwrap();
    let screen = v["screen"].as_str().unwrap();
    assert!(screen.contains("50") && !screen.contains("\n30\n"), "{screen}");
    let back = v["scrollback"].as_str().unwrap();
    assert!(back.lines().any(|l| l == "30"), "{back}");
}

#[test]
fn key_names_become_bytes() {
    assert_eq!(keys("ls<enter>"), b"ls\r");
    assert_eq!(keys("<C-c><up><tab>"), b"\x03\x1b[A\t");
    assert_eq!(keys("a<b>c"), b"a<b>c", "unknown names stay text");
    assert_eq!(keys("x < y"), b"x < y");
}

#[test]
fn output_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let d = server::job_dir(dir.path(), "j1");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("output.log"), (1..=500).map(|i| format!("l{i}\n")).collect::<String>()).unwrap();
    let v = output(dir.path(), "j1", Some(10), Some(12), None, None).unwrap();
    assert_eq!((v["from"].as_u64(), v["to"].as_u64(), v["next"].as_u64()), (Some(10), Some(12), Some(13)));
    assert_eq!(v["text"][0]["text"], "l10");
    assert_eq!(v.as_object().unwrap().keys().last().unwrap(), "next", "where to read on, last");
    let v = output(dir.path(), "j1", None, None, Some(3), None).unwrap();
    assert_eq!((v["from"].as_u64(), v["next"].is_null()), (Some(498), true));
    assert_eq!(tail(dir.path(), "j1", 2), "l499\nl500");
}
