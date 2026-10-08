<script setup>
import { markdown } from "../lib/markdown.js";
import { inject, onMounted, ref, watch } from "vue";
import { get, post } from "../lib/api.js";
import { ago } from "../lib/format.js";
import TaskRow from "./TaskRow.vue";

const live = inject("live");
const inbox = ref(null);
const active = ref([]);
const error = ref("");

async function load() {
  try {
    inbox.value = await get("/api/inbox");
    active.value = (await get("/api/tasks?active=true")).filter((t) => t.state !== "waiting");
  } catch (e) {
    error.value = e.message;
  }
}

async function seen() {
  const top = inbox.value?.notifications?.[0]?.id;
  if (top) await post("/api/notifications/seen", { upto: top });
  await load();
}

onMounted(load);
watch(() => live.tick, load);
</script>

<template>
  <section>
    <h2>waiting for you</h2>
    <p v-if="error" class="err">{{ error }}</p>
    <div v-if="inbox && !inbox.waiting.length && !inbox.failed.length" class="dim">nothing waits for you</div>
    <TaskRow v-for="t in inbox?.waiting ?? []" :key="t.id" :t="t" project />
    <TaskRow v-for="t in inbox?.failed ?? []" :key="t.id" :t="t" project />
    <h2>going on</h2>
    <div v-if="!active.length" class="dim">no task is running</div>
    <TaskRow v-for="t in active" :key="t.id" :t="t" project />
    <h2 class="row">notifications <button v-if="inbox?.notifications?.length" @click="seen">mark seen</button></h2>
    <div v-if="!inbox?.notifications?.length" class="dim">none new</div>
    <div v-for="n in inbox?.notifications ?? []" :key="n.id" class="note">
      <span class="dim">{{ ago(n.at) }} · {{ n.kind }}</span>
      <a v-if="n.task" :href="`#/task/${n.task}`" class="hi">{{ n.title }}</a>
      <span v-else class="hi">{{ n.title }}</span>
      <div class="dim md" v-html="markdown(n.body)"></div>
    </div>
  </section>
</template>
