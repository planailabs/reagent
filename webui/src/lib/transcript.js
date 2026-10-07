// A full transcript (`transcript` with full) in sections: messages the model
// still sees, and each compaction's messages, which it now sees only as the
// summary.

/**
 * The messages in order, cut into sections: `{compacted: null, items}` or
 * `{compacted: {summary}, items}`; each item is `{m, i}` (the message and
 * its index in the transcript).
 */
export function sections(messages, compacted = []) {
  const out = [];
  const starts = new Map(compacted.map((c) => [c.from, c]));
  let i = 0;
  while (i < messages.length) {
    const c = starts.get(i);
    const end = c ? Math.min(c.to, messages.length) : Math.min(...compacted.map((c) => c.from).filter((f) => f > i), messages.length);
    out.push({ compacted: c ? { summary: c.summary } : null, items: messages.slice(i, end).map((m, k) => ({ m, i: i + k })) });
    i = end > i ? end : i + 1;
  }
  return out;
}
