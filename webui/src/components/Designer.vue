<script setup>
// The prompt designer, as a guide: a rough goal, its questions (pick an
// answer or write one), then a proposal to use, with suggestions (a repo
// skill, a cron entry, a trigger) to tick and make.
import { ref } from "vue";
import { post } from "../lib/api.js";
import { markdown } from "../lib/markdown.js";

const props = defineProps({ project: { type: String, required: true }, target: { type: String, default: "task" } });
const emit = defineEmits(["use"]);
const goal = ref("");
const answers = ref([]);
const questions = ref([]);
const picked = ref({});
const proposal = ref(null);
const chosen = ref([]);
const made = ref([]);
const busy = ref(false);
const error = ref("");
const open = ref(false);

async function round(propose = false) {
  error.value = "";
  busy.value = true;
  try {
    // This round's answers join the earlier ones.
    for (const [i, q] of questions.value.entries()) {
      const a = (picked.value[i] ?? "").trim();
      answers.value.push({ question: q.question, answer: a || "no preference: decide" });
    }
    const step = await post("/api/design", { project: props.project, target: props.target, goal: goal.value, answers: answers.value, propose });
    questions.value = step.questions ?? [];
    picked.value = {};
    proposal.value = step.proposal ?? null;
    chosen.value = (proposal.value?.suggestions ?? []).map(() => true);
  } catch (e) {
    error.value = e.message;
  } finally {
    busy.value = false;
  }
}

async function lookFirst() {
  error.value = "";
  try {
    const t = await post("/api/design/task", { project: props.project, target: props.target, goal: goal.value });
    location.hash = `#/task/${t.id}`;
  } catch (e) {
    error.value = e.message;
  }
}

async function make() {
  const suggestions = proposal.value.suggestions.filter((_, i) => chosen.value[i]);
  if (!suggestions.length) return;
  try {
    made.value = await post("/api/design/apply", { project: props.project, suggestions });
  } catch (e) {
    error.value = e.message;
  }
}

function restart() {
  answers.value = [];
  questions.value = [];
  proposal.value = null;
  made.value = [];
}

const label = (s) =>
  s.type === "skill" ? `repo skill ${s.name}: ${s.description}` : s.type === "cron" ? `cron entry (${s.expr} ${s.tz}): ${s.title}` : `${s.mode} trigger ${s.name}${s.repo ? " in the repo" : ""}: ${s.title}`;
</script>

<template>
  <details class="designer card" :open="open" @toggle="open = $event.target.open">
    <summary>design it with the guide <span class="dim">— describe what you want; it asks, then proposes{{ target === "task" ? " (and what to keep: a skill, a cron entry, a trigger)" : "" }}</span></summary>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
    <template v-if="!questions.length && !proposal">
      <textarea v-model="goal" rows="3" placeholder="what do you want done? roughly is fine" aria-label="goal"></textarea>
      <div class="row">
        <button type="button" :disabled="busy || !goal.trim()" @click="round()">{{ busy ? "thinking…" : "design" }}</button>
        <button v-if="target === 'task'" type="button" :disabled="!goal.trim()" title="a read-only task reads the project first, asks you in the usual way, and reports a proposal" @click="lookFirst">look at the project first</button>
      </div>
    </template>
    <form v-else-if="questions.length" class="stack" @submit.prevent="round()">
      <div v-for="(q, i) in questions" :key="i" class="question" :aria-label="`designer question ${i + 1}`">
        <div class="hi">{{ q.question }}</div>
        <div class="row">
          <label v-for="o in q.options" :key="o"><input v-model="picked[i]" type="radio" :value="o" :name="`q${i}`" /> {{ o }}</label>
        </div>
        <input v-model="picked[i]" placeholder="or in your words (empty: it decides)" :aria-label="`answer ${i + 1}`" />
      </div>
      <div class="row">
        <button type="submit" :disabled="busy">{{ busy ? "thinking…" : "next" }}</button>
        <button type="button" :disabled="busy" @click="round(true)">propose now</button>
        <button type="button" @click="restart">start over</button>
      </div>
    </form>
    <div v-else class="stack" aria-label="proposal">
      <div><span class="dim">title</span> <span class="hi">{{ proposal.title }}</span></div>
      <div class="md proposal-prompt" v-html="markdown(proposal.prompt)"></div>
      <div class="dim">
        <span v-if="proposal.skills?.length">skills: {{ proposal.skills.join(", ") }} · </span>
        <span v-if="proposal.kind">kind {{ proposal.kind }} · </span>
        <span v-if="proposal.profile">profile {{ proposal.profile }} · </span>
        <span v-if="proposal.budget">budget {{ JSON.stringify(proposal.budget) }} · </span>
        {{ proposal.note }}
      </div>
      <div v-if="proposal.suggestions?.length" class="stack">
        <div class="dim">it also suggests keeping:</div>
        <div v-for="(s, i) in proposal.suggestions" :key="i" class="suggestion">
          <label><input v-model="chosen[i]" type="checkbox" /> {{ label(s) }}</label>
          <span class="dim"> — {{ s.why }}</span>
          <label v-if="s.type === 'trigger'" class="dim"><input v-model="s.repo" type="checkbox" /> in the repo</label>
          <details><summary class="dim">see it</summary><pre>{{ s.type === "skill" ? s.body : s.type === "trigger" ? `${s.script}\n\n---\n${s.prompt}` : s.prompt }}</pre></details>
        </div>
        <button type="button" @click="make">make the ticked ones</button>
        <div v-for="(m, i) in made" :key="i" :class="m.ok ? 'dim' : 'err'">{{ m.ok ? `✓ ${m.done}` : `✗ ${m.error}` }}</div>
      </div>
      <div class="row">
        <button type="button" @click="emit('use', proposal)">use it below</button>
        <button type="button" @click="restart">ask again</button>
      </div>
    </div>
  </details>
</template>
