<script setup>
// A design task's proposal: its items, each editable, to tick and create.
import { onMounted, ref, watch } from "vue";
import { get, post } from "../lib/api.js";
import { markdown } from "../lib/markdown.js";

const props = defineProps({ task: { type: Object, required: true } });
const proposal = ref(null);
const chosen = ref([]);
const editing = ref({});
const results = ref([]);
const error = ref("");

async function load() {
  proposal.value = await get(`/api/tasks/${props.task.id}/proposal`).catch((e) => ((error.value = e.message), null));
  chosen.value = (proposal.value?.items ?? []).map(() => true);
}

async function create() {
  error.value = "";
  const items = proposal.value.items.filter((_, i) => chosen.value[i]);
  if (!items.length) return;
  try {
    results.value = await post("/api/design/create", { project: props.task.project, items });
  } catch (e) {
    error.value = e.message;
  }
}

/** A task item, into the new-task form to change more there. */
function openInForm(it) {
  try {
    sessionStorage.setItem(`reagent-proposal-${props.task.project}`, JSON.stringify(it));
  } catch {}
  location.hash = `#/project/${props.task.project}/new`;
}

const label = (it) =>
  ({ task: `task: ${it.title}`, cron: `cron entry (${it.expr} ${it.tz}): ${it.title}`, trigger: `${it.mode} trigger ${it.name}${it.repo ? " (in the repo)" : ""}: ${it.title}`, skill: `repo skill ${it.name}: ${it.description}` })[it.type];
onMounted(load);
watch(() => props.task.state, load);
</script>

<template>
  <section v-if="proposal" class="proposal card" aria-label="proposal">
    <h3>proposal</h3>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
    <div class="md" v-html="markdown(proposal.note)"></div>
    <div v-for="(it, i) in proposal.items" :key="i" class="item" :aria-label="`item ${i + 1}`">
      <div class="row">
        <label class="grow"><input v-model="chosen[i]" type="checkbox" :aria-label="`take item ${i + 1}`" /> <span class="hi">{{ label(it) }}</span></label>
        <button type="button" @click="editing[i] = !editing[i]">{{ editing[i] ? "done editing" : "edit" }}</button>
        <button v-if="it.type === 'task'" type="button" @click="openInForm(it)">open in the form</button>
      </div>
      <div class="dim">{{ it.why }}</div>
      <div v-if="editing[i]" class="stack">
        <template v-if="it.type === 'skill'">
          <label>name <input v-model="it.name" /></label>
          <label>description <input v-model="it.description" size="60" /></label>
          <textarea v-model="it.body" class="code" rows="8" aria-label="skill body"></textarea>
        </template>
        <template v-else>
          <div class="row">
            <label v-if="it.type === 'trigger'">name <input v-model="it.name" size="14" /></label>
            <label v-if="it.type === 'trigger'">mode <select v-model="it.mode"><option>poll</option><option>watch</option><option>webhook</option></select></label>
            <label v-if="it.type === 'trigger' && it.mode === 'poll'">every <input v-model="it.every" size="5" /></label>
            <label v-if="it.type === 'cron'">schedule <input v-model="it.expr" size="12" /></label>
            <label v-if="it.type === 'cron'">time zone <input v-model="it.tz" size="14" /></label>
            <label v-if="it.type === 'trigger'"><input v-model="it.repo" type="checkbox" /> in the repo</label>
          </div>
          <textarea v-if="it.type === 'trigger'" v-model="it.script" class="code" rows="5" aria-label="trigger script"></textarea>
          <input v-model="it.title" aria-label="item title" />
          <textarea v-model="it.prompt" rows="8" aria-label="item prompt"></textarea>
        </template>
      </div>
      <div v-else-if="it.prompt || it.body" class="md item-text" v-html="markdown(it.prompt || it.body)"></div>
    </div>
    <button type="button" @click="create">create the ticked ones</button>
    <div v-for="(r, i) in results" :key="i" :class="r.ok ? 'dim' : 'err'">{{ r.ok ? `✓ ${r.done}` : `✗ ${r.error}` }}</div>
  </section>
</template>
