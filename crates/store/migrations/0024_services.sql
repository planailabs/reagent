-- Services: commands of a project that should always run (started, checked, restarted, repaired).
create table services (
    project text not null references projects (slug) on delete cascade,
    name text not null,
    -- db (kept here) or repo (.agents/services/<name>/SERVICE.md)
    source text not null default 'db',
    made_by text not null default 'person',
    description text not null default '',
    script text not null default '',
    script_file text not null default 'run',
    -- a command: exit 0 is healthy
    health text,
    -- seconds between checks while up
    interval integer not null default 30,
    ready_timeout integer not null default 60,
    stop_timeout integer not null default 10,
    -- a persistent worktree of the project to run in
    worktree text,
    devshell boolean not null default true,
    -- for whoever repairs it
    notes text not null default '',
    -- switched on: it should run
    enabled boolean not null default true,
    state text not null default '{}',
    created integer not null default (unixepoch()),
    primary key (project, name)
);
create table service_runs (
    id integer primary key autoincrement,
    project text not null,
    name text not null,
    job text,
    started integer not null,
    up integer,
    ended integer,
    exit integer,
    error text,
    output text not null default '',
    foreign key (project, name) references services (project, name) on delete cascade on update cascade
);
create index service_runs_by_service on service_runs (project, name, id);
-- Reading services is allowed, as new projects' starter rules do.
insert into rules (project, pos, tool, action) select slug, -1, 'services.service_list', 'allow' from projects;
insert into rules (project, pos, tool, action) select slug, -1, 'services.service_logs', 'allow' from projects;
