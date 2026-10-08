<script setup>
// The designer: a rough goal starts a design task that reads the project,
// asks, and proposes tasks, cron entries, triggers and repo skills (on its page).
import { ref } from "vue";
import { post } from "../lib/api.js";

const props = defineProps({ project: { type: String, required: true }, target: { type: String, default: "task" } });
const goal = ref("");
const error = ref("");
const busy = ref(false);

async function design() {
  error.value = "";
  busy.value = true;
  try {
    const t = await post("/api/design", { project: props.project, target: props.target, goal: goal.value });
    location.hash = `#/task/${t.id}`;
  } catch (e) {
    error.value = e.message;
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <details class="designer card">
    <summary>design it with the guide <span class="dim">— say roughly what you want; it reads the project, asks you, and proposes tasks, cron entries, triggers and skills to pick from</span></summary>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
    <textarea v-model="goal" rows="3" placeholder="what do you want done? roughly is fine" aria-label="goal"></textarea>
    <button type="button" :disabled="busy || !goal.trim()" @click="design">design</button>
  </details>
</template>
