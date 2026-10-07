<script setup>
import { ref } from "vue";
import { get } from "../lib/api.js";
import { glyph } from "../lib/format.js";

const q = ref("");
const res = ref(null);
const one = ref(null);
const error = ref("");

async function search() {
  error.value = "";
  one.value = null;
  try {
    res.value = await get(`/api/search?q=${encodeURIComponent(q.value)}`);
  } catch (e) {
    error.value = e.message;
  }
}

async function all(task, page = 1) {
  try {
    one.value = await get(`/api/search?q=${encodeURIComponent(q.value)}&task=${task}&page=${page}`);
  } catch (e) {
    error.value = e.message;
  }
}
</script>

<template>
  <section>
    <h2>search every task's conversation</h2>
    <form class="row" @submit.prevent="search">
      <input v-model="q" class="grow" placeholder="a regular expression" aria-label="search" required />
      <button type="submit">search</button>
    </form>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
    <div v-if="res && !res.tasks.length" class="dim">no conversation matches</div>
    <div v-for="r in res?.tasks ?? []" :key="r.task.id" class="card">
      <span class="glyph">{{ glyph(r.task.state) }}</span>
      <a :href="`#/task/${r.task.id}`" class="hi">{{ r.task.title }}</a>
      <span class="dim">{{ r.task.project }} · {{ r.matches }} matches</span>
      <button @click="all(r.task.id)">all of them</button>
      <pre v-for="h in r.hits" :key="h.n" class="dim">#{{ h.n }} {{ h.role }}: {{ h.text }}</pre>
    </div>
    <div v-if="one" class="card">
      <h2>{{ one.task.title }}: {{ one.matches }} matches, page {{ one.page }} of {{ one.pages }}</h2>
      <pre v-for="h in one.hits" :key="h.n">#{{ h.n }} {{ h.role }}{{ h.summarised ? " (summarised)" : "" }}: {{ h.text }}</pre>
      <button v-if="one.next_page" @click="all(one.task.id, one.next_page)">more</button>
    </div>
  </section>
</template>
