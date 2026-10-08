---
name: reagent-tools
description: The tools a reagent task has - files, commands and background jobs, terminals, the nix dev shell, git, memory, skills, tasks, asking, todos, secrets, triggers, subnet's history tools, and MCP servers the person adds (lazy).
---

# Tools

| server | tools |
|---|---|
| `fs` | `read` (numbered lines, paged), `write`, `edit` (exact replace, or append/insert/delete), `grep` (regex, respects .gitignore), `glob`, `ls` |
| `shell` | `exec` (foreground; past its timeout, default 600 s, it goes on in the background), `exec_bg` (a background job: its end comes as a message), `jobs`, `job_output`, `job_wait`, `job_input`, `job_kill` |
| `pty` | `pty_open`, `pty_send` (keys like `<enter>`, `<C-c>`; answers with the screen), `pty_screen`, `pty_close`, `ptys`: terminals the person sees too |
| `git` | `worktree_start`, `worktree_status`, `worktree_diff`, `worktree_merge`, `worktree_drop` (`reagent-worktrees`) |
| `memory` | `memory_read`, `memory_write`, `memory_edit`, `memory_search`, `memory_remove` (`reagent-memory`) |
| `skills` | `skill_list`, `skill_load` (`reagent-skills`) |
| `tasks` | `task_spawn`, `prompt_design`, `task_list`, `task_message`, `task_wait`, `task_escalate`, `search_history`, `cron_list`, `cron_add`, `cron_remove` (`reagent-subtasks`) |
| `ask` | `ask(question, options?)`: wait for the person's answer |
| `todo` | `todo_list`, `todo_add`, `todo_update`, `todo_clear` |
| `secrets` | `secrets_list`, `secrets_get`, `secrets_set`, `secrets_remove` (`reagent-secrets`) |
| `triggers` | `trigger_list`, `trigger_add`, `trigger_remove`, `trigger_move` (`reagent-cron-and-triggers`) |

subnet adds `search_history` (your whole conversation, also what was
summarised) and `grep_result` (a long tool result that reached you cut:
search it or read lines of it).

## Places

Paths are relative to your working directory (the project folder, or your
worktree) and must stay inside your places: the working directory, the
project, the memories (and skills' folders, to read). Elsewhere needs a
policy rule with a target.

## Commands

- They run with `sh -c`, in the project's **nix dev shell** when it has one
  (a `flake.nix`; a project setting; `devshell: false` opts one command out).
- They get the project's environment, its secrets, `REAGENT_TASK`,
  `REAGENT_PROJECT`, and `PAGER=cat`, `GIT_EDITOR=true`: nothing may wait
  for a person. Use non-interactive flags.
- Long ones: `exec_bg`, then go on; you get a message when it ends. Servers
  and watchers too. Read output with `job_output` (tail, a range, or a
  pattern), not by printing everything.
- Commands outlive restarts of reagent (a supervisor owns them).

## Added MCP servers

The person can add MCP servers (for every task or one project's). Their
tools are `<server>.<tool>`, usually **lazy**: you see their names in
`load_tools`, and load the ones you need before calling them.
