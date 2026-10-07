# reagent

A resumable coding agent for long-running work, on [subagent-net](https://git.plan.ai/plan-ai/subagent-net). Register project folders, start tasks (by hand, from cron, from other tasks, or from agents outside over MCP), and watch and steer them in a web interface: commands in the foreground and background, terminals, file edits, git worktrees merged back on approval, a markdown memory per project and a global one, skills, budgets, notifications. See [DESIGN.md](DESIGN.md).

```sh
nix develop
(cd webui && npm install && npm run build)
cargo build --release
reagent passwd                 # the web interface's password
$EDITOR ~/.local/share/reagent/reagent.hcl   # providers and profiles (written on first start)
echo DEEPSEEK_API_KEY=… >> ~/.local/share/reagent/.env
reagent up                     # http://127.0.0.1:8800
reagent token add my-agent     # a token for reagent's MCP API at /mcp
```

Tests: `cargo test` (Rust), `cd webui && npm test && npm run e2e` (browser).
