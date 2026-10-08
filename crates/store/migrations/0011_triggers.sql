-- Triggers: scripts that watch something and start tasks or message running
-- ones. Kept here (source 'db'), or read from the project's
-- .agents/triggers/<name>/TRIGGER.md (source 'repo'; this row is its copy,
-- with what reagent keeps about it: on/off, state).
create table triggers (
    project text not null references projects (slug) on delete cascade,
    name text not null,
    source text not null default 'db',
    -- person, task:<id>, mcp:<token name>, repo
    made_by text not null default 'person',
    description text not null default '',
    -- poll, watch or webhook
    mode text not null,
    -- poll: every this many seconds, or on a cron expression in tz
    every integer,
    cron text,
    tz text not null default 'UTC',
    -- the script's text (a repo trigger's file, as read)
    script text not null default '',
    -- the script's file name beside a repo trigger's TRIGGER.md
    script_file text not null default 'run',
    timeout integer not null default 60,
    overlap text not null default 'skip',
    title text not null,
    prompt text not null,
    -- {profile, kind, budget, skills}
    options text not null default '{}',
    -- a webhook's secret: the name of one of the project's secrets
    secret text,
    devshell boolean not null default true,
    enabled boolean not null default true,
    -- what runs keep: next and last run, failures, approval, queued events, …
    state text not null default '{}',
    created integer not null default (unixepoch()),
    primary key (project, name)
);

-- Event keys a trigger has seen (dropped after 30 days).
create table trigger_keys (
    project text not null,
    name text not null,
    key text not null,
    seen integer not null default (unixepoch()),
    primary key (project, name, key),
    foreign key (project, name) references triggers (project, name) on delete cascade on update cascade
);

-- A trigger's last runs (20 kept).
create table trigger_runs (
    id integer primary key autoincrement,
    project text not null,
    name text not null,
    started integer not null,
    ended integer,
    exit integer,
    ok boolean not null default false,
    events integer not null default 0,
    output text not null default '',
    error text,
    foreign key (project, name) references triggers (project, name) on delete cascade on update cascade
);
create index trigger_runs_by_trigger on trigger_runs (project, name, id);
