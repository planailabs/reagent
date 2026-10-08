-- A task's working memory (the wm tools): slots of key and JSON value.
create table working_memory (
    task text not null references tasks (id) on delete cascade,
    key text not null,
    value text not null,
    updated integer not null default (unixepoch()),
    primary key (task, key)
);
-- The task's own scratch space: every project allows it, as new projects' starter rules do.
insert into rules (project, pos, tool, action) select slug, -1, 'wm.*', 'allow' from projects;
