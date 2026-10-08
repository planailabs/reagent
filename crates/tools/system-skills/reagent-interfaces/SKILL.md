---
name: reagent-interfaces
description: How people and other agents use reagent - the web UI (inbox, projects, tasks, settings, docs), notifications and their buttons, the CLI, and the MCP API for outside agents.
---

# Interfaces

## Web UI

- **Inbox**: what waits for you (approvals, questions, merges, budgets,
  trigger scripts to allow), failed tasks, tasks going on, notifications.
- **Projects**: each with tabs: tasks (a tree with subtasks), new task (with
  the prompt designer), cron, triggers, policy, memory, skills, MCP servers,
  secrets, settings.
- **Task**: state, cost, todo list, what it waits for, the report, a message
  box, the transcript (live, markdown rendered), jobs (logs), terminals, the
  worktree's diff; pause, resume, start now, retry, clone and restart,
  cancel, switch model.
- **Settings**: notifications on this browser, tasks at once, model
  profiles, every project's secrets, API tokens, global memory.
- **Docs**: these system skills.

## Notifications

When a task is done, fails, waits for you, goes over budget, changes model,
or a trigger needs you: in the inbox, as browser push notifications (with
buttons: allow once, deny, an answer, merge) and through apprise (Slack,
email, ntfy, … as `reagent.hcl` says).

## CLI

`reagent up` runs it. Besides: `passwd`, `status`, `mcp add|list|remove`,
`secret set|list|get|remove`, `trigger add|list|remove|enable|disable|run|move|approve`,
`task design`, `token add|list|revoke`, `supervisor --stop`.

## MCP API (for other agents)

`<reagent's URL>/mcp`, streamable HTTP, with `Authorization: Bearer <token>`
(`reagent token add <name>`, or the settings page). Tools: `projects`,
`task_start`, `task_list`, `task_get`, `task_transcript`, `task_message`,
`task_pause`, `task_resume`, `task_cancel`, `task_retry`, `task_raise_budget`,
`task_switch_profile`, `task_approve`, `task_answer`, `task_merge`,
`task_wait`, `search`, `prompt_design`, `docs_list`, `docs_read`,
`trigger_list`, `trigger_add`, `trigger_remove`, `trigger_run`, `trigger_move`.
