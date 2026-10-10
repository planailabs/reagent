---
name: reagent-services
description: Services in reagent - commands of a project that should always run (dev servers, databases, workers); adding one with a health check, how reagent starts, checks, restarts and repairs them, managing them, and repairing one that keeps failing.
---

# Services

A service is a command of the project that should always run: a dev
server, a database, a queue worker, a tunnel. reagent starts it as a
background job (it outlives restarts of reagent), checks it, restarts it
when it ends or stops answering, and when that keeps failing, starts a task
to repair it.

## Tools

- `services.service_list()`: each service's state, since when, failures, last error, job.
- `services.service_add(name, script, health?, interval?, ready_timeout?, stop_timeout?, worktree?, devshell?, description?, notes?)`:
  adds or replaces one (a replaced running one restarts). The project's
  policy decides whether its command runs (`services.run`); it may wait for
  the person's approval.
- `services.service_start(name)` / `service_stop(name)`: switch it on or off;
  a stop holds until a start.
- `services.service_restart(name)`: stop it, start it again at once.
- `services.service_logs(name, lines?)`: the end of its output.
- `services.service_move(name, to)`: into the repo (`repo`) or back (`db`).
- `services.service_remove(name)`: stop and forget it.

## Writing one

- The script runs **for good**: `exec` the server (`exec npm run dev`), don't
  background it, don't exit. Any end is a failure.
- Give it a **health** command that exits 0 only when it really works
  (`curl -fs localhost:8080/health`, `pg_isready -h localhost`). Without one,
  it counts as up once it has lived 3 seconds.
- It runs in the project folder (or `worktree`, a persistent worktree),
  through the project's nix dev shell, with the project's environment and
  secrets and `$REAGENT_SERVICE`.
- `notes`: what it is, where its config is, how it's usually fixed: they go
  to whoever has to repair it.
- In the repo: `.agents/services/<name>/SERVICE.md` (frontmatter
  `description`, `script` (default `run`), `health`, `interval`,
  `ready_timeout`, `stop_timeout`, `worktree`, `devshell`; the body: the
  notes) with the script beside it. Read again within 10 seconds; a changed
  command restarts it.

## What reagent does

1. **starting**: started; checked every 2 s; **up** once a check passes. Not
   up within `ready_timeout` (default 60 s): stopped, a failure.
2. **up**: checked every `interval` (default 30 s); 3 failed checks in a row:
   stopped, a failure. Up for 60 s ends a failure streak.
3. A failure: started again after 5 s, doubling per failure in a row, up to
   an hour (**backoff**).
4. After 5 failures in a row: a task "Repair service <name>" starts (once per
   streak) with its command, notes, last error and output; the person hears it.
   It waits (queued) until nothing else goes on in the project.

## Repairing one

Read its output (`service_logs`) and the last error; fix the cause (its
config, a missing dependency, a port in use, the command itself); change the
service if its command was wrong (`service_add` with the same name, or its
files in the repo), or `service_restart` it once the cause is fixed. Check
it's up (`service_list`) before you report.
