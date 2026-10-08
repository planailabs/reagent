---
name: reagent-subtasks
description: When and how a task splits work into subtasks - task_spawn, designing their prompts, waiting for and reading their reports, messaging them, searching other tasks.
---

# Subtasks

A task can start other tasks (`tasks.task_spawn`): subtasks. Each is a full
agent of its own, with its own conversation, budget and model, in this
project or another one the policy allows. Subtasks never wait in the queue.

## When to split

- Parts that can run **in parallel** (tests of several packages, research of
  several options, changes in independent areas).
- A part that needs a **different model or kind** (a cheap model for a
  mechanical sweep, a strong one for a hard bug).
- A part whose detail would **flood your context** (reading a big log,
  surveying a large codebase): the subtask reads it and reports the gist.
- Not for a quick step you can do yourself: a subtask starts from nothing and
  must be told everything.

## How

1. **Design the prompt.** A subtask knows only what you write. Use
   `tasks.prompt_design(goal)` for anything non-trivial: it asks you
   questions (answer them from what you know) and returns a clear title and
   prompt. See `reagent-prompt-design`.
2. **Start it**: `tasks.task_spawn(title, prompt, project?, profile?, kind?,
   skills?, budget?)`. Give the paths, branch, constraints, what "done"
   means, and what to put in the report.
3. **Go on** with your own work; its report comes to you as a message
   (`[subtask … done]`) and wakes you. `tasks.task_wait(task)` waits for
   one, `tasks.task_list` shows them, `tasks.task_message` tells one
   something new.
4. **Check** what it did before relying on it (its diff, the tests).

## Other tasks' work

`tasks.search_history(pattern, task?, project?)` searches other tasks'
whole conversations: how a problem was solved before, what was decided.

## Worktrees

Subtasks that edit the same project at once should each use their own
worktree (`git.worktree_start`), merged back one by one
(`reagent-worktrees`).
