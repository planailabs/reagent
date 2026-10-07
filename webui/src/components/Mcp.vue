<script setup>
import { inject, onMounted, ref, watch } from "vue";
import { get } from "../lib/api.js";
import McpServers from "./McpServers.vue";

const live = inject("live");
const projects = ref([]);
const load = async () => (projects.value = await get("/api/projects").catch(() => []));
onMounted(load);
watch(() => live.tick, load);
</script>

<template>
  <section>
    <h2>MCP servers for every task</h2>
    <McpServers />
    <h2>per project</h2>
    <p v-if="!projects.length" class="dim">no projects yet</p>
    <details v-for="p in projects" :key="p.slug" class="card">
      <summary><span class="hi">{{ p.name }}</span> <span class="dim">{{ p.slug }}</span></summary>
      <McpServers :project="p.slug" />
    </details>
  </section>
</template>
