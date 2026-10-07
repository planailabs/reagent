-- Registered project folders and their settings.
create table projects (
    slug text primary key,
    name text not null,
    path text not null unique,
    -- central | repo
    memory text not null default 'central',
    worktrees text not null default 'central',
    -- approve | auto
    merge text not null default 'approve',
    -- what a command no rule matches does: allow | ask | deny
    default_action text not null default 'ask',
    profile text,
    -- {tokens?, cost?, minutes?} per task, {cost?} per day
    budget text not null default '{}',
    env text not null default '{}',
    created integer not null default (unixepoch())
);

-- A project's policy rules, first match wins.
create table rules (
    id integer primary key autoincrement,
    project text not null references projects (slug) on delete cascade,
    pos integer not null,
    tool text not null,
    command text,
    target text,
    -- allow | ask | deny
    action text not null
);

create index rules_project on rules (project, pos);

-- A task: one subnet agent working in a project.
create table tasks (
    id text primary key,
    agent text unique,
    project text not null references projects (slug) on delete cascade,
    parent text references tasks (id),
    title text not null,
    prompt text not null,
    -- ui | cron:<id> | task:<id>
    origin text not null,
    cwd text not null,
    -- {path, branch, base} while it works in a worktree
    worktree text,
    profile text not null,
    budget text not null default '{}',
    skills text not null default '[]',
    -- running | waiting | paused | done | failed | cancelled
    state text not null default 'running',
    -- what it waits for: {kind: question|merge|budget, …}
    wait text,
    report text,
    cost real not null default 0,
    tokens integer not null default 0,
    created integer not null default (unixepoch()),
    updated integer not null default (unixepoch()),
    finished integer
);

create index tasks_project on tasks (project, created desc);

create table cron (
    id integer primary key autoincrement,
    project text not null references projects (slug) on delete cascade,
    expr text not null,
    tz text not null default 'UTC',
    title text not null,
    prompt text not null,
    -- {profile?, budget?, worktree?, skills?}
    options text not null default '{}',
    -- skip | queue | parallel
    overlap text not null default 'skip',
    catch_up boolean not null default true,
    enabled boolean not null default true,
    last_run integer,
    next_run integer,
    -- a run that waits for the last to end (overlap = queue)
    queued boolean not null default false
);

create table settings (
    key text primary key,
    value text not null
);

create table sessions (
    hash text primary key,
    created integer not null default (unixepoch()),
    expires integer not null
);

create table push_subscriptions (
    endpoint text primary key,
    subscription text not null,
    created integer not null default (unixepoch())
);

create table notifications (
    id integer primary key autoincrement,
    at integer not null default (unixepoch()),
    kind text not null,
    task text,
    title text not null,
    body text not null,
    seen boolean not null default false
);
