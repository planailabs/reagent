---
name: reagent-memory
description: reagent's memory - the global and project markdown memories, their INDEX.md, what to keep there, and checkpoints of long tasks.
---

# Memory

Two memories, the global one and each project's: folders of markdown with an
`INDEX.md` (one line per file) that every task is shown, and shown again when
it changes. They're for whoever works here next, tasks and people alike.

```
INDEX.md                 kept by the tools
topics/<name>.md         build.md, conventions.md, decisions.md, …
folders/<path>.md        notes about a folder of the project
tasks/<id>.md            a long task's checkpoints (written by reagent)
```

- `memory.memory_read(scope, file?)`: the index, or a file.
- `memory.memory_write(scope, file, text, about)`: make or replace a file
  (`about`: its line in the index).
- `memory.memory_edit(scope, file, …)`: change part of one.
- `memory.memory_search(pattern, scope?)`, `memory.memory_remove(scope, file)`.

`scope` is `global` or `project`. A project's memory lives in reagent's data
folder, or in `.reagent/memory/` in the project (a setting), to commit.

**Keep**: how to build and test, conventions, decisions and why, pitfalls,
where things are, what an outside system expects. **Don't keep** what the
code or git history already says, or secrets.

When a long task's history is summarised, the summary is also written to
`tasks/<id>.md` in the project's memory: someone can pick the work up from it.

The person edits both memories in the web UI.
