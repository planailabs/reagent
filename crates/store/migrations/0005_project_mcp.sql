-- A server only one project's tasks get (none: every task).
alter table mcp_servers add column project text references projects (slug) on delete cascade;
