---
name: reagent-prompt-design
description: How to write a task prompt an agent can act on alone, and how reagent's prompt designer guides there - asking questions, suggesting skills, cron entries and triggers (web UI, CLI, MCP API, tasks.prompt_design).
---

# Prompt design

A task starts from nothing but its prompt (and the project's AGENTS.md,
memory index and skills, which it's shown). A good prompt lets it work
without asking.

## What a clear prompt has

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

## The prompt designer

reagent's designer turns a rough goal into such a prompt by asking
questions, a few at a time, each with likely answers to pick from. When it
knows enough (or is told to propose), it proposes a title, the prompt, and
fitting skills, kind, profile and budget, for the person to edit and start,
and suggests what else to keep: a repo skill, a cron entry, a trigger. The
person ticks the suggestions they want and they're made.

- **Web UI**: "design" in the new-task form (and the cron and trigger
  forms). Quick: a chat with a model, no tools. "Look at the project first":
  the designer runs as a read-only task that reads the project, asks through
  the usual questions, and reports a proposal ("use this proposal").
- **Agents**: `tasks.prompt_design(goal, answers?, propose?)` returns
  questions or a proposal; answer them yourself and call again with the
  answers, then `tasks.task_spawn` with the result.
- **Other agents**: the MCP API's `prompt_design`.
- **CLI**: `reagent task design --project <id> "<goal>"` asks in the
  terminal and prints the proposal.

## As the designer

Be a guide more than a prompt writer: help the person work out what they
actually want done and the best way for reagent to do it - once now, as a
skill to repeat, on a schedule, when something happens; with which skills,
model and budget. Point out what they may not have thought of (risks, the
checks that show it's done, what must not change). The prompt is one of the
things you hand over, not the whole of it.

When you design (as the quick designer or as a design task):

- Ask only what changes the prompt: unclear scope, success criteria, places,
  constraints, risky actions (push, delete, deploy). Don't ask what the
  project shows.
- At most 3-4 questions a round, each with 2-4 likely answers; stop asking
  once the prompt would not get better.
- Propose: a short title (a few words, imperative), the prompt (sections as
  above, plain and specific), skills that fit (by name), a kind or profile
  if one fits better than the default, and a budget when the job is
  open-ended.
- Decide what belongs where, and suggest it (the person takes or leaves each):
  - a **repo skill** (`.agents/skills/<name>/SKILL.md`) for a procedure that
    will come up again (how to release, how to fix this kind of failure):
    the steps go there and the prompt names the skill;
  - a **cron entry** when the work recurs on a schedule (weekly updates, a
    nightly check);
  - a **trigger** when the work should follow an event (CI failing, an issue
    opened, a webhook): its script finds the event and prints it, its
    templates make each task's prompt (`reagent-cron-and-triggers`); in the
    repo unless it's private to this machine.
  The task itself is what's to be done now; when the goal is only to set up
  something recurring, its prompt can be a first run, or say so in the note.
  Ask when it's unclear whether the person wants it once or for good.
