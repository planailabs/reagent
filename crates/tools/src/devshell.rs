//! Commands in a project's nix dev shell: `nix develop <flake>[#attr]
//! --command …` per command (exact, no cached environment to go stale).

use std::path::Path;

use reagent_store::Project;

/// Quotes a word for `sh`.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The flake whose dev shell a command in `cwd` runs in: the working
/// directory's (a worktree has the branch's), else the project's; none
/// when it's off, or auto and there's no flake.nix.
pub fn flake_for(p: &Project, cwd: &Path) -> Option<String> {
    let dir = [cwd, Path::new(&p.path)].into_iter().find(|d| d.join("flake.nix").is_file());
    let dir = match p.devshell.as_str() {
        "off" => return None,
        "on" => dir.unwrap_or(Path::new(&p.path)),
        _ => dir?,
    };
    Some(match p.devshell_attr.as_deref().map(|a| a.trim().trim_start_matches(".#").trim_start_matches('#')).filter(|a| !a.is_empty()) {
        Some(attr) => format!("{}#{attr}", dir.display()),
        None => dir.display().to_string(),
    })
}

/// A command line run in the dev shell (or as it is: `use_it` false, or no shell).
pub fn wrap(p: &Project, cwd: &Path, cmd: &str, use_it: bool) -> String {
    match flake_for(p, cwd).filter(|_| use_it) {
        Some(flake) => format!("nix develop {} --command sh -c {}", sh_quote(&flake), sh_quote(cmd)),
        None => cmd.to_string(),
    }
}

/// A terminal's command: the dev shell's (interactive) shell, or a command in it.
pub fn wrap_terminal(p: &Project, cwd: &Path, cmd: Option<&str>, use_it: bool) -> Option<String> {
    match (flake_for(p, cwd).filter(|_| use_it), cmd) {
        (Some(flake), Some(c)) => Some(format!("nix develop {} --command sh -c {}", sh_quote(&flake), sh_quote(c))),
        (Some(flake), None) => Some(format!("nix develop {}", sh_quote(&flake))),
        (None, c) => c.map(String::from),
    }
}

pub fn check(devshell: &str) -> Result<(), String> {
    if ["off", "auto", "on"].contains(&devshell) { Ok(()) } else { Err("devshell: off, auto or on".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_go_into_the_dev_shell_when_it_applies() {
        let d = tempfile::tempdir().unwrap();
        let (proj, wt) = (d.path().join("proj"), d.path().join("wt"));
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        let mut p = Project::new("x", "X", &proj.display().to_string());
        assert_eq!(wrap(&p, &proj, "make", true), "make", "auto without a flake: as it is");
        std::fs::write(proj.join("flake.nix"), "{}").unwrap();
        assert_eq!(wrap(&p, &proj, "echo 'hi'", true), format!("nix develop '{}' --command sh -c 'echo '\\''hi'\\'''", proj.display()));
        assert_eq!(wrap(&p, &proj, "make", false), "make", "opted out");
        // A worktree with its own flake uses that one.
        std::fs::write(wt.join("flake.nix"), "{}").unwrap();
        assert!(wrap(&p, &wt, "make", true).contains(&format!("'{}'", wt.display())));
        p.devshell_attr = Some(".#ci".into());
        assert!(wrap(&p, &proj, "make", true).contains(&format!("'{}#ci'", proj.display())));
        p.devshell = "off".into();
        assert_eq!(wrap(&p, &proj, "make", true), "make");
        assert_eq!(wrap_terminal(&p, &proj, None, true), None, "off: the user's shell");
        p.devshell = "on".into();
        assert!(wrap_terminal(&p, &proj, None, true).unwrap().starts_with("nix develop "));
        assert!(check("sometimes").is_err());
        // The quoting holds against sh.
        let out = std::process::Command::new("sh").arg("-c").arg(format!("printf %s {}", sh_quote("it's \"$HOME\" `x`"))).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "it's \"$HOME\" `x`");
    }
}
