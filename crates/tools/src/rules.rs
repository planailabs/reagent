//! Instruction rules: markdown files in `.agents/rules/` (or `.agent/rules/`),
//! `*.md` or Cursor's `*.mdc`, in subfolders too. Optional frontmatter says
//! when one applies: `alwaysApply: true` (or no `globs` and no `description`):
//! always in the context; `globs` (a list, or one string split on commas):
//! once the task touched a matching file; only a `description`: listed, for
//! the task to read when it fits.

use std::path::{Path, PathBuf};

pub const DIRS: [&str; 2] = [".agents/rules", ".agent/rules"];

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// Its path under the rules folder, without the extension.
    pub name: String,
    pub path: PathBuf,
    pub description: Option<String>,
    pub globs: Vec<String>,
    pub always: bool,
    pub body: String,
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Front {
    description: Option<String>,
    globs: Option<Globs>,
    always_apply: Option<bool>,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Globs {
    One(String),
    Many(Vec<String>),
}

/// A rule file's frontmatter and body.
pub fn parse(name: String, path: PathBuf, text: &str) -> Rule {
    let t = text.trim_start_matches('\u{feff}');
    let (front, body) = match t.strip_prefix("---").map(|r| r.trim_start_matches(['\r', '\n'])).and_then(|r| r.find("\n---").map(|end| (r, end))) {
        Some((r, end)) => (serde_yaml::from_str::<Option<Front>>(&r[..end]).ok().flatten().unwrap_or_default(), r[end + 4..].trim_start_matches('-').trim().to_string()),
        None => (Front::default(), t.trim().to_string()),
    };
    let globs: Vec<String> = match front.globs {
        Some(Globs::One(s)) => s.split(',').map(|g| g.trim().to_string()).filter(|g| !g.is_empty()).collect(),
        Some(Globs::Many(v)) => v,
        None => vec![],
    };
    let description = front.description.filter(|d| !d.trim().is_empty());
    let always = front.always_apply.unwrap_or(globs.is_empty() && description.is_none());
    Rule { name, path, description, globs, always, body }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<Rule>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            walk(root, &p, out);
        } else if matches!(p.extension().and_then(|e| e.to_str()), Some("md" | "mdc"))
            && let Ok(text) = std::fs::read_to_string(&p)
        {
            let name = p.strip_prefix(root).unwrap_or(&p).with_extension("").to_string_lossy().replace('\\', "/");
            out.push(parse(name, p, &text));
        }
    }
}

/// The rules a task sees: its working directory's (a worktree), then the
/// project folder's, then the global rules folders' (`~/.agents/rules`,
/// `<data>/rules`, plugins'), the nearer winning on a name.
pub fn discover(cwd: &Path, project: &Path, global: &[PathBuf]) -> Vec<Rule> {
    let mut places = vec![];
    for base in if cwd == project { vec![project] } else { vec![cwd, project] } {
        places.extend(DIRS.iter().map(|d| base.join(d)));
    }
    places.extend(global.iter().cloned());
    in_places(&places)
}

/// The rules in these rules folders, the first winning on a name.
pub fn in_places(places: &[PathBuf]) -> Vec<Rule> {
    let mut out: Vec<Rule> = vec![];
    for place in places {
        let mut found = vec![];
        walk(place, place, &mut found);
        for r in found {
            if !out.iter().any(|o| o.name == r.name) {
                out.push(r);
            }
        }
    }
    out
}

/// Whether a glob rule applies: one of its globs matches a touched path
/// (relative to the working directory or the project).
pub fn matches(rule: &Rule, touched: &[String], cwd: &Path, project: &Path) -> bool {
    let mut b = globset::GlobSetBuilder::new();
    for g in &rule.globs {
        if let Ok(g) = globset::Glob::new(g) {
            b.add(g);
        }
    }
    let Ok(set) = b.build() else { return false };
    touched.iter().any(|t| {
        let p = Path::new(t);
        let rel = p.strip_prefix(cwd).or_else(|_| p.strip_prefix(project)).unwrap_or(p);
        set.is_match(rel.to_string_lossy().trim_start_matches("./"))
    })
}

/// The `AGENTS.md` files in subfolders over the touched paths (relative to
/// the working directory or the project; outside paths don't count),
/// shallowest first. The root's own is the caller's.
pub fn nested_agents(touched: &[String], cwd: &Path, project: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = vec![];
    for t in touched {
        let p = Path::new(t);
        let (root, rel) = match (p.strip_prefix(cwd), p.strip_prefix(project)) {
            (Ok(r), _) => (cwd, r),
            (_, Ok(r)) => (project, r),
            _ if p.is_relative() => (cwd, p),
            _ => continue,
        };
        if rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            continue;
        }
        // A touched folder's own AGENTS.md counts; a file's starts at its folder.
        let start = if root.join(rel).is_dir() { Some(rel) } else { rel.parent() };
        for dir in start.into_iter().flat_map(Path::ancestors) {
            let f = root.join(dir).join("AGENTS.md");
            if !dir.as_os_str().is_empty() && dir != Path::new(".") && f.is_file() && !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out.sort_by_key(|f| f.components().count());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn nested_agents_md_over_touched_paths() {
        let t = tempfile::tempdir().unwrap();
        let (cwd, proj) = (t.path().join("wt"), t.path().join("proj"));
        write(&cwd, "AGENTS.md", "root");
        write(&cwd, "crates/a/AGENTS.md", "a");
        write(&cwd, "crates/a/src/lib.rs", "");
        write(&cwd, "crates/AGENTS.md", "crates");
        write(&cwd, "docs/AGENTS.md", "docs");
        write(&proj, "web/AGENTS.md", "web");
        let found = |touched: &[&str]| nested_agents(&touched.iter().map(|s| s.to_string()).collect::<Vec<_>>(), &cwd, &proj);
        assert_eq!(found(&["crates/a/src/lib.rs"]), [cwd.join("crates/AGENTS.md"), cwd.join("crates/a/AGENTS.md")], "shallowest first, not the root's");
        assert_eq!(found(&[cwd.join("docs").to_str().unwrap()]), [cwd.join("docs/AGENTS.md")], "a folder's own");
        assert_eq!(found(&[proj.join("web/x.vue").to_str().unwrap()]), [proj.join("web/AGENTS.md")], "under the project");
        assert!(found(&["README.md", "/etc/x/y", "../wt/docs/z"]).is_empty());
    }

    #[test]
    fn rules_are_found_parsed_and_matched() {
        let t = tempfile::tempdir().unwrap();
        let (wt, proj) = (t.path().join("wt"), t.path().join("proj"));
        write(&proj, ".agents/rules/style.md", "Use tabs.");
        write(&proj, ".agents/rules/rust/errors.mdc", "---\nglobs: src/**/*.rs, *.toml\n---\nNo unwrap.");
        write(&proj, ".agents/rules/release.md", "---\ndescription: How we release\n---\nTag it.");
        write(&proj, ".agent/rules/web.md", "---\nglobs: [\"webui/**\"]\nalwaysApply: true\n---\nVue 3.");
        write(&wt, ".agents/rules/style.md", "Use spaces.");
        write(&proj, ".agents/rules/notes.txt", "not a rule");
        write(t.path(), "global/style.md", "Use anything.");
        write(t.path(), "global/global-only.md", "From home.");
        let rules = discover(&wt, &proj, &[t.path().join("global")]);
        let by = |n: &str| rules.iter().find(|r| r.name == n).unwrap();
        assert_eq!(rules.len(), 5, "{rules:?}");
        assert_eq!(by("global-only").body, "From home.", "global ones come after, the project's win");
        assert_eq!((by("style").body.as_str(), by("style").always), ("Use spaces.", true), "the worktree's wins");
        let errors = by("rust/errors");
        assert_eq!((errors.globs.clone(), errors.always, errors.body.as_str()), (vec!["src/**/*.rs".to_string(), "*.toml".into()], false, "No unwrap."));
        assert_eq!((by("release").description.as_deref(), by("release").always), (Some("How we release"), false));
        assert!(by("web").always, "alwaysApply wins over globs");
        assert!(matches(errors, &["src/a/b.rs".into()], &wt, &proj));
        assert!(matches(errors, &[proj.join("Cargo.toml").to_string_lossy().into()], &wt, &proj));
        assert!(!matches(errors, &["README.md".into()], &wt, &proj));
    }
}
