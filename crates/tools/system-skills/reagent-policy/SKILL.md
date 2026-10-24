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

**The evaluator.** With `evaluator_profile` in reagent.hcl, a call the
rules would ask about goes to a second model first. It reads the task, its
recent conversation, the call and its `_reason` (every tool takes an
optional `_reason`: say why you make the call). Per project it **decides**
(the default: what it approves runs, what it rejects is refused with its
reasoning, or goes to the person) or **advises** (the person decides, with
its verdict). If it can't judge, the person decides. The person adds to its
instructions for every project (the settings) and per project. Static
checks catch, in calls that change something, a loop (the same call three
times in a row), a duplicate (the same non-shell call just made) and a
destructive call before anything was read: refused, or warned about, as
the project says. The task page lists every judgement.

**Starter rules** (a new project): files, memory, skills, questions, todos,
working memory and reading secrets are allowed; git worktree tools and read-only git, job
and terminal tools too; subtasks in the project itself; common read-only
commands and `git add/commit`, `cargo`, `npm test`, `npm run`. `git push`
and `sudo` ask; `rm -rf /*` is denied. Everything else gets the default.

## For the agent

- Give a `_reason` with calls that change something: one sentence on what
  you expect. The evaluator judges by it.
- `[evaluator refused] …` means the evaluator (or a static check) stopped
  the call: read why, and change course rather than retrying it as it was.
- A denied call isn't an error to work around: find another way that the
  policy allows, or ask the person (`ask.ask`) why it's needed.
- Prefer one clear command over a long chain: each piece is judged.
- Triggers' scripts are judged as `triggers.run` with their command
  (`reagent-cron-and-triggers`).
