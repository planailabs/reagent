<script setup>
import { onMounted, ref } from "vue";
import { del, get, post } from "../lib/api.js";
import Designer from "./Designer.vue";

const props = defineProps({ slug: { type: String, required: true }, config: Object });
const crons = ref([]);
const error = ref("");
const blank = () => ({ id: 0, expr: "0 3 * * *", tz: Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC", title: "", prompt: "", overlap: "skip", catch_up: true, enabled: true, options: { profile: null, skills: [] } });
const form = ref(blank());

async function load() {
  crons.value = await get(`/api/projects/${props.slug}/cron`).catch((e) => ((error.value = e.message), []));
}

async function save() {
  error.value = "";
  try {
    await post("/api/cron", { ...form.value, project: props.slug });
    form.value = blank();
    await load();
  } catch (e) {
    error.value = e.message;
  }
}

async function remove(c) {
  await del(`/api/cron/${c.id}`).catch((e) => (error.value = e.message));
  await load();
}

async function runNow(c) {
  try {
    const t = await post(`/api/cron/${c.id}/run`);
    location.hash = `#/task/${t.id}`;
  } catch (e) {
    error.value = e.message;
  }
}

async function toggle(c) {
  await post("/api/cron", { ...c, enabled: !c.enabled }).catch((e) => (error.value = e.message));
  await load();
}

const when = (t) => (t ? new Date(t * 1000).toLocaleString() : "–");
onMounted(load);
</script>

<template>
  <section>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
    <table>
      <tr><th>schedule</th><th>task</th><th>next</th><th>last</th><th></th></tr>
      <tr v-for="c in crons" :key="c.id">
        <td><code>{{ c.expr }}</code> <span class="dim">{{ c.tz }}</span></td>
        <td><span class="hi">{{ c.title }}</span> <span class="dim">{{ c.overlap }}{{ c.enabled ? "" : " · off" }}</span></td>
        <td>{{ c.enabled ? when(c.next_run) : "–" }}</td>
        <td class="dim">{{ when(c.last_run) }}</td>
        <td class="row">
          <button @click="runNow(c)">run now</button>
          <button @click="form = JSON.parse(JSON.stringify(c))">edit</button>
          <button @click="toggle(c)">{{ c.enabled ? "turn off" : "turn on" }}</button>
          <button aria-label="remove" @click="remove(c)">×</button>
        </td>
      </tr>
    </table>
    <p v-if="!crons.length" class="dim">no cron entries</p>
    <h2>{{ form.id ? "change" : "add" }} an entry</h2>
    <Designer :project="slug" target="cron" />
    <form class="stack" @submit.prevent="save">
      <div class="row">
        <label>schedule <input v-model="form.expr" aria-label="schedule" required /></label>
        <label>time zone <input v-model="form.tz" aria-label="time zone" /></label>
        <label>while the last run goes
          <select v-model="form.overlap"><option>skip</option><option>queue</option><option>parallel</option></select>
        </label>
        <label><input v-model="form.catch_up" type="checkbox" /> run missed ones once</label>
      </div>
      <input v-model="form.title" placeholder="task title" aria-label="cron title" required />
      <textarea v-model="form.prompt" placeholder="what the task does" aria-label="cron prompt" required></textarea>
      <div class="row"><button type="submit">save</button><button v-if="form.id" type="button" @click="form = blank()">new instead</button></div>
    </form>
  </section>
</template>
