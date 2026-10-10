-- Messages for tasks, kept until delivered (a request returns once one is here).
create table outbox (
    id integer primary key,
    task text not null references tasks (id) on delete cascade,
    text text not null,
    created integer not null default (unixepoch())
);
create index outbox_task on outbox (task, id);
