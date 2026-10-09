-- apprise channels the person adds (reagent.hcl's notify.apprise still works
-- too). The URL holds the service's token: encrypted like secrets.
create table notify_channels (
    id integer primary key,
    name text not null unique,
    nonce blob not null,
    url blob not null,
    -- a JSON list of the kinds it gets; null: all
    events text,
    enabled integer not null default 1,
    created integer not null default (unixepoch())
);
