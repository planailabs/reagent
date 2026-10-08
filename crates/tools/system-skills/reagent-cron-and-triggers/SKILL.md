---
name: reagent-cron-and-triggers
description: Starting tasks automatically - cron entries (schedules) and triggers (poll, watch and webhook scripts whose JSON events start tasks or message running ones), in reagent or in the repo.
---

# Cron and triggers

## Cron

A cron entry starts a task on a schedule: a five-field expression in a time
zone, a title and prompt, options (profile, kind, skills, budget), what
happens while the last run still goes (`skip`, `queue`, `parallel`), and
whether a run missed while reagent was down runs once. Managed on a project's
cron tab ("run now" too) and by tasks (`tasks.cron_list`, `tasks.cron_add`,
`tasks.cron_remove`).

## Triggers

A trigger runs a script that watches something and prints **events**, one
JSON object per line on stdout (other lines are its log):

```
{"key": "pipeline-812", "to": "new", "title": "…", "message": "…", "vars": {"url": "…"}}
```

- `key`: unique per happening; a key seen before (30 days) is dropped.
- `to`: `new` (start a task from the trigger's title and prompt templates),
  `running` (a message to the trigger's newest running task, else a new one),
  or a task id.
- Templates use `{{key}}`, `{{vars.url}}`, `{{message}}`, `{{title}}`.

Modes:

- **poll**: runs every so often (`every: 5m`) or on a cron expression.
- **watch**: runs for good (in the supervisor); each line is read as it
  comes; restarted when it ends.
- **webhook**: `POST /hook/<project>/<name>`, signed with one of the
  project's secrets (GitHub's `X-Hub-Signature-256`, GitLab's
  `X-Gitlab-Token`, or `Authorization: Bearer`). With a script, the request
  (`{headers, body}`) comes on its stdin; without, the body is the event.

A script runs in the project folder (in its dev shell unless `devshell:
false`) with the project's environment and secrets, and `REAGENT_STATE` (a
folder it keeps between runs), `REAGENT_LAST_RUN`, and `REAGENT_TASKS` (its
tasks still going, with their keys: to choose `to`). Its last 20 runs are kept.

**Where**: kept in reagent (the project's triggers tab, `reagent trigger
add`, `triggers.trigger_add`, the MCP API), or in the repo as
`.agents/triggers/<name>/TRIGGER.md` (frontmatter: `description`, `mode`,
`every`/`cron`, `tz`, `script` (file, default `run`), `timeout`, `overlap`,
`title`, `profile`, `kind`, `skills`, `budget`, `secret`, `devshell`; the
body is the prompt) with the script beside it. Either moves to the other.

**Trust**: a script runs if the person allowed it (what they save is
allowed; an approval holds until the script changes), else the policy
decides (`triggers.run`; a repo trigger's command is
`./.agents/triggers/<name>/<file>`). Asks wait in the inbox.

**Failures** back off (up to an hour); after 5 in a row a task is started to
repair the trigger.

Example poll script (CI on main):

```sh
gh run list --branch main --status failure --json databaseId,url -L 5 \
  | jq -c '.[] | {key: (.databaseId|tostring), vars: .}'
```
