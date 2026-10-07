//! Agent skills in the generic layout: `<dir>/.agents/skills/<name>/SKILL.md`
//! (and `.agent/skills/`), frontmatter `name` and `description`, then the
//! instructions; the folder may hold scripts and references.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// The skill's folder.
    pub dir: PathBuf,
    /// Where it was found: worktree, project or global.
    pub source: String,
}

#[derive(serde::Deserialize)]
struct Front {
    name: Option<String>,
    description: Option<String>,
}

/// A `SKILL.md`'s frontmatter and body.
pub fn parse(text: &str) -> (Option<String>, Option<String>, String) {
    let t = text.trim_start_matches('\u{feff}');
    if let Some(rest) = t.strip_prefix("---") {
        let rest = rest.trim_start_matches(['\r', '\n']);
        if let Some(end) = rest.find("\n---") {
            let front: Option<Front> = serde_yaml::from_str(&rest[..end]).ok();
            let body = rest[end + 4..].trim_start_matches(['-']).trim_start_matches(['\r', '\n']).to_string();
            return match front {
                Some(f) => (f.name, f.description, body),
                None => (None, None, body),
            };
        }
    }
    (None, None, t.to_string())
}

fn found_in(base: &Path, source: &str) -> Vec<Skill> {
    let mut out = vec![];
    for sub in [".agents/skills", ".agent/skills", ".claude/skills"] {
        let Ok(rd) = std::fs::read_dir(base.join(sub)) else { continue };
        let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.join("SKILL.md").is_file()).collect();
        dirs.sort();
        for dir in dirs {
            let Ok(text) = std::fs::read_to_string(dir.join("SKILL.md")) else { continue };
            let (name, description, body) = parse(&text);
            let name = name.unwrap_or_else(|| dir.file_name().unwrap().to_string_lossy().into_owned());
            let description = description.unwrap_or_else(|| body.lines().find(|l| !l.trim().is_empty() && !l.starts_with('#')).unwrap_or("").chars().take(200).collect());
            out.push(Skill { name, description, dir, source: source.into() });
        }
    }
    out
}

/// The skills a task sees, nearest first winning on a name: its working
/// directory (a worktree), the project folder, then the global places.
pub fn discover(cwd: &Path, project: &Path, global: &[PathBuf]) -> Vec<Skill> {
    let mut places: Vec<(PathBuf, &str)> = vec![];
    if cwd != project {
        places.push((cwd.to_path_buf(), "worktree"));
    }
    places.push((project.to_path_buf(), "project"));
    let mut out: Vec<Skill> = vec![];
    for (p, src) in places {
        for s in found_in(&p, src) {
            if !out.iter().any(|o| o.name == s.name) {
                out.push(s);
            }
        }
    }
    for g in global {
        // A global place is the skills folder itself (~/.agents/skills) or holds one.
        let mut here = found_in(g, "global");
        if let Ok(rd) = std::fs::read_dir(g) {
            let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.join("SKILL.md").is_file()).collect();
            dirs.sort();
            for dir in dirs {
                let Ok(text) = std::fs::read_to_string(dir.join("SKILL.md")) else { continue };
                let (name, description, body) = parse(&text);
                let name = name.unwrap_or_else(|| dir.file_name().unwrap().to_string_lossy().into_owned());
                let description = description.unwrap_or_else(|| body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(200).collect());
                here.push(Skill { name, description, dir, source: "global".into() });
            }
        }
        for s in here {
            if !out.iter().any(|o| o.name == s.name) {
                out.push(s);
            }
        }
    }
    out
}

/// A skill's instructions and its other files (relative paths).
pub fn load(s: &Skill) -> Result<(String, Vec<String>), String> {
    let text = std::fs::read_to_string(s.dir.join("SKILL.md")).map_err(|e| e.to_string())?;
    let (_, _, body) = parse(&text);
    let mut files: Vec<String> = ignore::WalkBuilder::new(&s.dir)
        .hidden(false)
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| e.path().strip_prefix(&s.dir).ok().map(|p| p.display().to_string()))
        .filter(|p| p != "SKILL.md")
        .collect();
    files.sort();
    Ok((body, files))
}

/// One line per skill, for the context.
pub fn listing(skills: &[Skill]) -> String {
    skills.iter().map(|s| format!("- {} — {}", s.name, s.description.lines().next().unwrap_or(""))).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(base: &Path, sub: &str, name: &str, desc: &str) {
        let d = base.join(sub).join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), format!("---\nname: {name}\ndescription: {desc}\n---\n\n# {name}\n\nDo the {name} thing.\n")).unwrap();
    }

    #[test]
    fn nearest_wins_and_all_places_count() {
        let t = tempfile::tempdir().unwrap();
        let (wt, proj, global) = (t.path().join("wt"), t.path().join("proj"), t.path().join("home/.agents/skills"));
        skill(&proj, ".agents/skills", "deploy", "ship it (project)");
        skill(&proj, ".agent/skills", "lint", "check style");
        skill(&proj, ".claude/skills", "rebase", "rebase the fork");
        skill(&wt, ".agents/skills", "deploy", "ship it (worktree)");
        std::fs::create_dir_all(&global).unwrap();
        skill(&global, "", "deploy", "ship it (global)");
        skill(&global, "", "review", "review code");
        let found = discover(&wt, &proj, &[global.clone()]);
        let by = |n: &str| found.iter().find(|s| s.name == n).unwrap().clone();
        assert_eq!(found.len(), 4, "{found:?}");
        assert_eq!(by("rebase").source, "project", ".claude/skills count too");
        assert_eq!((by("deploy").description.as_str(), by("deploy").source.as_str()), ("ship it (worktree)", "worktree"));
        assert_eq!(by("lint").source, "project");
        assert_eq!(by("review").source, "global");
        std::fs::write(by("deploy").dir.join("run.sh"), "echo hi").unwrap();
        let (body, files) = load(&by("deploy")).unwrap();
        assert!(body.starts_with("# deploy") && body.contains("Do the deploy thing."), "{body}");
        assert_eq!(files, ["run.sh"]);
        assert!(listing(&found).contains("- review — review code"));
    }

    #[test]
    fn frontmatter_is_optional() {
        assert_eq!(parse("---\nname: x\ndescription: \"y: z\"\n---\nbody\n"), (Some("x".into()), Some("y: z".into()), "body\n".into()));
        assert_eq!(parse("# Plain\ntext"), (None, None, "# Plain\ntext".into()));
    }
}
