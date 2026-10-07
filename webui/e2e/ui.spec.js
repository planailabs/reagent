import { expect, test } from "@playwright/test";

const project = new URL("../../target/tmp/e2e/project", import.meta.url).pathname;

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

test("a task runs and reports", async ({ page }) => {
  await startTask(page, "Hello there");
  await expect(page.locator(".report")).toContainText("Done: Hello there");
  await page.getByRole("link", { name: "projects" }).click();
  await page.getByRole("link", { name: "Site" }).click();
  await page.getByLabel("finished ones too").check();
  await expect(page.locator(".task-row", { hasText: "Hello there" })).toContainText("✓");
});

test("a call the policy asks about is approved from the inbox", async ({ page }) => {
  await startTask(page, "Touch a file");
  await page.getByRole("link", { name: /inbox/ }).click();
  await page.locator(".task-row", { hasText: "Touch a file" }).click();
  const box = page.getByLabel("approval");
  await expect(box).toContainText("shell.exec");
  await expect(box).toContainText("touch from-ui.txt");
  await box.getByRole("button", { name: "allow once" }).click();
  await expect(page.locator(".report")).toContainText("touched");
});

test("a task's todo list shows", async ({ page }) => {
  await startTask(page, "Todo list");
  await expect(page.locator(".report")).toContainText("planned and started");
  const list = page.getByLabel("todo list");
  await expect(list).toContainText("1 of 2 done");
  await expect(list.locator(".todo.done")).toContainText("look around");
  await expect(list.locator(".todo.pending")).toContainText("do the thing");
});

test("a question is answered with an option", async ({ page }) => {
  await startTask(page, "Ask a color");
  const box = page.getByLabel("question");
  await expect(box).toContainText("which color?");
  await box.getByRole("button", { name: "blue" }).click();
  await expect(page.locator(".report")).toContainText("you chose blue");
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
  await expect(page.locator(".report")).toContainText("ticker finished", { timeout: 20_000 });
});

test("a terminal takes keys", async ({ page }) => {
  await startTask(page, "Terminal host");
  await expect(page.locator(".report")).toContainText("Done");
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
  await expect(page.locator(".report")).toContainText("Done");
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

test("a task moves onto another model", async ({ page }) => {
  await startTask(page, "Switch me");
  await expect(page.locator(".report")).toContainText("Done: Switch me");
  await expect(page.locator(".row", { hasText: "Switch me" }).first()).toContainText("default");
  await page.getByLabel("switch model").selectOption("big");
  await page.getByRole("button", { name: "switch" }).click();
  await expect(page.locator("main")).toContainText("· big ·");
  const t = await (await page.request.get("/api/tasks?project=site&limit=50")).json();
  expect(t.find((x) => x.title === "Switch me").profile).toBe("big");
});

test("logging out locks the API", async ({ page }) => {
  await page.getByRole("button", { name: "log out" }).click();
  await expect(page.getByLabel("password")).toBeVisible();
  expect((await page.request.get("/api/projects")).status()).toBe(401);
});
