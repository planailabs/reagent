-- Reading reagent's own source (source.*) is allowed, as new projects' starter rules do.
insert into rules (project, pos, tool, action) select slug, -1, 'source.*', 'allow' from projects;
