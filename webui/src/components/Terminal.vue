<script setup>
import { onMounted, onUnmounted, ref } from "vue";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { del } from "../lib/api.js";

const props = defineProps({ pty: { type: String, required: true } });
const emit = defineEmits(["closed"]);
const el = ref(null);
const ended = ref(false);
let ws = null;
let term = null;
let fit = null;
let ro = null;

const b64 = {
  enc: (s) => btoa(String.fromCharCode(...new TextEncoder().encode(s))),
  dec: (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0)),
};

onMounted(() => {
  term = new Terminal({ fontSize: 13, convertEol: false, theme: { background: "#000" } });
  fit = new FitAddon();
  term.loadAddon(fit);
  term.open(el.value);
  fit.fit();
  ws = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/api/ptys/${props.pty}`);
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data);
    if (m.data) term.write(b64.dec(m.data));
    if (m.exit) ended.value = true;
  };
  ws.onopen = () => ws.send(JSON.stringify({ cols: term.cols, rows: term.rows }));
  term.onData((d) => ws?.readyState === 1 && ws.send(JSON.stringify({ data: b64.enc(d) })));
  ro = new ResizeObserver(() => {
    fit.fit();
    if (ws?.readyState === 1) ws.send(JSON.stringify({ cols: term.cols, rows: term.rows }));
  });
  ro.observe(el.value);
});

onUnmounted(() => {
  ro?.disconnect();
  ws?.close();
  term?.dispose();
});

async function close() {
  await del(`/api/ptys/${props.pty}`).catch(() => {});
  emit("closed");
}
</script>

<template>
  <div>
    <div class="row"><span class="dim">terminal {{ pty }}{{ ended ? " (its program ended)" : "" }}</span><span class="grow"></span><button @click="close">close terminal</button></div>
    <div ref="el" class="xterm-box" aria-label="terminal"></div>
  </div>
</template>
