<script setup>
import { computed, inject, onMounted, ref, watch } from "vue";
import { del, get, post, put } from "../lib/api.js";
import { budget, flatten, tree } from "../lib/format.js";
import TaskRow from "./TaskRow.vue";
import Rules from "./Rules.vue";
import Cron from "./Cron.vue";
import Memory from "./Memory.vue";
import McpServers from "./McpServers.vue";
import Secrets from "./Secrets.vue";
import Triggers from "./Triggers.vue";
import Designer from "./Designer.vue";
import SkillPicker from "./SkillPicker.vue";
import TemplateForm from "./TemplateForm.vue";
import Templates from "./Templates.vue";
import Worktrees from "./Worktrees.vue";
import Services from "./Services.vue";
import Sync from "./Sync.vue";
import Locks from "./Locks.vue";
import LockFields from "./LockFields.vue";

const props = defineProps({ slug: { type: String, required: true }, tab: { type: String, default: "tasks" } });
const live = inject("live");
const p = ref(null);
const tasks = ref([]);
const showAll = ref(false);
const skills = ref([]);
const rules = ref([]);
const config = ref(null);
const error = ref("");
const start = ref({ title: "", prompt: "", profile: "", kind: "", skills: [], tokens: "", cost: "", minutes: "", worktree: "", locks: [], on_busy: null });
const homes = ref([]);
const settings = ref(null);
const templates = ref([]);
const chosen = ref("");
const values = ref({});
const tf = ref(null);
const formLike = computed(() => !!tf.value?.current?.fields.length);

/** A template fills the form: a form-like one leaves the prompt to its fields, a plain one gives a prompt to edit. */
function applyTemplate(t) {
  if (!t) return;
  const b = t.budget ?? {};
  start.value = { ...start.value, title: t.fields.length ? "" : (t.title ?? ""), prompt: t.fields.length ? "" : t.prompt, profile: t.profile ?? "", kind: t.kind ?? "", skills: t.skills.filter((s) => skills.value.some((k) => k.name === s)), tokens: b.tokens ?? "", cost: b.cost ?? "", minutes: b.minutes ?? "", worktree: t.worktree ?? start.value.worktree, locks: t.locks ?? [], on_busy: t.on_busy ?? null };
}

async function load() {
  try {
    p.value = await get(`/api/projects/${props.slug}`);
    tasks.value = await get(`/api/tasks?project=${props.slug}&active=${!showAll.value}&limit=300`);
    skills.value = await get(`/api/projects/${props.slug}/skills`);
    rules.value = await get(`/api/projects/${props.slug}/agent-rules`);
    homes.value = (await get(`/api/projects/${props.slug}/worktrees`)).worktrees;
    templates.value = (await get(`/api/templates?project=${encodeURIComponent(props.slug)}`)).filter((t) => !t.project || t.project === props.slug);
    config.value ??= await get("/api/config");
    if (!settings.value) {
      const l = p.value.locking ?? {};
      settings.value = {
        ...p.value,
        budget: { ...p.value.budget },
        env: JSON.stringify(p.value.env, null, 2),
        known_locks: Object.entries(l.locks ?? {}).map(([n, d]) => (d ? `${n}: ${d}` : n)).join("\n"),
        on_busy: { person: "", agent: "", task: "", cron: "", trigger: "", ...(l.on_busy ?? {}) },
        merge_locks: l.merge_locks ?? [],
        merge_on_busy: l.merge_on_busy ?? null,
        evaluator: { role: "", on_reject: "", static_checks: "", prompt: "", review_prompt: "", ...(p.value.evaluator ?? {}), risks: JSON.stringify(p.value.evaluator?.risks ?? [], null, 2) },
      };
    }
  } catch (e) {
    error.value = e.message;
  }
}

const rows = computed(() => flatten(tree(tasks.value)));

async function startTask() {
  error.value = "";
  try {
    const t = await post("/api/tasks", {
      project: props.slug,
      title: start.value.title,
      prompt: start.value.prompt,
      profile: start.value.profile || undefined,
      kind: start.value.kind || undefined,
      skills: start.value.skills,
      budget: budget(start.value),
      worktree: start.value.worktree || undefined,
      locks: start.value.locks,
      on_busy: start.value.on_busy || undefined,
      ...(formLike.value ? { template: tf.value.current.name, values: values.value, prompt: "" } : {}),
    });
    location.hash = `#/task/${t.id}`;
  } catch (e) {
    error.value = e.message;
  }
}

async function save() {
  error.value = "";
  try {
    const s = settings.value;
    const body = { max_tasks: s.max_tasks === "" || s.max_tasks == null ? null : Number(s.max_tasks), kind: s.kind || null, devshell: s.devshell, devshell_attr: s.devshell_attr || null, name: s.name, path: s.path, memory: s.memory, worktrees: s.worktrees, merge: s.merge, worktree_sharing: s.worktree_sharing, default_action: s.default_action, profile: s.profile || null, budget: budget(s.budget), env: JSON.parse(s.env || "{}"), locking: locking(s), evaluator: evaluator(s.evaluator) };
    p.value = await put(`/api/projects/${props.slug}`, body);
    error.value = "saved";
  } catch (e) {
    error.value = e.message;
  }
}

/** The evaluator fields as the project keeps them (empty: the default). */
function evaluator(e) {
  return { role: e.role || null, on_reject: e.on_reject || null, static_checks: e.static_checks || null, prompt: e.prompt, review_prompt: e.review_prompt, risks: JSON.parse(e.risks || "[]") };
}

/** The settings' lock fields as the project keeps them. */
function locking(s) {
  const locks = {};
  for (const line of (s.known_locks || "").split("\n")) {
    const [name, ...about] = line.split(":");
    if (name.trim()) locks[name.trim()] = about.join(":").trim();
  }
  const on_busy = Object.fromEntries(Object.entries(s.on_busy).filter(([, v]) => v));
  return { locks, on_busy, merge_locks: s.merge_locks, merge_on_busy: s.merge_on_busy };
}

async function remove() {
  if (!confirm(`Remove project ${p.value.name}? (its folder stays; its tasks are forgotten)`)) return;
  try {
    await del(`/api/projects/${props.slug}`);
    location.hash = "#/projects";
  } catch (e) {
    error.value = e.message;
  }
}

/** A designer's proposal fills the form. */
function useProposal(pr) {
  const b = pr.budget ?? {};
  start.value = { ...start.value, title: pr.title, prompt: pr.prompt, profile: pr.profile ?? "", kind: pr.kind ?? "", skills: pr.skills ?? [], tokens: b.tokens ?? "", cost: b.cost ?? "", minutes: b.minutes ?? "" };
}

onMounted(() => {
  load();
  // A design task's proposal, brought over from its page.
  try {
    const kept = JSON.parse(sessionStorage.getItem(`reagent-proposal-${props.slug}`) || "null");
    if (kept) {
      sessionStorage.removeItem(`reagent-proposal-${props.slug}`);
      useProposal(kept);
    }
  } catch {}
});
watch(() => [props.slug, showAll.value, live.tick], load);
</script>

<template>
  <section v-if="p">
    <h2>{{ p.name }} <span class="dim">{{ p.path }}</span></h2>
    <nav class="row tabs">
      <a v-for="t in ['tasks', 'new', 'templates', 'worktrees', 'sync', 'cron', 'triggers', 'services', 'locks', 'policy', 'memory', 'skills', 'mcp', 'secrets', 'settings']" :key="t" :href="`#/project/${slug}/${t}`" :class="{ on: tab === t }">{{ t }}</a>
    </nav>
    <p v-if="error" class="err" role="alert">{{ error }}</p>

    <template v-if="tab === 'tasks'">
      <label class="row"><input v-model="showAll" type="checkbox" /> finished ones too</label>
      <div v-if="!rows.length" class="dim">no tasks{{ showAll ? "" : " going on" }}: <a :href="`#/project/${slug}/new`">start one</a></div>
      <TaskRow v-for="r in rows" :key="r.t.id" :t="r.t" :depth="r.depth" />
    </template>

    <Designer v-if="tab === 'new'" :project="slug" />
    <form v-if="tab === 'new'" class="stack" @submit.prevent="startTask">
      <TemplateForm v-if="templates.length" ref="tf" v-model:chosen="chosen" v-model:values="values" :templates="templates" :project="slug" @pick="applyTemplate" />
      <input v-model="start.title" :placeholder="formLike && tf.current.title ? `title (empty: ${tf.render(tf.current.title)})` : 'title (empty: the agent names it)'" aria-label="title" />
      <textarea v-if="!formLike" v-model="start.prompt" placeholder="what to do" aria-label="what to do" required></textarea>
      <label v-if="homes.length">works in
        <select v-model="start.worktree" aria-label="works in">
          <option value="">the project folder</option>
          <option v-for="w in homes" :key="w.name" :value="w.name" :disabled="w.missing">{{ w.name }} · {{ w.branch }}{{ w.sharing_now === "exclusive" ? " · one task at a time" : "" }}</option>
        </select>
      </label>
      <div class="row">
        <label>profile
          <select v-model="start.profile" aria-label="profile">
            <option value="">default ({{ p.profile || config?.default_profile }})</option>
            <option v-for="pr in config?.profiles ?? []" :key="pr.name" :value="pr.name">{{ pr.name }} · {{ pr.model }}</option>
          </select>
        </label>
        <label v-if="config?.kinds?.length">kind
          <select v-model="start.kind" aria-label="kind">
            <option value="">default ({{ p.kind || config?.default_kind || "none" }})</option>
            <option v-for="k in config.kinds" :key="k.name" :value="k.name" :title="k.description">{{ k.name }}{{ k.profile ? ` · ${k.profile}` : "" }}</option>
          </select>
        </label>
        <label>tokens <input v-model="start.tokens" size="8" inputmode="numeric" aria-label="token budget" /></label>
        <label>cost <input v-model="start.cost" size="6" inputmode="decimal" aria-label="cost budget" /></label>
        <label>minutes <input v-model="start.minutes" size="5" inputmode="numeric" aria-label="time budget" /></label>
      </div>
      <LockFields v-model:locks="start.locks" v-model:on-busy="start.on_busy" />
      <SkillPicker v-if="skills.length" v-model="start.skills" :skills="skills" />
      <button type="submit">start</button>
    </form>

    <Templates v-if="tab === 'templates'" :project="slug" @changed="load" />
    <Worktrees v-if="tab === 'worktrees'" :slug="slug" :sharing="p.worktree_sharing" @changed="load" />
    <Sync v-if="tab === 'sync'" :slug="slug" />
    <Cron v-if="tab === 'cron'" :slug="slug" :config="config" />
    <Triggers v-else-if="tab === 'triggers'" :slug="slug" :config="config" />
    <Services v-else-if="tab === 'services'" :slug="slug" />
    <Locks v-else-if="tab === 'locks'" :slug="slug" />
    <Rules v-else-if="tab === 'policy'" :slug="slug" />
    <Memory v-else-if="tab === 'memory'" :scope="slug" />
    <McpServers v-else-if="tab === 'mcp'" :project="slug" />
    <Secrets v-else-if="tab === 'secrets'" :project="slug" />

    <template v-else-if="tab === 'skills'">
      <div v-if="!skills.length" class="dim">no skills: they live in <code>.agents/skills/&lt;name&gt;/SKILL.md</code> (here, or in ~/.agents/skills)</div>
      <div v-for="s in skills" :key="s.name" class="card">
        <span class="hi">{{ s.name }}</span> <span class="dim">{{ s.source }} · {{ s.dir }}</span>
        <div>{{ s.description }}</div>
        <div v-if="s.files.length" class="dim">files: {{ s.files.join(", ") }}</div>
      </div>
      <h3>rules</h3>
      <div v-if="!rules.length" class="dim">no rules: they live in <code>.agents/rules/*.md</code> (here, in ~/.agents/rules, or a plugin)</div>
      <details v-for="r in rules" :key="r.name" class="card" :aria-label="`rule ${r.name}`">
        <summary><span class="hi">{{ r.name }}</span> <span class="dim">{{ r.when }} · {{ r.path }}</span><span v-if="r.description"> — {{ r.description }}</span></summary>
        <pre>{{ r.body }}</pre>
      </details>
    </template>

    <form v-else-if="tab === 'settings' && settings" class="stack" @submit.prevent="save">
      <label>name <input v-model="settings.name" /></label>
      <label>folder <input v-model="settings.path" size="50" /></label>
      <label>memory <select v-model="settings.memory"><option>central</option><option>repo</option></select> <span class="dim">central: in reagent's data; repo: .reagent/memory in the folder</span></label>
      <label>worktrees <select v-model="settings.worktrees"><option>central</option><option>repo</option></select></label>
      <label>merging <select v-model="settings.merge"><option>review</option><option>approve</option><option>auto</option></select></label>
      <label>persistent worktrees <select v-model="settings.worktree_sharing" aria-label="worktree sharing"><option value="shared">shared</option><option value="exclusive">one task at a time</option></select> <span class="dim">tasks in one persistent worktree at once (each worktree may set its own)</span></label>
      <label>nix dev shell <select v-model="settings.devshell"><option>auto</option><option>on</option><option>off</option></select>
        <input v-model="settings.devshell_attr" size="10" placeholder="default" aria-label="dev shell" />
        <span class="dim">auto: when there's a flake.nix; commands and terminals run in `nix develop`</span></label>
      <label>tasks at once <input v-model="settings.max_tasks" size="4" inputmode="numeric" placeholder="any" aria-label="project tasks at once" /> <span class="dim">more wait, queued (the global limit holds too)</span></label>
      <label>calls no rule covers <select v-model="settings.default_action"><option>ask</option><option>allow</option><option>deny</option></select></label>
      <label v-if="config?.kinds?.length">default kind
        <select v-model="settings.kind"><option :value="null">none</option><option v-for="k in config.kinds" :key="k.name" :value="k.name">{{ k.name }}</option></select>
      </label>
      <label>profile
        <select v-model="settings.profile"><option :value="null">default</option><option v-for="pr in config?.profiles ?? []" :key="pr.name" :value="pr.name">{{ pr.name }}</option></select>
      </label>
      <div class="row">budget per task:
        <label>tokens <input v-model="settings.budget.tokens" size="8" /></label>
        <label>cost <input v-model="settings.budget.cost" size="6" /></label>
        <label>minutes <input v-model="settings.budget.minutes" size="5" /></label>
        <label>per day <input v-model="settings.budget.daily_cost" size="6" /></label>
      </div>
      <fieldset class="stack" aria-label="locks">
        <legend>locks</legend>
        <label>known locks, one a line (name: what it guards) <textarea v-model="settings.known_locks" rows="3" placeholder="deploy: the production deploy" aria-label="known locks"></textarea></label>
        <div class="row">when a lock is busy:
          <label v-for="c in ['person', 'agent', 'task', 'cron', 'trigger']" :key="c">{{ c }}
            <select v-model="settings.on_busy[c]" :aria-label="`when busy: ${c}`">
              <option value="">{{ c === "cron" || c === "trigger" ? "skip" : "wait" }} (default)</option><option>wait</option><option>skip</option><option>fail</option>
            </select>
          </label>
        </div>
        <LockFields v-model:locks="settings.merge_locks" v-model:on-busy="settings.merge_on_busy" default-text="wait" />
        <span class="dim">the locks above are held while a branch is merged into its base (a task's, a persistent worktree's); when busy: wait or fail</span>
      </fieldset>
      <fieldset class="stack" aria-label="call evaluator">
        <legend>call evaluator <span class="dim">{{ config?.evaluator_profile ? `(profile ${config.evaluator_profile})` : "(off: no evaluator_profile in reagent.hcl)" }}</span></legend>
        <div class="row">
          <label>role <select v-model="settings.evaluator.role" aria-label="evaluator role"><option value="">decides (default)</option><option value="decides">decides</option><option value="advises">advises you</option><option value="off">off</option></select></label>
          <label>a rejected call <select v-model="settings.evaluator.on_reject" aria-label="a rejected call"><option value="">is refused (default)</option><option value="refuse">is refused</option><option value="person">comes to you</option></select></label>
          <label>loops, duplicates, writes before reading <select v-model="settings.evaluator.static_checks" aria-label="static checks"><option value="">refused (default)</option><option value="refuse">refused</option><option value="warn">warned about</option><option value="off">let be</option></select></label>
        </div>
        <span class="dim">calls the rules would ask you about go to the evaluator first; deciding, what it approves runs</span>
        <label>its instructions for this project <textarea v-model="settings.evaluator.prompt" rows="3" placeholder="e.g. This repo deploys on push to main: be strict about git push." aria-label="evaluator instructions for this project"></textarea></label>
        <label>risk rules (JSON: [{"tool": "shell.exec", "command": "rm -rf target*", "risk": "mutating"}], first match wins) <textarea v-model="settings.evaluator.risks" rows="2" aria-label="risk rules"></textarea></label>
      </fieldset>
      <fieldset class="stack" aria-label="merge reviewer">
        <legend>merge reviewer <span class="dim">{{ config?.reviewer_profile || config?.evaluator_profile ? `(profile ${config.reviewer_profile || config.evaluator_profile})` : "(off: no reviewer_profile or evaluator_profile in reagent.hcl: merges to review come to you)" }}</span></legend>
        <span class="dim">with merging on review, a review subtask reads a task's change before it's merged</span>
        <label>its instructions for this project <textarea v-model="settings.evaluator.review_prompt" rows="3" placeholder="e.g. Changes to the API need DESIGN.md updated." aria-label="reviewer instructions for this project"></textarea></label>
      </fieldset>
      <label>environment for its commands (JSON) <textarea v-model="settings.env" rows="4"></textarea></label>
      <div class="row"><button type="submit">save</button><span class="grow"></span><button type="button" @click="remove">remove project</button></div>
    </form>
  </section>
  <p v-else-if="error" class="err">{{ error }}</p>
</template>
