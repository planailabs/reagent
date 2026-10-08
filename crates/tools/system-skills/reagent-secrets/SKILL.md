---
name: reagent-secrets
description: Secrets in reagent - where tokens and keys are kept, how commands get them, masking, and how a task reads, sets and removes its project's secrets.
---

# Secrets

Tokens and keys (a GitHub token, a deploy key, an API key) are kept by
reagent, encrypted in its database, for every project or for one project
(a project's own win over every project's of the same name).

- **Commands and terminals get them as environment variables**: use
  `$NAME`. Their names are shown to the task; values are not.
- **Values are masked**: any tool result (other than the secrets tools'
  own), job log or message that contains a secret's value shows `***`.
- `secrets.secrets_list()`: the names, and whose each is.
- `secrets.secrets_get(name)`: a value (when a tool needs it outside a
  command).
- `secrets.secrets_set(name, value)`: keep a token you got or made, for this
  project (its commands, and its other tasks, get it from then on). Names
  are environment variable names. The person is told, never the value.
- `secrets.secrets_remove(name)`: remove one of this project's own.
  Every project's secrets are the person's to change.

Reading is allowed by the starter rules; setting and removing go by the
project's default action.

**Never** write a secret into a file, the memory, a commit, a report or a
prompt: keep it with `secrets_set` and refer to it by name.

The person manages secrets on the web UI's settings page (every project's)
and a project's secrets tab, with `reagent secret set|list|get|remove`, or
the API.
