<script setup>
import { onMounted, ref, watch } from "vue";
import { del, get, put } from "../lib/api.js";

const props = defineProps({ scope: { type: String, required: true } });
const m = ref(null);
const file = ref(null);
const edit = ref({ file: "", text: "", about: "" });
const error = ref("");

async function load() {
  try {
    m.value = await get(`/api/memory?scope=${encodeURIComponent(props.scope)}`);
  } catch (e) {
    error.value = e.message;
  }
}

async function open(f) {
  const r = await get(`/api/memory?scope=${encodeURIComponent(props.scope)}&file=${encodeURIComponent(f)}`);
  file.value = f;
  const line = (m.value.index.split("\n").find((l) => l.includes(`](${f})`)) || "").split(" — ")[1] || "";
  edit.value = { file: f, text: r.text, about: line };
}

async function save() {
  error.value = "";
  try {
    await put("/api/memory", { scope: props.scope, ...edit.value });
    file.value = edit.value.file;
    await load();
    error.value = "saved";
  } catch (e) {
    error.value = e.message;
  }
}

async function remove() {
  if (!confirm(`Remove ${file.value}?`)) return;
  await del(`/api/memory?scope=${encodeURIComponent(props.scope)}&file=${encodeURIComponent(file.value)}`).catch((e) => (error.value = e.message));
  file.value = null;
  edit.value = { file: "", text: "", about: "" };
  await load();
}

onMounted(load);
watch(() => props.scope, load);
</script>

<template>
  <section v-if="m" class="split">
    <div>
      <p class="dim">{{ m.dir }}</p>
      <pre class="index">{{ m.index }}</pre>
      <h2>files</h2>
      <div v-for="f in m.files" :key="f"><a href="#" :class="{ hi: f === file }" @click.prevent="open(f)">{{ f }}</a></div>
      <p v-if="!m.files.length" class="dim">none yet</p>
    </div>
    <form class="stack" @submit.prevent="save">
      <h2>{{ file ? "edit" : "new file" }}</h2>
      <input v-model="edit.file" placeholder="topics/name.md" aria-label="memory file" required />
      <input v-model="edit.about" placeholder="one line for the index" aria-label="about" required />
      <textarea v-model="edit.text" aria-label="memory text" rows="18"></textarea>
      <div class="row"><button type="submit">save</button><button v-if="file" type="button" @click="remove">remove</button><button type="button" @click="(file = null), (edit = { file: '', text: '', about: '' })">new</button></div>
      <p v-if="error" class="err" role="alert">{{ error }}</p>
    </form>
  </section>
</template>
