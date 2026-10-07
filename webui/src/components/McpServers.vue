<script setup>
import { inject, onMounted, ref, watch } from "vue";
import { del, get, put } from "../lib/api.js";

// A project's own servers, or (none) the ones every task gets.
const props = defineProps({ project: { type: String, default: null } });
const live = inject("live");
const servers = ref([]);
const error = ref("");
const busy = ref(false);
const blank = () => ({ name: "", description: "", kind: "url", url: "", command: "", env: "", header: "Authorization", header_env: "", prefix: "Bearer ", lazy: true, idempotent: "", enabled: true });
const form = ref(blank());

async function load() {
  servers.value = await get(`/api/mcp?project=${encodeURIComponent(props.project ?? "global")}`).catch((e) => ((error.value = e.message), []));
}

function edit(m) {
  form.value = {
    name: m.name,
    description: m.description,
    kind: m.url ? "url" : "command",
    url: m.url || "",
    command: (m.command || []).join(" "),
    env: Object.entries(m.env || {}).map(([k, v]) => `${k}=${v}`).join("\n"),
    header: m.credential?.header || "Authorization",
    header_env: m.credential?.env || "",
    prefix: m.credential?.prefix ?? "Bearer ",
    lazy: m.lazy,
    idempotent: (m.idempotent || []).join(", "),
    enabled: m.enabled,
  };
}

async function save() {
  error.value = "";
  busy.value = true;
  const f = form.value;
  const body = {
    description: f.description,
    url: f.kind === "url" ? f.url : null,
    command: f.kind === "command" ? f.command.split(/\s+/).filter(Boolean) : null,
    env: Object.fromEntries(f.env.split("\n").map((l) => l.trim()).filter(Boolean).map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)])),
    credential: f.kind === "url" && f.header_env ? { header: f.header, env: f.header_env, prefix: f.prefix } : null,
    lazy: f.lazy,
    idempotent: f.idempotent.split(",").map((s) => s.trim()).filter(Boolean),
    enabled: f.enabled,
    project: props.project,
  };
  try {
    const r = await put(`/api/mcp/${encodeURIComponent(f.name)}`, body);
    error.value = r.status?.ok ? `${f.name} runs` : `${f.name} was added, but it doesn't run: ${r.status?.error || "unknown"}`;
    form.value = blank();
    await load();
  } catch (e) {
    error.value = e.message;
  } finally {
    busy.value = false;
  }
}

async function remove(m) {
  if (!confirm(`Remove ${m.name}? Tasks lose its tools.`)) return;
  busy.value = true;
  await del(`/api/mcp/${encodeURIComponent(m.name)}`).catch((e) => (error.value = e.message));
  busy.value = false;
  await load();
}

onMounted(load);
watch(() => [live.tick, props.project], load);
</script>

<template>
  <section :aria-label="project ? `mcp servers of ${project}` : 'mcp servers for every task'">
    <p class="dim">{{ project ? `This project's tasks get these servers' tools` : "Every task gets these servers' tools" }} as <code>&lt;name&gt;.&lt;tool&gt;</code>: lazily (it sees their names and loads what it needs) unless eager. The project's policy decides each call. Names are unique across all projects.</p>
    <table>
      <tr><th>server</th><th>where</th><th>tools</th><th>state</th><th></th></tr>
      <tr v-for="m in servers" :key="m.name">
        <td class="hi">{{ m.name }} <span class="dim">{{ m.description }}</span></td>
        <td class="dim"><code>{{ m.url || (m.command || []).join(" ") }}</code></td>
        <td>{{ m.lazy ? "lazy" : "eager" }}</td>
        <td :class="{ dim: m.status?.ok }">{{ m.status?.ok ? "runs" : m.status?.error }}</td>
        <td class="row"><button @click="edit(m)">edit</button><button @click="remove(m)">remove</button></td>
      </tr>
    </table>
    <p v-if="!servers.length" class="dim">no servers added</p>
    <form class="stack" @submit.prevent="save">
      <div class="row">
        <input v-model="form.name" placeholder="name (its tools: name.tool)" aria-label="server name" required />
        <input v-model="form.description" class="grow" placeholder="what it's for" aria-label="server description" />
      </div>
      <div class="row">
        <label><input v-model="form.kind" type="radio" value="url" /> URL (streamable HTTP)</label>
        <label><input v-model="form.kind" type="radio" value="command" /> command (stdio)</label>
      </div>
      <template v-if="form.kind === 'url'">
        <input v-model="form.url" placeholder="https://…/mcp" aria-label="server url" />
        <div class="row">
          header <input v-model="form.header" size="14" aria-label="header" />
          from env <input v-model="form.header_env" size="18" placeholder="e.g. WEB_MCP_TOKEN" aria-label="header env" />
          prefix <input v-model="form.prefix" size="8" aria-label="prefix" />
          <span class="dim">(the value comes from reagent's environment: its .env)</span>
        </div>
      </template>
      <template v-else>
        <input v-model="form.command" placeholder="program --and args" aria-label="server command" />
        <textarea v-model="form.env" rows="3" placeholder="KEY=value per line ($VAR takes reagent's)" aria-label="server env"></textarea>
      </template>
      <div class="row">
        <label><input v-model="form.lazy" type="checkbox" /> lazy (tools loaded when needed)</label>
        <label><input v-model="form.enabled" type="checkbox" /> on</label>
        <input v-model="form.idempotent" class="grow" placeholder="tools safe to run again (comma separated)" aria-label="idempotent tools" />
      </div>
      <button type="submit" :disabled="busy">{{ busy ? "applying…" : "save and apply" }}</button>
    </form>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
  </section>
</template>
