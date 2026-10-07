//! The supervisor: a long-lived process owning reagent's commands (jobs)
//! and terminals (PTYs), so they outlive restarts of `reagent up`.

pub mod client;
pub mod proto;
pub mod server;

use std::path::Path;

pub use client::Client;
pub use proto::*;

/// Lines of a job's output: `from`..=`to` (1-based), or the last `tail`, or
/// those matching `pattern` (with their numbers). Read from its log file.
pub fn output(dir: &Path, job: &str, from: Option<usize>, to: Option<usize>, tail: Option<usize>, pattern: Option<&str>) -> Result<serde_json::Value, String> {
    let bytes = std::fs::read(server::job_dir(dir, job).join("output.log")).map_err(|e| format!("no output for {job}: {e}"))?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let numbered = |range: std::ops::Range<usize>| range.map(|i| serde_json::json!({"line": i + 1, "text": cut(lines[i])})).collect::<Vec<_>>();
    if let Some(p) = pattern {
        let re = regex::RegexBuilder::new(p).case_insensitive(true).size_limit(1 << 20).build().map_err(|e| format!("bad pattern: {e}"))?;
        let hits: Vec<_> = (0..total).filter(|&i| re.is_match(lines[i])).take(200).map(|i| serde_json::json!({"line": i + 1, "text": cut(lines[i])})).collect();
        return Ok(serde_json::json!({"matches": hits, "lines": total}));
    }
    let (start, end) = match (from, to, tail) {
        (Some(f), t, _) => (f.max(1) - 1, t.unwrap_or(f + 199).min(total)),
        (None, _, Some(n)) => (total.saturating_sub(n), total),
        (None, _, None) => (total.saturating_sub(100), total),
    };
    let end = end.max(start).min(start + 1000);
    Ok(serde_json::json!({"text": numbered(start.min(total)..end), "from": start + 1, "to": end, "lines": total, "next": (end < total).then_some(end + 1)}))
}

fn cut(l: &str) -> String {
    if l.len() <= 2000 {
        return l.to_string();
    }
    format!("{}…", &l[..l.floor_char_boundary(2000)])
}

/// The last `n` lines of a job's output, as one text.
pub fn tail(dir: &Path, job: &str, n: usize) -> String {
    let bytes = std::fs::read(server::job_dir(dir, job).join("output.log")).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|l| cut(l)).collect::<Vec<_>>().join("\n")
}

/// Text with key names (`<enter>`, `<tab>`, `<esc>`, `<up>`, `<C-c>`, …) as
/// the bytes a terminal sends for them.
pub fn keys(s: &str) -> Vec<u8> {
    let mut out = vec![];
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        out.extend_from_slice(rest[..i].as_bytes());
        let after = &rest[i..];
        let Some(j) = after.find('>') else {
            out.extend_from_slice(after.as_bytes());
            return out;
        };
        let name = &after[1..j];
        let bytes: Option<Vec<u8>> = match name.to_ascii_lowercase().as_str() {
            "enter" | "cr" | "return" => Some(b"\r".to_vec()),
            "tab" => Some(b"\t".to_vec()),
            "esc" | "escape" => Some(b"\x1b".to_vec()),
            "bs" | "backspace" => Some(b"\x7f".to_vec()),
            "space" => Some(b" ".to_vec()),
            "up" => Some(b"\x1b[A".to_vec()),
            "down" => Some(b"\x1b[B".to_vec()),
            "right" => Some(b"\x1b[C".to_vec()),
            "left" => Some(b"\x1b[D".to_vec()),
            "home" => Some(b"\x1b[H".to_vec()),
            "end" => Some(b"\x1b[F".to_vec()),
            "pageup" => Some(b"\x1b[5~".to_vec()),
            "pagedown" => Some(b"\x1b[6~".to_vec()),
            "del" | "delete" => Some(b"\x1b[3~".to_vec()),
            n if (n.starts_with("c-") || n.starts_with("ctrl-")) && n.rsplit('-').next().is_some_and(|k| k.len() == 1) => {
                let k = n.rsplit('-').next().unwrap().as_bytes()[0];
                k.is_ascii_alphabetic().then(|| vec![k.to_ascii_lowercase() - b'a' + 1])
            }
            _ => None,
        };
        match bytes {
            Some(b) => out.extend(b),
            None => out.extend_from_slice(&after.as_bytes()[..=j]),
        }
        rest = &after[j + 1..];
    }
    out.extend_from_slice(rest.as_bytes());
    out
}

/// Makes sure a supervisor answers on `socket`, starting `exe supervisor`
/// detached (its own session, so it outlives us) if none does.
pub async fn ensure_running(exe: &Path, data: &Path, socket: &Path) -> anyhow::Result<std::sync::Arc<Client>> {
    let c = Client::new(socket);
    if c.ping().await.is_ok() {
        return Ok(c);
    }
    let log = std::fs::OpenOptions::new().create(true).append(true).open(data.join("supervisor.log"))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("supervisor").env("REAGENT_DATA", data).stdin(std::process::Stdio::null()).stdout(log.try_clone()?).stderr(log);
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn()?;
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if c.ping().await.is_ok() {
            return Ok(c);
        }
    }
    anyhow::bail!("the supervisor didn't start (see {})", data.join("supervisor.log").display())
}

#[cfg(test)]
mod tests;
