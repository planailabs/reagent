<script setup>
import { inject, onMounted, onUnmounted, ref, watch } from "vue";
import { get, post } from "../lib/api.js";

const props = defineProps({ task: { type: String, required: true } });
const live = inject("live");
const jobs = ref([]);
const open = ref(null);
const out = ref("");
const error = ref("");
let es = null;

async function load() {
  jobs.value = await get(`/api/tasks/${props.task}/jobs`).catch((e) => ((error.value = e.message), []));
}

async function show(j) {
  es?.close();
  open.value = j.id;
  const o = await get(`/api/jobs/${j.id}/output?tail=500`);
  out.value = o.text.map((l) => l.text).join("\n");
  if (j.ended == null) {
    // Live from here on.
    es = new EventSource(`/api/jobs/${j.id}/stream`);
    es.addEventListener("output", (e) => (out.value += JSON.parse(e.data).text));
    es.addEventListener("exit", () => {
      es.close();
      load();
    });
  }
}

async function background(j) {
  await post(`/api/jobs/${j.id}/background`).catch((e) => (error.value = e.message));
  await load();
}

async function kill(j, signal) {
  await post(`/api/jobs/${j.id}/kill`, { signal }).catch((e) => (error.value = e.message));
  await load();
}

const how = (j) => (j.ended == null ? (j.fg ? "running · the task waits for it" : "running") : j.exit != null ? `exit ${j.exit}` : j.signal != null ? `signal ${j.signal}` : "ended");
onMounted(load);
onUnmounted(() => es?.close());
watch(() => live.tick, load);
</script>

<template>
  <section>
    <p v-if="error" class="err">{{ error }}</p>
    <div v-if="!jobs.length" class="dim">no commands yet</div>
    <div v-for="j in jobs" :key="j.id" class="job row">
      <a href="#" :class="{ hi: open === j.id }" @click.prevent="show(j)">{{ j.id }}</a>
      <code class="grow">{{ j.name || j.cmd }}</code>
      <span class="dim">{{ how(j) }}</span>
      <button v-if="j.ended == null && j.fg" title="let the task go on; the command keeps running" @click="background(j)">to background</button>
      <button v-if="j.ended == null" @click="kill(j)">stop</button>
      <button v-if="j.ended == null" @click="kill(j, 9)">kill</button>
    </div>
    <pre v-if="open" class="output" aria-label="output">{{ out }}</pre>
  </section>
</template>
