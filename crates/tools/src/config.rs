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
}

impl Price {
    pub fn cost(&self, input: u64, output: u64) -> f64 {
        (input as f64 * self.input + output as f64 * self.output) / 1_000_000.0
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
    /// Which events go out (default all): done, failed, waiting, budget, cron.
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
  price    = { input = 0.27, output = 1.10 }   # per million tokens, for budgets
  context  = 128000
}

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
        for (name, p) in &self.profile {
            anyhow::ensure!(self.provider.contains_key(&p.provider), "profile {name:?}: no provider {:?}", p.provider);
            anyhow::ensure!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'), "profile {name:?}: letters, digits, - and _ only");
        }
        Ok(())
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
        assert_eq!((name, p.model.as_str(), p.context), ("default", "deepseek-chat", 128000));
        assert!((p.price.cost(1_000_000, 1_000_000) - 1.37).abs() < 1e-9);
        assert!(c.notify.wants("done"));
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
