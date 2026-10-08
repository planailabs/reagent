// A scripted OpenAI-compatible model for the browser tests. What a task does
// depends on its title: "Ask …" asks a question, "Touch …" runs a command the
// policy asks about, "Run …" starts a background job; anything else answers.
import http from "node:http";

const port = Number(process.argv[2] || 8791);

const chunk = (o) => `data: ${JSON.stringify(o)}\n\n`;
const usage = { prompt_tokens: 100, completion_tokens: 20 };
const text = (s) => chunk({ choices: [{ delta: { content: s } }] }) + chunk({ choices: [{ delta: {}, finish_reason: "stop" }], usage }) + "data: [DONE]\n\n";
const call = (id, name, args) =>
  chunk({ choices: [{ delta: { tool_calls: [{ index: 0, id, type: "function", function: { name, arguments: JSON.stringify(args) } }] } }] }) +
  chunk({ choices: [{ delta: {}, finish_reason: "tool_calls" }], usage }) +
  "data: [DONE]\n\n";

function reply(body) {
  const msgs = body.messages;
  const first = msgs.find((m) => m.role === "user" && (m.content || "").startsWith("Task: "));
  const title = first ? first.content.split("\n")[0].slice(6) : "";
  const tools = msgs.filter((m) => m.role === "tool");
  const last = tools[tools.length - 1]?.content || "";
  const all = msgs.map((m) => m.content || "").join("\n");
  if (title.startsWith("Todo")) {
    if (!tools.length) return call("p1", "todo__todo_add", { items: ["look around", "do the thing"] });
    if (tools.length === 1) return call("p2", "todo__todo_update", { id: 1, status: "done" });
    return text("planned and started");
  }
  if (title.startsWith("Remember")) {
    if (!tools.length) return call("w1", "wm__wm_set", { key: "pr", value: 42 });
    if (tools.length === 1) return call("w2", "wm__wm_set", { key: "plan", value: { step: 2 } });
    return text("remembered");
  }
  if (title.startsWith("Design:")) {
    if (!tools.length) return call("d1", "ask__ask", { question: "Which page?", options: ["home", "about"] });
    const items = [
      { type: "task", title: "Polish the page", prompt: "Make the page **nicer**.", why: "once now" },
      { type: "skill", name: "page-polish", description: "How to polish a page", body: "1. Look.\n2. Polish.", why: "it comes up again" },
    ];
    return text("The plan.\n```json\n" + JSON.stringify({ note: `the ${last} page`, items }) + "\n```");
  }
  if (title.startsWith("Ask")) {
    if (!tools.length) return call("q1", "ask__ask", { question: "which color?", options: ["red", "blue"] });
    return text(`you chose ${last}`);
  }
  if (title.startsWith("Touch")) {
    if (!tools.length) return call("t1", "shell__exec", { cmd: "touch from-ui.txt" });
    return text("touched");
  }
  if (title.startsWith("Run")) {
    if (!tools.length) return call("r1", "shell__exec_bg", { cmd: "for i in 1 2 3; do echo tick $i; sleep 1; done", name: "ticker" });
    if (all.includes("[job ")) return text("ticker finished");
    return text("started the ticker");
  }
  return text(`Done: ${title}`);
}

http
  .createServer((req, res) => {
    let data = "";
    req.on("data", (c) => (data += c));
    req.on("end", () => {
      const body = JSON.parse(data || "{}");
      res.writeHead(200, { "content-type": "text/event-stream" });
      res.end(reply(body));
    });
  })
  .listen(port, "127.0.0.1");
