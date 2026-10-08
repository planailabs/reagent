<script setup>
import { inject, onMounted, ref, watch } from "vue";
import { del, get, post, put } from "../lib/api.js";
import { markdown } from "../lib/markdown.js";
import Designer from "./Designer.vue";

const props = defineProps({ slug: { type: String, required: true }, config: Object });
const live = inject("live");
const list = ref({ triggers: [], errors: [] });
const runs = ref({});
const error = ref("");
const blank = () => ({ name: "", mode: "poll", every: "5m", cron: "", tz: "UTC", script: "", timeout: 60, overlap: "skip", title: "", prompt: "", description: "", secret: "", devshell: true, options: { profile: null, kind: null, skills: [] }, editing: false });
const form = ref(blank());

async function load() {
  list.value = await get(`/api/projects/${props.slug}/triggers`).catch((e) => ((error.value = e.message), { triggers: [], errors: [] }));
  // Open run lists follow.
  for (const name of Object.keys(runs.value)) {
    runs.value[name] = await get(`/api/triggers/${props.slug}/${name}/runs`).catch(() => runs.value[name]);
  }
}

/** "90", "90s", "2m", "1h", "1d" as seconds. */
function seconds(s) {
  const m = String(s).trim().match(/^(\d+)\s*([smhd]?)$/);
  if (!m) throw new Error(`every: ${s}? (like 90s, 2m, 1h)`);
  return Number(m[1]) * { "": 1, s: 1, m: 60, h: 3600, d: 86400 }[m[2]];
}
const every = (n) => (n % 3600 === 0 ? `${n / 3600}h` : n % 60 === 0 ? `${n / 60}m` : `${n}s`);

async function save() {
  error.value = "";
  try {
    const f = form.value;
    const useCron = f.mode === "poll" && f.cron.trim();
    const body = { ...f, every: f.mode === "poll" && !useCron ? seconds(f.every) : f.mode === "watch" && f.every ? seconds(f.every) : null, cron: useCron ? f.cron.trim() : null, secret: f.secret || null, timeout: Number(f.timeout) };
    delete body.editing;
    await put(`/api/triggers/${props.slug}/${encodeURIComponent(f.name)}`, body);
    form.value = blank();
    await load();
  } catch (e) {
    error.value = e.message;
  }
}

function edit(t) {
  form.value = { ...JSON.parse(JSON.stringify(t)), every: t.every ? every(t.every) : "", cron: t.cron || "", secret: t.secret || "", editing: true };
}

async function act(t, path, body) {
  error.value = "";
  try {
    await post(`/api/triggers/${t.project}/${t.name}/${path}`, body);
    await load();
  } catch (e) {
    error.value = e.message;
  }
}

async function remove(t) {
  if (!confirm(`Remove trigger ${t.name}?${t.source === "repo" ? " (its files in the repo go too)" : ""}`)) return;
  await del(`/api/triggers/${t.project}/${t.name}`).catch((e) => (error.value = e.message));
  await load();
}

async function toggleRuns(t) {
  if (runs.value[t.name]) return delete runs.value[t.name];
  runs.value[t.name] = await get(`/api/triggers/${t.project}/${t.name}/runs`).catch((e) => ((error.value = e.message), []));
}

const when = (t) => (t ? new Date(t * 1000).toLocaleString() : "–");
const schedule = (t) => (t.mode === "poll" ? (t.cron ? `cron ${t.cron} ${t.tz}` : `every ${every(t.every)}`) : t.mode);
const hookUrl = (t) => `${location.origin}${t.hook}`;
onMounted(load);
watch(() => [props.slug, live.tick], load);
</script>

<template>
  <section>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
    <p v-for="e in list.errors" :key="e" class="err">repo trigger not read: {{ e }}</p>
    <div v-if="!list.triggers.length" class="dim">
      no triggers: a script that watches something (a CI pipeline, a queue) and prints a JSON line per event, each starting a task (or messaging a running one). In the repo they're <code>.agents/triggers/&lt;name&gt;/TRIGGER.md</code>.
    </div>
    <div v-for="t in list.triggers" :key="t.name" class="card trigger" :aria-label="`trigger ${t.name}`">
      <div class="row">
        <span class="hi">{{ t.name }}</span>
        <span class="dim">{{ schedule(t) }} · {{ t.source === "repo" ? "in the repo" : "in reagent" }} · by {{ t.made_by }}{{ t.enabled ? "" : " · off" }}</span>
        <span class="grow"></span>
        <button v-if="t.mode !== 'webhook'" @click="act(t, 'run')">run now</button>
        <button @click="edit(t)">edit</button>
        <button @click="act(t, 'enabled', { enabled: !t.enabled })">{{ t.enabled ? "turn off" : "turn on" }}</button>
        <button @click="act(t, 'move', { to: t.source === 'repo' ? 'db' : 'repo' })">{{ t.source === "repo" ? "move to reagent" : "move to repo" }}</button>
        <button @click="toggleRuns(t)">runs</button>
        <button aria-label="remove" @click="remove(t)">×</button>
      </div>
      <div v-if="t.description" class="dim">{{ t.description }}</div>
      <div>→ <span class="hi">{{ t.title }}</span> <span class="dim">({{ t.overlap }})</span></div>
      <div v-if="t.hook" class="dim">POST <code>{{ hookUrl(t) }}</code> signed with the secret <code>{{ t.secret }}</code> (GitHub's X-Hub-Signature-256, GitLab's X-Gitlab-Token, or Authorization: Bearer)</div>
      <div v-if="t.state.asking" class="err wait" aria-label="approval">
        <div>its script waits for your approval (by {{ t.made_by }}):</div>
        <pre>{{ t.script }}</pre>
        <div class="row">
          <button @click="act(t, 'approve', { approved: true })">allow this script</button>
          <button title="adds a policy rule allowing this command" @click="act(t, 'approve', { approved: true, always: true })">always allow</button>
          <button @click="act(t, 'approve', { approved: false })">deny</button>
        </div>
      </div>
      <div class="dim">
        last run {{ when(t.state.last_run) }}<span v-if="t.mode === 'poll' && t.enabled"> · next {{ when(t.state.next_run) }}</span>
        <span v-if="t.state.job"> · watching (job {{ t.state.job }})</span>
        <span v-if="t.state.queue?.length"> · {{ t.state.queue.length }} queued</span>
        <span v-if="t.state.repair"> · <a :href="`#/task/${t.state.repair}`">repair task</a></span>
      </div>
      <p v-if="t.state.failures" class="err">{{ t.state.failures }} failed in a row: {{ t.state.last_error }}</p>
      <div v-if="runs[t.name]" class="runs">
        <div v-if="!runs[t.name].length" class="dim">no runs yet</div>
        <details v-for="r in runs[t.name]" :key="r.id">
          <summary :class="{ err: !r.ok }">{{ when(r.started) }} · {{ r.ok ? "ok" : r.error }} · {{ r.events }} events</summary>
          <pre>{{ r.output }}</pre>
        </details>
      </div>
    </div>

    <h2>{{ form.editing ? `change ${form.name}` : "add a trigger" }}</h2>
    <Designer :project="slug" target="trigger" />
    <form class="stack" @submit.prevent="save">
      <div class="row">
        <label>name <input v-model="form.name" :disabled="form.editing" aria-label="trigger name" required /></label>
        <label>mode
          <select v-model="form.mode" aria-label="mode"><option value="poll">poll (runs every so often)</option><option value="watch">watch (runs for good)</option><option value="webhook">webhook (called)</option></select>
        </label>
        <template v-if="form.mode === 'poll'">
          <label>every <input v-model="form.every" size="5" aria-label="every" /></label>
          <label>or cron <input v-model="form.cron" size="12" placeholder="*/10 * * * *" aria-label="trigger cron" /></label>
          <label>time zone <input v-model="form.tz" size="12" /></label>
        </template>
        <label v-if="form.mode === 'watch'">restart after <input v-model="form.every" size="5" placeholder="5s" /></label>
        <label v-if="form.mode === 'webhook'">secret <input v-model="form.secret" placeholder="HOOK_SECRET" aria-label="webhook secret" /> <span class="dim">(one of the project's secrets)</span></label>
      </div>
      <label class="stack">script <span class="dim">— prints one JSON object per line per event: <code>{"key": "…", "to": "new" | "running" | "&lt;task id&gt;", "title"?, "message"?, "vars": {…}}</code>; gets $REAGENT_STATE, $REAGENT_LAST_RUN, $REAGENT_TASKS, the project's env and secrets{{ form.mode === "webhook" ? "; {headers, body} on stdin (empty: the body is the event)" : "" }}</span>
        <textarea v-model="form.script" class="code" rows="6" aria-label="script"></textarea>
      </label>
      <input v-model="form.title" placeholder="task title ({{key}}, {{vars.x}})" aria-label="trigger title" required />
      <textarea v-model="form.prompt" placeholder="what the task does ({{vars.url}}, {{message}}…)" aria-label="trigger prompt" required></textarea>
      <div class="row">
        <label>while its task goes <select v-model="form.overlap"><option>skip</option><option>queue</option><option>parallel</option></select></label>
        <label>timeout <input v-model="form.timeout" size="4" /> s</label>
        <label>profile <select v-model="form.options.profile"><option :value="null">default</option><option v-for="p in config?.profiles ?? []" :key="p.name" :value="p.name">{{ p.name }}</option></select></label>
        <label v-if="config?.kinds?.length">kind <select v-model="form.options.kind"><option :value="null">default</option><option v-for="k in config.kinds" :key="k.name" :value="k.name">{{ k.name }}</option></select></label>
        <label><input v-model="form.devshell" type="checkbox" /> in the dev shell</label>
      </div>
      <input v-model="form.description" placeholder="what it watches (optional)" />
      <div class="row"><button type="submit">save</button><button v-if="form.editing" type="button" @click="form = blank()">new instead</button></div>
    </form>
  </section>
</template>
