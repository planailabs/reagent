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
/// project folder's, the nearer winning on a name.
pub fn discover(cwd: &Path, project: &Path) -> Vec<Rule> {
    let mut out: Vec<Rule> = vec![];
    for base in if cwd == project { vec![project] } else { vec![cwd, project] } {
        for d in DIRS {
            let mut found = vec![];
            walk(&base.join(d), &base.join(d), &mut found);
            for r in found {
                if !out.iter().any(|o| o.name == r.name) {
                    out.push(r);
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
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
        let rules = discover(&wt, &proj);
        let by = |n: &str| rules.iter().find(|r| r.name == n).unwrap();
        assert_eq!(rules.len(), 4, "{rules:?}");
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
