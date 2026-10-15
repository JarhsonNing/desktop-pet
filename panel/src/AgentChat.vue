<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { call, listen } from "./bridge";
const frame = ref<HTMLIFrameElement>();
const emit = defineEmits<{ settings: [tab: string]; close: [] }>();
const commands = new Set(["agent_snapshot", "agent_restore", "agent_connect", "agent_sessions", "agent_session_delete", "agent_rpc", "agent_storage"]);
let dispose: (() => void) | undefined;
async function message(event: MessageEvent) {
  const data = event.data;
  if (event.source !== frame.value?.contentWindow || event.origin !== location.origin || data?.channel !== "mochi-pi-v1" || data.kind !== "request") return;
  if (!Number.isSafeInteger(data.id) || !data.args || typeof data.args !== "object" || Array.isArray(data.args)) return;
  const reply: Record<string, unknown> = { channel: "mochi-pi-v1", kind: "response", id: data.id };
  try {
    if (data.command === "open_agent_settings") {
      if (!["models", "mcp", "skills", "runtime"].includes(data.args.tab)) throw new Error("设置页面无效");
      reply.value = true; emit("settings", data.args.tab);
    } else if (data.command === "pet_context") {
      const snapshot: any = await call("snapshot");
      reply.value = { name: snapshot.config.name, canChase: snapshot.config.allow_cursor_chase && !snapshot.quiet, canHide: snapshot.config.allow_hide_icons && snapshot.config.level === "Devil" && !snapshot.quiet && !snapshot.hidden };
    } else if (data.command === "pet_action") {
      if (!["pet", "feed", "play", "busy", "home", "wake", "chase", "hide", "restore"].includes(data.args.action)) throw new Error("互动不受支持");
      reply.value = data.args.action === "restore" ? await call("restore") : await call("interact", { action: data.args.action });
    } else {
      if (!commands.has(data.command)) throw new Error("请求不受支持");
      reply.value = await call(data.command, data.args);
    }
  } catch (error) { reply.error = String(error); }
  frame.value?.contentWindow?.postMessage(reply, location.origin);
}
onMounted(async () => {
  window.addEventListener("message", message);
  dispose = await listen("agent-event", (value) => frame.value?.contentWindow?.postMessage({ channel: "mochi-pi-v1", kind: "event", value }, location.origin));
});
onUnmounted(() => { window.removeEventListener("message", message); dispose?.(); });
</script>
<template><iframe ref="frame" class="agent-frame" src="./agent.html" title="Pi 对话" sandbox="allow-scripts allow-same-origin" /></template>
