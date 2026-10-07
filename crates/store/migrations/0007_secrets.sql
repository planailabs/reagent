-- Secrets: every project's (project null) or one project's; values encrypted
-- with the key in secret.key next to this database.
create table secrets (
    project text references projects (slug) on delete cascade,
    name text not null,
    nonce blob not null,
    value blob not null,
    updated integer not null default (unixepoch())
);

create unique index secrets_name on secrets (coalesce(project, ''), name);
