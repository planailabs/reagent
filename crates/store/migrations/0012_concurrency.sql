-- How many tasks may run at once: a project's own limit (none: only the
-- global one, kept in settings as max_tasks). Tasks over it wait, queued.
alter table projects add column max_tasks integer;
-- When a task's agent started (a queued task waits before): its time budget counts from there.
alter table tasks add column started integer;
update tasks set started = created where agent is not null;
