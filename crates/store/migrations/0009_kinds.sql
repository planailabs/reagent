-- A task's kind (reagent.hcl's kind blocks), and a project's default one.
alter table tasks add column kind text;
alter table projects add column kind text;
