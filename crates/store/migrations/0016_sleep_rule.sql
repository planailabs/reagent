-- shell.sleep only waits: every project allows it, as new projects' starter rules do.
insert into rules (project, pos, tool, action) select slug, -1, 'shell.sleep', 'allow' from projects;
