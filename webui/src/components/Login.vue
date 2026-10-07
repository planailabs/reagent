<script setup>
import { ref } from "vue";
import { post } from "../lib/api.js";

defineProps({ passwordSet: Boolean });
const emit = defineEmits(["in"]);
const password = ref("");
const error = ref("");

async function login() {
  error.value = "";
  try {
    await post("/api/login", { password: password.value });
    password.value = "";
    emit("in");
  } catch (e) {
    error.value = e.message;
  }
}
</script>

<template>
  <main class="login">
    <h1>reagent</h1>
    <p v-if="!passwordSet" class="err">No password is set yet: run <code>reagent passwd</code> on the machine.</p>
    <form class="row" @submit.prevent="login">
      <input v-model="password" type="password" placeholder="password" aria-label="password" autofocus autocomplete="current-password" />
      <button type="submit">log in</button>
    </form>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
  </main>
</template>
