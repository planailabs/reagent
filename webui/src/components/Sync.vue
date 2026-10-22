<script setup>
// A project's git sync: each branch (the folder's, each persistent worktree's) pulled and pushed on an interval.
import { inject, onMounted, ref, watch } from "vue";
import { get, post, put } from "../lib/api.js";

const props = defineProps({ slug: { type: String, required: true } });
const live = inject("live");
const list = ref([]);
const forms = ref({});
const error = ref("");

const key = (s) => s.place || ".";
// In a URL the folder is "-": a browser drops a "." path segment.
const path = (s) => encodeURIComponent(s.place || "-");
const formOf = (k) => ({ enabled: k.enabled, every: `${Math.round(k.every / 60)}m`, strategy: k.strategy, remote: k.remote ?? "", branch: k.branch ?? "", notes: k.notes });

async function load() {
  list.value = await get(`/api/projects/${props.slug}/sync`).catch((e) => ((error.value = e.message), []));
  for (const s of list.value) if (!forms.value[key(s)]?.dirty) forms.value[key(s)] = formOf(s.kept);
}

/** "15m", "1h", "900s" as seconds. */
function seconds(s) {
  const m = String(s).trim().match(/^(\d+)\s*([smhd]?)$/);
  if (!m) throw new Error(`every: ${s}? (like 15m, 1h)`);
  return Number(m[1]) * { "": 60, s: 1, m: 60, h: 3600, d: 86400 }[m[2]];
}

async function save(s) {
  error.value = "";
  try {
    const f = forms.value[key(s)];
    await put(`/api/projects/${props.slug}/sync/${path(s)}`, { enabled: f.enabled, every: seconds(f.every), strategy: f.strategy, remote: f.remote || null, branch: f.branch || null, notes: f.notes });
    delete forms.value[key(s)];
    await load();
  } catch (e) {
    error.value = e.message;
  }
}

async function run(s) {
  error.value = "";
  await post(`/api/projects/${props.slug}/sync/${path(s)}/run`, {}).catch((e) => (error.value = e.message));
  await load();
}

const when = (t) => (t ? new Date(t * 1000).toLocaleString() : "–");
const overridden = (s, field) => s.from_repo.includes(field);
onMounted(load);
watch(() => [props.slug, live.tick], load);
</script>

<template>
  <section>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
    <p class="dim">
      A branch that's on is pulled with its strategy and pushed every interval. A conflict, or uncommitted changes no task works on, start a task to sort it out; reagent pushes after it. A checkout's
      <code>.agents/sync.md</code> (frontmatter <code>enabled, every, strategy, remote, branch</code>; body: notes) overrides what it sets.
    </p>
    <div v-for="s in list" :key="key(s)" class="card" :aria-label="`sync of ${key(s)}`">
      <div class="row">
        <span class="hi">{{ s.place ? `worktree ${s.place}` : "project folder" }}</span>
        <span class="dim">{{ s.local_branch ?? "(detached)" }}</span>
        <span :class="['badge', s.enabled ? s.state.status || 'new' : 'stopped']" :aria-label="`sync status of ${key(s)}`">{{ s.enabled ? s.state.status || "new" : "off" }}</span>
        <span class="grow"></span>
        <button @click="run(s)">{{ s.state.status === "stuck" ? "go on" : "sync now" }}</button>
      </div>
      <div v-if="s.state.message" :class="{ err: ['error', 'stuck'].includes(s.state.status) }">{{ s.state.message }}</div>
      <div class="dim">
        last round {{ when(s.state.last_run) }} · last in sync {{ when(s.state.last_ok) }}
        <span v-if="s.state.fixer"> · <a :href="`#/task/${s.state.fixer}`">its task</a></span>
      </div>
      <div v-if="s.repo_file" :class="s.repo_error ? 'err' : 'dim'">
        {{ s.repo_file }}: {{ s.repo_error ? `${s.repo_error} (ignored)` : s.from_repo.length ? `sets ${s.from_repo.join(", ")}` : "sets nothing" }}
      </div>
      <form v-if="forms[key(s)]" class="stack" @submit.prevent="save(s)" @input="forms[key(s)].dirty = true">
        <div class="row">
          <label :class="{ dim: overridden(s, 'enabled') }"><input v-model="forms[key(s)].enabled" type="checkbox" :aria-label="`sync ${key(s)} on`" /> on</label>
          <label :class="{ dim: overridden(s, 'every') }">every <input v-model="forms[key(s)].every" size="4" :aria-label="`sync interval of ${key(s)}`" /></label>
          <label :class="{ dim: overridden(s, 'strategy') }">pull with
            <select v-model="forms[key(s)].strategy" :aria-label="`sync strategy of ${key(s)}`">
              <option>rebase</option>
              <option>merge</option>
              <option>ff-only</option>
            </select>
          </label>
          <label :class="{ dim: overridden(s, 'remote') }">remote <input v-model="forms[key(s)].remote" size="8" placeholder="upstream" /></label>
          <label :class="{ dim: overridden(s, 'branch') }">branch <input v-model="forms[key(s)].branch" size="12" placeholder="upstream's" /></label>
        </div>
        <textarea v-model="forms[key(s)].notes" rows="2" :class="{ dim: overridden(s, 'notes') }" placeholder="notes for the task that resolves a conflict or decides about uncommitted changes"></textarea>
        <div class="row"><button type="submit">save</button><span v-if="s.from_repo.length" class="dim">greyed fields are set by the repo file, which wins</span></div>
      </form>
    </div>
  </section>
</template>
