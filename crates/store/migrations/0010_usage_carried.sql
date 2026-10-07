-- What a task's earlier agents used (an upgrade or a model switch starts a
-- new agent, whose usage starts at nothing).
alter table tasks add column base_tokens integer not null default 0;
alter table tasks add column base_cost real not null default 0;
