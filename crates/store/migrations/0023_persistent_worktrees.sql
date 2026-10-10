-- Persistent worktrees: named, long-lived checkouts tasks start in (their home).
create table persistent_worktrees (
    project text not null references projects(slug) on delete cascade,
    name text not null,
    path text not null,
    branch text not null,
    base text not null,
    -- shared, exclusive, or null: the project's worktree_sharing
    sharing text,
    -- recorded as git had it (removing it only forgets it, unless asked to delete)
    adopted integer not null default 0,
    created integer not null default (unixepoch()),
    primary key (project, name)
);
alter table projects add column worktree_sharing text not null default 'shared';
-- The persistent worktree a task works in (its name; null: the project folder).
alter table tasks add column home text;
