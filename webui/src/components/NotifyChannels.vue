<script setup>
// apprise channels: where notifications go besides this browser.
import { onMounted, ref } from "vue";
import { del, get, post, put } from "../lib/api.js";

const KINDS = ["done", "failed", "waiting", "budget", "cron", "trigger", "model", "secret"];
const list = ref(null);
const form = ref({ name: "", url: "", events: [] });
const tested = ref({});
const error = ref("");

async function load() {
  list.value = await get("/api/notify/channels").catch((e) => ((error.value = e.message), null));
}

async function run(f) {
  error.value = "";
  try {
    await f();
  } catch (e) {
    error.value = e.message;
  }
  await load();
}

const add = () =>
  run(async () => {
    await post("/api/notify/channels", form.value);
    form.value = { name: "", url: "", events: [] };
  });
const toggle = (c) => run(() => put(`/api/notify/channels/${c.id}`, { enabled: !c.enabled }));
const remove = (c) => confirm(`Remove the channel ${c.name}?`) && run(() => del(`/api/notify/channels/${c.id}`));
async function test(c) {
  tested.value[c.id] = "sending…";
  const r = await post(`/api/notify/channels/${c.id}/test`).catch((e) => ({ ok: false, error: e.message }));
  tested.value[c.id] = r.ok ? "sent" : `failed: ${r.error}`;
}

onMounted(load);
</script>

<template>
  <div v-if="list">
    <table v-if="list.channels.length">
      <tr><th>name</th><th>URL</th><th>gets</th><th>on</th><th></th></tr>
      <tr v-for="c in list.channels" :key="c.id" :aria-label="`channel ${c.name}`">
        <td class="hi">{{ c.name }}</td>
        <td class="dim">{{ c.url }}</td>
        <td>{{ c.events ? c.events.join(", ") : "everything" }}</td>
        <td><input type="checkbox" :checked="c.enabled" :aria-label="`${c.name} on`" @change="toggle(c)" /></td>
        <td class="row">
          <button @click="test(c)">send a test</button>
          <button @click="remove(c)">remove</button>
          <span v-if="tested[c.id]" :class="tested[c.id].startsWith('failed') ? 'err' : 'dim'">{{ tested[c.id] }}</span>
        </td>
      </tr>
    </table>
    <p v-else class="dim">no channels yet</p>
    <p v-if="list.config_urls" class="dim">and {{ list.config_urls }} URL(s) from reagent.hcl's notify block</p>
    <form class="stack" @submit.prevent="add">
      <div class="row">
        <input v-model="form.name" placeholder="a name (telegram)" aria-label="channel name" required />
        <input v-model="form.url" class="grow" placeholder="an apprise URL: tgram://bot-token/chat-id, ntfys://topic, mailto://…" aria-label="channel URL" required />
      </div>
      <div class="row">
        <span class="dim">gets (none ticked: everything)</span>
        <label v-for="k in KINDS" :key="k"><input v-model="form.events" type="checkbox" :value="k" /> {{ k }}</label>
        <button type="submit">add the channel</button>
      </div>
    </form>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
  </div>
</template>
