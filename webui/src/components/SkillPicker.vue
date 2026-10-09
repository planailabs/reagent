<script setup>
// Skills to load, picked by searching: type to filter, Enter or a click adds; × removes.
import { computed, ref } from "vue";

const props = defineProps({ skills: { type: Array, required: true } });
const picked = defineModel({ type: Array, default: () => [] });
const q = ref("");
const open = ref(false);

const matches = computed(() => {
  const s = q.value.trim().toLowerCase();
  return props.skills.filter((k) => !picked.value.includes(k.name) && (!s || k.name.toLowerCase().includes(s) || (k.description || "").toLowerCase().includes(s))).slice(0, 12);
});

function add(name) {
  if (name && !picked.value.includes(name)) picked.value = [...picked.value, name];
  q.value = "";
  open.value = false;
}
const remove = (name) => (picked.value = picked.value.filter((n) => n !== name));
function onEnter() {
  if (matches.value.length) add(matches.value[0].name);
}
</script>

<template>
  <div class="picker stack">
    <div class="row">
      <span class="dim">skills to load</span>
      <span v-for="n in picked" :key="n" class="chip">{{ n }} <button type="button" :aria-label="`drop skill ${n}`" @click="remove(n)">×</button></span>
    </div>
    <input
      v-model="q"
      placeholder="search skills…"
      aria-label="search skills"
      @focus="open = true"
      @input="open = true"
      @blur="open = false"
      @keydown.enter.prevent="onEnter"
      @keydown.esc="open = false"
    />
    <ul v-if="open && matches.length" class="picks" role="listbox" aria-label="skills found">
      <li v-for="k in matches" :key="k.name" role="option" @mousedown.prevent="add(k.name)">
        <span class="hi">{{ k.name }}</span> <span class="dim">{{ k.source }} — {{ k.description }}</span>
      </li>
    </ul>
  </div>
</template>
