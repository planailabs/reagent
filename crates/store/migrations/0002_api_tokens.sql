-- Tokens other agents use for reagent's MCP API (only their sha256 is kept).
create table api_tokens (
    hash text primary key,
    name text not null unique,
    created integer not null default (unixepoch()),
    last_used integer
);
