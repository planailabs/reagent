<script setup>
import { inject, onMounted, ref, watch } from "vue";
import { get, put } from "../lib/api.js";

const live = inject("live");
const projects = ref([]);
const form = ref({ slug: "", name: "", path: "" });
const error = ref("");

async function load() {
  projects.value = await get("/api/projects").catch((e) => ((error.value = e.message), []));
}

async function add() {
  error.value = "";
  try {
    const p = await put(`/api/projects/${encodeURIComponent(form.value.slug)}`, { name: form.value.name, path: form.value.path });
    form.value = { slug: "", name: "", path: "" };
    location.hash = `#/project/${p.slug}`;
  } catch (e) {
    error.value = e.message;
  }
}

onMounted(load);
watch(() => live.tick, load);
</script>

<template>
  <section>
    <h2>projects</h2>
    <table>
      <tr><th>project</th><th>folder</th><th>going on</th><th>merge</th></tr>
      <tr v-for="p in projects" :key="p.slug">
        <td><a :href="`#/project/${p.slug}`" class="hi">{{ p.name }}</a> <span class="dim">{{ p.slug }}</span></td>
        <td class="dim">{{ p.path }}</td>
        <td>{{ p.active }}</td>
        <td class="dim">{{ p.merge }}</td>
      </tr>
    </table>
    <p v-if="!projects.length" class="dim">no projects yet: add a folder below</p>
    <h2>add a project</h2>
    <form class="row" @submit.prevent="add">
      <input v-model="form.slug" placeholder="id (lowercase, -)" aria-label="id" required />
      <input v-model="form.name" placeholder="name" aria-label="name" />
      <input v-model="form.path" class="grow" placeholder="/path/to/the/folder" aria-label="folder" required />
      <button type="submit">add</button>
    </form>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
  </section>
</template>
