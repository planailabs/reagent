# reagent

A resumable coding agent for long-running work, built on subagent-net (Rust; web UI: Vue + Parcel in `webui/`).

## DESIGN.md is a target, not an afterthought

[DESIGN.md](DESIGN.md) describes the architecture, tools, storage and behaviour. A change that alters any of them updates DESIGN.md **in the same commit**; when a part is finished, move it to "implemented" in the Status section. If the code and DESIGN.md disagree, that is a bug.

## Rules

- subnet is pinned by git rev; general features it lacks go into subnet (with its DESIGN.md), not around it.
- Schema changes only through `sqlx migrate`.
- Linux and macOS: no Linux-only calls without a macOS path.
- Tests write long output to `target/test.log`.
- Tests: `nix develop -c cargo test`; web UI: `cd webui && npm test && npm run e2e` (Playwright, browsers from nix).
- Releases: a static musl binary with the web UI embedded: `cd webui && npm run build`, then `nix develop -c sh -c 'cargo build --release -p reagent --target $REAGENT_MUSL_TARGET'` (see DESIGN.md, Building).
