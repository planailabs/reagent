-- A queued message being handed to its task: no longer editable or cancellable.
alter table outbox add column sending integer not null default 0;
