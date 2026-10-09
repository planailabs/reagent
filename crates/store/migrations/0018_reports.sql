-- Every report a task made (one per time it ended); tasks.report stays the last.
create table reports (
    id integer primary key,
    task text not null references tasks (id) on delete cascade,
    content text not null,
    at integer not null default (unixepoch())
);
create index reports_task on reports (task, id);
insert into reports (task, content, at) select id, report, coalesce(finished, updated) from tasks where report is not null;
