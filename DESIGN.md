# reagent

A resumable coding agent for long-running work on the projects on one machine, built on [subagent-net](../subagent-net) (subnet). You register project folders, start tasks in them (by hand, from cron, from other tasks, or from agents outside over MCP), and watch and steer them in a web interface. Tasks run shell commands (foreground, background, interactive terminals), edit files, work in git worktrees and merge back, and keep notes in a markdown memory per project and a global one.

This document describes reagent as built; the Status section at the end lists what's implemented and what isn't.

## Decisions

| | |
|---|---|
| Shape | One Rust workspace pinning subnet by git rev; `reagent up` runs the hub, a node, reagent's MCP servers and the web server in one process (like Vesper). A separate `reagent supervisor` process owns commands and terminals. |
| Models | Any OpenAI-compatible endpoint (base URL, key from env, model name), as profiles. |
| Users | One user, reachable remotely: password login and a cookie session; TLS from a reverse proxy in front. Agents outside sign in to the MCP API with API tokens. |
| Command safety | Unsandboxed, as the user. A policy per project: allow / ask / deny by tool, command pattern and target; ask parks the call until it's approved. |
| Tasks | One subnet agent per task, in one project. Subtasks are tasks of their own (root agents) that reagent links to their parent: their reports come to it as messages. |
| Worktrees | The agent decides: tools to start a worktree, see its diff, merge it back, drop it. Merging is auto or needs approval, per project. |
| Commands | Foreground (the task waits), background jobs, and PTY terminals. A foreground command moves to the background at its timeout or from the UI. |
| Memory | Markdown: an `INDEX.md` per scope (global, each project) linking topic, folder and task files. Stored centrally, or in the project's repo, per project. The indexes are injected; tools read and change the files. |
| Skills | The generic layout: `.agents/skills/<name>/SKILL.md` (and `.agent/skills/`) in the working directory, the project and globally (`~/.agents/skills/`, `<data>/skills/`). Names and descriptions are injected; a tool loads one. The project's `AGENTS.md` is injected too. |
| Cron | Per project, in reagent's database, managed in the UI (and by tasks through tools). |
| Storage | SQLite for everything: reagent's own data, and subnet's hub through subnet's SQLite backend. |
| Notifications | Web Push and the apprise CLI. |
| Platforms | Linux and macOS (Unix PTYs, process groups, Unix sockets). |

## Processes

```
reagent supervisor   (long-lived; outlives restarts of `reagent up`)
  owns: jobs (sh -c, each in its own process group) and PTYs; logs and scrollback on disk
  talks: Unix socket <data>/supervisor.sock (newline-delimited JSON)

reagent up
  subnet hub (SQLite <data>/hub.db) + one node ("local")
  reagent's MCP servers for tasks: fs, shell, pty, git, memory, skills, tasks, ask, and the hooks
    (HTTP on a loopback port, /mcp/<server>, a token only the node has)
  followers: reports, task states, approvals, usage and budgets, job ends, compaction checkpoints
  cron scheduler, notifier
  web server (axum, reagent.hcl's listen): the UI, /api (JSON, SSE, WebSocket), /mcp (the MCP API)
```

`reagent up` starts the supervisor if none answers on the socket (`reagent supervisor`, detached in its own process group). Restarting `reagent up` (an upgrade, a crash) leaves every command running: on start it reconnects, and job ends it missed are passed on. Stopping (`Ctrl-C`) quick-pauses all running tasks first (running tool calls finish, nothing new starts), records them (`paused-at-stop.json`), and resumes them at the next start. Task agents of an older version (reagent.hcl or reagent changed) are upgraded at start: same history, new agent. `reagent supervisor --stop` ends the supervisor and everything it runs.

The supervisor's protocol: a request is `{"rid": n, "op": …}`, its answer `{"rid": n, "ok": …}` or `{"rid": n, "err": …}`; a connection that subscribed also gets `{"event": …}` lines (a job's output, a terminal's bytes with its scrollback first, every job's end). Ops: `spawn`, `jobs`, `job`, `wait` (until it ends, is moved to the background, or a timeout), `background`, `input`, `kill` (the whole process group), `ack` (its end was passed on), `pty_open`, `pty_send`, `pty_screen` (rendered by the `vt100` crate, after it's been quiet for a while, with scrollback), `pty_resize`, `pty_close`, `ptys`, `subscribe`, `shutdown`. A job running when the supervisor died is marked `lost` at its next start.

Data lives under the platform data dir (`directories`: `~/.local/share/reagent`, `~/Library/Application Support/reagent`), or `--data`, or `REAGENT_DATA`:

```
reagent.hcl          providers, model profiles, notifications, listen address (an example is written the first time)
.env                 secrets (read at start)
reagent.db           projects, rules, tasks, cron, settings, sessions, push subscriptions, notifications, API tokens
hub.db               subnet's store
cluster.hcl          the cluster file reagent made (for reference)
supervisor.sock, supervisor.log
jobs/<id>/           meta.json, output.log (stdout and stderr, as they came)
ptys/<id>/           output.log (the terminal's bytes)
memory/INDEX.md      the global memory
memory/projects/<project>/   a project's memory (when kept centrally)
worktrees/<project>/<branch>/  worktrees (when kept centrally)
paused-at-stop.json  tasks the last stop paused
```

## What subnet got for reagent

- **A SQLite store** next to Postgres, chosen by the database URL; migrations for both, kept equal (tested); leader election by a lock file next to the database. The whole subnet suite runs on either (`SUBNET_TEST_DB=sqlite`).
- **Who calls an MCP tool:** every tool call carries `_meta` with `subnet/agent` (and `subnet/parent`, `subnet/tenant`), also when routed through the hub: one reagent server serves every task and knows which one calls.
- **A pause during `pre_model` hooks holds the model call** until a resume (which makes it without asking the hooks again): the budget check in the context hook stops a task before its next call.

Besides: hooks (pre_tool with ask, pre_model with inject, pre_compact), approvals, pause modes, compaction, upgrades, `grep_results` and `search_history` were there.

## Projects

A project is registered in the UI (or the API): an id (lowercase, `-`), a name, a folder (one project per folder), and settings:

| setting | |
|---|---|
| `memory` | `central` (default: `<data>/memory/projects/<id>/`) or `repo` (`<folder>/.reagent/memory/`, committable) |
| `worktrees` | `central` (default: `<data>/worktrees/<id>/`) or `repo` (`<folder>/.worktrees/`, kept out of git status through `.git/info/exclude`) |
| `merge` | `approve` (default) or `auto` |
| `default_action` | what a call no rule covers gets: `ask` (default), `allow` or `deny` |
| `profile` | the default model profile for its tasks |
| `budget` | per task (`tokens`, `cost`, `minutes`) and for the project per day (`daily_cost`: no new tasks past it) |
| `env` | extra environment for its commands |

A new project gets the starter rules (below).

## Tasks

A task is a subnet agent spawned (as root) from the mixture `task-<profile>`, with reagent's record in `reagent.db`: project, parent, title, prompt, origin (`ui`, `cron:<id>`, `task:<parent>`, `mcp`), working directory (the project folder, or its worktree), profile, budget, skills loaded at start, state, what it waits for, report, tokens and cost, and the agent's id. States: `running`, `waiting` (for an approval, an answer, a merge, or over budget), `paused`, `done`, `failed`, `cancelled`.

- **Starting:** from the UI, the API, the MCP API, cron, or a task (`tasks.task_spawn`). The first message says the task, the project and the working directory, then the prompt, then the bodies of the skills asked for. A project over its daily cost starts nothing.
- **Steering:** a message (read before its next model call; a finished task goes on with it), pause (quick: running calls finish; safe: after this turn), resume, cancel (its jobs and terminals end too), approve or deny a call (or "always": a rule allowing the tool with that command goes in front), answer a question, merge or send back a worktree, retry a failed task (subnet resumes it where it failed: the model call again), raise its budget (a task paused over it goes on), change its profile or budget (a new profile applies when it next starts: subnet keeps an agent's type).
- **Following:** reports come to root's mailbox (a task is done, failed or cancelled); a loop every 1.5 s reads the agents' phases, the call waiting for approval, usage (cost from the profile's prices) and budgets. Job ends become messages to their task (which wakes it), except for a foreground command a tool still waits for. A subtask's report becomes a message to its parent.
- **Long runs:** compaction at three quarters of the profile's context; the `checkpoint` hook (pre_compact) asks for a summary someone could pick the work up from, and reagent writes each summary into the project's memory (`tasks/<id>.md`, linked from the index). The budget is checked before every model call (the `context` hook pauses a task over it, which holds the call) and by the follower.
- **Waiting across a restart:** a question or a merge waits in the tool call; after a restart that call has been aborted, the task hears so and may ask again. An approval survives (it's subnet's).

**The agent type.** reagent writes the cluster from `reagent.hcl` at every start: node `local`; per profile an agent `model-<profile>` (its provider's URL and key env, model, params, the system prompt `prompts/task.md`, `search_history`, `grep_results`, hooks `policy`, `context`, `checkpoint`, compaction) and a mixture `task-<profile>` with reagent's servers; the servers (`url`, a bearer token from `REAGENT_MCP_TOKEN`, their idempotent tools); the hooks. `spawns = []`: tasks start subtasks through reagent, not subnet's `spawn_agent`.

## Tools

The MCP servers of `reagent up`, for tasks only. Each call says which agent calls (`_meta.subnet/agent`), so paths, jobs and terminals are that task's. A path is relative to the task's working directory and must be inside its places (the working directory, the project folder, the memories; skills' folders for reading), unless a rule with a matching `target` allows it.

Besides these, subnet gives every task `search_history` (its own whole conversation) and `grep_result` (a long tool result, which reached it cut, searched or read by lines).

**Files (`fs`)**
| tool | |
|---|---|
| `read(path, from?, to?)` | lines, numbered, 400 at a time; the end says where to read on |
| `write(path, text)` | makes (with its folders) or replaces a file |
| `edit(path, …)` | `old`/`new` (exactly once, or `all`), or `op: append / insert (line or after a passage) / delete (lines or a passage)` |
| `grep(pattern, path?, glob?, context?, page?)` | regex over files, respecting `.gitignore`; case-insensitive unless the pattern has capitals; 100 a page |
| `glob(pattern, path?)` / `ls(path?)` | files by pattern (500 at most) / a folder |

**Commands (`shell`)**
| tool | |
|---|---|
| `exec(cmd, cwd?, timeout?, stdin?)` | `sh -c` in the foreground: exit code and output (the last 60 000 characters). Past `timeout` (default 600 s), or when the person moves it, it goes on as a background job: the call says so, with the output so far. |
| `exec_bg(cmd, cwd?, name?)` | a background job, its id at once; its end comes as a message |
| `jobs()` / `job_output(job, from?, to?, tail?, pattern?)` / `job_wait(job, timeout?)` / `job_input(job, text, close?)` / `job_kill(job, signal?)` | list, read, wait, write to stdin, signal the process group |

Commands get the project's env, `REAGENT_TASK`, `REAGENT_PROJECT`, and `PAGER=cat`, `GIT_PAGER=cat`, `GIT_EDITOR=true` (nothing waits for a person).

**Terminals (`pty`)**: `pty_open(cmd?, cwd?, cols?, rows?)`, `pty_send(pty, keys, quiet_ms?)` (text with `<enter>`, `<tab>`, `<esc>`, arrows, `<bs>`, `<C-x>`…; answers with the screen once it's quiet), `pty_screen(pty, quiet_ms?, scrollback?)`, `pty_close(pty)`, `ptys()`. The person sees and types into the same terminals (xterm.js).

**Git (`git`)**: `worktree_start(branch?, base?)` (branch `reagent/<title>-<id>`, from the project's current branch; the working directory moves there), `worktree_status()`, `worktree_diff(stat?)`, `worktree_merge(strategy?, message?)` (`merge`, `squash` or `rebase`; the worktree must be clean; with `merge = "approve"` it waits for the person, who sees the diff and merges or sends it back with a message; conflicts are aborted and named, to resolve in the worktree), `worktree_drop(force?)` (refused while it has unmerged commits). Into a checkout of the base (the one there is, else a temporary one); the git CLI does it (merges and worktrees aren't in gitoxide).

**Memory (`memory`)**: see Memory. **Skills (`skills`)**: `skill_list()`, `skill_load(name)`.

**Tasks (`tasks`)**: `task_spawn(title, prompt, project?, profile?, skills?, budget?)`, `task_list(project?, all?)`, `task_message(task, text)`, `task_wait(task, timeout?)`, `search_history(pattern, task?, project?, page?)` (other tasks' whole conversations: one task paged, or every task of a project or of all, a few hits each), `cron_list()`, `cron_add(expr, title, prompt, tz?, overlap?, profile?, skills?)`, `cron_remove(id)`.

**Asking (`ask`)**: `ask(question, options?)`: the task waits for the answer (a notification goes out).

## Policy

Per project, an ordered list of rules: `tool` (a glob over `<server>.<tool>`), `command` (a glob over the command line, for `shell.exec*`, `shell.job_input`, `pty.pty_open`, `pty.pty_send`), `target` (a glob over what a call is aimed at: the project of `task_spawn`, the path of a file tool), `action` (`allow`, `ask`, `deny`). The first rule that fits decides; none: the project's default. A command line is judged piece by piece (split at `&&`, `||`, `;`, `|`, `&`, newlines; `$(…)` and backticks are commands too; redirections like `2>&1` aren't): the strictest piece wins, so a rule for `cargo *` doesn't allow `cargo test && rm -rf ~`.

It runs as subnet's `pre_tool` hook (`policy`): ask parks the call in subnet's approval state; the person allows it once, always (a rule in front), or denies it; a deny says which rule. subnet's own tools (`search_history`, `grep_result`) are always allowed.

Starter rules: the file, memory, skills and ask tools; reading tools of git, jobs and terminals; subtasks in the project itself; `ls`, `cat`, `git status/diff/log/add/commit`, `rg`, `grep`, `cargo`, `npm test`, `npm run` are allowed; `git push` and `sudo` ask; `rm -rf /*` is denied. The rest asks.

## Memory

Two scopes, global and each project; each a folder of markdown with an `INDEX.md` the tools keep (one line per file, `- [name](path) — about`, in sections Topics, Folders, Tasks, Other by the path's first folder):

```
INDEX.md
topics/<name>.md          build.md, conventions.md, decisions.md …
folders/<path>.md         notes about a folder of the project
tasks/<id>.md             a task's checkpoints
```

| tool | |
|---|---|
| `memory_read(scope, file?)` | the index, or a file |
| `memory_write(scope, file, text, about)` | makes or replaces a file, with its index line |
| `memory_edit(scope, file, about?, …)` | part of a file (as `fs.edit`) |
| `memory_search(pattern, scope?)` | regex over both scopes' files |
| `memory_remove(scope, file)` | |

`scope` is `global` or `project`; paths are relative markdown paths (`INDEX.md` itself is the tools'). The `context` hook (pre_model) injects the global and the project index, `AGENTS.md` and the skills list when the task hasn't seen them as they are now (what changed is shown again). The UI has an editor for both memories.

## Skills

A skill is a folder with a `SKILL.md` (YAML frontmatter `name`, `description`, then the instructions) and files it refers to. Found (the nearer winning on a name) in the task's working directory (`.agents/skills/`, `.agent/skills/`), then the project folder, then globally (`~/.agents/skills/`, `<data>/skills/`); read when needed, never copied. The context lists them (`name — description`); `skill_load` gives the body and the skill's files; a task or a cron entry can start with skills loaded into its first message. The project's page lists them with where they're from.

## Cron

Per project: a five-field expression and a time zone (`croner`, `chrono-tz`), a task title and prompt, options (profile, budget, skills), what happens while the last run still goes (`skip`, default; `queue`: it starts when the last ends; `parallel`), and `catch_up` (a run missed while reagent was down runs once; without it, it's skipped). Checked every 15 s. Managed in the UI (next and last runs, run now, on/off) and by tasks. A run that can't start is a notification.

## Web interface

Vue + Parcel (`webui/`), served by `reagent up`; live over SSE (`/api/events`).

- **Inbox:** waiting tasks (approvals, questions, merges, budgets), failed ones, tasks going on, new notifications.
- **Projects:** the list and adding one; a project's tabs: tasks (a tree, subtasks under their parent), new task (profile, budget, skills), cron, policy (the rules, ordered), memory, skills, settings.
- **Task:** state, usage and cost; pause, pause after this turn, resume, retry, cancel, open a terminal; what it waits for (approve once / always / deny with the call's arguments; answer with an option or text; merge with the diff, or send back; raise the budget); its report; a message box; subtasks; tabs: the transcript (the whole conversation, summarised parts folded, the answer streaming), jobs (live output, to background, stop, kill), terminals (xterm.js over a WebSocket), the worktree's diff.
- **Search:** every task's conversation.
- **Settings:** push on this browser, the profiles, API tokens (make: shown once; revoke), the global memory.

Login: one password (`reagent passwd`, argon2), a session cookie (`HttpOnly`, `SameSite=Strict`, `Secure` behind TLS, 30 days, kept hashed), five tries a minute per address, writes only from the same site (Origin). The API, all behind the login: `/api/session`, `login`, `logout`, `config`, `events`, `inbox`, `notifications`, `push/*`, `projects` (+ `/{id}`, `/rules`, `/skills`, `/cron`), `memory`, `cron`, `tasks` (+ `/{id}` and `/transcript`, `/message`, `/pause`, `/resume`, `/cancel`, `/retry`, `/limits`, `/raise`, `/approve`, `/answer`, `/merge`, `/diff`, `/jobs`, `/ptys`), `search`, `jobs/{id}/output|stream|background|kill`, `ptys/{id}` (WebSocket), `tokens`.

## MCP API

Agents outside use reagent at `/mcp` (streamable HTTP) with `Authorization: Bearer <token>`. Tokens are made with `reagent token add <name>` or in the settings (shown once, kept hashed, revocable). Tools: `projects`, `task_start`, `task_list`, `task_get` (state, what it waits for, report, subtasks), `task_transcript`, `task_message`, `task_pause`, `task_resume`, `task_cancel`, `task_retry`, `task_raise_budget`, `task_approve`, `task_answer`, `task_merge`, `task_wait` (until it ends or waits for someone), `search`.

## Notifications

Events: `done`, `failed`, `waiting` (an approval, a question, a merge), `budget`, `cron` (a run that couldn't start). Every one is kept (the inbox shows them) and sent, as `notify.events` asks (default all):

- **Web Push:** VAPID keys made once (in the settings table), a browser subscribes from the settings page (a service worker); sent encrypted with `web-push-native` (pure Rust); a click opens the task. Gone subscriptions are forgotten.
- **apprise:** `apprise -t <title> -b <body and a link> <urls…>` with `notify.apprise` and `notify.apprise_env`'s URLs; it's a Python CLI, run, not linked.

## Config

`reagent.hcl`:

```hcl
listen = "127.0.0.1:8800"
default_profile = "default"

provider "deepseek" {
  base_url = "https://api.deepseek.com/v1"
  key_env  = "DEEPSEEK_API_KEY"
}

profile "default" {
  provider = "deepseek"
  model    = "deepseek-chat"
  price    = { input = 0.27, output = 1.10 }   # per million tokens, for budgets
  context  = 128000                            # compaction at three quarters
  # params = { temperature = 0.2 }
  # grep_results = { over = 8000 }             # this profile's own cut-off
}

# Long tool results reach a task cut, with a note; grep_result reads the rest. 0: off.
grep_results = { over = 12000, except = ["skills.skill_load", "fs.read", "shell.job_output"] }
search_history = true

notify {
  apprise = ["tgram://…"]                      # and/or apprise_env = "REAGENT_APPRISE"
  events  = ["done", "failed", "waiting", "budget", "cron"]
  url     = "https://reagent.example.org"      # links in notifications
}
```

## CLI

`reagent [--data <dir>] up [--listen <addr>]`, `supervisor [--stop]`, `passwd [--password-stdin]`, `status`, `token add|list|revoke`.

## Code layout

```
crates/reagent      the binary: up, stop, the CLI; tests (end to end, web, MCP API, notifications, restart)
crates/supervisor   the process/PTY daemon, its client and protocol
crates/tools        the App (tasks on subnet), the MCP servers, hooks, policy, memory, skills, git, cron, notifications, config, the cluster file
crates/store        reagent.db (sqlx, SQLite, migrations)
crates/web          axum: login, the API, SSE, WebSocket, the MCP API
webui/              Vue + Parcel; node tests (test/) and Playwright (e2e/)
prompts/task.md     the task agent's system prompt
```

## Testing

- Unit tests: config, policy (pieces, targets, starter rules), edits, memory and its index, skills (precedence), git (worktrees, merge, squash, rebase, conflicts), cron (time zones, catch-up), the cluster file (it parses as subnet's), the store, passwords and tokens.
- The supervisor: real processes and PTYs (foreground, background, timeout, stdin, process-group kill, lost jobs, scrollback, keys).
- End to end, against a scripted OpenAI-compatible model: reading, editing and running; approvals and always-allow; denials; background jobs waking their task; a foreground command moving to the background; a worktree merged after approval; questions; memory in the context; subtasks and cross-task search; cron; budgets (pausing, raising); retrying a failed task; paths outside the project; a restart resuming a task; notifications (apprise, web push); the web API (login, guard, projects, rules, memory, cron, skills, tasks, jobs, search, terminals over WebSocket); the MCP API with tokens.
- The web interface: node tests of its helpers, and Playwright against `reagent up` with a scripted model (`npm run e2e`).

## Status

Implemented: everything above.

Not done (yet): Windows; a profile change for a running task (it applies when the task is next started, as subnet keeps an agent's type); questions and merges waiting across a restart (the task is told the call was aborted and asks again); more than one user.
