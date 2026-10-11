-- Subtasks may start in any project: the starter rule allowing them only in the project itself allows them everywhere.
update rules set target = null where tool = 'tasks.task_spawn' and action = 'allow' and target = project;
