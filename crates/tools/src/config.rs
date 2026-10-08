//! `reagent.hcl`: where it listens, the model providers and profiles, and
//! notifications. Secrets stay in the environment (`key_env`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "listen")]
    pub listen: String,
    /// The profile a project without its own uses.
    #[serde(default = "default_profile")]
    pub default_profile: String,
    #[serde(default)]
    pub provider: BTreeMap<String, Provider>,
    #[serde(default)]
    pub profile: BTreeMap<String, Profile>,
    #[serde(default)]
    pub notify: Notify,
    /// Long tool results reach the model cut, with a note; subnet's
    /// grep_result searches or reads the whole (per profile: `grep_results`
    /// there wins). `over = 0` turns cutting off.
    #[serde(default)]
    pub grep_results: GrepResults,
    /// Tasks can search their whole history (subnet's search_history).
    #[serde(default = "yes")]
    pub search_history: bool,
    /// Kinds of task: which profile, and where to escalate.
    #[serde(default)]
    pub kind: BTreeMap<String, Kind>,
    /// The kind a task gets when nothing else picks one.
    #[serde(default)]
    pub default_kind: Option<String>,
    /// Kinds by where a task comes from.
    #[serde(default)]
    pub routing: Routing,
    /// The profile the prompt designer asks (default: `default_profile`).
    #[serde(default)]
    pub design_profile: Option<String>,
}

/// A kind of task (research, chore, …).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Kind {
    #[serde(default)]
    pub description: String,
    /// Its tasks' profile.
    #[serde(default)]
    pub profile: Option<String>,
    /// Where its tasks go when they escalate (themselves, the person, or after `escalate_after`).
    #[serde(default)]
    pub escalate: Option<String>,
    /// Escalate by itself after this many tool errors in a row.
    #[serde(default)]
    pub escalate_after: Option<u32>,
}

/// Kinds for tasks by origin (a task's own choice and the project's default come first).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Routing {
    #[serde(default)]
    pub subtask: Option<String>,
    #[serde(default)]
    pub cron: Option<String>,
    #[serde(default)]
    pub mcp: Option<String>,
}

fn yes() -> bool {
    true
}

/// When tool results are cut (characters), and which tools never are.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrepResults {
    #[serde(default = "grep_over")]
    pub over: usize,
    #[serde(default = "grep_except")]
    pub except: Vec<String>,
}

fn grep_over() -> usize {
    12_000
}

/// Paged tools: what they return is already a page.
fn grep_except() -> Vec<String> {
    vec!["skills.skill_load".into(), "fs.read".into(), "shell.job_output".into()]
}

impl Default for GrepResults {
    fn default() -> Self {
        GrepResults { over: grep_over(), except: grep_except() }
    }
}

fn listen() -> String {
    "127.0.0.1:8800".into()
}
fn default_profile() -> String {
    "default".into()
}

/// An OpenAI-compatible endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub base_url: String,
    /// The environment variable with its API key (none: no key).
    #[serde(default)]
    pub key_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub provider: String,
    pub model: String,
    /// Per million tokens, for budgets (in whatever unit: dollars, say).
    #[serde(default)]
    pub price: Price,
    /// The model's context window: history is compacted at 3/4 of it.
    #[serde(default = "context")]
    pub context: u64,
    #[serde(default)]
    pub params: BTreeMap<String, serde_json::Value>,
    /// This profile's cut-off for tool results (else the global one).
    #[serde(default)]
    pub grep_results: Option<GrepResults>,
    /// Where a task goes when a model call fails here (an outage, rate limits).
    #[serde(default)]
    pub fallback: Option<String>,
}

fn context() -> u64 {
    128_000
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Price {
    #[serde(default)]
    pub input: f64,
    #[serde(default)]
    pub output: f64,
    /// Input the provider served from its cache (DeepSeek: about a tenth of
    /// `input`); none: counted at the input price (never under-counts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached: Option<f64>,
}

impl Price {
    /// What `input` prompt tokens (of which `cached` came from the cache)
    /// and `output` completion tokens cost.
    pub fn cost(&self, input: u64, cached: u64, output: u64) -> f64 {
        let cached = cached.min(input);
        ((input - cached) as f64 * self.input + cached as f64 * self.cached.unwrap_or(self.input) + output as f64 * self.output) / 1_000_000.0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notify {
    /// apprise URLs (`tgram://…`, `mailto://…`, `ntfys://…`).
    #[serde(default)]
    pub apprise: Vec<String>,
    /// Or an environment variable with them (space separated): they're often secret.
    #[serde(default)]
    pub apprise_env: Option<String>,
    /// Which events go out (default all): done, failed, waiting, budget, cron, model, trigger.
    #[serde(default)]
    pub events: Option<Vec<String>>,
    /// The web UI's public URL, for links in notifications.
    #[serde(default)]
    pub url: Option<String>,
}

impl Notify {
    pub fn wants(&self, kind: &str) -> bool {
        self.events.as_ref().is_none_or(|e| e.iter().any(|k| k == kind))
    }

    pub fn apprise_urls(&self) -> Vec<String> {
        let mut urls = self.apprise.clone();
        if let Some(v) = self.apprise_env.as_ref().and_then(|e| std::env::var(e).ok()) {
            urls.extend(v.split_whitespace().map(String::from));
        }
        urls
    }
}

/// What a fresh data dir gets.
pub const EXAMPLE: &str = r#"# reagent's settings. Secrets come from the environment (or <data>/.env).
listen = "127.0.0.1:8800"
default_profile = "default"

provider "deepseek" {
  base_url = "https://api.deepseek.com/v1"
  key_env  = "DEEPSEEK_API_KEY"
}

profile "default" {
  provider = "deepseek"
  model    = "deepseek-chat"
  price    = { input = 0.27, cached = 0.07, output = 1.10 }   # per million tokens (cached: input from the provider's cache), for budgets
  context  = 1000000                           # compaction at three quarters
}

# Tool results longer than `over` characters reach the model cut, with a note;
# grep_result searches or reads the whole. 0 turns it off. A profile may set its own.
grep_results = { over = 12000, except = ["skills.skill_load", "fs.read", "shell.job_output"] }
search_history = true
# design_profile = "default"                   # the model the prompt designer asks

notify {
  apprise = []                                 # apprise URLs: tgram://…, ntfys://…, mailto://…
  # apprise_env = "REAGENT_APPRISE"
  # url = "https://reagent.example.org"        # for links in notifications
}
"#;

impl Config {
    pub fn parse(text: &str) -> anyhow::Result<Config> {
        let c: Config = hcl::from_str(text)?;
        c.check()?;
        Ok(c)
    }

    pub fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.profile.is_empty(), "reagent.hcl has no profile");
        anyhow::ensure!(self.profile.contains_key(&self.default_profile), "default_profile {:?} isn't a profile", self.default_profile);
        self.check_kinds()?;
        if let Some(d) = &self.design_profile {
            anyhow::ensure!(self.profile.contains_key(d), "design_profile {d:?} isn't a profile");
        }
        for g in std::iter::once(&self.grep_results).chain(self.profile.values().filter_map(|p| p.grep_results.as_ref())) {
            anyhow::ensure!(g.over == 0 || g.over >= 500, "grep_results.over: 0 (off) or at least 500 characters");
        }
        for (name, p) in &self.profile {
            if let Some(f) = &p.fallback {
                anyhow::ensure!(self.profile.contains_key(f) && f != name, "profile {name:?}: fallback {f:?} isn't another profile");
            }
            anyhow::ensure!(self.provider.contains_key(&p.provider), "profile {name:?}: no provider {:?}", p.provider);
            anyhow::ensure!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'), "profile {name:?}: letters, digits, - and _ only");
        }
        Ok(())
    }

    /// Checks the kinds and routing (after the profiles).
    fn check_kinds(&self) -> anyhow::Result<()> {
        for (name, k) in &self.kind {
            for p in [&k.profile, &k.escalate].into_iter().flatten() {
                anyhow::ensure!(self.profile.contains_key(p), "kind {name:?}: no profile {p:?}");
            }
        }
        for k in [&self.default_kind, &self.routing.subtask, &self.routing.cron, &self.routing.mcp].into_iter().flatten() {
            anyhow::ensure!(self.kind.contains_key(k), "no kind {k:?} (routing or default_kind)");
        }
        Ok(())
    }

    pub fn kind(&self, name: &str) -> Result<&Kind, String> {
        self.kind.get(name).ok_or_else(|| format!("no kind {name:?} (there are: {})", self.kind.keys().cloned().collect::<Vec<_>>().join(", ")))
    }

    /// A task's kind: its own, else by origin, else the project's, else the default.
    pub fn kind_for(&self, asked: Option<&str>, origin: &str, project_kind: Option<&str>) -> Result<Option<String>, String> {
        if let Some(k) = asked {
            self.kind(k)?;
            return Ok(Some(k.to_string()));
        }
        let by_origin = match origin.split(':').next() {
            Some("task") => self.routing.subtask.as_deref(),
            Some("cron") => self.routing.cron.as_deref(),
            Some("mcp") => self.routing.mcp.as_deref(),
            _ => None,
        };
        Ok(by_origin.or(project_kind).or(self.default_kind.as_deref()).map(String::from))
    }

    pub fn profile(&self, name: Option<&str>) -> Result<(&str, &Profile), String> {
        let name = name.unwrap_or(&self.default_profile);
        self.profile.get_key_value(name).map(|(k, v)| (k.as_str(), v)).ok_or_else(|| format!("no profile {name:?} (there are: {})", self.profile.keys().cloned().collect::<Vec<_>>().join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_parses() {
        let c = Config::parse(EXAMPLE).unwrap();
        assert_eq!(c.listen, "127.0.0.1:8800");
        let (name, p) = c.profile(None).unwrap();
        assert_eq!((name, p.model.as_str(), p.context), ("default", "deepseek-chat", 1_000_000));
        assert!((p.price.cost(1_000_000, 0, 1_000_000) - 1.37).abs() < 1e-9);
        // Cache hits at the cached price; without one, at the input price.
        assert!((p.price.cost(1_000_000, 900_000, 0) - (0.1 * 0.27 + 0.9 * 0.07)).abs() < 1e-9);
        let no_cache_price = Price { cached: None, ..p.price };
        assert!((no_cache_price.cost(1_000_000, 900_000, 0) - 0.27).abs() < 1e-9);
        assert!((p.price.cost(10, 99, 0) - 10.0 * 0.07 / 1e6).abs() < 1e-12, "cached never exceeds input");
        assert!(c.notify.wants("done"));
        assert_eq!((c.grep_results.over, c.search_history), (12000, true));
        let c = Config::parse(&format!("{EXAMPLE}\ngrep_results = {{ over = 100 }}\n").replace("grep_results = { over = 12000, except = [\"skills.skill_load\", \"fs.read\", \"shell.job_output\"] }\n", ""));
        assert!(c.unwrap_err().to_string().contains("at least 500"));
    }

    #[test]
    fn kinds_route_tasks() {
        let text = format!("{EXAMPLE}\nprofile \"big\" {{\n  provider = \"deepseek\"\n  model = \"deepseek-reasoner\"\n}}\nkind \"research\" {{\n  profile = \"big\"\n}}\nkind \"chore\" {{\n  profile = \"default\"\n  escalate = \"big\"\n  escalate_after = 3\n}}\ndefault_kind = \"chore\"\nrouting {{\n  cron = \"research\"\n}}\n");
        let c = Config::parse(&text).unwrap();
        assert_eq!(c.kind_for(Some("research"), "ui", None).unwrap().as_deref(), Some("research"), "asked for");
        assert_eq!(c.kind_for(None, "cron:3", Some("chore")).unwrap().as_deref(), Some("research"), "by origin");
        assert_eq!(c.kind_for(None, "ui", Some("research")).unwrap().as_deref(), Some("research"), "the project's");
        assert_eq!(c.kind_for(None, "task:x", None).unwrap().as_deref(), Some("chore"), "the default");
        assert!(c.kind_for(Some("nope"), "ui", None).is_err());
        assert_eq!(c.kind("chore").unwrap().escalate_after, Some(3));
        assert!(Config::parse(&format!("{EXAMPLE}\nkind \"x\" {{\n  profile = \"nope\"\n}}\n")).unwrap_err().to_string().contains("no profile"));
        assert!(Config::parse(&format!("{EXAMPLE}\nrouting {{\n  mcp = \"nope\"\n}}\n")).unwrap_err().to_string().contains("no kind"));
        let fb = EXAMPLE.replace("  context  = 1000000", "  context  = 1000000\n  fallback = \"default\"");
        assert!(Config::parse(&fb).unwrap_err().to_string().contains("isn't another profile"));
    }

    #[test]
    fn mistakes_are_named() {
        let e = Config::parse("profile \"x\" {\n  provider = \"nope\"\n  model = \"m\"\n}\ndefault_profile = \"x\"\n").unwrap_err();
        assert!(e.to_string().contains("no provider"), "{e}");
        assert!(Config::parse("listen = 1\nbogus = 2\n").is_err());
        let c = Config::parse(&format!("{EXAMPLE}\n")).unwrap();
        assert!(c.profile(Some("big")).unwrap_err().contains("there are: default"));
    }
}
