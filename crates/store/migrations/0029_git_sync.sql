-- Git sync: a branch of a project (its folder's, or a persistent worktree's) pushed and pulled on an interval.
create table git_sync (
    project text not null references projects (slug) on delete cascade,
    -- '' the project folder, else a persistent worktree's name
    place text not null default '',
    enabled boolean not null default false,
    -- seconds between rounds
    every integer not null default 900,
    -- rebase, merge or ff-only
    strategy text not null default 'rebase',
    -- none: the branch's upstream, else origin and the same name
    remote text,
    branch text,
    -- for the task that fixes a conflict or decides about uncommitted changes
    notes text not null default '',
    state text not null default '{}',
    primary key (project, place)
);
