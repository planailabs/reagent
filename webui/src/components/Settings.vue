<script setup>
import { onMounted, ref } from "vue";
import { del, get, post, put } from "../lib/api.js";
import Memory from "./Memory.vue";
import Secrets from "./Secrets.vue";
import NotifyChannels from "./NotifyChannels.vue";
import Plugins from "./Plugins.vue";
import Templates from "./Templates.vue";

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
const evaluatorPrompt = ref("");
const reviewerPrompt = ref("");
async function loadLimits() {
  limits.value = await get("/api/settings").catch((e) => ((error.value = e.message), null));
  maxTasks.value = limits.value?.max_tasks ?? "";
  evaluatorPrompt.value = limits.value?.evaluator_prompt ?? "";
  reviewerPrompt.value = limits.value?.reviewer_prompt ?? "";
}
async function saveReviewer() {
  error.value = "";
  try {
    limits.value = await put("/api/settings", { reviewer_prompt: reviewerPrompt.value });
    error.value = "saved";
  } catch (e) {
    error.value = e.message;
  }
}
async function saveEvaluator() {
  error.value = "";
  try {
    limits.value = await put("/api/settings", { evaluator_prompt: evaluatorPrompt.value });
    error.value = "saved";
  } catch (e) {
    error.value = e.message;
  }
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
    <h2>plugins <span class="dim">(git repositories whose skills and rules every task gets)</span></h2>
    <Plugins />
    <h2>prompt templates <span class="dim">(every project's)</span></h2>
    <Templates />
    <h2>tasks at once</h2>
    <form class="row" @submit.prevent="saveLimits">
      <label>at most <input v-model="maxTasks" size="4" inputmode="numeric" placeholder="any" aria-label="tasks at once" /> tasks run at once</label>
      <button type="submit">save</button>
      <span class="dim">more wait, queued, and start oldest first; subtasks don't count; a project can set its own limit too. Now: {{ limits?.running ?? "?" }} running, {{ limits?.queued ?? "?" }} queued.</span>
    </form>
    <h2>call evaluator <span class="dim">({{ config.evaluator_profile ? `profile ${config.evaluator_profile}` : "off: set evaluator_profile in reagent.hcl" }})</span></h2>
    <form class="stack" @submit.prevent="saveEvaluator">
      <span class="dim">Calls the rules would ask you about go to a second model first; per project it decides (approved calls run, rejected ones are refused or come to you) or advises you. What you add here goes into its instructions for every project (a project adds its own).</span>
      <textarea v-model="evaluatorPrompt" rows="4" placeholder="e.g. Never let a call reach production hosts." aria-label="evaluator instructions for every project"></textarea>
      <div><button type="submit">save</button></div>
    </form>
    <h2>merge reviewer <span class="dim">({{ config.reviewer_profile || config.evaluator_profile ? `profile ${config.reviewer_profile || config.evaluator_profile}` : "off: set reviewer_profile or evaluator_profile in reagent.hcl" }})</span></h2>
    <form class="stack" @submit.prevent="saveReviewer">
      <span class="dim">With a project's merging on review, a review subtask reads a task's change before it's merged: approved, it's merged; rejected, it goes back to the task. What you add here goes into its instructions for every project (a project adds its own).</span>
      <textarea v-model="reviewerPrompt" rows="4" placeholder="e.g. Every change needs a test that fails without it." aria-label="reviewer instructions for every project"></textarea>
      <div><button type="submit">save</button></div>
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
