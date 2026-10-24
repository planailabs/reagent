---
name: reagent-locks
description: Named locks in reagent - keeping two deploys, a migration and the tests, or a sync and a release from happening at once; naming the locks a task, template, trigger, cron entry or git sync needs (wait, skip or fail when busy), taking and releasing them while a task runs, seeing who holds what, and breaking a stuck one.
---

# Locks

A lock is a name for something that mustn't happen twice at once: `deploy`,
`db`, `release`. It's a project's (`deploy` in another project is another
lock) or every project's (`global:staging`). It's held **exclusive** (one
holder) or **shared** (`db:shared`: any number of shared holders, and no
exclusive one meanwhile).

## Who holds them

- A task's locks belong to its **task tree**: its top-level task holds them,
  so a subtask uses what its parent holds, and what a subtask takes stays
  with the tree. They're kept while the task waits or is paused, and let go
  when the whole tree is over, when the task releases one, or when the
  person breaks one.
- A trigger's script run (poll, webhook; not a watcher), a git sync round
  and a merge hold theirs only while they run.

## Up front: what an action needs

Name them before it starts, with what to do when one is busy (`on_busy`):

- `wait`: it waits its turn (a task shows as queued: "waits for a lock:
  deploy, held by task …"; it starts when they're all free).
- `skip`: not this time (a task isn't started; a cron firing or a trigger
  run waits for its next time; a trigger's event comes again; a sync round
  waits for its next interval).
- `fail`: like skip, but recorded as a failure, with a notification.

Where: a task's start (the new task form, `task_start`, `tasks.task_spawn`
with `locks` and `on_busy`, `reagent task start --lock deploy`), a
template's frontmatter (`locks: [deploy]`, `on_busy: wait`), a trigger
(`locks`, `on_busy` in its definition or TRIGGER.md), a cron entry, a
branch's sync settings (or its `.agents/sync.md`), and the project's
settings: its known locks (each with what it guards: tasks are told them),
the default `on_busy` per origin (`person`, `agent`, `task`, `cron`,
`trigger`; wait, but skip for cron and triggers), and `merge_locks` (held
while a branch is merged into its base; `merge_on_busy`: wait or fail).

## While a task runs (`locks`)

- `lock_acquire(name, shared?, wait?, timeout?)`: take one before doing what
  it guards. It waits until free (up to `timeout` seconds, default 600), or
  with `wait = false` answers at once with who holds it. A wait that would
  deadlock (the holder waits, in the end, for a lock your tree holds) is
  refused at once: release something first.
- `lock_release(name)`: let go as soon as the guarded work is done, so
  others can go on.
- `lock_list()`: the project's and the global locks, who holds and who waits.

Hold a lock for as short as you can, and never across asking the person
something unless that's the point.

## Seeing and breaking them

The project's **locks** tab lists them (holders, who waits) with a button
to break one; the API has `GET /api/projects/{id}/locks` and
`DELETE /api/locks/{project}/{name}` (`global` for a global one), the MCP
API `lock_list` and `lock_break`. A task whose lock is broken is told.
