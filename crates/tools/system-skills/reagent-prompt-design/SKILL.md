---
name: reagent-prompt-design
description: How reagent's designer guides a rough goal to clear work - a design task that reads the project, asks the person, and proposes tasks, cron entries, triggers and repo skills to pick from - and what makes a task prompt clear.
---

# The designer

reagent's designer is a guide more than a prompt writer. From a rough goal it
works out with the person what they actually want done and the best way for
reagent to do it - once now, on a schedule, when something happens, as a
procedure to repeat - and proposes it. It points out what the person may not
have thought of: risks, the checks that show it's done, what must not change.

## How it runs

The designer is a **design task**: a task that may only read (the code, the
memory, the skills, other tasks' histories) and ask. It reads first, then asks
the person what changes the result and the project doesn't show - one
question at a time, each with likely answers (the person may answer in their
own words, say "you decide", or tell it to propose now). It ends with a short
note and a **proposal**: a list of items of any kind, in any number:

- **task**: work to start now (a title, a prompt, skills, kind, profile,
  budget);
- **cron**: work that recurs on a schedule (`reagent-cron-and-triggers`);
- **trigger**: work that should follow an event (CI failing, an issue, a
  webhook), with the script that finds it; in the repo unless it's private to
  this machine;
- **skill**: a procedure that will come up again, as a repo skill
  (`.agents/skills/<name>/SKILL.md`) that the prompts then name.

The person edits the items, ticks the ones they want, and creates them.

- **Web UI**: "design it with the guide" in the new-task, cron and trigger
  forms starts a design task; its page shows the questions as it asks, a
  "propose now" button, and, once it's done, the proposal to edit, tick and
  create (a task can also be opened in the new-task form).
- **Agents**: `tasks.prompt_design(goal)` starts a design task as your
  subtask; the person answers its questions; its proposal comes to you as a
  message. Start what fits (`tasks.task_spawn`, `tasks.cron_add`,
  `triggers.trigger_add`, a skill with `fs.write`).
- **Other agents**: the MCP API's `prompt_design`, `design_proposal`,
  `design_create`.
- **CLI**: `reagent task design --project <id> "<goal>"` queues one;
  `reagent task proposal <id> [--make all|1,3]` shows or makes its items.

## What a clear task prompt has

A task starts from nothing but its prompt (and the project's AGENTS.md,
memory index and skills, which it's shown).

1. **The goal** in one or two sentences, and **why** (what it's for).
2. **Where**: the files, folders, branch, services or URLs involved.
3. **What done means**: the checks that must pass (tests, a build, a page
   that renders), what must not change.
4. **Constraints**: style, dependencies allowed, things to avoid, whether to
   push or merge, budget.
5. **What to report**: the summary the person (or parent task) needs.
6. **Context it can't find itself**: decisions already made, earlier
   attempts, links, credentials it should use (by secret name).

Leave out what the agent can see for itself (the code, AGENTS.md).

## As the designer

- Read before you ask; ask only what changes the result: scope, success
  criteria, places, constraints, risky actions (push, delete, deploy), once
  or for good. About ten questions at most; propose when told to.
- Decide what belongs where: a one-off is a task; recurring work a cron
  entry; work that follows an event a trigger (its script prints one JSON
  line per event; its templates use `{{key}}`, `{{vars.x}}`); a procedure
  that will come up again a skill, which the prompts name instead of
  repeating it. A cron entry's prompt runs again and again: it must make
  sense every time.
- Prompts as above, plain and specific; a title of a few words; skills,
  kinds and profiles only by names that exist; a budget when the job is
  open-ended; `why` in a line for each item.
