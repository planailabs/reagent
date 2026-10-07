import test from "node:test";
import assert from "node:assert/strict";
import { ago, applyAgent, budget, flatten, glyph, messageText, money, tree, waitText } from "../src/lib/format.js";
import { sections } from "../src/lib/transcript.js";

test("states have glyphs and waits words", () => {
  assert.equal(glyph("done"), "✓");
  assert.equal(glyph("nonsense"), "·");
  assert.equal(waitText({ state: "waiting", wait: { kind: "approval", call: { function: { name: "shell__exec" } } } }), "approve shell.exec");
  assert.equal(waitText({ state: "waiting", wait: { kind: "question", question: "red?" } }), "answer: red?");
  assert.equal(waitText({ state: "waiting", wait: { kind: "merge", branch: "reagent/x", base: "main" } }), "merge reagent/x into main");
  assert.equal(waitText({ state: "failed" }), "failed");
  assert.equal(waitText({ state: "running" }), "");
});

test("times and money", () => {
  assert.equal(ago(100, 130), "30s");
  assert.equal(ago(100, 100 + 7200), "2h");
  assert.equal(ago(0), "");
  assert.equal(money(0.004), "<0.01");
  assert.equal(money(1.5), "1.50");
});

test("subtasks hang under their parents", () => {
  const ts = [
    { id: "a", created: 1 },
    { id: "b", parent: "a", created: 2 },
    { id: "c", created: 3 },
    { id: "d", parent: "gone", created: 4 },
  ];
  const t = tree(ts);
  assert.deepEqual(t.map((x) => x.id), ["d", "c", "a"]);
  assert.deepEqual(flatten(t).map(({ t, depth }) => `${t.id}${depth}`), ["d0", "c0", "a0", "b1"]);
});

test("messages, streams and budgets", () => {
  assert.equal(messageText({ tool_calls: [{ function: { name: "fs__read", arguments: '{"path":"a"}' } }] }), '→ fs.read({"path":"a"})');
  const s = {};
  assert.equal(applyAgent(s, { agent: "x", event: { type: "llm_delta", delta: { content: "he" } } }), false);
  applyAgent(s, { agent: "x", event: { type: "llm_delta", delta: { content: "llo" } } });
  assert.equal(s.x, "hello");
  assert.equal(applyAgent(s, { agent: "x", event: { type: "llm_done" } }), true);
  assert.equal(s.x, undefined);
  assert.deepEqual(budget({ tokens: "1000", cost: "", minutes: null, daily_cost: "2.5" }), { tokens: 1000, daily_cost: 2.5 });
  assert.equal(sections([{}, {}, {}], [{ from: 0, to: 2, summary: "s" }]).length, 2);
});
