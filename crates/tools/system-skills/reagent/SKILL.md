---
name: reagent
description: What reagent is and how its parts fit together (projects, tasks, tools, policy, memory, skills, cron, triggers); start here, then the other reagent-* skills.
---

# reagent

reagent runs coding agents ("tasks") for a person, on a machine of theirs. A
task works on its own until it's done: it reads and edits files, runs
commands and terminals, uses git worktrees, asks the person when it must, and
reports. Tasks are resumable: they survive restarts of reagent, and long ones
are summarised as they go.

## The parts

- **Projects**: a folder (usually a git repository) with settings: where its
  memory and worktrees live, whether merges need approval, its policy, model
  profile, budgets, environment, nix dev shell, how many tasks may run at once.
- **Tasks**: one agent each, with a title and a prompt, in a project.
  See `reagent-tasks`. A task can start **subtasks** (`reagent-subtasks`).
- **Tools**: files, commands, terminals, git worktrees, memory, skills,
  subtasks, questions, todos, secrets, triggers, plus MCP servers the person
  adds (`reagent-tools`).
- **Policy**: per project, rules that allow, ask about or deny each tool
  call (`reagent-policy`).
- **Memory**: markdown notes, global and per project, that every task is
  shown an index of (`reagent-memory`).
- **Skills**: instructions for recurring jobs, in `.agents/skills/` folders,
  globally, and these system skills (`reagent-skills`).
- **Cron and triggers**: tasks started on a schedule, or when a script sees
  something happen (CI failing, say) (`reagent-cron-and-triggers`).
- **Secrets**: tokens and keys the person (or a task) keeps; commands get
  them as environment variables, results show them as `***`
  (`reagent-secrets`).
- **The designer**: a guide that turns a rough goal into clear work - tasks,
  cron entries, triggers, repo skills - by reading the project and asking
  (`reagent-prompt-design`).
- **Interfaces**: the web UI, the CLI, notifications, and the MCP API for
  other agents (`reagent-interfaces`).

## Under the hood

reagent runs on subagent-net (subnet): a hub keeps every agent's history as
events in SQLite next to reagent's own database, and a node runs the agents.
reagent writes subnet's cluster file from `reagent.hcl` (model providers,
profiles, kinds) at every start. A separate supervisor process owns commands
and terminals, so they keep running while reagent restarts.

Data lives in the data folder (`REAGENT_DATA`, by default
`~/.local/share/reagent`): `reagent.hcl`, `reagent.db` (projects, tasks,
rules, cron, triggers, secrets), `hub.db` (the agents), `memory/`,
`worktrees/`, `triggers/`, job logs, and these system skills
(`system-skills/`, written at every start).
