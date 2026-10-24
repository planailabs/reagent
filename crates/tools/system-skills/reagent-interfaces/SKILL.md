---
name: reagent-interfaces
description: How people and other agents use reagent - the web UI (inbox, projects, tasks, settings, docs), notifications and their buttons, the CLI, and the MCP API for outside agents.
---

# Interfaces

## Web UI

- **Inbox**: what waits for you (approvals, questions, merges, budgets,
  trigger scripts to allow), failed tasks, tasks going on, notifications.
- **Projects**: each with tabs: tasks (a tree with subtasks), new task (with
  the designer; where it works: the folder or a persistent worktree), templates,
  worktrees (persistent ones: make, adopt, sharing, diff, merge, remove), cron, triggers, services, policy, memory, skills, MCP servers,
  secrets, settings.
- **Task**: state, cost, todo list, what it waits for, the report, a message
  box, the transcript (live, markdown rendered), jobs (logs), terminals, the
  worktree's diff, the working memory; pause, resume, start now, retry, clone and restart,
  cancel, switch model.
- **Settings**: notifications on this browser, tasks at once, model
  profiles, every project's secrets, API tokens, global memory.
- **Docs**: these system skills.

## Notifications

When a task is done, fails, waits for you, goes over budget, changes model,
or a trigger needs you: in the inbox, as browser push notifications (with
buttons: allow once, deny, an answer, merge) and through apprise (Slack,
email, ntfy, Telegram, …: the channels on the settings page, each with the
kinds it gets; `reagent.hcl`'s notify block can add URLs too).

## CLI

`reagent up` runs it. Besides: `passwd`, `status`, `mcp add|list|remove`,
`secret set|list|get|remove`, `trigger add|list|remove|enable|disable|run|move|approve`,
`task start [--template <name> --set field=value] [--worktree <name>]`,
`worktree list|add|adopt|set|status|diff|merge|remove`,
`service add|list|remove|start|stop|restart|logs|move|approve`, `task design`, `task proposal`,
`template list|show`, `plugin add|list|update|rules-off|remove`, `token add|list|revoke`,
`supervisor --stop`.

## MCP API (for other agents)

`<reagent's URL>/mcp`, streamable HTTP, with `Authorization: Bearer <token>`
(`reagent token add <name>`, or the settings page). Its tools are the web
API's routes, one each, with the same input (path, query and body fields in
one object) and the same JSON answer: `projects`, `task_start`, `task_list`,
`task_get`, `task_transcript` (`tail`), `task_message`, `task_pause`,
`task_resume`, `task_cancel`, `task_retry`, `task_raise_budget`,
`task_switch_profile`, `task_approve`, `task_answer`, `task_merge`,
`task_diff`, `search`, `prompt_design`, `design_proposal`, `design_create`,
`docs_list`, `docs_read`, `memory_*`, `cron_*`, `template_*`, `plugin_*`,
`trigger_*`, `service_*`, `worktree_*`, `sync_*`, `mcp_server_*`,
`secret_*` (names only, never values), `inbox`, `notifications`, `config`,
`settings` and the rest (the tool list names them all). Only for agents:
`task_wait` (until a task ends or waits for someone), `trigger_list` and
`service_list` (every project's, a line each), `trigger_add` and
`service_add` (a short form), `sync_set`, `service_logs`,
`reagent_source_ls`, `reagent_source_read`. Not there: login, live streams,
terminals' WebSocket, web push, API tokens. `task_start` takes a `template`
and its `values`, and a persistent `worktree`.
