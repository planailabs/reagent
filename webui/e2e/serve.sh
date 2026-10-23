#!/usr/bin/env bash
# reagent for the browser tests: a fresh data folder, a scripted model
# (mock-llm.mjs), a project folder (a git repo), a password; started by
# Playwright's webServer (playwright.config.js) and stopped after.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
tmp="$root/target/tmp/e2e"
# The last run's supervisor (its own process group: it outlives reagent).
"$root/target/debug/reagent" --data "$tmp/data" supervisor --stop >/dev/null 2>&1 || true
rm -rf "$tmp"
mkdir -p "$tmp/data" "$tmp/project"
cargo build -q -p reagent --manifest-path "$root/Cargo.toml"
node "$here/mock-llm.mjs" 8791 &
mock=$!
cat > "$tmp/data/reagent.hcl" <<HCL
listen = "127.0.0.1:8790"
provider "mock" {
  base_url = "http://127.0.0.1:8791/v1"
}
profile "default" {
  provider = "mock"
  model    = "scripted"
  price    = { input = 1.0, output = 2.0 }
}
profile "big" {
  provider = "mock"
  model    = "scripted-big"
}
kind "research" {
  profile = "big"
}
HCL
# A plugin to add (a git repo with a skill and a rule).
mkdir -p "$tmp/plugin-src/skills/hello" "$tmp/plugin-src/.agents/rules"
printf -- '---\nname: hello\ndescription: greet nicely\n---\nSay hello.\n' > "$tmp/plugin-src/skills/hello/SKILL.md"
echo "Be brief." > "$tmp/plugin-src/.agents/rules/tone.md"
git -C "$tmp/plugin-src" init -q -b main
git -C "$tmp/plugin-src" add -A
git -C "$tmp/plugin-src" -c user.email=t@t -c user.name=t commit -q -m "first"
# A marketplace to add (Claude Code's, like trailofbits/skills): a local plugin and a remote one.
mkdir -p "$tmp/mkt-src/.claude-plugin" "$tmp/mkt-src/plugins/scan/skills/semgrep"
echo '{"name": "sec", "plugins": [{"name": "scan", "description": "finds bugs", "source": "./plugins/scan"}, {"name": "far", "source": {"source": "github", "repo": "o/far"}}]}' > "$tmp/mkt-src/.claude-plugin/marketplace.json"
printf -- '---\nname: semgrep\ndescription: run semgrep\n---\nScan.\n' > "$tmp/mkt-src/plugins/scan/skills/semgrep/SKILL.md"
git -C "$tmp/mkt-src" init -q -b main
git -C "$tmp/mkt-src" add -A
git -C "$tmp/mkt-src" -c user.email=t@t -c user.name=t commit -q -m "first"
# Prompt templates in the project's repo: a form and a plain one.
mkdir -p "$tmp/project/.agents/templates"
cat > "$tmp/project/.agents/templates/ci.md" <<'MD'
---
description: Fix a failing CI job
title: "Fix CI: {{job}}"
budget: { minutes: 45 }
fields:
  - name: job
    label: Job name
    required: true
  - name: branch
    type: select
    options: [main, staging]
    default: main
---
The CI job {{job}} fails on {{branch}}. Fix it.
MD
printf -- '---\ndescription: Review the open PRs\ntitle: Review PRs\n---\nReview every open PR and say what blocks each.\n' > "$tmp/project/.agents/templates/review.md"
cd "$tmp/project"
git init -q -b main
git -c user.email=t@t -c user.name=t commit -q --allow-empty -m init
echo "e2e-password" | "$root/target/debug/reagent" --data "$tmp/data" passwd --password-stdin
export HOME="$tmp/home"
"$root/target/debug/reagent" --data "$tmp/data" up &
up=$!
# Playwright ends this script: end reagent and its supervisor with it.
trap 'kill $mock $up 2>/dev/null; "$root/target/debug/reagent" --data "$tmp/data" supervisor --stop >/dev/null 2>&1 || true' EXIT TERM INT
wait $up
