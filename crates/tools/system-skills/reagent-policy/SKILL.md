---
name: reagent-policy
description: reagent's per-project policy - rules (tool, command, target, action), how command lines are judged piece by piece, approvals, the starter rules, and what a denial means for an agent.
---

# Policy

Every tool call a task makes is judged by its project's policy before it
runs: an ordered list of rules; the first that fits decides; none fits: the
project's default action (`ask` unless the person changed it).

A rule has:

- `tool`: a glob over `<server>.<tool>` (`shell.exec*`, `fs.*`,
  `secrets.secrets_{list,get}`),
- `command` (optional): a glob over the command line, for `shell.exec`,
  `shell.exec_bg`, `shell.job_input`, `pty.pty_open`, `pty.pty_send` and
  `triggers.run`,
- `target` (optional): what a call is aimed at - the project of
  `tasks.task_spawn`, the path of a file tool (paths outside the task's
  places need an allowing rule with a target),
- `action`: `allow`, `ask` or `deny`.

**Command lines are judged piece by piece**: split at `&&`, `||`, `;`, `|`,
`&`, newlines, and `$(…)` and backticks count as commands; the strictest
piece decides. A rule for `cargo *` doesn't allow `cargo test && rm -rf ~`.

**Ask** parks the call until the person allows it once, always (a rule is
added in front), or denies it; they're notified (with buttons). **Deny**
returns an error to the task that names the rule.

**Starter rules** (a new project): files, memory, skills, questions, todos,
working memory and reading secrets are allowed; git worktree tools and read-only git, job
and terminal tools too; subtasks in the project itself; common read-only
commands and `git add/commit`, `cargo`, `npm test`, `npm run`. `git push`
and `sudo` ask; `rm -rf /*` is denied. Everything else gets the default.

## For the agent

- A denied call isn't an error to work around: find another way that the
  policy allows, or ask the person (`ask.ask`) why it's needed.
- Prefer one clear command over a long chain: each piece is judged.
- Triggers' scripts are judged as `triggers.run` with their command
  (`reagent-cron-and-triggers`).
