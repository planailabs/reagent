-- The prompt designer (tasks.prompt_design) asks a model and changes nothing:
-- every project allows it, as new projects' starter rules do.
insert into rules (project, pos, tool, action) select slug, -1, 'tasks.prompt_design', 'allow' from projects;
