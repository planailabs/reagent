<script setup>
// reagent's own documentation: the system skills tasks read too.
import { onMounted, ref, watch } from "vue";
import { get } from "../lib/api.js";
import { markdown } from "../lib/markdown.js";

const props = defineProps({ name: { type: String, default: "" } });
const list = ref([]);
const doc = ref(null);
const error = ref("");

async function load() {
  list.value = await get("/api/docs").catch((e) => ((error.value = e.message), []));
  const name = props.name || list.value[0]?.name;
  doc.value = name ? await get(`/api/docs/${name}`).catch((e) => ((error.value = e.message), null)) : null;
}
onMounted(load);
watch(() => props.name, load);
</script>

<template>
  <section class="docs">
    <p v-if="error" class="err">{{ error }}</p>
    <nav class="stack doc-list">
      <a v-for="d in list" :key="d.name" :href="`#/docs/${d.name}`" :class="{ on: doc?.name === d.name }" :title="d.description">{{ d.name }}</a>
    </nav>
    <article v-if="doc" class="md" v-html="markdown(doc.body)"></article>
  </section>
</template>
