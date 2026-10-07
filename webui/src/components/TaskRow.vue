<script setup>
import { ago, glyph, money, waitText } from "../lib/format.js";

defineProps({ t: { type: Object, required: true }, depth: { type: Number, default: 0 }, project: Boolean });
</script>

<template>
  <a :href="`#/task/${t.id}`" :class="['task-row', t.state]" :style="{ paddingLeft: 8 + depth * 18 + 'px' }">
    <span class="glyph" :title="t.state">{{ glyph(t.state) }}</span>
    <span class="hi title">{{ t.title }}</span>
    <span v-if="project" class="dim">{{ t.project }}</span>
    <span class="dim wait">{{ waitText(t) }}</span>
    <span class="grow"></span>
    <span class="dim">{{ t.tokens }} tok · {{ money(t.cost) }} · {{ ago(t.updated) }}</span>
  </a>
</template>
