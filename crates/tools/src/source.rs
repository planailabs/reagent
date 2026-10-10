//! reagent's own source, built into the binary: what an agent reads to see
//! how reagent works (`source.reagent_source_ls`, `reagent_source_read`).
//! The tracked kinds of files only: no build output, no node_modules.

/// The repository's files as this build had them.
#[derive(rust_embed::Embed)]
#[folder = "../.."]
#[include = "crates/**"]
#[include = "webui/src/**"]
#[include = "webui/e2e/**"]
#[include = "webui/test/**"]
#[include = "webui/package.json"]
#[include = "webui/playwright.config.js"]
#[include = "deploy/**"]
#[include = "prompts/**"]
#[include = "*.md"]
#[include = "*.toml"]
#[include = "*.nix"]
#[include = "flake.lock"]
#[include = "Cargo.lock"]
#[include = ".gitignore"]
#[exclude = "**/target/**"]
#[exclude = "**/node_modules/**"]
#[exclude = "**/.parcel-cache/**"]
struct Source;

/// Lines a read gives at most.
const READ_LINES: usize = 400;

fn clean(path: &str) -> String {
    path.trim().trim_start_matches("./").trim_matches('/').to_string()
}

/// A folder's entries (folders end in `/`, files with their size); `recursive`: every file under it.
pub fn ls(path: &str, recursive: bool) -> Result<String, String> {
    let dir = clean(path);
    let prefix = if dir.is_empty() { String::new() } else { format!("{dir}/") };
    let mut dirs = std::collections::BTreeSet::new();
    let mut files = vec![];
    for f in Source::iter() {
        let Some(rest) = f.strip_prefix(prefix.as_str()) else { continue };
        match rest.split_once('/') {
            Some((d, _)) if !recursive => {
                dirs.insert(format!("{d}/"));
            }
            _ => files.push(format!("{rest}  ({} bytes)", Source::get(&f).map(|e| e.data.len()).unwrap_or(0))),
        }
    }
    if dirs.is_empty() && files.is_empty() {
        return Err(if Source::get(&dir).is_some() { format!("{dir} is a file: reagent_source_read reads it") } else { format!("no folder {dir:?} in reagent's source (reagent_source_ls() lists the top)") });
    }
    files.sort();
    Ok(dirs.into_iter().chain(files).collect::<Vec<_>>().join("\n"))
}

/// A file's lines `from`..=`to` (numbered; 400 at most).
pub fn read(path: &str, from: Option<usize>, to: Option<usize>) -> Result<String, String> {
    let p = clean(path);
    let f = Source::get(&p).ok_or_else(|| format!("no file {p:?} in reagent's source (reagent_source_ls lists them)"))?;
    let text = std::str::from_utf8(&f.data).map_err(|_| format!("{p} isn't text"))?;
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Ok(format!("({p} is empty)"));
    }
    let from = from.unwrap_or(1).max(1);
    if from > lines.len() {
        return Err(format!("{p} has {} lines", lines.len()));
    }
    let to = to.unwrap_or(from + READ_LINES - 1).min(lines.len()).min(from + READ_LINES - 1).max(from);
    let width = to.to_string().len();
    let mut out: String = (from..=to).map(|n| format!("{n:>width$}│{}\n", lines[n - 1])).collect();
    out.push_str(&if to < lines.len() { format!("[lines {from}-{to} of {}; from: {} reads on]", lines.len(), to + 1) } else { format!("[lines {from}-{to} of {}: the end]", lines.len()) });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_source_is_listed_and_read() {
        let top = ls("", false).unwrap();
        assert!(top.contains("crates/") && top.contains("DESIGN.md") && top.contains("webui/") && !top.contains("target/"), "{top}");
        assert!(ls("crates/tools/src", false).unwrap().contains("source.rs  ("));
        assert!(ls("webui", true).unwrap().contains("src/App.vue"));
        assert!(!ls("webui", true).unwrap().contains("node_modules"));
        let r = read("crates/tools/src/source.rs", Some(1), Some(2)).unwrap();
        assert!(r.starts_with("1│//! reagent's own source") && r.contains("[lines 1-2 of"), "{r}");
        assert!(read("./DESIGN.md", None, None).unwrap().contains("# reagent"));
        assert!(read("nope.rs", None, None).is_err() && ls("nope", false).is_err());
        assert!(ls("DESIGN.md", false).unwrap_err().contains("is a file"));
    }
}
