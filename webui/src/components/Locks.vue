<script setup>
import { inject, onMounted, ref, watch } from "vue";
import { del, get } from "../lib/api.js";

// A project's locks and the global ones: who holds each, who waits; the person breaks one.
const props = defineProps({ slug: { type: String, required: true } });
const live = inject("live");
const locks = ref([]);
const error = ref("");

async function load() {
  locks.value = await get(`/api/projects/${props.slug}/locks`).catch((e) => ((error.value = e.message), []));
}

async function breakLock(l) {
  if (!confirm(`Break ${l.lock}? Its holders lose it (a task is told).`)) return;
  error.value = "";
  await del(`/api/locks/${l.global ? "global" : props.slug}/${encodeURIComponent(l.name)}`).catch((e) => (error.value = e.message));
  await load();
}

/** A task holder links to its task. */
const href = (h) => (h.startsWith("task:") ? `#/task/${h.slice(5)}` : null);

onMounted(load);
watch(() => [props.slug, live.tick], load);
</script>

<template>
  <section aria-label="locks">
    <p class="dim">
      What may not happen at once. Tasks, cron entries, triggers and git sync name the locks they need (<code>deploy</code>, <code>db:shared</code>, <code>global:staging</code>) and wait, skip or fail while one is busy; tasks also take them with locks.lock_acquire. A task's locks are its task tree's until it's over. The known ones and the defaults are in the settings.
    </p>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
    <p v-if="!locks.length" class="dim">no locks held, none known</p>
    <div v-for="l in locks" :key="l.lock" class="card" :aria-label="`lock ${l.lock}`">
      <div class="row">
        <span class="hi">{{ l.lock }}</span>
        <span v-if="l.about" class="dim">{{ l.about }}</span>
        <span class="grow"></span>
        <button v-if="l.holders.length" @click="breakLock(l)">break</button>
      </div>
      <div v-if="!l.holders.length" class="dim">free</div>
      <div v-for="h in l.holders" :key="h.holder">
        held by <a v-if="href(h.holder)" :href="href(h.holder)">{{ h.who }}</a><span v-else>{{ h.who }}</span>{{ h.shared ? " (shared)" : "" }} <span class="dim">since {{ new Date(h.since * 1000).toLocaleString() }}</span>
      </div>
      <div v-for="w in l.waiting" :key="w.holder" class="dim">
        waits: <a v-if="href(w.holder)" :href="href(w.holder)">{{ w.who }}</a><span v-else>{{ w.who }}</span>{{ w.shared ? " (shared)" : "" }}
      </div>
    </div>
  </section>
</template>
