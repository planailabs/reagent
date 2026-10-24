-- Named locks: what may not happen at once in a project (or, project '', in every project).
create table locks (
    -- a project's slug, or '' for a global lock
    project text not null,
    name text not null,
    -- task:<top-level task>, sync:<project>/<place>, merge:<project>/<name>, trigger:<project>/<name>
    holder text not null,
    shared boolean not null default false,
    since integer not null,
    primary key (project, name, holder)
);
-- The locks a task waits for and takes when it starts.
alter table tasks add column locks text not null default '[]';
-- The project's known locks, default busy policies per origin, and the locks merges take.
alter table projects add column locking text not null default '{}';
-- The locks a sync round takes, and what it does when one is busy.
alter table git_sync add column locks text not null default '[]';
alter table git_sync add column on_busy text;
-- Tasks take and release locks themselves (locks.*): every project allows it, as new projects' starter rules do.
insert into rules (project, pos, tool, action) select slug, -1, 'locks.*', 'allow' from projects;
