-- Whether a queued message is the person's (typed on the task's page): only those can be changed or cancelled.
alter table outbox add column person boolean not null default false;
update outbox set person = true where text like '[from the person]%';
