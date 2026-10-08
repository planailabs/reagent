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
