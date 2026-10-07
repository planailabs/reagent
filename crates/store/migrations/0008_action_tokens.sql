-- One-time tokens for a notification's buttons (approve, deny, an answer, merge):
-- only their sha256 is kept.
create table action_tokens (
    hash text primary key,
    task text not null references tasks (id) on delete cascade,
    -- {kind: approve|deny|answer|merge, call_id?, question?, answer?, branch?}
    action text not null,
    expires integer not null,
    used boolean not null default false
);
