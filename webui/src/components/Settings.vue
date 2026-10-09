<script setup>
import { onMounted, ref } from "vue";
import { del, get, post, put } from "../lib/api.js";
import Memory from "./Memory.vue";
import Secrets from "./Secrets.vue";
import NotifyChannels from "./NotifyChannels.vue";

const config = ref(null);
const push = ref("");
const origin = location.origin;
const error = ref("");
const tokens = ref([]);
const tokenName = ref("");
const newToken = ref(null);

async function loadTokens() {
  tokens.value = await get("/api/tokens").catch(() => []);
}

async function addToken() {
  error.value = "";
  try {
    newToken.value = await post("/api/tokens", { name: tokenName.value });
    tokenName.value = "";
    await loadTokens();
  } catch (e) {
    error.value = e.message;
  }
}

async function revoke(name) {
  if (!confirm(`Revoke ${name}? Agents using it lose access.`)) return;
  await del(`/api/tokens/${encodeURIComponent(name)}`).catch((e) => (error.value = e.message));
  await loadTokens();
}

async function load() {
  config.value = await get("/api/config").catch((e) => ((error.value = e.message), null));
  if ("serviceWorker" in navigator && "PushManager" in window) {
    const reg = await navigator.serviceWorker.getRegistration();
    const sub = await reg?.pushManager.getSubscription();
    push.value = sub ? "on" : "off";
  } else {
    push.value = "unsupported";
  }
}

function key(b64) {
  const s = atob(b64.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - (b64.length % 4)) % 4));
  return Uint8Array.from(s, (c) => c.charCodeAt(0));
}

async function enablePush() {
  error.value = "";
  try {
    const reg = await navigator.serviceWorker.register(new URL("../sw.js", import.meta.url), { type: "module" });
    const { key: k } = await get("/api/push/key");
    const sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key(k) });
    await post("/api/push/subscribe", sub.toJSON());
    push.value = "on";
  } catch (e) {
    error.value = `push: ${e.message}`;
  }
}

async function disablePush() {
  const reg = await navigator.serviceWorker.getRegistration();
  const sub = await reg?.pushManager.getSubscription();
  if (sub) {
    await post("/api/push/unsubscribe", { endpoint: sub.endpoint });
    await sub.unsubscribe();
  }
  push.value = "off";
}

const limits = ref(null);
const maxTasks = ref("");
async function loadLimits() {
  limits.value = await get("/api/settings").catch((e) => ((error.value = e.message), null));
  maxTasks.value = limits.value?.max_tasks ?? "";
}
async function saveLimits() {
  error.value = "";
  try {
    limits.value = await put("/api/settings", { max_tasks: maxTasks.value === "" ? null : Number(maxTasks.value) });
  } catch (e) {
    error.value = e.message;
  }
}

onMounted(() => {
  load();
  loadTokens();
  loadLimits();
});
</script>

<template>
  <section v-if="config">
    <h2>notifications on this browser</h2>
    <div class="row">
      <span>push: {{ push }}</span>
      <button v-if="push === 'off'" @click="enablePush">turn on</button>
      <button v-if="push === 'on'" @click="disablePush">turn off</button>
      <span class="dim">apprise: {{ config.notify.apprise ? "set up" : "not set up (reagent.hcl)" }}</span>
    </div>
    <h2>notification channels <span class="dim">(apprise: Telegram, ntfy, mail, …)</span></h2>
    <NotifyChannels />
    <h2>tasks at once</h2>
    <form class="row" @submit.prevent="saveLimits">
      <label>at most <input v-model="maxTasks" size="4" inputmode="numeric" placeholder="any" aria-label="tasks at once" /> tasks run at once</label>
      <button type="submit">save</button>
      <span class="dim">more wait, queued, and start oldest first; subtasks don't count; a project can set its own limit too. Now: {{ limits?.running ?? "?" }} running, {{ limits?.queued ?? "?" }} queued.</span>
    </form>
    <h2>model profiles <span class="dim">(reagent.hcl)</span></h2>
    <table>
      <tr><th>profile</th><th>model</th><th>provider</th><th>price in/out per M</th><th>context</th></tr>
      <tr v-for="p in config.profiles" :key="p.name">
        <td class="hi">{{ p.name }}{{ p.name === config.default_profile ? " (default)" : "" }}</td>
        <td>{{ p.model }}</td>
        <td class="dim">{{ p.provider }}</td>
        <td>{{ p.price.input }} / {{ p.price.output }}</td>
        <td>{{ p.context }}</td>
      </tr>
    </table>
    <p class="dim">tool results over {{ config.grep_results.over || "∞" }} characters reach a task cut (grep_result reads the rest)</p>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
    <p class="dim">MCP servers for tasks: <a href="#/mcp">the mcp page</a>.</p>
    <h2>secrets for every project</h2>
    <Secrets />
    <h2>API tokens <span class="dim">(for agents using reagent's MCP API at {{ origin }}/mcp)</span></h2>
    <table>
      <tr><th>name</th><th>made</th><th>last used</th><th></th></tr>
      <tr v-for="t in tokens" :key="t.name">
        <td class="hi">{{ t.name }}</td>
        <td class="dim">{{ new Date(t.created * 1000).toLocaleString() }}</td>
        <td class="dim">{{ t.last_used ? new Date(t.last_used * 1000).toLocaleString() : "never" }}</td>
        <td><button @click="revoke(t.name)">revoke</button></td>
      </tr>
    </table>
    <form class="row" @submit.prevent="addToken">
      <input v-model="tokenName" placeholder="a name (who uses it)" aria-label="token name" required />
      <button type="submit">make a token</button>
    </form>
    <div v-if="newToken" class="err" aria-label="new token">
      the token for <span class="hi">{{ newToken.name }}</span> (shown only now): <code>{{ newToken.token }}</code>
      <div class="dim">send it as <code>Authorization: Bearer …</code></div>
    </div>
    <h2>global memory</h2>
    <Memory scope="global" />
  </section>
</template>
