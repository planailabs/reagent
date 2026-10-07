-- A task's own todo list (the todo tools).
create table todos (
    task text not null references tasks (id) on delete cascade,
    id integer not null,
    text text not null,
    -- pending | in_progress | done | cancelled
    status text not null default 'pending',
    updated integer not null default (unixepoch()),
    primary key (task, id)
);
