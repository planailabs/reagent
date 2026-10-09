<script setup>
// A prompt for any project, fast: the project, what to do, limits from presets (or typed), skills.
import { computed, onMounted, ref, watch } from "vue";
import { get, post } from "../lib/api.js";
import SkillPicker from "./SkillPicker.vue";
import TemplateForm from "./TemplateForm.vue";

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
const templates = ref([]);
const chosen = ref("");
const values = ref({});
const tf = ref(null);
/** A template picked for another project: applied once that project's lists are in. */
let pending = null;
const form_like = computed(() => !!tf.value?.current?.fields.length);
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

/** A template fills the form: a form-like one leaves the prompt to its fields, a plain one gives a prompt to edit. */
function applyTemplate(t) {
  if (!t) return;
  if (t.project && t.project !== form.value.project) {
    pending = t;
    form.value.project = t.project;
    return;
  }
  form.value.title = t.fields.length ? "" : (t.title ?? "");
  form.value.prompt = t.fields.length ? "" : t.prompt;
  form.value.skills = t.skills.filter((s) => skills.value.some((k) => k.name === s));
  for (const l of LIMITS) {
    const v = t.budget?.[l.key];
    if (v != null) choose(l.key, v);
  }
}

async function startTask() {
  error.value = "";
  const budget = {};
  const unlimited = [];
  for (const [k, v] of Object.entries(limits.value)) {
    if (v === "unlimited") unlimited.push(k);
    else if (typeof v === "number") budget[k] = v;
  }
  try {
    const body = { project: form.value.project, title: form.value.title, prompt: form.value.prompt, skills: form.value.skills, budget, unlimited };
    if (form_like.value) Object.assign(body, { template: tf.value.current.name, values: values.value, prompt: "" });
    const t = await post("/api/tasks", body);
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
    templates.value = slug ? await get(`/api/templates?project=${encodeURIComponent(slug)}`).catch(() => []) : [];
    if (pending) {
      const t = pending;
      pending = null;
      applyTemplate(t);
    } else {
      chosen.value = "";
    }
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
        <input v-model="form.title" class="grow" :placeholder="form_like && tf.current.title ? `title (empty: ${tf.render(tf.current.title)})` : 'title (empty: the agent names it)'" aria-label="title" />
      </div>
      <TemplateForm v-if="templates.length" ref="tf" v-model:chosen="chosen" v-model:values="values" :templates="templates" :project="form.project" @pick="applyTemplate" />
      <textarea v-if="!form_like" v-model="form.prompt" rows="6" placeholder="what to do" aria-label="what to do" required></textarea>
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
