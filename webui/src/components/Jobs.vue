<script setup>
import { inject, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { get, post } from "../lib/api.js";

const props = defineProps({ task: { type: String, required: true } });
const live = inject("live");
const jobs = ref([]);
const open = ref(null);
const lines = ref([]);
const first = ref(1);
const error = ref("");
const logEl = ref(null);
let es = null;

async function load() {
  jobs.value = await get(`/api/tasks/${props.task}/jobs`).catch((e) => ((error.value = e.message), []));
  if (open.value) open.value = jobs.value.find((j) => j.id === open.value.id) ?? open.value;
}

const atBottom = () => !logEl.value || logEl.value.scrollHeight - logEl.value.scrollTop - logEl.value.clientHeight < 40;
async function toBottom() {
  await nextTick();
  if (logEl.value) logEl.value.scrollTop = logEl.value.scrollHeight;
}

/** Opens a job's log: its last 1000 lines, then live while it runs. */
async function show(j) {
  close();
  open.value = j;
  error.value = "";
  try {
    const o = await get(`/api/jobs/${j.id}/output?tail=1000`);
    lines.value = o.text.map((l) => l.text);
    first.value = o.from;
  } catch (e) {
    error.value = e.message;
    lines.value = [];
  }
  toBottom();
  if (j.ended == null) {
    es = new EventSource(`/api/jobs/${j.id}/stream`);
    es.addEventListener("output", (e) => {
      const follow = atBottom();
      const text = JSON.parse(e.data).text;
      // Output comes in chunks: the first piece finishes the last line.
      const parts = text.split("\n");
      if (lines.value.length) lines.value[lines.value.length - 1] += parts.shift();
      lines.value.push(...parts);
      if (follow) toBottom();
    });
    es.addEventListener("exit", () => {
      es?.close();
      load();
    });
  }
}

/** The 1000 lines before what's shown. */
async function earlier() {
  if (first.value <= 1) return;
  const from = Math.max(1, first.value - 1000);
  const o = await get(`/api/jobs/${open.value.id}/output?from=${from}&to=${first.value - 1}`).catch((e) => ((error.value = e.message), null));
  if (!o) return;
  const h = logEl.value?.scrollHeight ?? 0;
  lines.value = [...o.text.map((l) => l.text), ...lines.value];
  first.value = from;
  await nextTick();
  if (logEl.value) logEl.value.scrollTop += logEl.value.scrollHeight - h;
}

function close() {
  es?.close();
  es = null;
  open.value = null;
}

async function background(j) {
  await post(`/api/jobs/${j.id}/background`).catch((e) => (error.value = e.message));
  await load();
}

async function kill(j, signal) {
  await post(`/api/jobs/${j.id}/kill`, { signal }).catch((e) => (error.value = e.message));
  await load();
}

const how = (j) => (j.ended == null ? (j.fg ? "running · the task waits for it" : "running") : j.lost ? "lost" : j.exit != null ? `exit ${j.exit}` : j.signal != null ? `signal ${j.signal}` : "ended");
const onKey = (e) => e.key === "Escape" && open.value && close();
onMounted(() => {
  load();
  addEventListener("keydown", onKey);
});
onUnmounted(() => {
  close();
  removeEventListener("keydown", onKey);
});
watch(() => live.tick, load);
</script>

<template>
  <section>
    <p v-if="error && !open" class="err">{{ error }}</p>
    <div v-if="!jobs.length" class="dim">no commands yet</div>
    <div v-for="j in jobs" :key="j.id" class="job row" role="button" tabindex="0" :aria-label="`job ${j.id}`" @click="show(j)" @keydown.enter="show(j)">
      <span class="hi">{{ j.id }}</span>
      <code class="grow">{{ j.name || j.cmd }}</code>
      <span class="dim">{{ how(j) }}</span>
    </div>

    <div v-if="open" class="modal-back" @click.self="close">
      <div class="modal" role="dialog" aria-modal="true" :aria-label="`log of ${open.id}`">
        <div class="row">
          <span class="hi">{{ open.id }}</span>
          <span class="dim">{{ how(open) }} · {{ lines.length }} lines shown</span>
          <span class="grow"></span>
          <button v-if="open.ended == null && open.fg" title="let the task go on; the command keeps running" @click="background(open)">to background</button>
          <button v-if="open.ended == null" @click="kill(open)">stop</button>
          <button v-if="open.ended == null" @click="kill(open, 9)">kill</button>
          <button aria-label="close" @click="close">×</button>
        </div>
        <code class="cmd">{{ open.cmd }}</code>
        <p v-if="error" class="err">{{ error }}</p>
        <pre ref="logEl" class="output modal-log" aria-label="output"><button v-if="first > 1" class="earlier" @click="earlier">show earlier lines (from line {{ Math.max(1, first - 1000) }})</button>{{ lines.join("\n") }}</pre>
      </div>
    </div>
  </section>
</template>
