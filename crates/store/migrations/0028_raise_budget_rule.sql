-- A task raises its own subtasks' budgets (tasks.task_raise_budget): every project allows it, as new projects' starter rules do.
insert into rules (project, pos, tool, action) select slug, -1, 'tasks.task_raise_budget', 'allow' from projects;
