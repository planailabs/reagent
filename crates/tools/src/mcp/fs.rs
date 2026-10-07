//! Files: read, write, edit, grep, glob, ls, in the task's places.

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::service::RequestContext;
use rmcp::{RoleServer, schemars, tool, tool_router};
use serde::Deserialize;

use super::{caller, more, resolve};
use crate::app::App;
use crate::edit::{self, Edit};

#[derive(Clone)]
pub struct FsTools(pub Arc<App>);

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Read {
    /// Relative to your working directory, or absolute.
    pub path: String,
    /// The first line (default 1).
    pub from: Option<usize>,
    /// The last line (default: 400 lines on).
    pub to: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Write {
    pub path: String,
    pub text: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct EditArgs {
    pub path: String,
    #[serde(flatten)]
    pub edit: Edit,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Grep {
    /// A regular expression (case-insensitive unless it has capitals).
    pub pattern: String,
    /// A folder or file to search (default: your working directory).
    pub path: Option<String>,
    /// Only files matching this glob (`*.rs`, `src/**/*.ts`).
    pub glob: Option<String>,
    /// Lines around each match (default 0, at most 5).
    pub context: Option<usize>,
    pub page: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct GlobArgs {
    /// `**/*.rs`, `src/*/mod.rs`, …
    pub pattern: String,
    /// Where to look (default: your working directory).
    pub path: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct Ls {
    pub path: Option<String>,
}

const READ_LINES: usize = 400;
const MAX_FILE: u64 = 20 * 1024 * 1024;

fn text_of(bytes: &[u8], path: &std::path::Path) -> Result<String, String> {
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Err(format!("{} looks binary: not shown", path.display()));
    }
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

/// Files under `root` (respecting .gitignore), as paths relative to it.
fn walk(root: &std::path::Path) -> impl Iterator<Item = ignore::DirEntry> {
    ignore::WalkBuilder::new(root).hidden(false).filter_entry(|e| e.file_name() != ".git").build().flatten().filter(|e| e.file_type().is_some_and(|t| t.is_file()))
}

#[tool_router(server_handler)]
impl FsTools {
    #[tool(description = "Read a text file's lines, numbered (400 at a time; the end says where to read on).")]
    async fn read(&self, Parameters(a): Parameters<Read>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let path = resolve(&self.0, &t, &p, "fs.read", &a.path, false).await?;
        let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", a.path))?;
        if meta.is_dir() {
            return Err(format!("{} is a folder: fs.ls lists it", a.path));
        }
        if meta.len() > MAX_FILE {
            return Err(format!("{} is {} MB: read parts with shell.exec (sed -n, head)", a.path, meta.len() / 1_000_000));
        }
        let text = text_of(&std::fs::read(&path).map_err(|e| e.to_string())?, &path)?;
        let lines: Vec<&str> = text.lines().collect();
        let from = a.from.unwrap_or(1).max(1);
        if lines.is_empty() {
            return Ok(format!("({} is empty)", a.path));
        }
        if from > lines.len() {
            return Err(format!("{} has {} lines", a.path, lines.len()));
        }
        let to = a.to.unwrap_or(from + READ_LINES - 1).min(lines.len()).max(from);
        let width = to.to_string().len();
        let mut out: String = (from..=to).map(|n| format!("{n:>width$}│{}\n", lines[n - 1])).collect();
        if to < lines.len() {
            out.push_str(&more(format!("lines {from}-{to} of {}; fs.read(path: {:?}, from: {}) reads on", lines.len(), a.path, to + 1)));
        } else {
            out.push_str(&more(format!("lines {from}-{to} of {}: the end", lines.len())));
        }
        Ok(out)
    }

    #[tool(description = "Write a file (made with its folders, or replaced).")]
    async fn write(&self, Parameters(a): Parameters<Write>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let path = resolve(&self.0, &t, &p, "fs.write", &a.path, true).await?;
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let existed = path.exists();
        std::fs::write(&path, &a.text).map_err(|e| e.to_string())?;
        Ok(format!("{} {} ({} lines)", if existed { "replaced" } else { "wrote" }, a.path, a.text.lines().count()))
    }

    #[tool(description = "Change part of a file: replace an exact string (old → new, once, or all = true), or append / insert (before a line, or after an exact passage) / delete (lines, or an exact passage).")]
    async fn edit(&self, Parameters(a): Parameters<EditArgs>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let path = resolve(&self.0, &t, &p, "fs.edit", &a.path, true).await?;
        let body = text_of(&std::fs::read(&path).map_err(|e| format!("{}: {e}", a.path))?, &path)?;
        let new = edit::apply(&body, &a.edit)?;
        std::fs::write(&path, &new).map_err(|e| e.to_string())?;
        Ok(format!("changed {} ({} lines now)", a.path, new.lines().count()))
    }

    #[tool(description = "Search files for a regular expression (respects .gitignore): matching lines with their paths and numbers, 100 a page.")]
    async fn grep(&self, Parameters(a): Parameters<Grep>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let root = resolve(&self.0, &t, &p, "fs.grep", a.path.as_deref().unwrap_or("."), false).await?;
        let caseless = !a.pattern.chars().any(|c| c.is_uppercase());
        let re = regex::RegexBuilder::new(&a.pattern).case_insensitive(caseless).size_limit(1 << 22).build().map_err(|e| format!("bad pattern: {e}"))?;
        let glob = a.glob.as_deref().map(|g| globset::GlobBuilder::new(g).literal_separator(false).build().map(|g| g.compile_matcher())).transpose().map_err(|e| e.to_string())?;
        let ctxl = a.context.unwrap_or(0).min(5);
        let (page, per) = (a.page.unwrap_or(1).max(1), 100);
        let mut hits = vec![];
        let base = if root.is_file() { root.parent().unwrap_or(&root).to_path_buf() } else { root.clone() };
        let files: Box<dyn Iterator<Item = std::path::PathBuf>> = if root.is_file() { Box::new(std::iter::once(root.clone())) } else { Box::new(walk(&root).map(|e| e.into_path())) };
        for f in files {
            let rel = f.strip_prefix(&base).unwrap_or(&f).display().to_string();
            if glob.as_ref().is_some_and(|g| !g.is_match(&rel) && !g.is_match(f.file_name().unwrap_or_default())) {
                continue;
            }
            if std::fs::metadata(&f).map(|m| m.len() > MAX_FILE).unwrap_or(true) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&f) else { continue };
            let Ok(text) = text_of(&bytes, &f) else { continue };
            let lines: Vec<&str> = text.lines().collect();
            for (i, l) in lines.iter().enumerate() {
                if re.is_match(l) {
                    let mut s = String::new();
                    for k in i.saturating_sub(ctxl)..(i + ctxl + 1).min(lines.len()) {
                        let mark = if k == i { ':' } else { '-' };
                        let line: String = lines[k].chars().take(300).collect();
                        s.push_str(&format!("{rel}{mark}{}{mark}{line}\n", k + 1));
                    }
                    hits.push(s);
                }
            }
        }
        let total = hits.len();
        if total == 0 {
            return Ok(format!("no match for {:?}", a.pattern));
        }
        let pages = total.div_ceil(per);
        let page = page.min(pages);
        let mut out: String = hits.into_iter().skip((page - 1) * per).take(per).collect::<Vec<_>>().join(if ctxl > 0 { "--\n" } else { "" });
        out.push_str(&more(format!("{total} matches, page {page} of {pages}{}", if page < pages { format!("; page: {} for more", page + 1) } else { String::new() })));
        Ok(out)
    }

    #[tool(description = "Files matching a glob (respects .gitignore), at most 500.")]
    async fn glob(&self, Parameters(a): Parameters<GlobArgs>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let root = resolve(&self.0, &t, &p, "fs.glob", a.path.as_deref().unwrap_or("."), false).await?;
        let g = globset::GlobBuilder::new(&a.pattern).literal_separator(true).build().map_err(|e| e.to_string())?.compile_matcher();
        let mut found: Vec<String> = walk(&root).filter_map(|e| e.path().strip_prefix(&root).ok().map(|r| r.display().to_string())).filter(|r| g.is_match(r)).collect();
        found.sort();
        let total = found.len();
        found.truncate(500);
        let mut out = found.join("\n");
        out.push_str(&more(if total > 500 { format!("{total} files, the first 500 shown: narrow the pattern") } else { format!("{total} files") }));
        Ok(out)
    }

    #[tool(description = "A folder's entries (folders end in /), with file sizes.")]
    async fn ls(&self, Parameters(a): Parameters<Ls>, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let (t, p) = caller(&self.0, &ctx).await?;
        let dir = resolve(&self.0, &t, &p, "fs.ls", a.path.as_deref().unwrap_or("."), false).await?;
        let mut v: Vec<(bool, String, u64)> = std::fs::read_dir(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .flatten()
            .map(|e| {
                let m = e.metadata().ok();
                (m.as_ref().is_some_and(|m| m.is_dir()), e.file_name().to_string_lossy().into_owned(), m.map(|m| m.len()).unwrap_or(0))
            })
            .collect();
        v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut out: String = v.iter().map(|(d, n, s)| if *d { format!("{n}/\n") } else { format!("{n}  ({s} B)\n") }).collect();
        out.push_str(&more(format!("{} entries in {}", v.len(), dir.display())));
        Ok(out)
    }
}
