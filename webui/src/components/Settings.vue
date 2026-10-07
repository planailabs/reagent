<script setup>
import { onMounted, ref } from "vue";
import { get, post } from "../lib/api.js";
import Memory from "./Memory.vue";

const config = ref(null);
const push = ref("");
const error = ref("");

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

onMounted(load);
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
    <h2>global memory</h2>
    <Memory scope="global" />
  </section>
</template>
