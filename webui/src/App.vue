<script setup>
import { computed, onMounted, onUnmounted, provide, reactive, ref } from "vue";
import { get, post, subscribe } from "./lib/api.js";
import { applyAgent } from "./lib/format.js";
import Login from "./components/Login.vue";
import Inbox from "./components/Inbox.vue";
import Projects from "./components/Projects.vue";
import Project from "./components/Project.vue";
import Task from "./components/Task.vue";
import Search from "./components/Search.vue";
import Settings from "./components/Settings.vue";
import Mcp from "./components/Mcp.vue";

// Hash routes: #/inbox, #/projects, #/project/<slug>, #/task/<id>, #/search, #/settings.
const route = ref(location.hash.slice(1) || "/inbox");
const onHash = () => (route.value = location.hash.slice(1) || "/inbox");
const parts = computed(() => route.value.split("/").filter(Boolean));
const page = computed(() => parts.value[0] || "inbox");
const arg = computed(() => decodeURIComponent(parts.value[1] || ""));

const session = ref(null);
// What everyone shares: live events, streamed text, the inbox count.
const live = reactive({ tick: 0, tasks: {}, streams: {}, agentTick: {}, todos: {}, notifications: [], waiting: 0 });
provide("live", live);
let stop = null;

async function check() {
  session.value = await get("/api/session").catch(() => ({ logged_in: false, password_set: true }));
  if (session.value.logged_in && !stop) start();
}

async function refreshInbox() {
  const i = await get("/api/inbox").catch(() => null);
  if (i) live.waiting = i.waiting.length + i.failed.length;
}

function start() {
  refreshInbox();
  stop = subscribe((e) => {
    if (e.kind === "task") {
      live.tasks[e.task.id] = e.task;
      live.tick++;
      refreshInbox();
    } else if (e.kind === "notification") {
      live.notifications.unshift(e);
      live.notifications.length = Math.min(live.notifications.length, 50);
      refreshInbox();
    } else if (e.kind === "agent") {
      const n = e.notice;
      if (applyAgent(live.streams, n)) live.agentTick[n.agent] = (live.agentTick[n.agent] || 0) + 1;
    } else if (e.kind === "todos") {
      live.todos[e.task] = e.todos;
    } else if (e.kind === "job" || e.kind === "project") {
      live.tick++;
    }
  });
}

async function logout() {
  await post("/api/logout").catch(() => {});
  stop?.();
  stop = null;
  session.value = { logged_in: false, password_set: true };
}

const onLoggedOut = () => {
  stop?.();
  stop = null;
  session.value = { logged_in: false, password_set: true };
};

onMounted(() => {
  addEventListener("hashchange", onHash);
  addEventListener("reagent-logged-out", onLoggedOut);
  check();
});
onUnmounted(() => {
  removeEventListener("hashchange", onHash);
  removeEventListener("reagent-logged-out", onLoggedOut);
  stop?.();
});
</script>

<template>
  <Login v-if="session && !session.logged_in" :password-set="session.password_set" @in="check" />
  <template v-else-if="session">
    <header>
      <span class="brand">reagent</span>
      <a href="#/inbox" :class="{ on: page === 'inbox' }">inbox<span v-if="live.waiting" class="badge">{{ live.waiting }}</span></a>
      <a href="#/projects" :class="{ on: page === 'projects' || page === 'project' }">projects</a>
      <a href="#/search" :class="{ on: page === 'search' }">search</a>
      <a href="#/mcp" :class="{ on: page === 'mcp' }">mcp</a>
      <a href="#/settings" :class="{ on: page === 'settings' }">settings</a>
      <span class="grow"></span>
      <button @click="logout">log out</button>
    </header>
    <main>
      <Inbox v-if="page === 'inbox'" />
      <Projects v-else-if="page === 'projects'" />
      <Project v-else-if="page === 'project'" :slug="arg" :tab="parts[2] || 'tasks'" />
      <Task v-else-if="page === 'task'" :id="arg" />
      <Search v-else-if="page === 'search'" />
      <Settings v-else-if="page === 'settings'" />
      <Mcp v-else-if="page === 'mcp'" />
      <p v-else class="dim">nothing here: <a href="#/inbox">the inbox</a></p>
    </main>
  </template>
</template>
