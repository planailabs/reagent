// Pure helpers for showing tasks. Tested in test/format.test.js.

/** A glyph per state (state is never shown by colour alone). */
export function glyph(state) {
  return { running: "▶", waiting: "?", paused: "‖", done: "✓", failed: "✗", cancelled: "–" }[state] ?? "·";
}

/** What a waiting task waits for, in words. */
export function waitText(t) {
  const w = t?.wait;
  if (!w) return t?.state === "failed" ? "failed" : "";
  switch (w.kind) {
    case "approval":
      return `approve ${toolName(w.call?.function?.name)}`;
    case "question":
      return `answer: ${w.question}`;
    case "merge":
      return `merge ${w.branch} into ${w.base}`;
    case "budget":
      return "over budget";
    default:
      return w.kind;
  }
}

/** `fs__read` → `fs.read`. */
export function toolName(n) {
  return (n || "").replace("__", ".");
}

/** Seconds ago, short. */
export function ago(unix, now = Date.now() / 1000) {
  if (!unix) return "";
  const s = Math.max(0, Math.round(now - unix));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

export function money(x) {
  return (x ?? 0) < 0.01 && x > 0 ? "<0.01" : (x ?? 0).toFixed(2);
}

/** Tasks as a tree: roots with `kids`, newest first; a subtask whose parent isn't listed is a root. */
export function tree(tasks) {
  const byId = new Map(tasks.map((t) => [t.id, { ...t, kids: [] }]));
  const roots = [];
  for (const t of byId.values()) {
    const p = t.parent && byId.get(t.parent);
    (p ? p.kids : roots).push(t);
  }
  const order = (a, b) => b.created - a.created;
  const sort = (list) => {
    list.sort(order);
    list.forEach((t) => sort(t.kids));
    return list;
  };
  return sort(roots);
}

/** Flattens a tree for display, with depth. */
export function flatten(nodes, depth = 0, out = []) {
  for (const n of nodes) {
    out.push({ t: n, depth });
    flatten(n.kids, depth + 1, out);
  }
  return out;
}

/** A message's text for the transcript: its content, or its tool calls. */
export function messageText(m) {
  if (m.content) return m.content;
  if (m.tool_calls?.length) return m.tool_calls.map((c) => `→ ${toolName(c.function.name)}(${c.function.arguments})`).join("\n");
  return "";
}

/** Folds an agent event into streamed text per agent; true when the transcript should reload. */
export function applyAgent(streams, n) {
  const t = n?.event?.type;
  if (t === "llm_delta" && n.event.delta?.content) {
    streams[n.agent] = ((streams[n.agent] || "") + n.event.delta.content).slice(-4000);
    return false;
  }
  if (t === "llm_done" || t === "llm_aborted" || t === "llm_failed") delete streams[n.agent];
  return t !== "llm_delta";
}

/** A budget from a form's strings: empty fields are left out. */
export function budget(form) {
  const out = {};
  for (const k of ["tokens", "minutes"]) if (form[k] !== "" && form[k] != null) out[k] = parseInt(form[k], 10);
  for (const k of ["cost", "daily_cost"]) if (form[k] !== "" && form[k] != null) out[k] = parseFloat(form[k]);
  return out;
}
