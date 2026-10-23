//! Plugins: git repositories cloned into `<data>/plugins/<id>/` whose skills
//! and rules every task gets, like the global ones. The folder is the record
//! (no table): the clone's remote, the ref it follows (`git config
//! reagent.ref`, empty: the remote's default branch), its commit and the
//! rules switched off (`git config reagent.ruleOff`, one value each).
//! Updating fetches that ref and resets to it (a plugin's folder is
//! reagent's: changes made there go).
//!
//! A plugin may be a marketplace (Claude Code's `.claude-plugin/marketplace.json`,
//! the generic `.agents/plugins/marketplace.json`): its plugins in local folders
//! are its parts, with their skills, rules and templates.

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
    /// Its marketplace's name, if it is one.
    pub marketplace: Option<String>,
    /// Its marketplace's plugins (`skills` and `rules` above: those outside them).
    pub plugins: Vec<Part>,
}

/// One of a marketplace's plugins.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Part {
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    /// Its folder in the clone (None: not one reagent has).
    pub path: Option<String>,
    pub skills: Vec<String>,
    pub rules: Vec<String>,
    /// Why it isn't used (a remote source, a missing folder).
    pub error: Option<String>,
}

const MARKETPLACES: [&str; 2] = [".claude-plugin/marketplace.json", ".agents/plugins/marketplace.json"];
const MANIFESTS: [&str; 2] = [".claude-plugin/plugin.json", ".codex-plugin/plugin.json"];

fn json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// A folder named in a manifest, relative to `base` and inside `root`.
fn inside(root: &Path, base: &Path, rel: &str) -> Result<PathBuf, String> {
    use std::path::Component;
    let r = Path::new(rel);
    if r.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
        return Err(format!("{rel} is outside the repository"));
    }
    let d = base.join(r);
    match (d.canonicalize(), root.canonicalize()) {
        (Ok(c), Ok(r)) if c.starts_with(&r) => Ok(d),
        (Ok(_), _) => Err(format!("{rel} is outside the repository")),
        _ => Err(format!("{rel} isn't there")),
    }
}

/// A string or a list of them.
fn strings(v: &serde_json::Value) -> Vec<&str> {
    match v {
        serde_json::Value::String(s) => vec![s.as_str()],
        v => v.as_array().into_iter().flatten().filter_map(|s| s.as_str()).collect(),
    }
}

/// The folders a plugin folder's skills are in: `skills/`, `skill/`, the
/// generic places under it, and those its manifest (or marketplace entry) names.
fn own_skill_places(root: &Path, d: &Path, extra: &[&str]) -> Vec<PathBuf> {
    let manifest = MANIFESTS.iter().find_map(|m| json(&d.join(m)));
    let named = manifest.as_ref().map(|m| strings(&m["skills"])).unwrap_or_default();
    let mut v = vec![d.join("skills"), d.join("skill"), d.to_path_buf()];
    v.extend(named.iter().chain(extra).filter_map(|rel| inside(root, d, rel).ok()));
    v
}

fn own_rule_places(d: &Path) -> Vec<PathBuf> {
    let mut v = vec![d.join("rules")];
    v.extend(crate::rules::DIRS.iter().map(|r| d.join(r)));
    v
}

/// A marketplace's plugins: each one's entry and its folder (or why none).
/// Both kinds of marketplace may be there; the first wins on a name.
fn entries(root: &Path) -> (Option<String>, Vec<(serde_json::Value, Result<PathBuf, String>)>) {
    let (mut name, mut out) = (None, vec![]);
    for m in MARKETPLACES {
        let Some(mk) = json(&root.join(m)) else { continue };
        name = name.or_else(|| mk["name"].as_str().map(str::to_string));
        // Claude Code's `metadata.pluginRoot`: the base of bare relative sources.
        let base = mk["metadata"]["pluginRoot"].as_str().and_then(|b| inside(root, root, b).ok()).unwrap_or_else(|| root.to_path_buf());
        for e in mk["plugins"].as_array().into_iter().flatten() {
            let Some(n) = e["name"].as_str() else { continue };
            if out.iter().any(|(o, _): &(serde_json::Value, _)| o["name"] == n) {
                continue;
            }
            let src = &e["source"];
            let dir = match (src.as_str(), src["source"].as_str(), src["path"].as_str()) {
                (Some(rel), ..) => inside(root, &base, rel),
                (None, Some("local"), Some(rel)) => inside(root, root, rel),
                (None, Some(kind), _) => {
                    let at = src["url"].as_str().map(str::to_string).or_else(|| src["repo"].as_str().map(|r| format!("https://github.com/{r}")));
                    Err(match at {
                        Some(u) => format!("a {kind} source: add {u} as its own plugin"),
                        None => format!("a {kind} source, which reagent doesn't fetch"),
                    })
                }
                _ => Err("no source".into()),
            };
            out.push((e.clone(), dir));
        }
    }
    (name, out)
}

/// A plugin's folders: its own (None) and its marketplace's local plugins'.
pub fn parts(plugin: &Path) -> Vec<(Option<String>, PathBuf)> {
    let mut v = vec![(None, plugin.to_path_buf())];
    v.extend(entries(plugin).1.into_iter().filter_map(|(e, d)| Some((Some(e["name"].as_str()?.to_string()), d.ok()?))));
    v
}

/// What a plugin's skill or template shows as its source: `plugin:<id>`, `plugin:<id>/<part>`.
pub fn source(id: &str, part: Option<&str>) -> String {
    match part {
        Some(p) => format!("plugin:{id}/{p}"),
        None => format!("plugin:{id}"),
    }
}

/// The part a file of a plugin belongs to: the deepest folder holding it (None: the plugin's own).
pub fn part_of(parts: &[(Option<String>, PathBuf)], file: &Path) -> Option<String> {
    parts.iter().filter(|(_, d)| file.starts_with(d)).max_by_key(|(n, d)| (d.components().count(), n.is_some())).and_then(|(n, _)| n.clone())
}

pub fn dir(data: &Path) -> PathBuf {
    data.join("plugins")
}

/// The folders a plugin's skills are in, its marketplace's plugins' too.
pub fn skill_places(plugin: &Path) -> Vec<PathBuf> {
    let mut v = own_skill_places(plugin, plugin, &[]);
    for (e, d) in entries(plugin).1 {
        if let Ok(d) = d {
            v.extend(own_skill_places(plugin, &d, &strings(&e["skills"])));
        }
    }
    v
}

/// The folders a plugin's rules are in, its marketplace's plugins' too.
pub fn rule_places(plugin: &Path) -> Vec<PathBuf> {
    parts(plugin).iter().flat_map(|(_, d)| own_rule_places(d)).collect()
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
    let (marketplace, entries) = entries(d);
    let all = parts(d);
    let mut skills: Vec<(Option<String>, String)> = crate::skills::in_places(&skill_places(d)).into_iter().map(|s| (part_of(&all, &s.dir), s.name)).collect();
    let mut rules: Vec<(Option<String>, String)> = crate::rules::in_places(&rule_places(d)).into_iter().map(|r| (part_of(&all, &r.path), r.name)).collect();
    let take = |v: &mut Vec<(Option<String>, String)>, part: Option<&str>| {
        let (mine, rest) = std::mem::take(v).into_iter().partition(|(p, _)| p.as_deref() == part);
        *v = rest;
        mine.into_iter().map(|(_, n)| n).collect::<Vec<String>>()
    };
    let plugins = entries
        .into_iter()
        .map(|(e, dir)| {
            let name = e["name"].as_str().unwrap_or_default().to_string();
            let manifest = dir.as_ref().ok().and_then(|p| MANIFESTS.iter().find_map(|m| json(&p.join(m)))).unwrap_or_default();
            let text = |k: &str| e[k].as_str().or(manifest[k].as_str()).map(str::to_string);
            Part {
                description: text("description"),
                version: text("version"),
                path: dir.as_ref().ok().map(|p| p.strip_prefix(d).unwrap_or(p).to_string_lossy().to_string()).map(|p| if p.is_empty() { ".".into() } else { p }),
                skills: take(&mut skills, Some(&name)),
                rules: take(&mut rules, Some(&name)),
                error: dir.err(),
                name,
            }
        })
        .collect();
    let (skills, rules) = (take(&mut skills, None), take(&mut rules, None));
    Ok(Plugin { id: id.into(), url, reference, commit, subject, date, skills, rules, rules_off: rules_off(d), marketplace, plugins })
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
    // Absolute: git clones into it from inside it.
    let root = std::path::absolute(dir(data)).map_err(|e| e.to_string())?;
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

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn commit(src: &Path) {
        sh(src, &["init", "-q", "-b", "main"]);
        sh(src, &["add", "-A"]);
        sh(src, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "one"]);
    }

    fn skill(dir: &Path, name: &str) {
        write(&dir.join(name).join("SKILL.md"), &format!("---\nname: {name}\ndescription: d\n---\nx"));
    }

    #[test]
    fn a_marketplace_has_its_plugins_as_parts() {
        let t = tempfile::tempdir().unwrap();
        let (src, data) = (t.path().join("src"), t.path().join("data"));
        // Claude Code's, like trailofbits/skills: bare relative sources, a manifest naming more skills.
        write(
            &src.join(".claude-plugin/marketplace.json"),
            r#"{"name": "tob", "plugins": [
                {"name": "static", "description": "scans", "source": "./plugins/static"},
                {"name": "remote", "source": {"source": "github", "repo": "o/r"}},
                {"name": "escape", "source": "../elsewhere"},
                {"name": "gone", "source": "./plugins/gone"}]}"#,
        );
        skill(&src.join("plugins/static/skills"), "semgrep");
        write(&src.join("plugins/static/.claude-plugin/plugin.json"), r#"{"name": "static", "version": "1.5", "skills": ["./extra"]}"#);
        skill(&src.join("plugins/static/extra"), "codeql");
        write(&src.join("plugins/static/rules/scan.md"), "Scan first.");
        // The generic (Codex) one: a local source object; the same name again loses.
        write(
            &src.join(".agents/plugins/marketplace.json"),
            r#"{"name": "other", "plugins": [
                {"name": "lint", "source": {"source": "local", "path": "./plugins/lint"}},
                {"name": "static", "source": {"source": "local", "path": "./plugins/lint"}}]}"#,
        );
        skill(&src.join("plugins/lint/skills"), "clippy");
        write(&src.join("plugins/lint/templates/fix.md"), "Fix it.");
        // The repository's own skill, outside its plugins.
        skill(&src.join("skills"), "own");
        commit(&src);
        let p = add(&data, &format!("file://{}", src.display()), Some("tob"), None).unwrap();
        assert_eq!((p.marketplace.as_deref(), p.skills.clone(), p.rules.clone()), (Some("tob"), vec!["own".to_string()], vec![]));
        let by = |n: &str| p.plugins.iter().find(|x| x.name == n).unwrap();
        let st = by("static");
        assert_eq!((st.path.as_deref(), st.version.as_deref(), st.description.as_deref()), (Some("plugins/static"), Some("1.5"), Some("scans")));
        assert_eq!((st.skills.clone(), st.rules.clone()), (vec!["semgrep".to_string(), "codeql".to_string()], vec!["scan".to_string()]));
        assert_eq!(by("lint").skills, ["clippy"]);
        assert!(by("remote").error.as_deref().unwrap().contains("https://github.com/o/r"));
        assert!(by("escape").error.as_deref().unwrap().contains("outside"));
        assert!(by("gone").error.as_deref().unwrap().contains("isn't there"));
        assert_eq!(p.plugins.len(), 5);
        let d = dir(&data).join("tob");
        let all = parts(&d);
        let s = crate::skills::in_places(&skill_places(&d));
        let codeql = s.iter().find(|s| s.name == "codeql").unwrap();
        assert_eq!(source("tob", part_of(&all, &codeql.dir).as_deref()), "plugin:tob/static");
        assert_eq!(rules(&d).len(), 1);
        let ts = crate::templates::global_places(&data);
        assert!(ts.iter().any(|(p, s)| p.ends_with("plugins/lint/templates") && s == "plugin:tob/lint"));
    }

    #[test]
    fn a_marketplace_of_itself_is_one_part() {
        let t = tempfile::tempdir().unwrap();
        let (src, data) = (t.path().join("src"), t.path().join("data"));
        write(&src.join(".claude-plugin/marketplace.json"), r#"{"name": "m", "plugins": [{"name": "me", "source": "./"}]}"#);
        skill(&src.join("skills"), "one");
        commit(&src);
        let p = add(&data, &format!("file://{}", src.display()), Some("me"), None).unwrap();
        assert!(p.skills.is_empty());
        assert_eq!((p.plugins[0].path.as_deref(), p.plugins[0].skills.clone()), (Some("."), vec!["one".to_string()]));
    }
}
