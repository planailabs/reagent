<script setup>
// A project's services: commands that should always run, started, checked and restarted by reagent.
import { inject, onMounted, ref, watch } from "vue";
import { del, get, post, put } from "../lib/api.js";

const props = defineProps({ slug: { type: String, required: true } });
const live = inject("live");
const list = ref({ services: [], errors: [] });
const homes = ref([]);
const runs = ref({});
const logs = ref({});
const error = ref("");
const blank = () => ({ name: "", script: "", health: "", interval: "30s", ready_timeout: "60s", stop_timeout: "10s", worktree: "", devshell: true, description: "", notes: "", editing: false });
const form = ref(blank());

async function load() {
  list.value = await get(`/api/projects/${props.slug}/services`).catch((e) => ((error.value = e.message), { services: [], errors: [] }));
  homes.value = (await get(`/api/projects/${props.slug}/worktrees`).catch(() => ({ worktrees: [] }))).worktrees;
  for (const name of Object.keys(runs.value)) runs.value[name] = await get(`/api/services/${props.slug}/${name}/runs`).catch(() => runs.value[name]);
  for (const name of Object.keys(logs.value)) await showLogs(list.value.services.find((v) => v.name === name), true);
}

/** "90", "90s", "2m", "1h" as seconds. */
function seconds(s, what) {
  const m = String(s).trim().match(/^(\d+)\s*([smhd]?)$/);
  if (!m) throw new Error(`${what}: ${s}? (like 30s, 2m)`);
  return Number(m[1]) * { "": 1, s: 1, m: 60, h: 3600, d: 86400 }[m[2]];
}
const dur = (n) => (n % 3600 === 0 ? `${n / 3600}h` : n % 60 === 0 ? `${n / 60}m` : `${n}s`);

async function save() {
  error.value = "";
  try {
    const f = form.value;
    const body = { script: f.script, health: f.health || null, interval: seconds(f.interval, "interval"), ready_timeout: seconds(f.ready_timeout, "ready timeout"), stop_timeout: seconds(f.stop_timeout, "stop timeout"), worktree: f.worktree || null, devshell: f.devshell, description: f.description, notes: f.notes };
    await put(`/api/services/${props.slug}/${encodeURIComponent(f.name)}`, body);
    form.value = blank();
    await load();
  } catch (e) {
    error.value = e.message;
  }
}

function edit(v) {
  form.value = { name: v.name, script: v.script, health: v.health ?? "", interval: dur(v.interval), ready_timeout: dur(v.ready_timeout), stop_timeout: dur(v.stop_timeout), worktree: v.worktree ?? "", devshell: v.devshell, description: v.description, notes: v.notes, editing: true };
}

async function act(v, path, body) {
  error.value = "";
  try {
    await post(`/api/services/${v.project}/${v.name}/${path}`, body);
    await load();
  } catch (e) {
    error.value = e.message;
  }
}

async function remove(v) {
  if (!confirm(`Remove service ${v.name}? It's stopped.${v.source === "repo" ? " (its files in the repo go too)" : ""}`)) return;
  await del(`/api/services/${v.project}/${v.name}`).catch((e) => (error.value = e.message));
  await load();
}

async function toggleRuns(v) {
  if (runs.value[v.name]) return delete runs.value[v.name];
  runs.value[v.name] = await get(`/api/services/${v.project}/${v.name}/runs`).catch((e) => ((error.value = e.message), []));
}

/** The end of its output: its running job's, else its last start's. */
async function showLogs(v, refresh = false) {
  if (!v) return;
  if (logs.value[v.name] !== undefined && !refresh) return delete logs.value[v.name];
  const job = v.state.job ?? (await get(`/api/services/${v.project}/${v.name}/runs`).catch(() => []))[0]?.job;
  logs.value[v.name] = job ? (await get(`/api/jobs/${job}/output?tail=60`).catch((e) => ({ text: e.message }))).text : "it hasn't run yet";
}

const when = (t) => (t ? new Date(t * 1000).toLocaleString() : "–");
const status = (v) => (!v.enabled && !v.state.job ? "stopped" : v.state.status || "new");
onMounted(load);
watch(() => [props.slug, live.tick], load);
</script>

<template>
  <section>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
    <p v-for="e in list.errors" :key="e" class="err">repo service not read: {{ e }}</p>
    <div v-if="!list.services.length" class="dim">
      no services: commands that should always run (a dev server, a database, a worker). reagent starts each, checks it, restarts it when it ends or stops answering, and has a task repair it when that doesn't help. In the repo they're <code>.agents/services/&lt;name&gt;/SERVICE.md</code>.
    </div>
    <div v-for="v in list.services" :key="v.name" class="card" :aria-label="`service ${v.name}`">
      <div class="row">
        <span class="hi">{{ v.name }}</span>
        <span :class="['badge', status(v)]" :aria-label="`status of ${v.name}`">{{ status(v) }}</span>
        <span class="dim">since {{ when(v.state.since) }} · {{ v.source === "repo" ? "in the repo" : "in reagent" }} · by {{ v.made_by }}<span v-if="v.worktree"> · in worktree {{ v.worktree }}</span></span>
        <span class="grow"></span>
        <button v-if="v.enabled" @click="act(v, 'stop')">stop</button>
        <button v-else @click="act(v, 'start')">start</button>
        <button @click="act(v, 'restart')">restart</button>
        <button @click="showLogs(v)">output</button>
        <button @click="edit(v)">edit</button>
        <button @click="act(v, 'move', { to: v.source === 'repo' ? 'db' : 'repo' })">{{ v.source === "repo" ? "move to reagent" : "move to repo" }}</button>
        <button @click="toggleRuns(v)">starts</button>
        <button aria-label="remove" @click="remove(v)">×</button>
      </div>
      <div v-if="v.description" class="dim">{{ v.description }}</div>
      <div><code>{{ v.script.trim().split("\n")[0] }}</code><span v-if="v.health" class="dim"> · checked by <code>{{ v.health }}</code> every {{ dur(v.interval) }}</span></div>
      <div v-if="v.state.asking" class="err wait" aria-label="approval">
        <div>its command waits for your approval (by {{ v.made_by }}):</div>
        <pre>{{ v.state.command }}</pre>
        <div class="row">
          <button @click="act(v, 'approve', { approved: true })">allow this command</button>
          <button title="adds policy rules allowing these commands" @click="act(v, 'approve', { approved: true, always: true })">always allow</button>
          <button @click="act(v, 'approve', { approved: false })">deny</button>
        </div>
      </div>
      <div class="dim">
        <span v-if="v.state.job">job {{ v.state.job }}</span>
        <span v-if="v.state.status === 'backoff' && v.enabled"> · starts again {{ when(v.state.next_start) }}</span>
        <span v-if="v.state.repair"> · <a :href="`#/task/${v.state.repair}`">repair task</a></span>
      </div>
      <p v-if="v.state.failures" class="err">{{ v.state.failures }} failed in a row: {{ v.state.last_error }}</p>
      <pre v-if="logs[v.name] !== undefined" :aria-label="`output of ${v.name}`">{{ logs[v.name] }}</pre>
      <div v-if="runs[v.name]" class="runs">
        <div v-if="!runs[v.name].length" class="dim">not started yet</div>
        <details v-for="r in runs[v.name]" :key="r.id">
          <summary :class="{ err: r.error }">{{ when(r.started) }} · {{ r.up ? `up at ${when(r.up)}` : "never up" }}<span v-if="r.ended"> · ended {{ when(r.ended) }}{{ r.error ? `: ${r.error}` : "" }}</span><span v-else> · running</span></summary>
          <pre>{{ r.output }}</pre>
        </details>
      </div>
    </div>

    <h2>{{ form.editing ? `change ${form.name}` : "add a service" }}</h2>
    <form class="stack" @submit.prevent="save">
      <div class="row">
        <label>name <input v-model="form.name" :disabled="form.editing" aria-label="service name" required /></label>
        <label v-if="homes.length">runs in
          <select v-model="form.worktree" aria-label="service worktree">
            <option value="">the project folder</option>
            <option v-for="w in homes" :key="w.name" :value="w.name">{{ w.name }} · {{ w.branch }}</option>
          </select>
        </label>
        <label><input v-model="form.devshell" type="checkbox" /> in the dev shell</label>
      </div>
      <label class="stack">command <span class="dim">— runs for good (it shouldn't end); gets the project's env and secrets and $REAGENT_SERVICE</span>
        <textarea v-model="form.script" class="code" rows="4" aria-label="service command" required></textarea>
      </label>
      <div class="row">
        <label class="grow">health check <input v-model="form.health" class="grow" placeholder="curl -fs localhost:8080/health (empty: up while it runs)" aria-label="health check" /></label>
        <label>every <input v-model="form.interval" size="4" aria-label="check interval" /></label>
        <label>up within <input v-model="form.ready_timeout" size="4" aria-label="ready timeout" /></label>
        <label>stop grace <input v-model="form.stop_timeout" size="4" aria-label="stop timeout" /></label>
      </div>
      <input v-model="form.description" placeholder="what it is (optional)" aria-label="service description" />
      <textarea v-model="form.notes" rows="3" placeholder="notes for whoever repairs it: where its config is, how it's usually fixed" aria-label="service notes"></textarea>
      <div class="row"><button type="submit">save</button><button v-if="form.editing" type="button" @click="form = blank()">new instead</button></div>
    </form>
  </section>
</template>
