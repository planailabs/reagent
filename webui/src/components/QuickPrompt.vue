<script setup>
// A prompt for any project, fast: the project, what to do, limits from presets (or typed), skills.
import { computed, onMounted, ref, watch } from "vue";
import { get, post } from "../lib/api.js";
import SkillPicker from "./SkillPicker.vue";

const LIMITS = [
  { key: "tokens", label: "tokens", presets: [["200k", 200_000], ["1M", 1_000_000], ["5M", 5_000_000], ["20M", 20_000_000]], parse: (v) => parseInt(v, 10) },
  { key: "cost", label: "cost", presets: [["0.50", 0.5], ["2", 2], ["10", 10], ["50", 50]], parse: parseFloat },
  { key: "minutes", label: "minutes", presets: [["15m", 15], ["1h", 60], ["4h", 240], ["1d", 1440]], parse: (v) => parseInt(v, 10) },
];
const KEPT = "reagent-quick";

const projects = ref([]);
const skills = ref([]);
const form = ref({ project: "", title: "", prompt: "", skills: [] });
/** Per limit: "default" (the project's), "unlimited", or a number. */
const limits = ref({ tokens: "default", cost: "default", minutes: "default" });
const typed = ref({ tokens: "", cost: "", minutes: "" });
const error = ref("");
const project = computed(() => projects.value.find((p) => p.slug === form.value.project));

function choose(key, v) {
  limits.value[key] = v;
  typed.value[key] = "";
}
function type(l) {
  const v = l.parse(typed.value[l.key]);
  limits.value[l.key] = Number.isFinite(v) && v > 0 ? v : "default";
}
const projectDefault = (key) => project.value?.budget?.[key] ?? "none";

async function startTask() {
  error.value = "";
  const budget = {};
  const unlimited = [];
  for (const [k, v] of Object.entries(limits.value)) {
    if (v === "unlimited") unlimited.push(k);
    else if (typeof v === "number") budget[k] = v;
  }
  try {
    const t = await post("/api/tasks", { project: form.value.project, title: form.value.title, prompt: form.value.prompt, skills: form.value.skills, budget, unlimited });
    try {
      localStorage.setItem(KEPT, JSON.stringify({ project: form.value.project, limits: limits.value }));
    } catch {}
    location.hash = `#/task/${t.id}`;
  } catch (e) {
    error.value = e.message;
  }
}

onMounted(async () => {
  projects.value = await get("/api/projects").catch((e) => ((error.value = e.message), []));
  let kept = null;
  try {
    kept = JSON.parse(localStorage.getItem(KEPT) || "null");
  } catch {}
  if (kept?.limits) limits.value = { ...limits.value, ...kept.limits };
  form.value.project = projects.value.some((p) => p.slug === kept?.project) ? kept.project : (projects.value[0]?.slug ?? "");
});
watch(
  () => form.value.project,
  async (slug) => {
    form.value.skills = [];
    skills.value = slug ? await get(`/api/projects/${slug}/skills`).catch(() => []) : [];
  },
);
</script>

<template>
  <section>
    <h2>quick prompt</h2>
    <p v-if="!projects.length" class="dim">no projects yet: <a href="#/projects">add one</a></p>
    <form v-else class="stack" @submit.prevent="startTask">
      <div class="row">
        <label>project
          <select v-model="form.project" aria-label="project">
            <option v-for="p in projects" :key="p.slug" :value="p.slug">{{ p.name }}</option>
          </select>
        </label>
        <input v-model="form.title" class="grow" placeholder="title (empty: the agent names it)" aria-label="title" />
      </div>
      <textarea v-model="form.prompt" rows="6" placeholder="what to do" aria-label="what to do" required></textarea>
      <div v-for="l in LIMITS" :key="l.key" class="row choices" role="group" :aria-label="`${l.label} limit`">
        <span class="dim limit">{{ l.label }}</span>
        <button type="button" :class="{ on: limits[l.key] === 'default' }" @click="choose(l.key, 'default')">project default ({{ projectDefault(l.key) }})</button>
        <button v-for="[name, v] in l.presets" :key="name" type="button" :class="{ on: limits[l.key] === v }" @click="choose(l.key, v)">{{ name }}</button>
        <button type="button" :class="{ on: limits[l.key] === 'unlimited' }" @click="choose(l.key, 'unlimited')">unlimited</button>
        <input v-model="typed[l.key]" size="8" :inputmode="l.key === 'cost' ? 'decimal' : 'numeric'" :placeholder="typeof limits[l.key] === 'number' && !l.presets.some(([, v]) => v === limits[l.key]) ? String(limits[l.key]) : 'other'" :aria-label="`other ${l.label} limit`" @input="type(l)" />
      </div>
      <SkillPicker v-if="skills.length" v-model="form.skills" :skills="skills" />
      <div class="row"><button type="submit" :disabled="!form.project">start</button></div>
    </form>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
  </section>
</template>
