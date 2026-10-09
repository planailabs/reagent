-- Prompt templates kept in reagent (the whole markdown file); project null: every project's.
create table templates (
    id integer primary key,
    project text references projects (slug) on delete cascade,
    name text not null,
    text text not null,
    created integer not null default (unixepoch()),
    updated integer not null default (unixepoch())
);
create unique index templates_name on templates (coalesce(project, ''), name);
-- Tasks list templates and start subtasks from them: every project allows listing, as new projects' starter rules do.
insert into rules (project, pos, tool, action) select slug, -1, 'tasks.template_list', 'allow' from projects;
