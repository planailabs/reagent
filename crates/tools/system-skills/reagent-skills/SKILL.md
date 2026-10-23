---
name: reagent-skills
description: Skills in reagent - where they're found (worktree, project, global, system), their SKILL.md format, loading them, and starting tasks with them.
---

# Skills

A skill is a folder with a `SKILL.md`: YAML frontmatter (`name`,
`description`) and instructions, plus any files it refers to (scripts,
templates, references).

```
---
name: deploy
description: Deploy the site to production - build, upload, check.
---

# Deploy
1. …
```

**Where they're found** (the first place with a name wins):

1. **system**: reagent's own documentation (these `reagent-*` skills), built
   into reagent and the same in every project.
2. the task's **worktree** and then the **project** folder:
   `.agents/skills/<name>/`, `.agent/skills/<name>/`, `.claude/skills/<name>/`,
   `.codex/skills/<name>/`.
3. **global**, for every project: `~/.agents/skills/`, Claude Code's
   `~/.claude/skills/`, Codex's `~/.codex/skills/` (or `$CODEX_HOME/skills/`),
   and `<data>/skills/`.

Every task is shown the list (name and description). `skills.skill_load(name)`
gives a skill's instructions and its files' paths; `skills.skill_list()` the
list. A task, a cron entry or a trigger can start with skills already loaded
into its first message. The project's skills tab lists them with where they're
from; the web UI's docs page shows the system ones.

A good skill is specific and ordered (steps that can be followed), says how to
check the result, and names its files by relative path.

## Rules

Rules are standing instructions, not procedures: markdown files in
`.agents/rules/` (or `.agent/rules/`), `*.md` or Cursor's `*.mdc`, subfolders
too, in the worktree and the project folder (the worktree's wins on a name).
Optional frontmatter says when one applies:

- no frontmatter, or `alwaysApply: true`: always in the task's context;
- `globs` (`src/**/*.rs, *.toml`, or a list; Claude Code's `paths` too): shown once the task's file tools
  touch a matching file (paths relative to the project);
- only a `description`: listed with its path, for the task to read when it fits.

Rules also come from the global folders `~/.agents/rules/` and
`<data>/rules/`, and from plugins; a project's win on a name.

`AGENTS.md` is shown too: the global ones (`~/.agents/AGENTS.md`,
`<data>/AGENTS.md`), the project's (from the working directory, else the
project folder), and a subfolder's once the task touched a path under it
(nearer wins where they differ): put a part's conventions next to it.

Use a rule for "how we do things here" (style, conventions, don'ts) and a
skill for a job's steps.

## Plugins

A plugin is a git repository with skills (`skills/`, `skill/`, `.agents/skills/`)
and rules (`rules/`, `.agents/rules/`) that every task gets, after the global
ones (a skill's source shows `plugin:<id>`). It's cloned into
`<data>/plugins/<id>/` and updated by fetching what it follows (a branch or tag,
else the default branch); changes made in its folder are dropped then. The
person adds, updates and removes plugins on the settings page or with `reagent
plugin add <url> [--id] [--ref] | list | update [ids…] | rules-off <id> [rules…] |
remove <id>`; outside agents with the MCP API's `plugin_*` tools. A plugin's
rules that don't fit can be switched off one by one (kept across updates).
A plugin may be a marketplace (Claude Code's `.claude-plugin/marketplace.json`
or the generic `.agents/plugins/marketplace.json`, like trailofbits/skills):
its plugins in local folders are its parts, their skills' source
`plugin:<id>/<part>`; one from elsewhere is listed with why it isn't used (add
it as its own plugin).

## Prompt templates

Prompts to start tasks from: `.agents/templates/<name>.md` in the project (or
kept in reagent: a project's templates tab, the settings; or global:
`~/.agents/templates/`, `<data>/templates/`, plugins' `templates/`).
Frontmatter: `description`, `title`, `skills`, `budget`, `profile`, `kind`,
`project` (pins a global one), and `fields` (`name`, `label`, `type`: text,
multiline, select with `options`, checkbox; `default`, `required`). The body
is the prompt with `{{field}}` placeholders.

With fields a template is a form: the person fills it on the quick prompt or a
project's new task tab, an agent passes `values` to `tasks.task_spawn`
(`tasks.template_list` shows them). Without fields its body is a prompt to edit.
What isn't given comes from the template.

```markdown
---
description: Fix a failing CI job
title: "Fix CI: {{job}}"
budget: { cost: 2, minutes: 60 }
fields:
  - name: job
    label: Job name
    required: true
  - name: branch
    type: select
    options: [main, staging]
    default: main
---
The CI job {{job}} fails on {{branch}}. Find out why and fix it.
```
