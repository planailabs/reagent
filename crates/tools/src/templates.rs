//! Prompt templates: markdown with YAML frontmatter, the body the prompt
//! with `{{field}}` placeholders. With `fields` a template is a form (each
//! field filled in, the prompt rendered); without, its body is a starting
//! point to edit. Frontmatter: `description`, `title` (placeholders too;
//! none: the task names itself), `skills`, `budget`, `profile`, `kind`,
//! `project` (a global one pinned to a project), `worktree` (a persistent
//! worktree of its project the task starts in), `fields` (`name`, `label`,
//! `type`: text, multiline, select (`options`), checkbox; `default`,
//! `required`).
//!
//! Found, the first winning on a name: the project's `.agents/templates/`
//! (or `.agent/templates/`), those kept in reagent.db (the project's, then
//! every project's), the global folders (`~/.agents/templates/`,
//! `<data>/templates/`), then plugins' (`templates/`, `.agents/templates/`).

use std::path::{Path, PathBuf};

use reagent_store::{Budget, Project, Store};
use serde_json::{Map, Value};

use crate::app::StartTask;

pub const DIRS: [&str; 2] = [".agents/templates", ".agent/templates"];

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Field {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    /// text (default), multiline, select, checkbox.
    #[serde(rename = "type", default = "text")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default)]
    pub required: bool,
}

fn text() -> String {
    "text".into()
}

#[derive(serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Front {
    description: Option<String>,
    title: Option<String>,
    #[serde(default)]
    skills: Vec<String>,
    budget: Option<Budget>,
    profile: Option<String>,
    kind: Option<String>,
    project: Option<String>,
    worktree: Option<String>,
    #[serde(default)]
    fields: Vec<Field>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Template {
    /// Its path under the templates folder without `.md` (or its name, kept in reagent).
    pub name: String,
    pub description: String,
    pub title: Option<String>,
    /// The body: the prompt, with placeholders.
    pub prompt: String,
    pub fields: Vec<Field>,
    pub skills: Vec<String>,
    pub budget: Option<Budget>,
    pub profile: Option<String>,
    pub kind: Option<String>,
    /// The project it's for (a global one pinned to it).
    pub project: Option<String>,
    /// The persistent worktree its task starts in.
    pub worktree: Option<String>,
    /// project, db, global, plugin:<id>.
    pub source: String,
    pub path: Option<PathBuf>,
    /// Kept in reagent: its row.
    pub id: Option<i64>,
    /// The whole file, as written.
    pub text: String,
    /// Why it can't be used (it's listed anyway, to be fixed).
    pub error: Option<String>,
}

/// A template's file; a broken one carries its error.
pub fn parse(name: &str, text: &str, source: &str) -> Template {
    let mut t = Template { name: name.into(), description: String::new(), title: None, prompt: String::new(), fields: vec![], skills: vec![], budget: None, profile: None, kind: None, project: None, worktree: None, source: source.into(), path: None, id: None, text: text.into(), error: None };
    let s = text.trim_start_matches('\u{feff}');
    let (front, body) = match s.strip_prefix("---").map(|r| r.trim_start_matches(['\r', '\n'])).and_then(|r| r.find("\n---").map(|end| (r, end))) {
        Some((r, end)) => match serde_yaml::from_str::<Option<Front>>(&r[..end]) {
            Ok(f) => (f.unwrap_or_default(), r[end + 4..].trim_start_matches('-').trim().to_string()),
            Err(e) => {
                t.error = Some(format!("its frontmatter: {e}"));
                return t;
            }
        },
        None => (Front::default(), s.trim().to_string()),
    };
    t.description = front.description.unwrap_or_else(|| body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(120).collect());
    (t.title, t.prompt, t.skills, t.budget, t.profile, t.kind, t.project, t.worktree) = (front.title, body, front.skills, front.budget, front.profile, front.kind, front.project, front.worktree);
    t.fields = front.fields;
    t.error = check(&t).err();
    t
}

fn check(t: &Template) -> Result<(), String> {
    if t.prompt.is_empty() {
        return Err("no prompt (the body is empty)".into());
    }
    for (i, f) in t.fields.iter().enumerate() {
        if f.name.is_empty() || !f.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(format!("field {}: a name is letters, digits, _ and -", i + 1));
        }
        if t.fields[..i].iter().any(|g| g.name == f.name) {
            return Err(format!("field {} twice", f.name));
        }
        match f.kind.as_str() {
            "text" | "multiline" | "checkbox" => {}
            "select" if !f.options.is_empty() => {}
            "select" => return Err(format!("field {}: a select needs options", f.name)),
            k => return Err(format!("field {}: no type {k} (text, multiline, select, checkbox)", f.name)),
        }
    }
    Ok(())
}

fn walk(root: &Path, dir: &Path, source: &str, out: &mut Vec<Template>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            walk(root, &p, source, out);
        } else if p.extension().is_some_and(|e| e == "md")
            && let Ok(text) = std::fs::read_to_string(&p)
        {
            let name = p.strip_prefix(root).unwrap_or(&p).with_extension("").to_string_lossy().replace('\\', "/");
            let mut t = parse(&name, &text, source);
            t.path = Some(p);
            out.push(t);
        }
    }
}

/// The global places, in order, with their source: `~/.agents/templates`,
/// `<data>/templates`, then each plugin's.
pub fn global_places(data: &Path) -> Vec<(PathBuf, String)> {
    let mut out = vec![];
    if let Some(h) = std::env::var_os("HOME") {
        out.push((PathBuf::from(h).join(".agents/templates"), "global".to_string()));
    }
    out.push((data.join("templates"), "global".into()));
    for (id, d) in crate::plugins::dirs(data) {
        for (part, d) in crate::plugins::parts(&d) {
            for sub in ["templates", ".agents/templates"] {
                out.push((d.join(sub), crate::plugins::source(&id, part.as_deref())));
            }
        }
    }
    out
}

/// The templates a project's tasks may start from (none: every project's,
/// only the global ones). Pinned to another project, a global one is listed
/// too (picking it moves to that project).
pub async fn discover(store: &Store, data: &Path, project: Option<&Project>) -> Vec<Template> {
    let mut found = vec![];
    if let Some(p) = project {
        for d in DIRS {
            let root = Path::new(&p.path).join(d);
            walk(&root, &root, "project", &mut found);
        }
    }
    // Kept in reagent: the project's, every project's, then those pinned to another.
    let here = project.map(|p| p.slug.as_str());
    let mut kept = store.templates().await.unwrap_or_default();
    kept.sort_by_key(|k| match (k.project.as_deref(), here) {
        (Some(p), Some(h)) if p == h => 0,
        (None, _) => 1,
        _ => 2,
    });
    for k in kept {
        let mut t = parse(&k.name, &k.text, "db");
        t.id = Some(k.id);
        if k.project.is_some() {
            t.project = k.project;
        }
        found.push(t);
    }
    for (dir, source) in global_places(data) {
        walk(&dir, &dir, &source, &mut found);
    }
    let mut out: Vec<Template> = vec![];
    for t in found {
        if !out.iter().any(|o| o.name == t.name && o.project == t.project) {
            out.push(t);
        }
    }
    out
}

/// One template by name, for a project (or by its pin, with none).
pub async fn find(store: &Store, data: &Path, project: Option<&Project>, name: &str) -> Result<Template, String> {
    let all = discover(store, data, project).await;
    let here = project.map(|p| p.slug.as_str());
    let t = all
        .iter()
        .find(|t| t.name == name && (t.project.is_none() || t.project.as_deref() == here))
        .or_else(|| all.iter().find(|t| t.name == name))
        .cloned()
        .ok_or_else(|| format!("no template {name:?}"))?;
    if let Some(e) = &t.error {
        return Err(format!("the template {name} is broken: {e}"));
    }
    if let (Some(pin), Some(here)) = (&t.project, here)
        && pin != here
    {
        return Err(format!("the template {name} is for the project {pin}"));
    }
    Ok(t)
}

/// What agents are shown of templates: no file text or paths.
pub fn summaries(list: &[Template]) -> Vec<Value> {
    list.iter()
        .map(|t| {
            let mut v = serde_json::json!({"name": t.name, "description": t.description, "source": t.source});
            for (k, x) in [("title", serde_json::json!(t.title)), ("project", serde_json::json!(t.project)), ("profile", serde_json::json!(t.profile)), ("kind", serde_json::json!(t.kind)), ("worktree", serde_json::json!(t.worktree)), ("budget", serde_json::json!(t.budget)), ("error", serde_json::json!(t.error))] {
                if !x.is_null() {
                    v[k] = x;
                }
            }
            if !t.skills.is_empty() {
                v["skills"] = serde_json::json!(t.skills);
            }
            if t.fields.is_empty() {
                v["prompt"] = serde_json::json!(t.prompt);
            } else {
                v["fields"] = serde_json::json!(t.fields);
            }
            v
        })
        .collect()
}

/// The values a form gives, defaults filled and checked.
pub fn values(t: &Template, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    if let Some(k) = given.keys().find(|k| !t.fields.iter().any(|f| &f.name == *k)) {
        return Err(format!("the template {} has no field {k}", t.name));
    }
    let mut out = Map::new();
    for f in &t.fields {
        let v = given.get(&f.name).cloned().filter(|v| !v.is_null() && v != &Value::String(String::new())).or_else(|| f.default.clone());
        let v = match (f.kind.as_str(), v) {
            ("checkbox", v) => Value::String(if v.is_some_and(|v| v == true || v == "true" || v == "yes") { "yes" } else { "no" }.into()),
            (_, None) if f.required => return Err(format!("{} is needed", f.label.as_deref().unwrap_or(&f.name))),
            (_, None) => Value::String(String::new()),
            ("select", Some(v)) => {
                let s = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                if !f.options.contains(&s) {
                    return Err(format!("{}: one of {}", f.label.as_deref().unwrap_or(&f.name), f.options.join(", ")));
                }
                Value::String(s)
            }
            (_, Some(Value::String(s))) => Value::String(s),
            (_, Some(v)) => Value::String(v.to_string()),
        };
        out.insert(f.name.clone(), v);
    }
    Ok(out)
}

/// A task started from a template: what wasn't asked for comes from it
/// (the prompt and title rendered from the values; skills, limits, profile,
/// kind; the project it's pinned to).
pub fn apply(t: &Template, given: &Map<String, Value>, req: &mut StartTask) -> Result<(), String> {
    let ctx = Value::Object(values(t, given)?);
    if req.prompt.trim().is_empty() {
        req.prompt = crate::triggers::render(&t.prompt, &ctx).trim().to_string();
    }
    if req.title.trim().is_empty()
        && let Some(title) = &t.title
    {
        req.title = crate::triggers::render(title, &ctx).trim().to_string();
    }
    if req.project.is_empty()
        && let Some(p) = &t.project
    {
        req.project = p.clone();
    }
    if req.skills.is_empty() {
        req.skills = t.skills.clone();
    }
    if let Some(tb) = &t.budget {
        let b = req.budget.get_or_insert_with(Default::default);
        b.tokens = b.tokens.or(tb.tokens);
        b.cost = b.cost.or(tb.cost);
        b.minutes = b.minutes.or(tb.minutes);
    }
    req.profile = req.profile.take().or(t.profile.clone());
    req.kind = req.kind.take().or(t.kind.clone());
    req.worktree = req.worktree.take().or(t.worktree.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CI: &str = "---\ndescription: Fix a failing CI job\ntitle: \"Fix CI: {{job}}\"\nskills: [deploy]\nbudget: { cost: 2, minutes: 60 }\nfields:\n  - name: job\n    label: Job name\n    required: true\n  - name: branch\n    type: select\n    options: [trunk, staging]\n    default: trunk\n  - name: urgent\n    type: checkbox\n  - name: notes\n    type: multiline\n---\nThe CI job {{job}} fails on {{branch}} (urgent: {{urgent}}).\n{{notes}}";

    #[test]
    fn a_form_template_is_parsed_checked_and_applied() {
        let t = parse("ci", CI, "project");
        assert_eq!((t.error.as_deref(), t.fields.len(), t.description.as_str()), (None, 4, "Fix a failing CI job"));
        let m = |v: Value| v.as_object().unwrap().clone();
        assert_eq!(values(&t, &m(json!({}))).unwrap_err(), "Job name is needed");
        assert!(values(&t, &m(json!({"job": "x", "branch": "main"}))).unwrap_err().contains("one of trunk, staging"));
        assert!(values(&t, &m(json!({"job": "x", "nope": 1}))).unwrap_err().contains("no field nope"));
        let mut req = StartTask { project: "site".into(), budget: Some(Budget { cost: Some(5.0), ..Default::default() }), ..Default::default() };
        apply(&t, &m(json!({"job": "build", "urgent": true})), &mut req).unwrap();
        assert_eq!(req.title, "Fix CI: build");
        assert_eq!(req.prompt, "The CI job build fails on trunk (urgent: yes).");
        assert_eq!(req.skills, ["deploy"]);
        let b = req.budget.unwrap();
        assert_eq!((b.cost, b.minutes), (Some(5.0), Some(60)), "what was asked for wins, the rest from the template");
    }

    #[test]
    fn plain_and_broken_templates() {
        let t = parse("plain", "Review the open PRs.", "global");
        assert_eq!((t.error, t.fields.len(), t.description.as_str(), t.prompt.as_str()), (None, 0, "Review the open PRs.", "Review the open PRs."));
        assert!(parse("x", "---\nfields: [{name: a, type: radio}]\n---\nx", "db").error.unwrap().contains("no type radio"));
        assert!(parse("x", "---\nfields: [{name: a, type: select}]\n---\nx", "db").error.unwrap().contains("needs options"));
        assert!(parse("x", "---\nnope: 1\n---\nx", "db").error.unwrap().contains("frontmatter"));
        assert!(parse("x", "---\ntitle: t\n---\n", "db").error.unwrap().contains("no prompt"));
    }
}
