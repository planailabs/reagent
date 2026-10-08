import { test } from "node:test";
import assert from "node:assert/strict";
import { markdown } from "../src/lib/markdown.js";

test("markdown renders, and raw HTML and script links don't", () => {
  assert.match(markdown("**bold** and `code`"), /<strong>bold<\/strong> and <code>code<\/code>/);
  assert.match(markdown("```\nx < y\n```"), /<pre><code>x &lt; y/);
  assert.doesNotMatch(markdown("<img src=x onerror=alert(1)>"), /<img/);
  assert.doesNotMatch(markdown("[x](javascript:alert(1))"), /href="javascript/);
  assert.match(markdown("see https://example.com"), /<a href="https:\/\/example.com" target="_blank" rel="noopener noreferrer">/);
});

test("markdown as plain text", async () => {
  const { plain } = await import("../src/lib/markdown.js");
  assert.equal(plain("**Merge** `main`?\n\n- one\n- [two](https://x.y)"), "Merge main?\n\n• one\n• two");
  assert.equal(plain("```\na < b\n```"), "a < b");
});
