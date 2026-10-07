<script setup>
import { onMounted, ref } from "vue";
import { get, put } from "../lib/api.js";

const props = defineProps({ slug: { type: String, required: true } });
const rules = ref([]);
const error = ref("");

async function load() {
  rules.value = await get(`/api/projects/${props.slug}/rules`).catch((e) => ((error.value = e.message), []));
}

async function save() {
  error.value = "";
  try {
    rules.value = await put(`/api/projects/${props.slug}/rules`, rules.value.map((r) => ({ tool: r.tool, command: r.command || null, target: r.target || null, action: r.action })));
    error.value = "saved";
  } catch (e) {
    error.value = e.message;
  }
}

const add = () => rules.value.unshift({ tool: "shell.exec*", command: "", target: "", action: "allow" });
const move = (i, d) => {
  const j = i + d;
  if (j < 0 || j >= rules.value.length) return;
  const r = rules.value.splice(i, 1)[0];
  rules.value.splice(j, 0, r);
};

onMounted(load);
</script>

<template>
  <section>
    <p class="dim">The first rule that fits a call decides; none: the project's default. A command line is judged piece by piece (<code>a &amp;&amp; b</code>, <code>$(…)</code>): the strictest piece wins.</p>
    <div class="row"><button @click="add">add a rule</button><button @click="save">save</button></div>
    <p v-if="error" class="err" role="alert">{{ error }}</p>
    <table class="rules">
      <tr><th></th><th>tool</th><th>command (glob)</th><th>target</th><th>action</th><th></th></tr>
      <tr v-for="(r, i) in rules" :key="i">
        <td><button aria-label="up" @click="move(i, -1)">↑</button><button aria-label="down" @click="move(i, 1)">↓</button></td>
        <td><input v-model="r.tool" aria-label="tool" /></td>
        <td><input v-model="r.command" aria-label="command" placeholder="any" /></td>
        <td><input v-model="r.target" aria-label="target" placeholder="any" /></td>
        <td><select v-model="r.action" aria-label="action"><option>allow</option><option>ask</option><option>deny</option></select></td>
        <td><button aria-label="remove" @click="rules.splice(i, 1)">×</button></td>
      </tr>
    </table>
  </section>
</template>
