//! Commands in the project's nix dev shell (a stand-in `nix` records how
//! it's called and runs the command, so no network or evaluation is needed).

mod common;

use common::*;
use serde_json::json;

#[tokio::test]
async fn commands_run_in_the_dev_shell_unless_they_opt_out() {
    let bin = tempfile::tempdir().unwrap();
    let log = bin.path().join("nix.log");
    // nix develop <flake> --command sh -c <cmd>
    std::fs::write(bin.path().join("nix"), format!("#!/bin/sh\necho \"$2\" >> {}\nshift 3\nexport IN_DEVSHELL=yes\nexec \"$@\"\n", log.display())).unwrap();
    std::fs::set_permissions(bin.path().join("nix"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    unsafe { std::env::set_var("PATH", format!("{}:{}", bin.path().display(), std::env::var("PATH").unwrap_or_default())) };

    let r = start().await;
    r.allow_all().await;
    std::fs::write(r.project.path().join("flake.nix"), "{}").unwrap();
    r.push("Dev", |_| call("c1", "shell.exec", json!({"cmd": "echo in=$IN_DEVSHELL"})));
    r.push("Dev", |b| {
        assert!(last_result(b).contains("in=yes"), "in the dev shell: {}", last_result(b));
        call("c2", "shell.exec", json!({"cmd": "echo in=$IN_DEVSHELL", "devshell": false}))
    });
    r.push("Dev", |b| {
        assert!(last_result(b).contains("in=\n") || last_result(b).trim_end().ends_with("in="), "opted out: {}", last_result(b));
        text("ok")
    });
    let t = r.start_task("Dev", "x").await;
    r.done(&t.id).await;
    let calls = std::fs::read_to_string(&log).unwrap();
    assert_eq!(calls.lines().collect::<Vec<_>>(), [r.project.path().canonicalize().unwrap().display().to_string()], "one call, with the project's flake");
    // The job keeps the command as the task gave it, for people to read.
    let jobs = r.run.app.sup.jobs(Some(&t.id)).await.unwrap();
    assert_eq!(jobs[0].name.as_deref(), Some("echo in=$IN_DEVSHELL"));
    assert!(jobs[0].cmd.starts_with("nix develop "));

    // Turned off for the project: as it is.
    let mut p = r.run.app.project("site").await.unwrap();
    p.devshell = "off".into();
    r.run.app.put_project(p.clone()).await.unwrap();
    r.push("Plain", |_| call("c1", "shell.exec", json!({"cmd": "echo in=$IN_DEVSHELL"})));
    r.push("Plain", |b| {
        assert!(!last_result(b).contains("in=yes"), "{}", last_result(b));
        text("ok")
    });
    let t = r.start_task("Plain", "x").await;
    r.done(&t.id).await;
    p.devshell = "sometimes".into();
    assert!(r.run.app.put_project(p).await.is_err());
}
