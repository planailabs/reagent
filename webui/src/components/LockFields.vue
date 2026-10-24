<script setup>
import { computed } from "vue";

// The locks an action takes up front, and what it does when one is busy.
const props = defineProps({ locks: { type: Array, default: () => [] }, onBusy: { type: String, default: null }, defaultText: { type: String, default: "the project's" } });
const emit = defineEmits(["update:locks", "update:onBusy"]);
const text = computed({
  get: () => (props.locks ?? []).join(", "),
  set: (v) => emit("update:locks", v.split(/[\s,]+/).filter(Boolean)),
});
const busy = computed({
  get: () => props.onBusy ?? "",
  set: (v) => emit("update:onBusy", v || null),
});
</script>

<template>
  <div class="row">
    <label>locks <input v-model="text" size="24" placeholder="deploy, db:shared, global:staging" aria-label="locks" /></label>
    <label>when busy
      <select v-model="busy" aria-label="when a lock is busy">
        <option value="">default ({{ defaultText }})</option>
        <option value="wait">wait</option>
        <option value="skip">skip</option>
        <option value="fail">fail</option>
      </select>
    </label>
  </div>
</template>
