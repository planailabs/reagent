// Playwright ends serve.sh hard; the supervisor it started (its own process
// group, meant to outlive reagent) is stopped here.
import { execFileSync } from "node:child_process";

export default function teardown() {
  const root = new URL("../..", import.meta.url).pathname;
  try {
    execFileSync(`${root}target/debug/reagent`, ["--data", `${root}target/tmp/e2e/data`, "supervisor", "--stop"], { stdio: "ignore" });
  } catch {
    /* not running */
  }
}
