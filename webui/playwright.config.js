// Browser tests against a real reagent and a scripted model (e2e/serve.sh):
// `npm run e2e` inside `nix develop` (Playwright's browsers come from nix).
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "e2e",
  // One reagent: the tests share it, one at a time.
  workers: 1,
  timeout: 60_000,
  use: { baseURL: "http://127.0.0.1:8790", trace: "retain-on-failure", viewport: { width: 1280, height: 860 } },
  webServer: { command: "bash e2e/serve.sh", url: "http://127.0.0.1:8790/api/session", timeout: 900_000, reuseExistingServer: false, stdout: "pipe" },
  globalTeardown: "./e2e/teardown.js",
  reporter: [["list"]],
});
