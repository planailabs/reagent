<script setup>
// Prompt templates: those found (a project's repo, global folders, plugins) and those kept in reagent, which are edited here.
import { onMounted, ref, watch } from "vue";
import { del, get, put } from "../lib/api.js";

const props = defineProps({ project: { type: String, default: "" } });
const emit = defineEmits(["changed"]);
const EXAMPLE = `---
description: Fix a failing CI job
title: "Fix CI: {{job}}"
budget: { cost: 2, minutes: 60 }
fields:
  - name: job
    label: Job name
    required: true
  - name: branch
    type: select
    options: [main, staging]
    default: main
---
The CI job {{job}} fails on {{branch}}. Find out why and fix it.
`;
const list = ref([]);
const editing = ref(null);
const error = ref("");

const mine = (t) => t.source === "db" && (t.project ?? "") === props.project;
async function load() {
  list.value = await get(`/api/templates${props.project ? `?project=${encodeURIComponent(props.project)}` : ""}`).catch((e) => ((error.value = e.message), []));
}
async function run(f) {
  error.value = "";
  try {
    await f();
    editing.value = null;
    emit("changed");
  } catch (e) {
    error.value = e.message;
  }
  await load();
}
const save = () => run(() => put("/api/templates", { id: editing.value.id ?? null, project: props.project || null, name: editing.value.name, text: editing.value.text }));
const remove = (t) => confirm(`Remove the template ${t.name}?`) && run(() => del(`/api/templates/${t.id}`));
const where = (t) => (t.path ? t.path : t.project ? `kept in reagent for ${t.project}` : "kept in reagent, every project's");

onMounted(load);
watch(() => props.project, load);
</script>

<template>
  <div class="stack">
    <p class="dim">
      Markdown with frontmatter: with <code>fields</code> a template is a form, without its body is a prompt to edit. Found in <code>.agents/templates/*.md</code> (the project's), <code>~/.agents/templates</code>, the data folder's <code>templates/</code> and plugins; or kept here.
    </p>
    <div v-for="t in list" :key="`${t.source}:${t.project}:${t.name}`" class="card" :aria-label="`template ${t.name}`">
      <div class="row">
        <span class="hi">{{ t.name }}</span>
        <span class="dim grow">{{ t.source }} · {{ where(t) }}{{ t.fields.length ? ` · form: ${t.fields.map((f) => f.name).join(", ")}` : "" }}</span>
        <button v-if="mine(t)" @click="editing = { id: t.id, name: t.name, text: t.text }">edit</button>
        <button v-if="mine(t)" @click="remove(t)">remove</button>
      </div>
      <div>{{ t.description }}</div>
      <div v-if="t.error" class="err">broken: {{ t.error }}</div>
    </div>
    <p v-if="!list.length" class="dim">no templates yet</p>
    <form v-if="editing" class="stack" @submit.prevent="save">
      <input v-model="editing.name" placeholder="name" aria-label="template name" required />
      <textarea v-model="editing.text" rows="16" aria-label="template text" required></textarea>
      <div class="row">
        <button type="submit">save the template</button>
        <button type="button" @click="editing = null">cancel</button>
      </div>
    </form>
    <div v-else class="row"><button @click="editing = { name: '', text: EXAMPLE }">new template{{ project ? " for this project" : " for every project" }}</button></div>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
  </div>
</template>
