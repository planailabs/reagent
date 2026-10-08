---
name: reagent-tasks
description: How a reagent task lives - states, the queue, steering (messages, pause, retry, clone), budgets, models and escalation, todos, reports, restarts.
---

# Tasks

A task is one agent working on one job in a project. It starts with a first
message: its title, the project, its working directory, the prompt, and the
skills it was started with.

## States

`queued` (waiting for a place) → `running` ⇄ `waiting` (for an approval, an
answer, a merge, or over its budget) ⇄ `paused` → `done`, `failed` or
`cancelled`.

**Tasks at once.** The person can limit how many tasks run at once, for all
of reagent (settings) and per project. A task started over a limit waits as
`queued` and starts when a place frees, oldest first; the person can start
it at once ("start now"). Subtasks never wait (their parent waits for them).

## Steering (by the person, or another agent through the MCP API)

- **message**: read before the task's next model call; a finished task goes
  on with it.
- **pause** (quick: running calls finish; or after this turn), **resume**,
  **cancel** (its commands and terminals end too).
- **approve / deny** a call the policy asks about (once, or always: a rule).
- **answer** a question; **merge** a worktree or send it back.
- **retry** a failed task where it failed; **clone and restart** one that's
  over (a new task, same prompt and settings).
- **switch model**: the task goes on on another profile with its whole
  conversation.

## Budgets

Per task: tokens, cost (from the profile's prices; cached input at its own
price) and minutes (from when it started). Over any: it's paused before its
next model call and the person is told; raising the budget lets it go on.
A project can also cap its cost per day (no new tasks past it).

## Models, kinds, escalation

`reagent.hcl` defines profiles (a provider and a model, prices, context) and
kinds of task (`research`, say: a profile, where to escalate, after how many
tool errors in a row). A task's profile is the one asked for, else its
kind's, else the project's, else the default. A task can move itself to a
stronger model (`tasks.task_escalate`); a profile's `fallback` takes over
when a model call fails.

## For the agent

- Plan work of more than a few steps as a todo list (`todo.todo_add`, then
  `todo.todo_update` as you go): it survives summaries and restarts, and the
  person follows it.
- Long histories are summarised at three quarters of the context; the
  summary is also kept as a checkpoint in the project's memory.
  `search_history` searches your whole conversation, `grep_result` a long
  tool result that reached you cut.
- When you're done, answer with a short report: what you did, what you
  checked, what's left, where things are. It's what the person (or your
  parent task) reads.
- reagent restarts don't lose you: a question or a merge you wait on is
  waited on again.
