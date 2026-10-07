<script setup>
import { computed, inject, onMounted, ref, watch } from "vue";
import { del, get, post, put } from "../lib/api.js";
import { budget, flatten, tree } from "../lib/format.js";
import TaskRow from "./TaskRow.vue";
import Rules from "./Rules.vue";
import Cron from "./Cron.vue";
import Memory from "./Memory.vue";
import McpServers from "./McpServers.vue";
import Secrets from "./Secrets.vue";

const props = defineProps({ slug: { type: String, required: true }, tab: { type: String, default: "tasks" } });
const live = inject("live");
const p = ref(null);
const tasks = ref([]);
const showAll = ref(false);
const skills = ref([]);
const config = ref(null);
const error = ref("");
const start = ref({ title: "", prompt: "", profile: "", skills: [], tokens: "", cost: "", minutes: "" });
const settings = ref(null);

async function load() {
  try {
    p.value = await get(`/api/projects/${props.slug}`);
    tasks.value = await get(`/api/tasks?project=${props.slug}&active=${!showAll.value}&limit=300`);
    skills.value = await get(`/api/projects/${props.slug}/skills`);
    config.value ??= await get("/api/config");
    if (!settings.value) {
      settings.value = { ...p.value, budget: { ...p.value.budget }, env: JSON.stringify(p.value.env, null, 2) };
    }
  } catch (e) {
    error.value = e.message;
  }
}

const rows = computed(() => flatten(tree(tasks.value)));

async function startTask() {
  error.value = "";
  try {
    const t = await post("/api/tasks", {
      project: props.slug,
      title: start.value.title,
      prompt: start.value.prompt,
      profile: start.value.profile || undefined,
      skills: start.value.skills,
      budget: budget(start.value),
    });
    location.hash = `#/task/${t.id}`;
  } catch (e) {
    error.value = e.message;
  }
}

async function save() {
  error.value = "";
  try {
    const s = settings.value;
    const body = { devshell: s.devshell, devshell_attr: s.devshell_attr || null, name: s.name, path: s.path, memory: s.memory, worktrees: s.worktrees, merge: s.merge, default_action: s.default_action, profile: s.profile || null, budget: budget(s.budget), env: JSON.parse(s.env || "{}") };
    p.value = await put(`/api/projects/${props.slug}`, body);
    error.value = "saved";
  } catch (e) {
    error.value = e.message;
  }
}

async function remove() {
  if (!confirm(`Remove project ${p.value.name}? (its folder stays; its tasks are forgotten)`)) return;
  try {
    await del(`/api/projects/${props.slug}`);
    location.hash = "#/projects";
  } catch (e) {
    error.value = e.message;
  }
}

onMounted(load);
watch(() => [props.slug, showAll.value, live.tick], load);
</script>

<template>
  <section v-if="p">
    <h2>{{ p.name }} <span class="dim">{{ p.path }}</span></h2>
    <nav class="row tabs">
      <a v-for="t in ['tasks', 'new', 'cron', 'policy', 'memory', 'skills', 'mcp', 'secrets', 'settings']" :key="t" :href="`#/project/${slug}/${t}`" :class="{ on: tab === t }">{{ t }}</a>
    </nav>
    <p v-if="error" class="err" role="alert">{{ error }}</p>

    <template v-if="tab === 'tasks'">
      <label class="row"><input v-model="showAll" type="checkbox" /> finished ones too</label>
      <div v-if="!rows.length" class="dim">no tasks{{ showAll ? "" : " going on" }}: <a :href="`#/project/${slug}/new`">start one</a></div>
      <TaskRow v-for="r in rows" :key="r.t.id" :t="r.t" :depth="r.depth" />
    </template>

    <form v-else-if="tab === 'new'" class="stack" @submit.prevent="startTask">
      <input v-model="start.title" placeholder="title" aria-label="title" required />
      <textarea v-model="start.prompt" placeholder="what to do" aria-label="what to do" required></textarea>
      <div class="row">
        <label>profile
          <select v-model="start.profile" aria-label="profile">
            <option value="">default ({{ p.profile || config?.default_profile }})</option>
            <option v-for="pr in config?.profiles ?? []" :key="pr.name" :value="pr.name">{{ pr.name }} · {{ pr.model }}</option>
          </select>
        </label>
        <label>tokens <input v-model="start.tokens" size="8" inputmode="numeric" aria-label="token budget" /></label>
        <label>cost <input v-model="start.cost" size="6" inputmode="decimal" aria-label="cost budget" /></label>
        <label>minutes <input v-model="start.minutes" size="5" inputmode="numeric" aria-label="time budget" /></label>
      </div>
      <div v-if="skills.length" class="row">
        skills to load:
        <label v-for="s in skills" :key="s.name" :title="s.description"><input v-model="start.skills" type="checkbox" :value="s.name" /> {{ s.name }}</label>
      </div>
      <button type="submit">start</button>
    </form>

    <Cron v-else-if="tab === 'cron'" :slug="slug" :config="config" />
    <Rules v-else-if="tab === 'policy'" :slug="slug" />
    <Memory v-else-if="tab === 'memory'" :scope="slug" />
    <McpServers v-else-if="tab === 'mcp'" :project="slug" />
    <Secrets v-else-if="tab === 'secrets'" :project="slug" />

    <template v-else-if="tab === 'skills'">
      <div v-if="!skills.length" class="dim">no skills: they live in <code>.agents/skills/&lt;name&gt;/SKILL.md</code> (here, or in ~/.agents/skills)</div>
      <div v-for="s in skills" :key="s.name" class="card">
        <span class="hi">{{ s.name }}</span> <span class="dim">{{ s.source }} · {{ s.dir }}</span>
        <div>{{ s.description }}</div>
        <div v-if="s.files.length" class="dim">files: {{ s.files.join(", ") }}</div>
      </div>
    </template>

    <form v-else-if="tab === 'settings' && settings" class="stack" @submit.prevent="save">
      <label>name <input v-model="settings.name" /></label>
      <label>folder <input v-model="settings.path" size="50" /></label>
      <label>memory <select v-model="settings.memory"><option>central</option><option>repo</option></select> <span class="dim">central: in reagent's data; repo: .reagent/memory in the folder</span></label>
      <label>worktrees <select v-model="settings.worktrees"><option>central</option><option>repo</option></select></label>
      <label>merging <select v-model="settings.merge"><option>approve</option><option>auto</option></select></label>
      <label>nix dev shell <select v-model="settings.devshell"><option>auto</option><option>on</option><option>off</option></select>
        <input v-model="settings.devshell_attr" size="10" placeholder="default" aria-label="dev shell" />
        <span class="dim">auto: when there's a flake.nix; commands and terminals run in `nix develop`</span></label>
      <label>calls no rule covers <select v-model="settings.default_action"><option>ask</option><option>allow</option><option>deny</option></select></label>
      <label>profile
        <select v-model="settings.profile"><option :value="null">default</option><option v-for="pr in config?.profiles ?? []" :key="pr.name" :value="pr.name">{{ pr.name }}</option></select>
      </label>
      <div class="row">budget per task:
        <label>tokens <input v-model="settings.budget.tokens" size="8" /></label>
        <label>cost <input v-model="settings.budget.cost" size="6" /></label>
        <label>minutes <input v-model="settings.budget.minutes" size="5" /></label>
        <label>per day <input v-model="settings.budget.daily_cost" size="6" /></label>
      </div>
      <label>environment for its commands (JSON) <textarea v-model="settings.env" rows="4"></textarea></label>
      <div class="row"><button type="submit">save</button><span class="grow"></span><button type="button" @click="remove">remove project</button></div>
    </form>
  </section>
  <p v-else-if="error" class="err">{{ error }}</p>
</template>
