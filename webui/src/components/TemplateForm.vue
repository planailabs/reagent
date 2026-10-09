<script setup>
// A prompt template's picker and, for a form-like one, its fields with the prompt rendered live.
import { computed } from "vue";

const props = defineProps({ templates: { type: Array, required: true }, project: { type: String, default: "" } });
const chosen = defineModel("chosen", { type: String, default: "" });
const values = defineModel("values", { type: Object, default: () => ({}) });
const emit = defineEmits(["pick"]);

const key = (t) => `${t.source}:${t.project ?? ""}:${t.name}`;
const usable = computed(() => props.templates.filter((t) => !t.error));
const current = computed(() => usable.value.find((t) => key(t) === chosen.value) ?? null);

function pick(k) {
  chosen.value = k;
  const t = usable.value.find((x) => key(x) === k) ?? null;
  const v = {};
  for (const f of t?.fields ?? []) v[f.name] = f.type === "checkbox" ? f.default === true || f.default === "yes" : (f.default ?? "");
  values.value = v;
  emit("pick", t);
}

/** The prompt (or title) as it will be: {{field}} filled, like the server does. */
function render(text) {
  const t = current.value;
  return (text ?? "").replace(/\{\{\s*([A-Za-z0-9_.-]+)\s*\}\}/g, (_, name) => {
    const f = t?.fields.find((x) => x.name === name);
    const v = values.value[name];
    if (f?.type === "checkbox") return v ? "yes" : "no";
    return v ?? "";
  });
}
defineExpose({ render, current });
</script>

<template>
  <div class="stack">
    <label class="row">template
      <select :value="chosen" aria-label="template" class="grow" @change="pick($event.target.value)">
        <option value="">none: write the prompt</option>
        <option v-for="t in usable" :key="key(t)" :value="key(t)">
          {{ t.name }} — {{ t.description }} ({{ t.source }}{{ t.project && t.project !== project ? `, for ${t.project}` : "" }}{{ t.fields.length ? ", form" : "" }})
        </option>
      </select>
    </label>
    <div v-if="current?.fields.length" class="stack card" aria-label="template form">
      <template v-for="f in current.fields" :key="f.name">
        <label v-if="f.type === 'checkbox'" class="row"><input v-model="values[f.name]" type="checkbox" :aria-label="f.label || f.name" /> {{ f.label || f.name }}</label>
        <label v-else class="stack">
          <span>{{ f.label || f.name }}<span v-if="f.required" class="err"> *</span></span>
          <select v-if="f.type === 'select'" v-model="values[f.name]" :aria-label="f.label || f.name" :required="f.required">
            <option v-if="!f.required" value=""></option>
            <option v-for="o in f.options" :key="o" :value="o">{{ o }}</option>
          </select>
          <textarea v-else-if="f.type === 'multiline'" v-model="values[f.name]" rows="3" :aria-label="f.label || f.name" :required="f.required"></textarea>
          <input v-else v-model="values[f.name]" :aria-label="f.label || f.name" :required="f.required" />
        </label>
      </template>
      <details open>
        <summary class="dim">the prompt</summary>
        <pre aria-label="rendered prompt">{{ render(current.prompt) }}</pre>
      </details>
    </div>
  </div>
</template>
