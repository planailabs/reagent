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
   `.agents/skills/<name>/`, `.agent/skills/<name>/`, `.claude/skills/<name>/`.
3. **global**: `~/.agents/skills/<name>/` and `<data>/skills/<name>/`, for
   every project.

Every task is shown the list (name and description). `skills.skill_load(name)`
gives a skill's instructions and its files' paths; `skills.skill_list()` the
list. A task, a cron entry or a trigger can start with skills already loaded
into its first message. The project's skills tab lists them with where they're
from; the web UI's docs page shows the system ones.

A good skill is specific and ordered (steps that can be followed), says how to
check the result, and names its files by relative path.
