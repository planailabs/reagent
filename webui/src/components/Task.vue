<script setup>
import { computed, inject, onMounted, ref, watch } from "vue";
import { get, post } from "../lib/api.js";
import { ago, budget, glyph, messageText, money, toolName, waitText } from "../lib/format.js";
import { sections } from "../lib/transcript.js";
import TaskRow from "./TaskRow.vue";
import Jobs from "./Jobs.vue";
import Terminal from "./Terminal.vue";

const props = defineProps({ id: { type: String, required: true } });
const live = inject("live");
const t = ref(null);
const tr = ref(null);
const diff = ref(null);
const msg = ref("");
const answerText = ref("");
const rejectText = ref("");
const raiseForm = ref({ tokens: "", cost: "", minutes: "" });
const error = ref("");
const tab = ref("transcript");
const ptys = ref([]);
const term = ref(null);

async function load() {
  try {
    t.value = await get(`/api/tasks/${props.id}`);
    if (t.value.agent) tr.value = await get(`/api/tasks/${props.id}/transcript?full=true`).catch(() => tr.value);
    diff.value = t.value.worktree ? await get(`/api/tasks/${props.id}/diff`).catch(() => null) : null;
    ptys.value = await get(`/api/tasks/${props.id}/ptys`).catch(() => []);
  } catch (e) {
    error.value = e.message;
  }
}

async function act(path, body = {}) {
  error.value = "";
  try {
    const r = await post(`/api/tasks/${props.id}/${path}`, body);
    await load();
    return r;
  } catch (e) {
    error.value = e.message;
  }
}

function cancelTask() {
  if (confirm("Cancel this task (and its commands)?")) act("cancel");
}

async function send() {
  if (!msg.value.trim()) return;
  if (await act("message", { text: msg.value }) !== undefined) msg.value = "";
}

async function answer(text) {
  if (await act("answer", { text }) !== undefined) answerText.value = "";
}

async function openTerminal() {
  try {
    const p = await post(`/api/tasks/${props.id}/ptys`, {});
    ptys.value = await get(`/api/tasks/${props.id}/ptys`);
    term.value = p.id;
    tab.value = "terminals";
  } catch (e) {
    error.value = e.message;
  }
}

const todos = computed(() => live.todos[props.id] ?? t.value?.todos ?? []);
const todoMark = (s) => ({ in_progress: "[~]", done: "[x]", cancelled: "[-]" })[s] ?? "[ ]";
const streaming = computed(() => (t.value?.agent ? live.streams[t.value.agent] : ""));
const call = computed(() => t.value?.wait?.kind === "approval" ? t.value.wait.call : null);
const args = computed(() => {
  try {
    return JSON.stringify(JSON.parse(call.value?.function?.arguments || "{}"), null, 2);
  } catch {
    return call.value?.function?.arguments;
  }
});

onMounted(load);
watch(() => props.id, load);
watch(() => live.tasks[props.id], (x) => x && ((t.value = { ...t.value, ...x }), load()));
watch(() => (t.value?.agent ? live.agentTick[t.value.agent] : 0), () => load());
</script>

<template>
  <section v-if="t">
    <div class="row">
      <span class="glyph big" :title="t.state">{{ glyph(t.state) }}</span>
      <h2 class="title">{{ t.title }}</h2>
      <a :href="`#/project/${t.project}`" class="dim">{{ t.project }}</a>
      <span class="dim">{{ t.state }} · {{ t.profile }} · {{ t.tokens }} tokens · {{ money(t.cost) }} · {{ ago(t.created) }} ago · {{ t.origin }}</span>
    </div>
    <div v-if="t.parent" class="dim">subtask of <a :href="`#/task/${t.parent}`">{{ t.parent.slice(0, 8) }}</a></div>
    <div class="dim">in {{ t.cwd }}<span v-if="t.worktree"> · worktree {{ t.worktree.branch }} (from {{ t.worktree.base }})</span></div>

    <div class="row controls">
      <button :disabled="!['running', 'waiting'].includes(t.state)" @click="act('pause', { mode: 'quick' })">pause</button>
      <button :disabled="!['running', 'waiting'].includes(t.state)" title="finish this turn, then stop" @click="act('pause', { mode: 'safe' })">pause after this turn</button>
      <button :disabled="t.state !== 'paused'" @click="act('resume')">resume</button>
      <button v-if="t.state === 'failed'" @click="act('retry')">retry from the failure</button>
      <button :disabled="['done', 'cancelled'].includes(t.state)" @click="cancelTask">cancel</button>
      <button @click="openTerminal">open a terminal</button>
    </div>
    <p v-if="error" class="err" role="alert">{{ error }}</p>

    <!-- What it waits for -->
    <div v-if="call" class="err wait" aria-label="approval">
      <div>wants to run <span class="hi">{{ toolName(call.function.name) }}</span></div>
      <pre>{{ args }}</pre>
      <div class="row">
        <button @click="act('approve', { call_id: call.id, approved: true })">allow once</button>
        <button title="adds a rule allowing this tool with this command" @click="act('approve', { call_id: call.id, approved: true, always: true })">always allow this</button>
        <button @click="act('approve', { call_id: call.id, approved: false })">deny</button>
      </div>
    </div>
    <div v-if="t.wait?.kind === 'question'" class="err wait" aria-label="question">
      <div class="hi">{{ t.wait.question }}</div>
      <div class="row">
        <button v-for="o in t.wait.options ?? []" :key="o" @click="answer(o)">{{ o }}</button>
      </div>
      <form class="row" @submit.prevent="answer(answerText)">
        <input v-model="answerText" class="grow" placeholder="your answer" aria-label="answer" />
        <button type="submit">answer</button>
      </form>
    </div>
    <div v-if="t.wait?.kind === 'merge'" class="err wait" aria-label="merge">
      <div>ready to merge <span class="hi">{{ t.wait.branch }}</span> into <span class="hi">{{ t.wait.base }}</span> ({{ t.wait.ahead }} commits, {{ t.wait.strategy }})</div>
      <pre class="dim">{{ t.wait.stat }}</pre>
      <div class="row">
        <button @click="act('merge', { merge: true })">merge</button>
        <input v-model="rejectText" class="grow" placeholder="what to change first" aria-label="send back" />
        <button @click="act('merge', { merge: false, message: rejectText })">send back</button>
        <button @click="tab = 'diff'">see the diff</button>
      </div>
    </div>
    <div v-if="t.wait?.kind === 'budget'" class="err wait" aria-label="budget">
      <div>over its budget: {{ t.wait.tokens }} tokens, {{ money(t.wait.cost) }}, {{ t.wait.minutes }} min (budget: {{ JSON.stringify(t.budget) }})</div>
      <form class="row" @submit.prevent="act('raise', budget(raiseForm))">
        raise by
        <label>tokens <input v-model="raiseForm.tokens" size="8" aria-label="more tokens" /></label>
        <label>cost <input v-model="raiseForm.cost" size="6" aria-label="more cost" /></label>
        <label>minutes <input v-model="raiseForm.minutes" size="5" aria-label="more minutes" /></label>
        <button type="submit">raise and go on</button>
      </form>
    </div>
    <div v-if="t.state === 'failed'" class="err wait" aria-label="failed">
      <pre>{{ t.report }}</pre>
      <button @click="act('retry')">retry from the failure</button>
    </div>
    <div v-if="t.report && t.state !== 'failed'" class="report">
      <div class="dim">report</div>
      <pre>{{ t.report }}</pre>
    </div>

    <form class="row" @submit.prevent="send">
      <input v-model="msg" class="grow" :placeholder="['done', 'failed'].includes(t.state) ? 'a message goes on with the task' : 'a message (read before its next step)'" aria-label="message" />
      <button type="submit">send</button>
    </form>

    <div v-if="todos.length" class="todos" aria-label="todo list">
      <h2>todo <span class="dim">{{ todos.filter((x) => x.status === "done").length }} of {{ todos.filter((x) => x.status !== "cancelled").length }} done</span></h2>
      <div v-for="x in todos" :key="x.id" :class="['todo', x.status]"><span class="mark">{{ todoMark(x.status) }}</span> {{ x.text }}</div>
    </div>

    <div v-if="t.subtasks?.length">
      <h2>subtasks</h2>
      <TaskRow v-for="k in t.subtasks" :key="k.id" :t="k" />
    </div>

    <nav class="row tabs">
      <a v-for="x in ['transcript', 'jobs', 'terminals', 'diff']" :key="x" href="#" :class="{ on: tab === x }" @click.prevent="tab = x">{{ x }}</a>
    </nav>

    <div v-if="tab === 'transcript' && tr" class="transcript">
      <template v-for="(s, k) in sections(tr.messages, tr.compacted)" :key="k">
        <details v-if="s.compacted" class="compacted">
          <summary>summarised: {{ s.items.length }} messages (a checkpoint in the project's memory)</summary>
          <pre class="summary">{{ s.compacted.summary }}</pre>
          <div v-for="{ m, i } in s.items" :key="i" :class="['msg', m.role]">
            <div class="who">{{ m.role }}<span v-if="m.tool_call_id"> · {{ m.tool_call_id }}</span></div>
            <pre>{{ messageText(m) }}</pre>
          </div>
        </details>
        <template v-else>
          <div v-for="{ m, i } in s.items" :key="i" :class="['msg', m.role]">
            <div class="who">{{ m.role }}<span v-if="m.tool_call_id"> · {{ m.tool_call_id }}</span></div>
            <pre>{{ messageText(m) }}</pre>
          </div>
        </template>
      </template>
      <div v-if="streaming" class="msg assistant partial">
        <div class="who">assistant · writing</div>
        <pre>{{ streaming }}</pre>
      </div>
    </div>
    <Jobs v-else-if="tab === 'jobs'" :task="id" />
    <div v-else-if="tab === 'terminals'">
      <div class="row">
        <a v-for="p in ptys" :key="p.id" href="#" :class="{ on: term === p.id }" @click.prevent="term = p.id">{{ p.id }} {{ p.cmd || "shell" }}{{ p.alive ? "" : " (ended)" }}</a>
        <span v-if="!ptys.length" class="dim">no terminals</span>
      </div>
      <Terminal v-if="term" :key="term" :pty="term" @closed="(term = null), load()" />
    </div>
    <div v-else-if="tab === 'diff'">
      <p v-if="!diff" class="dim">it doesn't work in a worktree</p>
      <template v-else>
        <pre class="dim">{{ diff.status }}</pre>
        <pre>{{ diff.stat }}</pre>
        <pre class="diff">{{ diff.diff }}</pre>
      </template>
    </div>
  </section>
  <p v-else-if="error" class="err">{{ error }}</p>
</template>
