//! Memory: a folder of markdown per scope (global, each project) with an
//! `INDEX.md` the tools keep: one line per file (`- [name](path) — about`),
//! in a section by kind (topics/, folders/, tasks/, the rest).

use std::path::{Component, Path, PathBuf};

use crate::edit::{self, Edit};

pub const INDEX: &str = "INDEX.md";

const SECTIONS: [(&str, &str); 4] = [("topics/", "Topics"), ("folders/", "Folders"), ("tasks/", "Tasks"), ("", "Other")];

pub struct Memory {
    pub dir: PathBuf,
    /// What the index's heading calls it ("Global memory", "Memory of site").
    pub title: String,
    /// Subfolders that aren't this memory's (the global one holds the projects').
    pub skip: Vec<String>,
}

/// An index line.
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    path: String,
    about: String,
}

fn section_of(path: &str) -> &'static str {
    SECTIONS.iter().find(|(p, _)| path.starts_with(p)).map(|(_, s)| *s).unwrap_or("Other")
}

impl Memory {
    pub fn new(dir: PathBuf, title: &str) -> Self {
        Memory { dir, title: title.into(), skip: vec![] }
    }

    /// A file's path inside the memory: relative, markdown, no `..`.
    fn file(&self, rel: &str) -> Result<PathBuf, String> {
        let rel = rel.trim().trim_start_matches("./");
        let p = Path::new(rel);
        if rel.is_empty() || p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
            return Err(format!("{rel:?}: a path inside the memory, like topics/build.md"));
        }
        if !rel.ends_with(".md") {
            return Err(format!("{rel:?}: memory files are markdown (.md)"));
        }
        if rel == INDEX {
            return Err("INDEX.md is kept by the memory tools: write the files, with their `about`".into());
        }
        if self.skip.iter().any(|s| rel.starts_with(&format!("{s}/"))) {
            return Err(format!("{rel:?} isn't in this memory"));
        }
        Ok(self.dir.join(rel))
    }

    fn entries(&self) -> Vec<Entry> {
        let text = std::fs::read_to_string(self.dir.join(INDEX)).unwrap_or_default();
        text.lines()
            .filter_map(|l| {
                let l = l.trim().strip_prefix("- [")?;
                let (_, rest) = l.split_once("](")?;
                let (path, about) = rest.split_once(')')?;
                Some(Entry { path: path.to_string(), about: about.trim().trim_start_matches('—').trim_start_matches('-').trim().to_string() })
            })
            .collect()
    }

    fn write_index(&self, mut entries: Vec<Entry>) -> std::io::Result<()> {
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let mut out = format!("# {}\n", self.title);
        for (_, name) in SECTIONS {
            let here: Vec<&Entry> = entries.iter().filter(|e| section_of(&e.path) == name).collect();
            if here.is_empty() {
                continue;
            }
            out.push_str(&format!("\n## {name}\n\n"));
            for e in here {
                let label = e.path.trim_end_matches(".md");
                let label = SECTIONS.iter().find_map(|(p, _)| (!p.is_empty()).then(|| label.strip_prefix(p)).flatten()).unwrap_or(label);
                out.push_str(&format!("- [{label}]({}) — {}\n", e.path, e.about));
            }
        }
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.dir.join(INDEX), out)
    }

    /// The index (made if there's none yet).
    pub fn index(&self) -> String {
        match std::fs::read_to_string(self.dir.join(INDEX)) {
            Ok(t) => t,
            Err(_) => format!("# {}\n\n(empty: nothing written yet)\n", self.title),
        }
    }

    pub fn read(&self, rel: &str) -> Result<String, String> {
        std::fs::read_to_string(self.file(rel)?).map_err(|_| format!("no memory file {rel:?} (the index lists them)"))
    }

    /// Creates or replaces a file, with its index line.
    pub fn write(&self, rel: &str, text: &str, about: &str) -> Result<String, String> {
        let about = about.trim().replace('\n', " ");
        if about.is_empty() {
            return Err("about: one line saying what's in it (it goes in the index)".into());
        }
        let p = self.file(rel)?;
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&p, text).map_err(|e| e.to_string())?;
        let rel = rel.trim().trim_start_matches("./").to_string();
        let mut entries = self.entries();
        entries.retain(|e| e.path != rel);
        entries.push(Entry { path: rel.clone(), about });
        self.write_index(entries).map_err(|e| e.to_string())?;
        Ok(format!("wrote {rel}"))
    }

    /// Changes part of a file (its index line stays, or takes a new `about`).
    pub fn edit(&self, rel: &str, e: &Edit, about: Option<&str>) -> Result<String, String> {
        let body = self.read(rel)?;
        let new = edit::apply(&body, e)?;
        let p = self.file(rel)?;
        std::fs::write(&p, new).map_err(|e| e.to_string())?;
        let rel = rel.trim().trim_start_matches("./").to_string();
        let mut entries = self.entries();
        let old_about = entries.iter().find(|e| e.path == rel).map(|e| e.about.clone());
        if about.is_some() || old_about.is_none() {
            entries.retain(|e| e.path != rel);
            entries.push(Entry { path: rel.clone(), about: about.map(String::from).or(old_about).unwrap_or_else(|| "(no description)".into()) });
            self.write_index(entries).map_err(|e| e.to_string())?;
        }
        Ok(format!("changed {rel}"))
    }

    pub fn remove(&self, rel: &str) -> Result<String, String> {
        let p = self.file(rel)?;
        std::fs::remove_file(&p).map_err(|_| format!("no memory file {rel:?}"))?;
        let rel = rel.trim().trim_start_matches("./").to_string();
        let mut entries = self.entries();
        entries.retain(|e| e.path != rel);
        self.write_index(entries).map_err(|e| e.to_string())?;
        Ok(format!("removed {rel}"))
    }

    /// Every markdown file's lines matching `pattern` (case-insensitive).
    pub fn search(&self, re: &regex::Regex) -> Vec<serde_json::Value> {
        let mut hits = vec![];
        let skip: Vec<PathBuf> = self.skip.iter().map(|s| self.dir.join(s)).collect();
        for e in ignore::WalkBuilder::new(&self.dir).hidden(false).git_ignore(false).build().flatten() {
            let p = e.path();
            if !p.extension().is_some_and(|x| x == "md") || skip.iter().any(|s| p.starts_with(s)) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(p) else { continue };
            let rel = p.strip_prefix(&self.dir).unwrap_or(p).display().to_string();
            for (i, l) in text.lines().enumerate() {
                if re.is_match(l) {
                    hits.push(serde_json::json!({"file": rel, "line": i + 1, "text": l.chars().take(400).collect::<String>()}));
                    if hits.len() >= 100 {
                        return hits;
                    }
                }
            }
        }
        hits
    }

    /// When the index last changed (for re-injecting it).
    pub fn stamp(&self) -> Option<std::time::SystemTime> {
        std::fs::metadata(self.dir.join(INDEX)).and_then(|m| m.modified()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_index_follows_the_files() {
        let d = tempfile::tempdir().unwrap();
        let m = Memory::new(d.path().to_path_buf(), "Memory of site");
        assert!(m.index().contains("empty"));
        m.write("topics/build.md", "cargo needs nix develop", "how to build").unwrap();
        m.write("folders/crates/core.md", "pure, no I/O", "the core crate").unwrap();
        m.write("notes.md", "x", "loose notes").unwrap();
        let idx = m.index();
        assert!(idx.starts_with("# Memory of site\n"), "{idx}");
        assert!(idx.contains("## Topics\n\n- [build](topics/build.md) — how to build\n"), "{idx}");
        assert!(idx.contains("## Folders\n\n- [crates/core](folders/crates/core.md) — the core crate\n"), "{idx}");
        assert!(idx.contains("## Other\n\n- [notes](notes.md) — loose notes\n"), "{idx}");
        assert!(idx.find("## Topics").unwrap() < idx.find("## Folders").unwrap());
        // Rewritten with a new description: one line, the new one.
        m.write("topics/build.md", "cargo build", "building, briefly").unwrap();
        assert_eq!(m.index().matches("topics/build.md").count(), 1);
        m.edit("topics/build.md", &Edit { op: Some(crate::edit::Op::Append), text: Some("tests: target/test.log".into()), ..Default::default() }, None).unwrap();
        assert_eq!(m.read("topics/build.md").unwrap(), "cargo build\ntests: target/test.log");
        assert!(m.index().contains("building, briefly"), "the description stays");
        m.remove("notes.md").unwrap();
        assert!(!m.index().contains("notes.md") && !m.index().contains("## Other"));
        let re = regex::RegexBuilder::new("PURE").case_insensitive(true).build().unwrap();
        assert_eq!(m.search(&re)[0]["file"], "folders/crates/core.md");
    }

    #[test]
    fn paths_stay_inside() {
        let d = tempfile::tempdir().unwrap();
        let mut m = Memory::new(d.path().to_path_buf(), "Global memory");
        m.skip = vec!["projects".into()];
        assert!(m.write("../x.md", "x", "x").is_err());
        assert!(m.write("/etc/x.md", "x", "x").is_err());
        assert!(m.write("x.txt", "x", "x").unwrap_err().contains("markdown"));
        assert!(m.write("INDEX.md", "x", "x").unwrap_err().contains("kept by"));
        assert!(m.write("projects/site/a.md", "x", "x").is_err());
        assert!(m.write("a.md", "x", " ").unwrap_err().contains("about"));
        std::fs::create_dir_all(d.path().join("projects/site")).unwrap();
        std::fs::write(d.path().join("projects/site/secret.md"), "needle").unwrap();
        let re = regex::Regex::new("needle").unwrap();
        assert!(m.search(&re).is_empty(), "a project's memory isn't the global one's");
    }
}
