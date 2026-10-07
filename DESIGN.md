# reagent

A resumable coding agent for long-running work on the projects on one machine, built on [subagent-net](../subagent-net) (subnet). You register project folders, start tasks in them (by hand, from cron, or from other tasks), and watch and steer them in a web interface. Tasks run shell commands (foreground, background, interactive terminals), edit files, work in git worktrees and merge back, and keep notes in a markdown memory per project and a global one.

This is the target design. The Status section at the end says what's built.

## Decisions

| | |
|---|---|
| Shape | One Rust workspace pinning subnet by git rev; `reagent up` runs hub, node, reagent's MCP servers and the web server in one process (like Vesper). A separate `reagent supervisor` process owns commands and terminals. |
| Models | Any OpenAI-compatible endpoint (base URL, key from env, model name), as profiles. |
| Users | One user, reachable remotely: password login and a cookie session; TLS from a reverse proxy in front. |
| Command safety | Unsandboxed, as the user. A policy per project: allow / ask / deny by tool and command pattern; ask parks the call until approved in the UI. |
| Tasks | One subnet agent per task, in one project; tasks spawn subtasks (child agents). |
| Worktrees | The agent decides: tools to start a worktree, see its diff, merge it back, drop it. Merging is auto or needs approval, per project. |
| Commands | Foreground (blocks the task), background jobs, and PTY terminals. A foreground command can be moved to the background from the UI or by its timeout. |
| Memory | Markdown: an `INDEX.md` per scope (global, each project) linking topic files and folder files. Stored centrally by default, or in the project's repo, per project. The indexes are injected; tools read and change the files. |
| Skills | The generic layout: `.agents/skills/<name>/SKILL.md` in the project (and `.agent/skills/`), and globally in `~/.agents/skills/`. Names and descriptions are injected; a tool loads one. The project's `AGENTS.md` is injected too. |
| Cron | Per project, in reagent's database, managed in the UI (and by tasks through a tool). |
| Storage | SQLite for everything: reagent's own data, and subnet's hub through a new SQLite backend in subnet (Postgres stays as subnet's other backend). |
| Notifications | Web Push and the apprise CLI. |
| Platforms | Linux and macOS (Unix PTYs, process groups, Unix sockets). Windows later, if ever. |

## Processes

```
reagent supervisor   (long-lived; outlives restarts of `reagent up`)
  owns: background jobs, foreground commands, PTYs; their logs and scrollback on disk
  talks: Unix socket <data>/supervisor.sock (length-prefixed JSON)

reagent up
  subnet hub (SQLite <data>/hub.db) + one node
  reagent MCP servers (fs, shell, pty, git, memory, tasks, cron, ask): HTTP, /mcp/<server>
  hooks endpoint (policy, context, checkpoint): /hooks/mcp
  scheduler (cron), notifier, budget watcher
  web server (axum): UI, REST/SSE API, auth
```

`reagent up` starts the supervisor if none answers on the socket. Restarting `reagent up` (an upgrade, a crash) leaves every command running: on start it reconnects, lists the jobs, and delivers what finished meanwhile. Stopping (`Ctrl-C`) quick-pauses all running tasks first (running tool calls finish, nothing new starts), records them, and resumes them at the next start. `reagent supervisor --stop` ends the supervisor and the processes it owns (after asking, in the UI or on the terminal, when jobs run).

Data lives under the platform data dir (`directories`: `~/.local/share/reagent`, `~/Library/Application Support/reagent`), overridable with `REAGENT_DATA`:

```
reagent.hcl          providers, model profiles, notifications, listen address
reagent.db           projects, tasks, cron, policies, sessions, push subscriptions, notifications
hub.db               subnet's store
supervisor.sock
jobs/<id>/           meta.json, output.log (stdout+stderr, interleaved), pty scrollback
memory/INDEX.md      global memory
memory/projects/<project>/INDEX.md   a project's memory (when kept centrally)
worktrees/<project>/<task>/          worktrees (when kept centrally)
```

## Subnet changes

**SQLite backend.** subnet's hub store (`hub/db.rs`, ~420 lines, plus `ha.rs`) is Postgres-only today. The hub gets a store with two backends, chosen by the database URL (`postgres://…`, `sqlite://…`):

- `Db` becomes an enum over `PgPool` and `SqlitePool` (sqlx supports both); each method keeps one query where the SQL is the same and a per-backend query where it isn't (`jsonb` → JSON text, `bigserial` → `integer primary key`, `bytea` → `blob`, timestamps as unix seconds or text).
- Migrations split into `migrations/postgres` and `migrations/sqlite`, each run by `sqlx migrate`, kept equivalent (a test compares the schemas by table and column).
- HA (advisory-lock leader election, several hubs on one database) stays Postgres-only: with SQLite there is one hub, and `ha` is refused in the config.
- SQLite runs in WAL mode with one writer (sqlx's pool with `max_connections` for readers, writes serialised); event appends stay transactional.
- The test suite runs against both backends (Postgres as now, SQLite in a temp file).

**Nothing else is needed from subnet** so far: hooks (pre_tool Ask, pre_model inject, pre_compact), approvals, pause/quick pause, budgets, compaction, residents, upgrades, `grep_results` all exist. Whatever turns out to be missing goes into subnet as a general feature, with its DESIGN.md.

## Projects

A project is registered in the UI: a name (its slug is its id), a folder, and settings:

| setting | |
|---|---|
| `memory` | `central` (default: `<data>/memory/projects/<slug>/`) or `repo` (`<folder>/.reagent/memory/`, committable) |
| `worktrees` | `central` (default: `<data>/worktrees/<slug>/`) or `repo` (`<folder>/.worktrees/`, git-ignored) |
| `merge` | `approve` (default) or `auto` |
| `policy` | rules (below) and the default action |
| `profile` | the default model profile for its tasks |
| `budget` | per task (tokens, cost, wall time) and per day for the project (cost) |
| `env` | extra environment for its commands (secrets by reference to reagent's env) |

## Tasks

A task is a subnet agent of type `task` in a project, with reagent's record of it in `reagent.db`: project, title, prompt, origin (`ui`, `cron:<id>`, `task:<parent>`), working directory (the project folder, or its worktree), profile, budgets, and the subnet agent id. Its state is subnet's (thinking, tools, waiting for approval, paused, done, failed) plus reagent's waits: asking you a question, waiting for a merge approval, over budget.

- **Starting:** from the UI (project, title, prompt, profile, budget), from cron, or by a task (`task_spawn`): a subtask is a subnet child of the task that spawned it, so it reports back to it; it may be in another project only if the policy allows.
- **Steering:** you can message a running task (it reads it before its next model call), pause it (safe, quick), resume it, cancel it, change its profile or budget, approve or deny its calls, answer its questions, merge or reject its worktree.
- **Ending:** a task ends with a report (its last answer). Reports, the whole transcript and the commands' logs stay viewable; a finished task can be continued with a new message (it's resumable: subnet keeps its history).
- **Long runs:** subnet compaction with a `pre_compact` hook that first asks the task to write a checkpoint (what's done, what's next, what it learned) into the project memory (`tasks/<task>.md`, linked from the index), so a summary never loses the plan. Budgets are checked before each model call (tokens from subnet's usage, cost from the profile's prices, wall time); over budget, the task pauses and you're notified.

**Agent type.** reagent generates subnet's cluster file from its config: one `task` agent type per model profile (`task@<profile>`), a mixture with reagent's MCP servers, `grep_results` on (long results cut, `grep_result` reads them), `search_history`, compaction, the hooks below, and the system prompt (`prompts/task.md`: how to work, the tools, memory, worktrees, when to ask).

## Tools

All tools are MCP servers in `reagent up`, scoped to the calling task (its token says which task): paths resolve against its working directory and must stay inside the project folder or its worktree (or the memory dirs), unless the policy allows more.

**Files (`fs`)**
| tool | |
|---|---|
| `read(path, from?, to?)` | a file's lines with numbers (default the first 400; says how many there are and where to read on, at the end) |
| `write(path, text)` | creates or replaces a file |
| `edit(path, old, new, all?)` | replaces an exact string (once, or every one); or `op: append/insert/delete` with lines or a passage (the same edit as Vesper's `share_edit`) |
| `grep(pattern, path?, glob?, context?, page?)` | regex search (ripgrep's crates: `grep-searcher`, `ignore`), respecting `.gitignore` |
| `glob(pattern)` / `ls(path?)` | files by pattern / a folder |

**Commands (`shell`)**
| tool | |
|---|---|
| `exec(cmd, cwd?, timeout?, stdin?)` | runs `sh -c cmd` in the foreground: the task waits. Returns exit code and output (long output cut; `grep_result` or `job_output` reads all). At `timeout` (default 10 min), or when you press "to background" in the UI, the call returns `{backgrounded: <job>, output so far}` and the command keeps running as a job. |
| `exec_bg(cmd, cwd?, name?)` | starts a background job, returns its id at once. When it ends, the task gets a message (exit code, the last lines), which wakes it if it's idle. |
| `jobs()` / `job_output(job, from?, to?, pattern?)` / `job_wait(job, timeout?)` / `job_input(job, text)` / `job_kill(job, signal?)` | list, read, wait for, write to, signal (the whole process group) |

**Terminals (`pty`)**
| tool | |
|---|---|
| `pty_open(cmd?, cols?, rows?)` | an interactive terminal (a shell by default), returns its id |
| `pty_send(pty, keys)` | text and keys (`<enter>`, `<C-c>`, `<up>`…) |
| `pty_screen(pty, wait_ms?, scrollback?)` | the screen as text (rendered by a terminal emulator, the `vt100` crate), optionally after it's been quiet for `wait_ms` |
| `pty_close(pty)` | |

You see and type into the same terminals in the UI (xterm.js).

**Git (`git`)**
| tool | |
|---|---|
| `worktree_start(branch?, base?)` | a worktree on a new branch (`reagent/<task-slug>` by default) from `base` (default the current branch); the task's working directory moves there |
| `worktree_status()` / `worktree_diff(stat?)` | what changed against the base |
| `worktree_merge(strategy?, message?)` | `merge` (default), `squash` or `rebase` onto the base. Per project: `auto` merges at once (conflicts: aborted, and the task is told which files, to resolve in its worktree and try again); `approve` sets the task waiting for you, with the diff in the UI: merge, or send it back with a message. |
| `worktree_drop()` | removes the worktree and its branch (asks first if it has unmerged commits) |

git is run as the `git` CLI (merges, rebases and worktrees aren't covered by gitoxide; the CLI is the tool here).

**Memory (`memory`)** — see Memory.

**Tasks and cron (`tasks`)**: `task_spawn(title, prompt, project?, profile?, budget?, worktree?)`, `task_list(project?)`, `task_message(task, text)`, `task_wait(task, timeout?)`; `cron_list()`, `cron_add(expr, title, prompt, options?)`, `cron_remove(id)` (in its own project).

**Skills (`skills`)**: `skill_list()`, `skill_load(name)` (see Skills).

**Asking (`ask`)**: `ask(question, options?)`: the task waits for your answer (a notification goes out); your answer is the result.

## Policy

Per project: an ordered list of rules, the first match decides, else the project's default (`ask` unless set).

```hcl
rule { tool = "shell.exec*"  command = "cargo *"            action = "allow" }
rule { tool = "shell.exec*"  command = "git push*"          action = "ask" }
rule { tool = "shell.exec*"  command = "rm -rf /*"          action = "deny" }
rule { tool = "fs.*"                                        action = "allow" }
rule { tool = "git.worktree_merge"                          action = "allow" }   # merging itself is governed by `merge`
rule { tool = "tasks.task_spawn"  project = "other"         action = "ask" }
```

`command` is a glob over the command line (also applied to `pty_send` lines and `job_input`); rules are kept in `reagent.db` and edited in the UI as a list. It runs as a subnet `pre_tool` hook (`policy`, served by reagent, decisions Allow / Deny / Ask): Ask parks the call in subnet's approval state; the UI shows it (with "allow once", "always allow this pattern", "deny with a message"). A deny tells the task why.

## Memory

Two scopes: global and each project. Each is a folder of markdown:

```
INDEX.md
topics/<name>.md          # build.md, conventions.md, decisions.md, people.md …
folders/<path>.md         # notes about a folder of the project: folders/crates/core.md
tasks/<task>.md           # a task's checkpoints
```

`INDEX.md` has a section per kind, one line per file: `- [build](topics/build.md) — cargo needs nix develop; tests write target/test.log`. The tools keep the index: writing a file takes its one-line description, removing it drops its line.

| tool | |
|---|---|
| `memory_read(scope, file?)` | the index, or a file |
| `memory_write(scope, file, text, about)` | creates or replaces a file, with its index line (`about`) |
| `memory_edit(scope, file, …)` | append / insert / delete (as `fs.edit`) |
| `memory_search(pattern, scope?)` | regex over both scopes' files |
| `memory_remove(scope, file)` | |

`scope` is `global` or `project`. A `pre_model` hook (`context`) injects the global and the project index (with the skills list and `AGENTS.md`, see Skills) at the start and again whenever one changed since it last did, so the task always knows what's there without reading it. Memory is plain files: you can edit it in the UI (a file tree and an editor) or with any editor, and a project's memory kept in the repo is versioned with it.

## Skills

Skills are the generic agent skills: a folder with a `SKILL.md` (YAML frontmatter `name` and `description`, then the instructions) and any files it refers to (scripts, references, templates).

Where they're found, the nearer one winning on a name clash:
1. the task's working directory: `.agents/skills/*/SKILL.md` (a worktree has the branch's skills), and `.agent/skills/` as well;
2. the project folder, the same (when the task works in a worktree, the folder's own skills still count, for skills not committed yet);
3. global: `~/.agents/skills/` (and `<data>/skills/`).

They're read when needed (a skill added or changed while a task runs is seen at its next model call), never copied.

- **Listed, not loaded:** the `context` hook injects one line per skill (`name — description`) with the memory indexes, again when the list changed, so a task knows what it has without paying for the bodies.
- **Loaded by the task:** `skill_load(name)` returns the `SKILL.md` body with the paths of the skill's other files; the task reads those with `fs.read` and runs its scripts with `shell.exec` (a skill's folder is readable even when it's global, outside the project).
- **Started with a skill:** a task (or a cron job) can be started with skills preloaded (`skills = ["deploy"]`): their bodies go into its first message.
- **Instructions:** the project's `AGENTS.md` (in the working directory, else the project folder; it may link skills) is injected at the start like the indexes, and again when it changed.
- In the UI: a project's page lists its skills and the global ones (where each comes from), and shows a skill's files.

## Cron

Per project, in `reagent.db`: a cron expression with a time zone (`croner`), a task title and prompt, task options (profile, budget, worktree), and what to do when the last run is still going (`skip`, default; `queue`; `parallel`). A run missed while reagent was down runs once at start (`catch_up`, default on). Managed in the UI (with the next runs shown) and by tasks (`cron_*`). Each run is a task with origin `cron:<id>`; a cron's runs are listed with it.

## Web UI

Vue + Parcel (as subnet's webui), served by `reagent up`. Live updates over SSE.

- **Inbox:** everything waiting for you: approvals, questions, merges, budget stops, failures.
- **Projects:** list; a project's page: its tasks, cron, policy, settings, memory (tree + editor).
- **Tasks:** a tree of tasks (subtasks under their parent), filtered by project and state.
- **Task:** the live transcript (thinking, messages, tool calls and results, from subnet's events; the agent panel cloned from subnet's webui and kept in sync like Vesper's brain view), a message box, pause / quick pause / resume / cancel, profile and budget, pending approvals and questions, its jobs (live output, "to background" on a foreground one, kill), its terminals (xterm.js, type into them), its worktree (diff, merge, send back), its usage and cost.
- **Settings:** providers and profiles (from `reagent.hcl`, read-only in the UI at first), notifications, password, push on this browser.

Auth: one password (`reagent passwd`, argon2), a session cookie (`HttpOnly`, `Secure` behind TLS, `SameSite=Strict`) and an Origin check on writes; a login rate limit. Bound to `127.0.0.1` by default; remote use goes through a reverse proxy with TLS.

## Notifications

Events: task done, failed, waiting for you (approval, question, merge), over budget, a cron run that couldn't start. Each can be on or off per channel.

- **Web Push:** the UI registers a service worker; reagent keeps VAPID keys (made once) and the subscriptions, and sends with the `web-push` crate. A click opens the task.
- **apprise:** the apprise CLI with the URLs from `reagent.hcl` (`apprise -t <title> -b <body> <urls…>`): Telegram, Matrix, mail, ntfy… The CLI is the tool here (it's Python, no library to call); it comes from the nix devshell.

## Config

`reagent.hcl` (data dir):

```hcl
listen = "127.0.0.1:8800"

provider "deepseek" {
  base_url = "https://api.deepseek.com/v1"
  key_env  = "DEEPSEEK_API_KEY"
}

profile "default" {
  provider = "deepseek"
  model    = "deepseek-chat"
  price    = { input = 0.27, output = 1.10 }   # per million tokens, for budgets
  context  = 128000
}

notify {
  apprise = ["tgram://…"]                        # or from env: apprise_env = "REAGENT_APPRISE"
}
```

Secrets come from the environment (a `.env` in the data dir is read at start).

## Code layout

```
crates/reagent      the binary: up, supervisor, passwd, status; wiring
crates/supervisor   the process/PTY daemon and its client (jobs, PTYs, logs, the socket protocol)
crates/tools        the MCP servers (fs, shell, pty, git, memory, skills, tasks, ask) and the hooks (policy, context, checkpoint)
crates/store        reagent.db (sqlx, SQLite, migrations): projects, tasks, cron, policies, sessions, push
crates/web          axum: API, SSE, auth, static UI
webui/              Vue + Parcel
prompts/task.md     the task agent's system prompt
```

## Testing

- Unit tests per crate (policy matching, memory index upkeep, edits, cron next-run, path scoping, skill discovery and precedence).
- The supervisor: real processes and PTYs (a job outlives a client restart; process-group kill; "to background").
- End to end: `reagent up` against a scripted OpenAI-compatible model (as subnet's and Vesper's tests do): a task edits a file in a worktree, runs a test in the foreground and a server in the background, asks for approval, merges; cron starts a run; a restart in the middle keeps the job and resumes the task.
- subnet: its whole suite on both store backends.
- Web UI: Playwright against a scripted `reagent up`.

## Build order

1. subnet: SQLite store backend (both suites green), committed to subnet.
2. reagent skeleton: workspace, config, `reagent up` with hub + node on SQLite, one `task` type, starting a task from the CLI.
3. Supervisor + `shell` + `pty`.
4. `fs`, `memory`, skills and `AGENTS.md` (+ context hook), policy hook + approvals.
5. `git` worktrees and merging.
6. Tasks API, subtasks, budgets, checkpoints.
7. Cron.
8. Web UI (inbox, projects, tasks, task view, terminals, diffs, memory editor) + auth.
9. Notifications.
10. Playwright tests.

## Status

Nothing built yet: this document is the plan.
