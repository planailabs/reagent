-- A task names itself (tasks.task_title): every project allows it, as new projects' starter rules do.
insert into rules (project, pos, tool, action) select slug, -1, 'tasks.task_title', 'allow' from projects;
