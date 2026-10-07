<script setup>
import { onMounted, ref, watch } from "vue";
import { del, get, put } from "../lib/api.js";

// A project's own secrets, or (none) every project's.
const props = defineProps({ project: { type: String, default: null } });
const secrets = ref([]);
const shown = ref({});
const form = ref({ name: "", value: "" });
const error = ref("");
const q = () => `project=${encodeURIComponent(props.project ?? "global")}`;

async function load() {
  secrets.value = await get(`/api/secrets?${q()}`).catch((e) => ((error.value = e.message), []));
}

async function save() {
  error.value = "";
  try {
    await put("/api/secrets", { project: props.project, name: form.value.name.trim(), value: form.value.value });
    error.value = `${form.value.name} saved`;
    form.value = { name: "", value: "" };
    await load();
  } catch (e) {
    error.value = e.message;
  }
}

async function remove(s) {
  if (!confirm(`Remove ${s.name}?`)) return;
  await del(`/api/secrets?${q()}&name=${encodeURIComponent(s.name)}`).catch((e) => (error.value = e.message));
  await load();
}

onMounted(load);
watch(() => props.project, load);
</script>

<template>
  <section :aria-label="project ? `secrets of ${project}` : 'secrets for every project'">
    <p class="dim">
      {{ project ? "This project's tasks get these" : "Every project's tasks get these" }} as environment variables in their commands{{ project ? " (over every project's of the same name)" : "" }}, and can read them with secrets.secrets_get; their values in other tool results show as ***. Kept encrypted.
    </p>
    <table>
      <tr><th>name</th><th>value</th><th>changed</th><th></th></tr>
      <tr v-for="s in secrets" :key="s.name">
        <td class="hi"><code>{{ s.name }}</code></td>
        <td><code v-if="shown[s.name]" class="secret">{{ s.value }}</code><span v-else class="dim">••••••••</span></td>
        <td class="dim">{{ new Date(s.updated * 1000).toLocaleString() }}</td>
        <td class="row">
          <button @click="shown[s.name] = !shown[s.name]">{{ shown[s.name] ? "hide" : "show" }}</button>
          <button @click="form = { name: s.name, value: s.value }">edit</button>
          <button @click="remove(s)">remove</button>
        </td>
      </tr>
    </table>
    <p v-if="!secrets.length" class="dim">none</p>
    <form class="row" @submit.prevent="save">
      <input v-model="form.name" placeholder="NAME" aria-label="secret name" required />
      <input v-model="form.value" class="grow" type="password" placeholder="value" aria-label="secret value" autocomplete="off" required />
      <button type="submit">save</button>
    </form>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
  </section>
</template>
