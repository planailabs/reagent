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
- `globs` (`src/**/*.rs, *.toml`, or a list): shown once the task's file tools
  touch a matching file (paths relative to the project);
- only a `description`: listed with its path, for the task to read when it fits.

Use a rule for "how we do things here" (style, conventions, don'ts) and a
skill for a job's steps.
