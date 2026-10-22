import { execSync } from "node:child_process";
import { expect, test } from "@playwright/test";

const project = new URL("../../target/tmp/e2e/project", import.meta.url).pathname;
const pluginSrc = new URL("../../target/tmp/e2e/plugin-src", import.meta.url).pathname;

async function login(page) {
  await page.goto("/");
  await page.getByLabel("password").fill("e2e-password");
  await page.getByRole("button", { name: "log in" }).click();
  await expect(page.getByRole("link", { name: "inbox" })).toBeVisible();
}

async function ensureProject(page) {
  const r = await page.request.get("/api/projects/site");
  if (r.ok()) return;
  await page.goto("/#/projects");
  await page.getByLabel("id").fill("site");
  await page.getByLabel("name", { exact: true }).fill("Site");
  await page.getByLabel("folder").fill(project);
  await page.getByRole("button", { name: "add" }).click();
  await expect(page.getByRole("heading", { name: /Site/ })).toBeVisible();
}

async function startTask(page, title, prompt = "go") {
  await page.goto("/#/project/site/new");
  await page.getByLabel("title", { exact: true }).fill(title);
  await page.getByLabel("what to do").fill(prompt);
  await page.getByRole("button", { name: "start" }).click();
  await expect(page.getByRole("heading", { name: title })).toBeVisible();
}

test.beforeEach(async ({ page }) => {
  await login(page);
  await ensureProject(page);
});

test("a wrong password is refused", async ({ browser }) => {
  const page = await (await browser.newContext()).newPage();
  await page.goto("/");
  await page.getByLabel("password").fill("nope-nope");
  await page.getByRole("button", { name: "log in" }).click();
  await expect(page.getByRole("alert")).toContainText("wrong password");
});

test("a notification unfolds, and is marked seen on its own", async ({ page }) => {
  await startTask(page, "Notice one");
  await expect(page.locator(".reports")).toContainText("Done: Notice one");
  await startTask(page, "Notice two");
  await expect(page.locator(".reports")).toContainText("Done: Notice two");
  await page.getByRole("link", { name: "inbox" }).click();
  const one = page.locator("details.note", { hasText: "Notice one" });
  const two = page.locator("details.note", { hasText: "Notice two" });
  await expect(one.locator(".md")).toBeHidden();
  await one.locator("summary").click({ position: { x: 2, y: 5 } });
  await expect(one.locator(".md")).toBeVisible();
  await one.getByRole("button", { name: "mark seen" }).click();
  await expect(one).toHaveCount(0);
  await expect(two).toBeVisible();
});

test("a task runs and reports", async ({ page }) => {
  await startTask(page, "Hello there");
  await expect(page.locator(".reports")).toContainText("Done: Hello there");
  await page.setViewportSize({ width: 800, height: 200 });
  await page.getByRole("button", { name: "to the bottom" }).click();
  await expect.poll(() => page.evaluate(() => scrollY)).toBeGreaterThan(0);
  await page.getByRole("button", { name: "to the top" }).click();
  await expect.poll(() => page.evaluate(() => scrollY)).toBe(0);
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.getByRole("link", { name: "projects" }).click();
  await page.getByRole("link", { name: "Site" }).click();
  await page.getByLabel("finished ones too").check();
  await expect(page.locator(".task-row", { hasText: "Hello there" })).toContainText("✓");
});

test("messages and the report render markdown", async ({ page }) => {
  await startTask(page, "Say **loud**");
  await expect(page.locator(".reports strong")).toHaveText("loud");
  await expect(page.locator(".msg.assistant strong")).toHaveText("loud");
});

test("a finished task is cloned and restarted", async ({ page }) => {
  await startTask(page, "Twice over");
  await expect(page.locator(".reports")).toContainText("Done: Twice over");
  const first = page.url();
  await page.getByRole("button", { name: "clone and restart" }).click();
  await expect(page).not.toHaveURL(first);
  await expect(page.getByRole("heading", { name: "Twice over" })).toBeVisible();
  await expect(page.locator(".reports")).toContainText("Done: Twice over");
});

test("a call the policy asks about is approved from the inbox", async ({ page }) => {
  await startTask(page, "Touch a file");
  await page.getByRole("link", { name: /inbox/ }).click();
  await page.locator(".task-row", { hasText: "Touch a file" }).click();
  const box = page.getByLabel("approval");
  await expect(box).toContainText("shell.exec");
  await expect(box).toContainText("touch from-ui.txt");
  await box.getByRole("button", { name: "allow once" }).click();
  await expect(page.locator(".reports")).toContainText("touched");
});

test("a task's todo list shows", async ({ page }) => {
  await startTask(page, "Todo list");
  await expect(page.locator(".reports")).toContainText("planned and started");
  const list = page.getByLabel("todo list");
  await expect(list).toContainText("1 of 2 done");
  await expect(list.locator(".todo.done")).toContainText("look around");
  await expect(list.locator(".todo.pending")).toContainText("do the thing");
});

test("a task's working memory shows on its tab", async ({ page }) => {
  await startTask(page, "Remember things");
  await expect(page.locator(".reports")).toContainText("remembered");
  await page.getByRole("link", { name: "working memory" }).click();
  const wm = page.getByLabel("working memory");
  await expect(wm.locator("tr", { hasText: "pr" })).toContainText("42");
  await expect(wm.locator("tr", { hasText: "plan" })).toContainText('"step": 2');
});

test("a question is answered with an option", async ({ page }) => {
  await startTask(page, "Ask a color");
  // Pinned on top; it folds to one line and back.
  const pinned = page.getByLabel("waits for you");
  await expect(pinned).toContainText("waits for you: answer: which color?");
  await pinned.getByRole("button", { name: "fold" }).click();
  await expect(page.getByLabel("question")).toHaveCount(0);
  await pinned.getByRole("button", { name: "unfold" }).click();
  const box = page.getByLabel("question");
  await expect(box).toContainText("which color?");
  await box.getByRole("button", { name: "blue" }).click();
  await expect(page.locator(".reports")).toContainText("you chose blue");
  await expect(pinned).toHaveCount(0);
});

test("a task started without a title names itself", async ({ page }) => {
  await page.goto("/#/project/site/new");
  await page.getByLabel("what to do").fill("say hi");
  await page.getByRole("button", { name: "start" }).click();
  await expect(page.getByRole("heading", { name: "Named by itself" })).toBeVisible();
  await expect(page.locator(".reports")).toContainText("named");
});

test("a quick prompt picks the project, limits from presets, typed or unlimited, and skills by search", async ({ page }) => {
  await page.goto("/#/quick");
  await page.getByLabel("project").selectOption("site");
  await page.getByLabel("what to do").fill("say hi quickly");
  await expect(page.getByLabel("tokens limit").getByRole("button", { name: /project default/ })).toHaveClass(/on/);
  await page.getByLabel("tokens limit").getByRole("button", { name: "1M" }).click();
  await page.getByLabel("cost limit").getByRole("button", { name: "unlimited" }).click();
  await page.getByLabel("other minutes limit").fill("30");
  await page.getByLabel("search skills").fill("subtask");
  await page.getByLabel("search skills").press("Enter");
  await expect(page.getByRole("button", { name: "drop skill reagent-subtasks" })).toBeVisible();
  await page.getByRole("button", { name: "start" }).click();
  await expect(page.getByRole("heading", { name: "Named by itself" })).toBeVisible();
  const id = page.url().split("/task/")[1];
  const t = await (await page.request.get(`/api/tasks/${id}`)).json();
  expect(t.budget).toEqual({ tokens: 1000000, minutes: 30 });
  expect(t.skills).toEqual(["reagent-subtasks"]);
  // The choices are remembered for the next one.
  await page.goto("/#/quick");
  await expect(page.getByLabel("tokens limit").getByRole("button", { name: "1M" })).toHaveClass(/on/);
});

test("templates: a form fills the prompt, a plain one gives it to edit, and new ones are kept", async ({ page }) => {
  await page.goto("/#/quick");
  await page.getByLabel("project").selectOption("site");
  await page.getByLabel("template", { exact: true }).selectOption({ label: "ci — Fix a failing CI job (project, form)" });
  await expect(page.getByLabel("what to do")).toHaveCount(0);
  await page.getByLabel("Job name").fill("build");
  await expect(page.getByLabel("rendered prompt")).toHaveText("The CI job build fails on main. Fix it.");
  await expect(page.getByLabel("other minutes limit")).toHaveAttribute("placeholder", "45");
  await page.getByRole("button", { name: "start" }).click();
  await expect(page.getByRole("heading", { name: "Fix CI: build" })).toBeVisible();
  const id = page.url().split("/task/")[1];
  const t = await (await page.request.get(`/api/tasks/${id}`)).json();
  expect([t.prompt, t.budget.minutes]).toEqual(["The CI job build fails on main. Fix it.", 45]);
  // A plain one: its prompt to edit.
  await page.goto("/#/project/site/new");
  await page.getByLabel("template", { exact: true }).selectOption({ label: "review — Review the open PRs (project)" });
  await expect(page.getByLabel("what to do")).toHaveValue("Review every open PR and say what blocks each.");
  await expect(page.getByLabel("title", { exact: true })).toHaveValue("Review PRs");
  // Kept in reagent: a broken one is refused, a good one listed.
  await page.goto("/#/project/site/templates");
  await page.getByRole("button", { name: "new template for this project" }).click();
  await page.getByLabel("template name").fill("deploy");
  await page.getByLabel("template text").fill("---\nfields: [{name: host, type: radio}]\n---\nDeploy {{host}}.");
  await page.getByRole("button", { name: "save the template" }).click();
  await expect(page.getByRole("alert")).toContainText("no type radio");
  await page.getByLabel("template text").fill("---\ndescription: Deploy a host\nfields: [{name: host}]\n---\nDeploy {{host}}.");
  await page.getByRole("button", { name: "save the template" }).click();
  await expect(page.getByLabel("template deploy")).toContainText("kept in reagent for site");
  page.once("dialog", (d) => d.accept());
  await page.getByLabel("template deploy").getByRole("button", { name: "remove" }).click();
  await expect(page.getByLabel("template deploy")).toHaveCount(0);
});

test("queued messages are shown, edited, cancelled one by one and all at once", async ({ page }) => {
  await startTask(page, "Touch and hold");
  await expect(page.getByLabel("waits for you")).toBeVisible();
  // Paused: messages wait in the queue until it's resumed.
  await page.getByRole("button", { name: "pause", exact: true }).click();
  for (const m of ["one", "two", "three", "four"]) {
    await page.getByLabel("message", { exact: true }).fill(m);
    await page.getByRole("button", { name: "send" }).click();
    await expect(page.getByLabel("message", { exact: true })).toHaveValue("");
  }
  const q = page.getByLabel("queued messages");
  await expect(q).toContainText("four");
  const row = (text) => q.locator(".row", { hasText: text });
  await row("three").getByRole("button", { name: "edit" }).click();
  await q.getByLabel("edit the message").fill("three, changed");
  await q.getByRole("button", { name: "save" }).click();
  await expect(q).toContainText("three, changed");
  await row("two").getByRole("button", { name: "cancel" }).click();
  await expect(q).not.toContainText("two");
  await q.getByRole("button", { name: "cancel all" }).click();
  await expect(q).toHaveCount(0);
  // One more, kept: resuming delivers it, and the task answers it.
  await page.getByLabel("message", { exact: true }).fill("five");
  await page.getByRole("button", { name: "send" }).click();
  await expect(q).toContainText("five");
  await page.getByRole("button", { name: "resume" }).click();
  await expect(q).toHaveCount(0);
  // It would hold a place for the tests after.
  page.once("dialog", (d) => d.accept());
  await page.getByRole("button", { name: "cancel", exact: true }).click();
  await expect(page.locator(".state", { hasText: "cancelled" }).or(page.getByText("cancelled").first())).toBeVisible();
});

test("reports are kept, each folds and so do they all", async ({ page }) => {
  await startTask(page, "Report twice");
  await expect(page.getByLabel("report 1", { exact: true })).toContainText("the first report");
  await page.getByLabel("message", { exact: true }).fill("once more");
  await page.getByRole("button", { name: "send" }).click();
  await expect(page.getByLabel("report 2", { exact: true })).toContainText("the second report");
  // The earlier one folds away once there's a newer one; each unfolds on its own.
  await expect(page.getByLabel("report 1", { exact: true })).not.toContainText("the first report");
  await page.getByRole("button", { name: "unfold report 1" }).click();
  await expect(page.getByLabel("report 1", { exact: true })).toContainText("the first report");
  await page.getByRole("button", { name: "fold report 2" }).click();
  await expect(page.getByLabel("report 2", { exact: true })).not.toContainText("the second report");
  // The whole section folds.
  await page.getByRole("button", { name: "fold the reports" }).click();
  await expect(page.getByLabel("report 1", { exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "unfold the reports" }).click();
  await expect(page.getByLabel("report 1", { exact: true })).toContainText("the first report");
});

test("a background job's output shows, and its end wakes the task", async ({ page }) => {
  await startTask(page, "Run a ticker");
  // A loop isn't a starter rule: it asks first.
  await page.getByLabel("approval").getByRole("button", { name: "allow once" }).click();
  await page.getByRole("link", { name: "jobs" }).click();
  await page.locator(".job").first().click();
  const modal = page.getByRole("dialog");
  await expect(modal).toContainText("for i in 1 2 3");
  await expect(modal.getByLabel("output")).toContainText("tick 1");
  // Live: the next lines come in while it's open.
  await expect(modal.getByLabel("output")).toContainText("tick 3", { timeout: 10_000 });
  await page.keyboard.press("Escape");
  await expect(modal).toHaveCount(0);
  await page.locator(".job").first().click();
  await page.getByRole("dialog").getByRole("button", { name: "close" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".reports")).toContainText("ticker finished", { timeout: 20_000 });
});

test("a terminal takes keys", async ({ page }) => {
  await startTask(page, "Terminal host");
  await expect(page.locator(".reports")).toContainText("Done");
  await page.getByRole("button", { name: "open a terminal" }).click();
  const term = page.locator(".xterm-box");
  await term.click();
  await page.keyboard.type("echo hi-from-xterm\n");
  await expect(term).toContainText("hi-from-xterm", { timeout: 10_000 });
  await page.getByRole("button", { name: "close terminal" }).click();
});

test("policy, memory and cron are edited, and conversations searched", async ({ page }) => {
  await page.goto("/#/project/site/policy");
  await page.getByRole("button", { name: "add a rule" }).click();
  await page.getByLabel("tool").first().fill("shell.exec");
  await page.getByLabel("command").first().fill("make *");
  await page.getByRole("button", { name: "save" }).click();
  await expect(page.getByRole("alert")).toContainText("saved");
  const rules = await (await page.request.get("/api/projects/site/rules")).json();
  expect(rules[0].command).toBe("make *");

  await page.goto("/#/project/site/memory");
  await page.getByLabel("memory file").fill("topics/build.md");
  await page.getByLabel("about").fill("how to build");
  await page.getByLabel("memory text").fill("make all");
  await page.getByRole("button", { name: "save" }).click();
  await expect(page.locator(".index")).toContainText("- [build](topics/build.md) — how to build");

  await page.goto("/#/project/site/cron");
  await page.getByLabel("schedule").fill("30 2 * * *");
  await page.getByLabel("time zone").fill("UTC");
  await page.getByLabel("cron title").fill("Nightly check");
  await page.getByLabel("cron prompt").fill("check");
  await page.getByRole("button", { name: "save" }).click();
  await expect(page.locator("table")).toContainText("Nightly check");

  await startTask(page, "Searchable one", "find me");
  await expect(page.locator(".reports")).toContainText("Done");
  await page.getByRole("link", { name: "search" }).click();
  await page.getByLabel("search").fill("Searchable");
  await page.getByRole("button", { name: "search" }).click();
  await expect(page.locator(".card").first()).toContainText("Searchable one");
});

test("an API token is made and revoked", async ({ page }) => {
  await page.goto("/#/settings");
  await page.getByLabel("token name").fill("ci-agent");
  await page.getByRole("button", { name: "make a token" }).click();
  const shown = page.getByLabel("new token");
  await expect(shown).toContainText("rgt_");
  const token = (await shown.locator("code").first().textContent()).trim();
  const r = await page.request.post("/mcp", {
    headers: { authorization: `Bearer ${token}`, accept: "application/json, text/event-stream", "content-type": "application/json" },
    data: { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "t", version: "1" } } },
  });
  expect(r.status()).toBe(200);
  page.once("dialog", (d) => d.accept());
  await page.locator("tr", { hasText: "ci-agent" }).getByRole("button", { name: "revoke" }).click();
  await expect(page.locator("tr", { hasText: "ci-agent" })).toHaveCount(0);
  const denied = await page.request.post("/mcp", { headers: { authorization: `Bearer ${token}`, "content-type": "application/json" }, data: {} });
  expect(denied.status()).toBe(401);
});

test("an MCP server is added, reported and removed", async ({ page }) => {
  await page.goto("/#/mcp");
  const every = page.getByLabel("mcp servers for every task");
  await every.getByLabel("server name").fill("nowhere");
  await every.getByLabel("server url").fill("http://127.0.0.1:9/mcp");
  await every.getByRole("button", { name: "save and apply" }).click();
  await expect(every.getByRole("alert")).toContainText("doesn't run", { timeout: 30_000 });
  const row = page.locator("tr", { hasText: "nowhere" });
  await expect(row).toContainText("lazy");
  page.once("dialog", (d) => d.accept());
  await row.getByRole("button", { name: "remove" }).click();
  await expect(page.locator("tr", { hasText: "nowhere" })).toHaveCount(0);
});

test("a project's own MCP server is managed on its tab", async ({ page }) => {
  await page.goto("/#/project/site/mcp");
  await page.getByLabel("server name").fill("site-only");
  await page.getByLabel("server url").fill("http://127.0.0.1:9/mcp");
  await page.getByRole("button", { name: "save and apply" }).click();
  await expect(page.getByRole("alert")).toContainText("doesn't run", { timeout: 30_000 });
  const mine = await (await page.request.get("/api/mcp?project=site")).json();
  expect(mine.map((m) => m.name)).toEqual(["site-only"]);
  const global = await (await page.request.get("/api/mcp?project=global")).json();
  expect(global.map((m) => m.name)).not.toContain("site-only");
  await page.goto("/#/mcp");
  await page.locator("summary", { hasText: "Site" }).click();
  await expect(page.locator("details", { hasText: "Site" })).toContainText("site-only");
  page.once("dialog", (d) => d.accept());
  await page.locator("details", { hasText: "Site" }).locator("tr", { hasText: "site-only" }).getByRole("button", { name: "remove" }).click();
  await expect(page.locator("tr", { hasText: "site-only" })).toHaveCount(0);
});

test("secrets are set, shown on demand and removed", async ({ page }) => {
  await page.goto("/#/settings");
  const every = page.getByLabel("secrets for every project");
  await every.getByLabel("secret name").fill("SHARED_TOKEN");
  await every.getByLabel("secret value").fill("s3cr3t-everywhere");
  await every.getByRole("button", { name: "save" }).click();
  const row = every.locator("tr", { hasText: "SHARED_TOKEN" });
  await expect(row).toContainText("••••");
  await expect(row).not.toContainText("s3cr3t-everywhere");
  await row.getByRole("button", { name: "show" }).click();
  await expect(row).toContainText("s3cr3t-everywhere");
  await page.goto("/#/project/site/secrets");
  const own = page.getByLabel("secrets of site");
  await own.getByLabel("secret name").fill("SITE_ONLY");
  await own.getByLabel("secret value").fill("for-site");
  await own.getByRole("button", { name: "save" }).click();
  await expect(own.locator("tr", { hasText: "SITE_ONLY" })).toHaveCount(1);
  page.once("dialog", (d) => d.accept());
  await own.locator("tr", { hasText: "SITE_ONLY" }).getByRole("button", { name: "remove" }).click();
  await expect(own.locator("tr", { hasText: "SITE_ONLY" })).toHaveCount(0);
});

test("a trigger is added, starts a task, moves to the repo and goes", async ({ page }) => {
  await page.goto("/#/project/site/triggers");
  await page.getByLabel("trigger name").fill("ping");
  await page.getByLabel("every").fill("1h");
  await page.getByLabel("script").fill(`echo '{"key": "k1", "vars": {"what": "the build"}}'`);
  await page.getByLabel("trigger title").fill("Triggered {{key}}");
  await page.getByLabel("trigger prompt").fill("Check {{vars.what}}");
  await page.getByRole("button", { name: "save" }).click();
  const card = page.getByLabel("trigger ping");
  await expect(card).toContainText("every 1h");
  await card.getByRole("button", { name: "runs" }).click();
  await expect(card.locator(".runs summary").first()).toContainText("ok · 1 events");
  await page.goto("/#/project/site");
  await page.getByLabel("finished ones too").check();
  await expect(page.locator(".task-row", { hasText: "Triggered k1" })).toBeVisible();
  await page.goto("/#/project/site/triggers");
  await card.getByRole("button", { name: "move to repo" }).click();
  await expect(card).toContainText("in the repo");
  page.once("dialog", (d) => d.accept());
  await card.getByRole("button", { name: "remove" }).click();
  await expect(card).toHaveCount(0);
});

test("a task moves onto another model", async ({ page }) => {
  await startTask(page, "Switch me");
  await expect(page.locator(".reports")).toContainText("Done: Switch me");
  await expect(page.locator(".row", { hasText: "Switch me" }).first()).toContainText("default");
  await page.getByLabel("switch model").selectOption("big");
  await page.getByRole("button", { name: "switch" }).click();
  await expect(page.locator("main")).toContainText("· big ·");
  const t = await (await page.request.get("/api/tasks?project=site&limit=50")).json();
  expect(t.find((x) => x.title === "Switch me").profile).toBe("big");
});

test("over the limit a task is queued, and started now by hand", async ({ page }) => {
  await page.goto("/#/settings");
  await page.getByLabel("tasks at once").fill("1");
  await page.getByRole("button", { name: "save" }).first().click();
  await expect(page.getByText(/running, \d+ queued/)).toBeVisible();
  // One waits for an answer (it holds the place), the next is queued.
  await startTask(page, "Ask holder");
  await expect(page.getByLabel("question")).toBeVisible();
  await startTask(page, "Queued one");
  await page.getByRole("button", { name: "start now" }).click();
  await expect(page.locator(".reports")).toContainText("Done: Queued one");
  await page.goto("/#/settings");
  await page.getByLabel("tasks at once").fill("");
  await page.getByRole("button", { name: "save" }).first().click();
  await expect(page.getByLabel("tasks at once")).toHaveValue("");
});

test("the guide reads, asks, and proposes items the person picks and creates", async ({ page }) => {
  await page.goto("/#/project/site/new");
  await page.getByText("design it with the guide").click();
  await page.getByLabel("goal").fill("make the page nicer");
  await page.getByRole("button", { name: "design", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Design: make the page nicer" })).toBeVisible();
  await expect(page.getByRole("button", { name: "propose now" })).toBeVisible();
  await page.getByRole("button", { name: "home" }).click();
  const p = page.getByLabel("proposal");
  await expect(p).toContainText("the home page");
  await expect(p.getByLabel("item 1", { exact: true })).toContainText("task: Polish the page");
  await expect(p.getByLabel("item 1", { exact: true }).locator("strong")).toHaveText("nicer");
  // Edit the task's title, leave the skill out, create.
  await p.getByLabel("item 1", { exact: true }).getByRole("button", { name: "edit" }).click();
  await p.getByLabel("item 1", { exact: true }).getByLabel("item title").fill("Polish the home page");
  await p.getByLabel("take item 2").uncheck();
  // The skill's body has a code fence of its own; the report shows without the JSON.
  await expect(p.getByLabel("item 2", { exact: true })).toContainText("npm run polish");
  await expect(page.getByLabel("reports", { exact: true })).toContainText("The plan.");
  await expect(page.getByLabel("reports", { exact: true })).not.toContainText("items");
  // The third becomes a cron entry: not without a schedule.
  const third = p.getByLabel("item 3", { exact: true });
  await third.getByRole("button", { name: "edit" }).click();
  await third.getByLabel("item kind").selectOption("cron");
  await p.getByRole("button", { name: "create the ticked ones" }).click();
  await expect(p).toContainText("Check the page: a cron entry needs a schedule");
  await third.getByLabel("schedule").fill("0 4 * * 1");
  await p.getByRole("button", { name: "create the ticked ones" }).click();
  await expect(p).toContainText("✓ task Polish the home page started");
  await expect(p).toContainText("✓ cron entry");
  await expect(p.locator(".err")).toHaveCount(0);
  await expect(p).not.toContainText("page-polish written");
});

test("a design without a readable proposal is asked for it again", async ({ page }) => {
  await page.goto("/#/project/site/new");
  await page.getByText("design it with the guide").click();
  await page.getByLabel("goal").fill("garbled");
  await page.getByRole("button", { name: "design", exact: true }).click();
  const p = page.getByLabel("proposal");
  await expect(p).toContainText("no proposal that can be read");
  await p.getByRole("button", { name: "ask it to write the proposal again" }).click();
  await expect(p.getByLabel("item 1", { exact: true })).toContainText("task: Check it");
});

test("notification channels are added without showing their token, switched off and removed", async ({ page }) => {
  await page.goto("/#/settings");
  await page.getByLabel("channel name").fill("telegram");
  await page.getByLabel("channel URL").fill("tgram://123:SECRETTOKEN/261988314");
  await page.getByLabel("failed", { exact: true }).check();
  await page.getByRole("button", { name: "add the channel" }).click();
  const row = page.getByLabel("channel telegram");
  await expect(row).toContainText("tgram://…988314");
  await expect(row).toContainText("failed");
  await expect(page.locator("body")).not.toContainText("SECRETTOKEN");
  await row.getByLabel("telegram on").uncheck();
  await page.reload();
  await expect(page.getByLabel("telegram on")).not.toBeChecked();
  page.once("dialog", (d) => d.accept());
  await page.getByLabel("channel telegram").getByRole("button", { name: "remove" }).click();
  await expect(page.getByLabel("channel telegram")).toHaveCount(0);
});

test("a plugin is added from git, its skills reach projects, it updates and goes", async ({ page }) => {
  await page.goto("/#/settings");
  await page.getByLabel("plugin URL").fill(`file://${pluginSrc}`);
  await page.getByRole("button", { name: "add the plugin" }).click();
  const row = page.getByLabel("plugin plugin-src");
  await expect(row).toContainText("hello");
  await expect(row).toContainText("tone");
  await page.goto("/#/project/site/skills");
  await expect(page.locator("body")).toContainText("plugin:plugin-src");
  await expect(page.getByLabel("rule tone")).toContainText("always");
  // Its rule switched off: no task gets it, and it stays off.
  await page.goto("/#/settings");
  await page.getByLabel("plugin plugin-src").getByText("1 rules").click();
  await page.getByLabel("plugin rule tone").uncheck();
  await expect(page.getByLabel("plugin plugin-src")).toContainText("1 off");
  await page.goto("/#/project/site/skills");
  await expect(page.locator("body")).toContainText("plugin:plugin-src");
  await expect(page.getByLabel("rule tone")).toHaveCount(0);
  // A new commit upstream: update moves onto it.
  execSync(`git -C ${pluginSrc} -c user.email=t@t -c user.name=t commit -q --allow-empty -m second`);
  await page.goto("/#/settings");
  await page.getByLabel("plugin plugin-src").getByRole("button", { name: "update" }).click();
  await expect(page.getByLabel("plugin plugin-src")).toContainText("updated");
  await page.getByRole("button", { name: "update all" }).click();
  await expect(page.getByLabel("plugin plugin-src")).toContainText("up to date");
  await expect(page.getByLabel("plugin plugin-src")).toContainText("1 off");
  await page.getByLabel("plugin plugin-src").getByText("1 rules").click();
  await page.getByLabel("plugin rule tone").check();
  await expect(page.getByLabel("plugin plugin-src")).not.toContainText("off");
  page.once("dialog", (d) => d.accept());
  await page.getByLabel("plugin plugin-src").getByRole("button", { name: "remove" }).click();
  await expect(page.getByLabel("plugin plugin-src")).toHaveCount(0);
});

test("a persistent worktree is made, a task starts in it, it merges into main and goes", async ({ page }) => {
  await page.goto("/#/project/site/worktrees");
  await page.getByLabel("worktree name").fill("next");
  await page.getByRole("button", { name: "make a worktree" }).click();
  const card = page.getByLabel("worktree next");
  await expect(card).toContainText("next → main");
  await card.getByLabel("sharing of next").selectOption("exclusive");
  await expect(card.getByLabel("sharing of next")).toHaveValue("exclusive");
  await page.goto("/#/project/site/new");
  await page.getByLabel("works in").selectOption("next");
  await page.getByLabel("title", { exact: true }).fill("Feature work");
  await page.getByLabel("what to do").fill("go");
  await page.getByRole("button", { name: "start" }).click();
  await expect(page.getByRole("heading", { name: "Feature work" })).toBeVisible();
  await expect(page.getByRole("link", { name: "next", exact: true })).toBeVisible();
  await expect(page.locator(".reports")).toContainText("Done: Feature work");
  // A commit there makes it ahead of main; the person merges it.
  const w = (await (await page.request.get("/api/projects/site/worktrees/next")).json()).worktree;
  execSync(`echo x > wt.txt && git add wt.txt && git -c user.email=t@t -c user.name=t commit -q -m wt`, { cwd: w.path, shell: "/bin/sh" });
  await page.goto("/#/project/site/worktrees");
  await expect(card).toContainText("1 ahead");
  await card.getByRole("button", { name: /next/ }).first().click();
  await expect(card.getByRole("link", { name: "Feature work" })).toBeVisible();
  await expect(card).toContainText("wt.txt");
  await card.getByRole("button", { name: "merge into main" }).click();
  await expect(card).toContainText("merged next into main");
  page.once("dialog", (d) => d.accept());
  await card.getByRole("button", { name: "remove" }).click();
  await expect(page.getByText("no persistent worktrees yet")).toBeVisible();
});

test("a service is added, comes up, shows its output, stops and goes", async ({ page }) => {
  await page.goto("/#/project/site/services");
  await page.getByLabel("service name").fill("web");
  await page.getByLabel("service command").fill("echo serving; exec sleep 600");
  await page.getByLabel("health check").fill("true");
  await page.getByLabel("check interval").fill("1s");
  await page.getByRole("button", { name: "save" }).click();
  const card = page.getByLabel("service web");
  await expect(card.getByLabel("status of web")).toHaveText("up", { timeout: 15000 });
  await card.getByRole("button", { name: "output" }).click();
  await expect(card.getByLabel("output of web")).toContainText("serving");
  await card.getByRole("button", { name: "starts" }).click();
  await expect(card.locator(".runs summary").first()).toContainText("up at");
  await card.getByRole("button", { name: "stop", exact: true }).click();
  await expect(card.getByLabel("status of web")).toHaveText("stopped", { timeout: 15000 });
  await expect(card.getByRole("button", { name: "start", exact: true })).toBeVisible();
  page.once("dialog", (d) => d.accept());
  await card.getByRole("button", { name: "remove" }).click();
  await expect(page.getByText("no services")).toBeVisible();
});

test("a branch's sync settings are kept on the sync tab", async ({ page }) => {
  await page.goto("/#/project/site/sync");
  const card = page.getByLabel("sync of .");
  await expect(card).toContainText("project folder");
  await expect(card.getByLabel("sync status of .")).toHaveText("off");
  await card.getByLabel("sync interval of .").fill("2h");
  await card.getByLabel("sync strategy of .").selectOption("merge");
  const saved = page.waitForResponse((r) => r.request().method() === "PUT" && r.url().includes("/sync/"));
  await card.getByRole("button", { name: "save" }).click();
  expect((await saved).ok()).toBe(true);
  await page.reload();
  await expect(card.getByLabel("sync interval of .")).toHaveValue("120m");
  await expect(card.getByLabel("sync strategy of .")).toHaveValue("merge");
  const kept = (await (await page.request.get("/api/projects/site/sync")).json())[0].kept;
  expect([kept.enabled, kept.every, kept.strategy]).toEqual([false, 7200, "merge"]);
});

test("a stuck foreground command is moved to the background and the task goes on", async ({ page }) => {
  const rules = await (await page.request.get("/api/projects/site/rules")).json();
  await page.request.put("/api/projects/site/rules", { data: [{ tool: "shell.exec*", command: "sleep *", action: "allow" }, ...rules] });
  await startTask(page, "Hang on");
  const box = page.getByLabel(/^foreground command /);
  await expect(box).toContainText("sleep 600");
  await expect(box).toContainText(/running \d+s/);
  await box.getByRole("button", { name: "move shell to background" }).click();
  await expect(page.locator(".reports")).toContainText("moved on");
  await expect(box).toHaveCount(0);
  // The job goes on in the background.
  const id = page.url().split("/task/")[1];
  const jobs = await (await page.request.get(`/api/tasks/${id}/jobs`)).json();
  expect(jobs.some((j) => j.backgrounded && j.ended == null)).toBe(true);
  for (const j of jobs) await page.request.post(`/api/jobs/${j.id}/kill`, { data: {} });
  await page.request.put("/api/projects/site/rules", { data: rules });
});

test("↓ follows the bottom as the page grows, until the person scrolls", async ({ page }) => {
  await page.goto("/#/inbox");
  const grow = (px) => page.evaluate((h) => document.querySelector("main").insertAdjacentHTML("beforeend", `<div class="grown" style="height:${h}px"></div>`), px);
  const atBottom = () => page.evaluate(() => Math.abs(scrollY + innerHeight - document.documentElement.scrollHeight) < 4);
  await grow(3000);
  const down = page.getByRole("button", { name: "to the bottom" });
  await down.click();
  await expect(down).toHaveAttribute("aria-pressed", "true");
  await expect.poll(atBottom).toBe(true);
  await grow(2000);
  await expect.poll(atBottom).toBe(true);
  // The person scrolls up: no more following.
  await page.mouse.move(200, 200);
  await page.mouse.wheel(0, -600);
  await expect(down).toHaveAttribute("aria-pressed", "false");
  await grow(2000);
  await page.waitForTimeout(300);
  expect(await atBottom()).toBe(false);
});

test("reagent's docs are readable", async ({ page }) => {
  await page.goto("/#/docs");
  await expect(page.locator("article")).toContainText("reagent runs coding agents");
  await page.getByRole("link", { name: "reagent-subtasks" }).click();
  await expect(page.locator("article h1")).toHaveText("Subtasks");
});

test("logging out locks the API", async ({ page }) => {
  await page.getByRole("button", { name: "log out" }).click();
  await expect(page.getByLabel("password")).toBeVisible();
  expect((await page.request.get("/api/projects")).status()).toBe(401);
});
