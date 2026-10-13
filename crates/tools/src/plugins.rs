//! Plugins: git repositories cloned into `<data>/plugins/<id>/` whose skills
//! and rules every task gets, like the global ones. The folder is the record
//! (no table): the clone's remote, the ref it follows (`git config
//! reagent.ref`, empty: the remote's default branch), its commit and the
//! rules switched off (`git config reagent.ruleOff`, one value each).
//! Updating fetches that ref and resets to it (a plugin's folder is
//! reagent's: changes made there go).

use std::path::{Path, PathBuf};

use crate::git::git;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Plugin {
    pub id: String,
    pub url: String,
    /// The branch or tag it follows (None: the remote's default branch).
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub commit: String,
    pub subject: String,
    /// Its commit's time (unix seconds).
    pub date: i64,
    pub skills: Vec<String>,
    pub rules: Vec<String>,
    /// Its rules no task gets (a name may outlive its rule across an update).
    pub rules_off: Vec<String>,
}

pub fn dir(data: &Path) -> PathBuf {
    data.join("plugins")
}

/// The folders a plugin's skills are in: `skills/`, `skill/`, and the
/// generic places under its root (`.agents/skills/`, …).
pub fn skill_places(plugin: &Path) -> Vec<PathBuf> {
    vec![plugin.join("skills"), plugin.join("skill"), plugin.to_path_buf()]
}

/// The folders a plugin's rules are in.
pub fn rule_places(plugin: &Path) -> Vec<PathBuf> {
    let mut v = vec![plugin.join("rules")];
    v.extend(crate::rules::DIRS.iter().map(|d| plugin.join(d)));
    v
}

/// The plugins' folders, by id.
pub fn dirs(data: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(dir(data)) else { return vec![] };
    let mut out: Vec<(String, PathBuf)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join(".git").exists())
        .filter_map(|p| Some((p.file_name()?.to_str()?.to_string(), p)))
        .filter(|(id, _)| valid_id(id))
        .collect();
    out.sort();
    out
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && !id.starts_with(['.', '-']) && id.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// The id a URL gives: its last part without `.git`.
pub fn id_of(url: &str) -> String {
    let last = url.trim_end_matches('/').rsplit(['/', ':']).next().unwrap_or_default();
    last.trim_end_matches(".git").chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect::<String>().trim_start_matches(['.', '-']).to_string()
}

fn read(id: &str, d: &Path) -> Result<Plugin, String> {
    let url = git(d, &["remote", "get-url", "origin"])?.trim().to_string();
    let reference = git(d, &["config", "reagent.ref"]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let log = git(d, &["log", "-1", "--format=%h%x00%ct%x00%s"])?;
    let mut f = log.trim_end().splitn(3, '\0');
    let (commit, date, subject) = (f.next().unwrap_or_default().to_string(), f.next().and_then(|s| s.parse().ok()).unwrap_or(0), f.next().unwrap_or_default().to_string());
    let skills = crate::skills::in_places(&skill_places(d)).into_iter().map(|s| s.name).collect();
    let rules = crate::rules::in_places(&rule_places(d)).into_iter().map(|r| r.name).collect();
    Ok(Plugin { id: id.into(), url, reference, commit, subject, date, skills, rules, rules_off: rules_off(d) })
}

fn rules_off(d: &Path) -> Vec<String> {
    git(d, &["config", "--get-all", "reagent.ruleOff"]).map(|s| s.lines().map(str::to_string).filter(|l| !l.is_empty()).collect()).unwrap_or_default()
}

/// A plugin's rules that tasks get: those not switched off.
pub fn rules(plugin: &Path) -> Vec<crate::rules::Rule> {
    let off = rules_off(plugin);
    crate::rules::in_places(&rule_places(plugin)).into_iter().filter(|r| !off.contains(&r.name)).collect()
}

/// Switches these of a plugin's rules off (and the others on again).
pub fn set_rules_off(data: &Path, id: &str, names: &[String]) -> Result<Plugin, String> {
    let p = get(data, id)?;
    if let Some(n) = names.iter().find(|n| !p.rules.contains(n)) {
        return Err(format!("{id} has no rule {n} (it has: {})", p.rules.join(", ")));
    }
    let d = dir(data).join(id);
    // ponytail: exit 5 just means there was none to unset.
    let _ = git(&d, &["config", "--unset-all", "reagent.ruleOff"]);
    for n in names {
        git(&d, &["config", "--add", "reagent.ruleOff", n])?;
    }
    read(id, &d)
}

pub fn list(data: &Path) -> Vec<Plugin> {
    dirs(data).into_iter().filter_map(|(id, d)| read(&id, &d).ok()).collect()
}

pub fn get(data: &Path, id: &str) -> Result<Plugin, String> {
    let d = dir(data).join(id);
    if !valid_id(id) || !d.join(".git").exists() {
        return Err(format!("no plugin {id}"));
    }
    read(id, &d)
}

/// Clones a plugin (a shallow clone of `reference`, else the default branch).
pub fn add(data: &Path, url: &str, id: Option<&str>, reference: Option<&str>) -> Result<Plugin, String> {
    let url = url.trim();
    if url.is_empty() || url.starts_with('-') {
        return Err("a plugin needs a git URL".into());
    }
    let id = id.map(str::to_string).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| id_of(url));
    if !valid_id(&id) {
        return Err(format!("{id:?} can't be a plugin's id (letters, digits, . _ -)"));
    }
    let root = dir(data);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let d = root.join(&id);
    if d.exists() {
        return Err(format!("there's a plugin {id} already"));
    }
    let reference = reference.map(str::trim).filter(|r| !r.is_empty());
    let mut args = vec!["clone", "--depth", "1", "--quiet"];
    if let Some(r) = reference {
        args.extend(["--branch", r]);
    }
    let target = d.to_string_lossy().into_owned();
    args.extend(["--", url, &target]);
    if let Err(e) = git(&root, &args) {
        let _ = std::fs::remove_dir_all(&d);
        return Err(e);
    }
    git(&d, &["config", "reagent.ref", reference.unwrap_or("")])?;
    read(&id, &d)
}

/// Fetches what the plugin follows and moves onto it: the commits before and after.
pub fn update(data: &Path, id: &str) -> Result<(String, Plugin), String> {
    let before = get(data, id)?;
    let d = dir(data).join(id);
    let r = before.reference.clone().unwrap_or_else(|| "HEAD".into());
    git(&d, &["fetch", "--depth", "1", "--quiet", "origin", &r])?;
    git(&d, &["reset", "--hard", "--quiet", "FETCH_HEAD"])?;
    git(&d, &["clean", "-fdq"])?;
    Ok((before.commit, read(id, &d)?))
}

pub fn remove(data: &Path, id: &str) -> Result<(), String> {
    get(data, id)?;
    std::fs::remove_dir_all(dir(data).join(id)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str]) {
        git(dir, args).unwrap();
    }

    #[test]
    fn ids_come_from_urls() {
        assert_eq!(id_of("https://github.com/DietrichGebert/ponytail"), "ponytail");
        assert_eq!(id_of("git@github.com:mkg20001/skill.git"), "skill");
        assert_eq!(id_of("https://x/y/.hidden/"), "hidden");
        assert!(!valid_id("../x") && !valid_id(".git") && !valid_id("") && valid_id("my_plugin-2.0"));
    }

    #[test]
    fn a_plugin_is_cloned_read_updated_and_removed() {
        let t = tempfile::tempdir().unwrap();
        let (src, data) = (t.path().join("src"), t.path().join("data"));
        std::fs::create_dir_all(src.join("skills/greet")).unwrap();
        std::fs::create_dir_all(src.join(".agents/rules")).unwrap();
        std::fs::write(src.join("skills/greet/SKILL.md"), "---\nname: greet\ndescription: say hi\n---\nHi.").unwrap();
        std::fs::write(src.join(".agents/rules/tone.md"), "Be brief.").unwrap();
        sh(&src, &["init", "-q", "-b", "main"]);
        sh(&src, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qam", "one", "--allow-empty"]);
        sh(&src, &["add", "-A"]);
        sh(&src, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "two"]);
        let url = format!("file://{}", src.display());
        let p = add(&data, &url, Some("tools"), None).unwrap();
        assert_eq!((p.id.as_str(), p.subject.as_str(), p.skills.clone(), p.rules.clone(), p.reference.clone()), ("tools", "two", vec!["greet".to_string()], vec!["tone".to_string()], None));
        assert!(add(&data, &url, Some("tools"), None).unwrap_err().contains("already"));
        assert!(add(&data, &url, Some("../x"), None).is_err());
        assert!(add(&data, "-oops", None, None).is_err());
        // A new commit upstream; local changes go.
        std::fs::create_dir_all(src.join("skill/second")).unwrap();
        std::fs::write(src.join("skill/second/SKILL.md"), "---\nname: second\ndescription: d\n---\nx").unwrap();
        sh(&src, &["add", "-A"]);
        sh(&src, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "three"]);
        std::fs::write(dir(&data).join("tools/skills/greet/SKILL.md"), "edited").unwrap();
        let (before, after) = update(&data, "tools").unwrap();
        assert_eq!((before, after.subject.as_str()), (p.commit, "three"));
        assert_eq!(after.skills, ["greet", "second"]);
        assert!(set_rules_off(&data, "tools", &["nope".into()]).unwrap_err().contains("no rule nope"));
        assert_eq!(set_rules_off(&data, "tools", &["tone".into()]).unwrap().rules_off, ["tone"]);
        assert!(rules(&dir(&data).join("tools")).is_empty(), "a rule switched off");
        update(&data, "tools").unwrap();
        assert_eq!(get(&data, "tools").unwrap().rules_off, ["tone"], "kept over an update");
        assert!(set_rules_off(&data, "tools", &[]).unwrap().rules_off.is_empty());
        assert_eq!(rules(&dir(&data).join("tools")).len(), 1, "on again");
        assert_eq!(list(&data).len(), 1);
        remove(&data, "tools").unwrap();
        assert!(list(&data).is_empty() && remove(&data, "tools").is_err());
    }
}
