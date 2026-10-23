<script setup>
// Plugins: git repositories whose skills and rules every task gets.
import { onMounted, ref } from "vue";
import { del, get, post, put } from "../lib/api.js";
import { ago } from "../lib/format.js";

const list = ref(null);
const form = ref({ url: "", id: "", ref: "" });
const busy = ref("");
const said = ref({});
const error = ref("");

async function load() {
  list.value = await get("/api/plugins").catch((e) => ((error.value = e.message), null));
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
}

const outcome = (r) => (!r.ok ? `failed: ${r.error}` : r.changed ? `updated ${r.before} → ${r.after}` : "up to date");
const add = () =>
  run("adding", async () => {
    await post("/api/plugins", { url: form.value.url, id: form.value.id || null, ref: form.value.ref || null });
    form.value = { url: "", id: "", ref: "" };
  });
const update = (p) => run(p.id, async () => (said.value[p.id] = outcome(await post(`/api/plugins/${p.id}/update`))));
const updateAll = () => run("all", async () => (await post("/api/plugins/update")).forEach((r) => (said.value[r.id] = outcome(r))));
const toggle = (p, rule, on) =>
  run(p.id, () => put(`/api/plugins/${p.id}/rules-off`, { rules: on ? p.rules_off.filter((r) => r !== rule) : [...p.rules_off, rule] }));
const remove = (p) => confirm(`Remove the plugin ${p.id}?`) && run(p.id, () => del(`/api/plugins/${p.id}`));

onMounted(load);
</script>

<template>
  <div v-if="list">
    <table v-if="list.length">
      <tr><th>plugin</th><th>from</th><th>at</th><th>skills</th><th>rules</th><th></th></tr>
      <template v-for="p in list" :key="p.id">
      <tr :aria-label="`plugin ${p.id}`">
        <td class="hi">{{ p.id }}</td>
        <td class="dim">{{ p.url }}<span v-if="p.ref"> @ {{ p.ref }}</span></td>
        <td :title="p.subject">{{ p.commit }} <span class="dim">{{ ago(p.date) }} ago</span></td>
        <td>{{ p.skills.join(", ") || "—" }}</td>
        <td>
          <details v-if="p.rules.length">
            <summary>{{ p.rules.length }} rules<span v-if="p.rules_off.length" class="dim">, {{ p.rules_off.length }} off</span></summary>
            <label v-for="r in p.rules" :key="r" class="row">
              <input type="checkbox" :checked="!p.rules_off.includes(r)" :disabled="!!busy" :aria-label="`plugin rule ${r}`" @change="toggle(p, r, $event.target.checked)" />
              {{ r }}
            </label>
          </details>
          <span v-else>—</span>
        </td>
        <td class="row">
          <button :disabled="!!busy" @click="update(p)">update</button>
          <button :disabled="!!busy" @click="remove(p)">remove</button>
          <span v-if="busy === p.id" class="dim">…</span>
          <span v-else-if="said[p.id]" :class="said[p.id].startsWith('failed') ? 'err' : 'dim'">{{ said[p.id] }}</span>
        </td>
      </tr>
      <tr v-if="p.plugins.length" :aria-label="`marketplace ${p.id}`">
        <td colspan="6">
          <details>
            <summary>{{ p.plugins.length }} plugins<span v-if="p.marketplace" class="dim"> in the marketplace {{ p.marketplace }}</span></summary>
            <ul class="tree">
              <li v-for="x in p.plugins" :key="x.name" :aria-label="`plugin ${p.id}/${x.name}`">
                <span class="hi">{{ x.name }}</span> <span v-if="x.version" class="dim">{{ x.version }}</span>
                <span v-if="!x.error" class="dim"> {{ x.path }}</span>
                <div v-if="x.description" class="dim">{{ x.description }}</div>
                <div v-if="x.error" class="err">{{ x.error }}</div>
                <ul v-if="!x.error">
                  <li>skills: {{ x.skills.join(", ") || "—" }}</li>
                  <li v-if="x.rules.length">rules: {{ x.rules.join(", ") }}</li>
                </ul>
              </li>
            </ul>
          </details>
        </td>
      </tr>
      </template>
    </table>
    <p v-else class="dim">no plugins yet</p>
    <form class="row" @submit.prevent="add">
      <input v-model="form.url" class="grow" placeholder="a git URL: https://github.com/…, git@…" aria-label="plugin URL" required />
      <input v-model="form.id" placeholder="id (from the URL)" aria-label="plugin id" />
      <input v-model="form.ref" placeholder="branch or tag" aria-label="plugin ref" />
      <button type="submit" :disabled="!!busy">{{ busy === "adding" ? "cloning…" : "add the plugin" }}</button>
      <button v-if="list.length" type="button" :disabled="!!busy" @click="updateAll">{{ busy === "all" ? "updating…" : "update all" }}</button>
    </form>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
  </div>
</template>
