-- The call evaluator: a project's settings, and what it judged.
alter table projects add column evaluator text not null default '{}';
create table evaluations (
    id integer primary key,
    task text not null references tasks (id) on delete cascade,
    call_id text not null default '',
    tool text not null,
    args text not null default '',
    reason text not null default '',
    -- read_only, mutating, destructive
    risk text not null,
    -- static (a loop, a duplicate, a write before any read), model, or error (the model couldn't judge)
    source text not null,
    approved boolean not null,
    reasoning text not null default '',
    -- allowed, refused (to the task), asked (the person), warned
    outcome text not null,
    created integer not null
);
create index evaluations_task on evaluations (task, id);
