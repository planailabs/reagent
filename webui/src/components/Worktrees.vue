<script setup>
// A project's persistent worktrees: named, long-lived checkouts tasks start in.
import { onMounted, ref, watch } from "vue";
import { del, get, post, put } from "../lib/api.js";
import { ago } from "../lib/format.js";

const props = defineProps({ slug: { type: String, required: true }, sharing: { type: String, default: "shared" } });
const emit = defineEmits(["changed"]);
const list = ref(null);
const adoptable = ref([]);
const form = ref({ name: "", branch: "", base: "", sharing: "" });
const open = ref({});
const merge = ref({});
const busy = ref("");
const said = ref({});
const error = ref("");
const base = () => `/api/projects/${props.slug}/worktrees`;

async function load() {
  try {
    const r = await get(base());
    list.value = r.worktrees;
    adoptable.value = r.adoptable;
    for (const name of Object.keys(open.value)) await details(name);
  } catch (e) {
    error.value = e.message;
  }
}

async function details(name) {
  merge.value[name] ??= { strategy: "", message: "" };
  open.value[name] = await get(`${base()}/${encodeURIComponent(name)}`).catch((e) => ({ error: e.message }));
}

async function toggle(name) {
  if (open.value[name]) delete open.value[name];
  else await details(name);
}

async function run(what, f) {
  error.value = "";
  busy.value = what;
  try {
    await f();
  } catch (e) {
    error.value = e.message;
  }
  busy.value = "";
  await load();
  emit("changed");
}

const orNull = (s) => (s ? s : null);
const add = () =>
  run("adding", async () => {
    const f = form.value;
    await post(base(), { name: f.name, branch: orNull(f.branch), base: orNull(f.base), sharing: orNull(f.sharing) });
    form.value = { name: "", branch: "", base: "", sharing: "" };
  });
const adopt = (a) => {
  const name = prompt(`A name for the worktree on ${a.branch} (${a.path})`, a.branch.replace(/[^A-Za-z0-9._-]/g, "-"));
  if (name) run("adopting", () => post(base(), { name, path: a.path }));
};
const setSharing = (w, v) => run(w.name, () => put(`${base()}/${encodeURIComponent(w.name)}`, { sharing: orNull(v) }));
const doMerge = (w) =>
  run(w.name, async () => {
    const m = merge.value[w.name];
    said.value[w.name] = (await post(`${base()}/${encodeURIComponent(w.name)}/merge`, { strategy: m.strategy || "merge", message: orNull(m.message) })).merged;
  });
function remove(w) {
  const force = w.ahead > 0;
  const what = w.adopted ? `Forget ${w.name}? (its folder ${w.path} and branch ${w.branch} stay)` : `Remove ${w.name}: ${w.path} and its branch ${w.branch}?${force ? ` It has ${w.ahead} commit(s) not in ${w.base}: they go.` : ""}`;
  if (confirm(what)) run(w.name, () => del(`${base()}/${encodeURIComponent(w.name)}${force ? "?force=true" : ""}`));
}

onMounted(load);
watch(() => props.slug, load);
</script>

<template>
  <div v-if="list" class="stack">
    <p class="dim">Persistent worktrees: a branch checked out apart that tasks start in (picked on "new"). A task there works directly; a worktree it starts branches from it and merges back into it.</p>
    <div v-if="!list.length" class="dim">no persistent worktrees yet</div>
    <div v-for="w in list" :key="w.name" class="card" :aria-label="`worktree ${w.name}`">
      <div class="row">
        <button type="button" class="link" :aria-expanded="!!open[w.name]" @click="toggle(w.name)">{{ open[w.name] ? "▾" : "▸" }} <span class="hi">{{ w.name }}</span></button>
        <span>{{ w.branch }} → {{ w.base }}</span>
        <span v-if="w.missing" class="err">its folder is gone</span>
        <span v-else class="dim">{{ w.ahead ?? "?" }} ahead<span v-if="w.changes">, {{ w.changes }} changed</span><span v-if="w.active"> · {{ w.active }} task(s) there now</span></span>
        <span class="grow"></span>
        <label>sharing
          <select :value="w.sharing ?? ''" :aria-label="`sharing of ${w.name}`" :disabled="!!busy" @change="setSharing(w, $event.target.value)">
            <option value="">project's ({{ sharing }})</option>
            <option value="shared">shared</option>
            <option value="exclusive">one task at a time</option>
          </select>
        </label>
        <button type="button" :disabled="!!busy" @click="remove(w)">{{ w.adopted ? "forget" : "remove" }}</button>
      </div>
      <div class="dim">{{ w.path }}<span v-if="w.adopted"> · adopted</span></div>
      <template v-if="open[w.name]">
        <p v-if="open[w.name].error" class="err">{{ open[w.name].error }}</p>
        <template v-else>
          <pre v-if="open[w.name].status">{{ open[w.name].status }}</pre>
          <details v-if="open[w.name].stat">
            <summary>changes against {{ w.base }}: <span class="dim">{{ open[w.name].stat.trim().split("\n").pop() }}</span></summary>
            <pre>{{ open[w.name].diff }}</pre>
          </details>
          <div v-else class="dim">no changes against {{ w.base }}</div>
          <form v-if="w.ahead" class="row" @submit.prevent="doMerge(w)">
            <select v-model="merge[w.name].strategy" :aria-label="`strategy for ${w.name}`">
              <option value="">merge</option>
              <option value="squash">squash</option>
              <option value="rebase">rebase</option>
            </select>
            <input v-model="merge[w.name].message" class="grow" :placeholder="`Merge ${w.branch} (${w.name})`" :aria-label="`merge message for ${w.name}`" />
            <button type="submit" :disabled="!!busy">merge into {{ w.base }}</button>
          </form>
          <h4>tasks</h4>
          <div v-if="!open[w.name].tasks.length" class="dim">none yet</div>
          <div v-for="t in open[w.name].tasks" :key="t.id">
            <a :href="`#/task/${t.id}`">{{ t.title }}</a> <span class="dim">{{ t.state }} · {{ ago(t.created) }} ago<span v-if="t.parent"> · subtask</span></span>
          </div>
        </template>
      </template>
      <p v-if="said[w.name]" class="dim">{{ said[w.name] }}</p>
    </div>

    <form class="row" @submit.prevent="add">
      <input v-model="form.name" placeholder="name" aria-label="worktree name" required />
      <input v-model="form.branch" placeholder="branch (default: the name)" aria-label="worktree branch" />
      <input v-model="form.base" placeholder="base (default: the folder's branch)" aria-label="worktree base" />
      <select v-model="form.sharing" aria-label="worktree sharing">
        <option value="">project's sharing</option>
        <option value="shared">shared</option>
        <option value="exclusive">one task at a time</option>
      </select>
      <button type="submit" :disabled="!!busy">{{ busy === "adding" ? "making…" : "make a worktree" }}</button>
    </form>
    <template v-if="adoptable.length">
      <h4>worktrees git has</h4>
      <div v-for="a in adoptable" :key="a.path" class="row">
        <span>{{ a.branch }}</span> <span class="dim">{{ a.path }}</span>
        <button type="button" :disabled="!!busy" @click="adopt(a)">adopt</button>
      </div>
    </template>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
  </div>
  <p v-else-if="error" class="err" role="alert">{{ error }}</p>
</template>
