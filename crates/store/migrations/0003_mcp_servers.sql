-- MCP servers the person adds: offered to every task (lazily, unless lazy = false).
create table mcp_servers (
    name text primary key,
    description text not null default '',
    -- streamable HTTP, or a command (stdio)
    url text,
    command text,
    -- {"KEY": "value or $VAR"} for a command's environment
    env text not null default '{}',
    -- {header, env, prefix}: a header from an environment variable (url servers)
    credential text,
    lazy boolean not null default true,
    idempotent text not null default '[]',
    enabled boolean not null default true,
    created integer not null default (unixepoch())
);
