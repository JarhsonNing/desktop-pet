import { host, subscribe } from "./transport";
import { streamUpdate } from "./stream";

function presentation(message: any) {
  if (message.role !== "user" || !Array.isArray(message.content) || !message.content.some((b: any) => b.type === "image")) return message;
  return { ...message, role: "user-with-attachments", content: message.content.filter((b: any) => b.type === "text").map((b: any) => b.text).join("\n"),
    attachments: message.content.filter((b: any) => b.type === "image").map((b: any, index: number) => ({ id: `${message.timestamp}-${index}`, type: "image", fileName: `图片 ${index + 1}`, mimeType: b.mimeType, size: b.data.length * 3 / 4, content: b.data, preview: b.data })) };
}

// Implements only the official UI's Agent presentation contract. All execution is RPC.
export class RpcAgent {
  state: any = { model: undefined, thinkingLevel: "off", messages: [], tools: [], pendingToolCalls: new Set(), isStreaming: false };
  generation = 0;
  sequence = 0;
  sessionId = "";
  runtime: any = {};
  interactions: any[] = [];
  private callbacks = new Set<(event: any) => void>();
  private syncing = 0;
  private syncSerial = 0;
  private queue: any[] = [];
  private partial: any = null;
  onChange?: () => void;
  constructor() { subscribe((event) => { if (this.syncing) this.queue.push(event); else this.accept(event); }); }
  subscribe(callback: (event: any) => void) { this.callbacks.add(callback); return () => this.callbacks.delete(callback); }
  private emit(event: any) { for (const callback of this.callbacks) callback(event); this.onChange?.(); }
  async sync(command = "agent_snapshot", args: Record<string, unknown> = {}) {
    const serial = ++this.syncSerial;
    this.syncing++;
    try { const snapshot = await host(command, args); if (serial === this.syncSerial) this.restore(snapshot); }
    finally { if (--this.syncing === 0) { const queue = this.queue; this.queue = []; for (const event of queue) this.accept(event); } }
  }
  restore(snapshot: any) {
    this.generation = snapshot.generation; this.sequence = snapshot.sequence;
    this.sessionId = snapshot.state?.sessionId ?? "";
    this.runtime = snapshot.runtime; this.interactions = snapshot.interactions ?? [];
    this.partial = snapshot.partial;
    this.state = { ...this.state, ...snapshot.state, messages: (snapshot.messages ?? []).map(presentation), isStreaming: snapshot.runtime.busy, tools: [], pendingToolCalls: new Set() };
    for (const progress of snapshot.toolProgress ?? []) this.progress(progress);
    this.emit({ type: "message_end" });
    if (snapshot.partial) {
      // Pi may include the active assistant in get_messages; it belongs in the streaming container.
      this.state.messages = this.state.messages.filter((m: any) => !(m.role === "assistant" && m.timestamp === snapshot.partial.timestamp));
      this.emit({ type: "message_update", message: snapshot.partial });
    }
  }
  accept(envelope: any) {
    if (!Number.isSafeInteger(envelope.generation) || !Number.isSafeInteger(envelope.sequence)) return;
    if (envelope.generation < this.generation || (envelope.generation === this.generation && envelope.sequence <= this.sequence)) return;
    if (envelope.generation !== this.generation) { void this.sync().catch(showError); return; }
    if (envelope.sessionId && this.sessionId && envelope.sessionId !== this.sessionId) return;
    this.sequence = envelope.sequence; this.runtime = envelope.runtime;
    const event = envelope.event;
    if (event.type === "message_start" && event.message?.role === "assistant") this.partial = structuredClone(event.message);
    if (event.type === "message_update") {
      this.partial = streamUpdate(this.partial, event);
      if (!this.partial) return;
      event.message = structuredClone(this.partial);
    }
    // agent_end is a low-level boundary. Pi can still compact/retry; wait for agent_settled.
    this.state.isStreaming = envelope.runtime.busy;
    if (event.type === "message_end") {
      this.partial = null;
      const message = event.message && presentation(event.message);
      if (message && !this.state.messages.some((m: any) => m.role === message.role && m.timestamp === message.timestamp && JSON.stringify(m) === JSON.stringify(message)))
        this.state.messages = [...this.state.messages, message];
    }
    if (["tool_execution_start", "tool_execution_update"].includes(event.type)) this.progress(event);
    if (event.type === "tool_execution_end") {
      this.state.pendingToolCalls.delete(event.toolCallId);
      this.state.messages = this.state.messages.filter((m: any) => !(m.mochiProgress && m.toolCallId === event.toolCallId));
    }
    if (event.type === "extension_ui_request" && ["select", "confirm", "input", "editor"].includes(event.method)) {
      this.interactions = [...this.interactions.filter((r) => r.id !== event.id), event];
    }
    if (event.type === "extension_ui_expired") this.interactions = this.interactions.filter((request) => request.id !== event.id);
    if (event.type === "agent_settled") { this.state.pendingToolCalls.clear(); this.emit({ type: "agent_end" }); void this.sync().catch(showError); }
    else if (event.type !== "agent_end") this.emit(event);
    if (["runtime_exit", "runtime_idle_exit", "configuration_applied"].includes(event.type)) this.emit({ type: "agent_end" });
  }
  private progress(event: any) {
    this.state.pendingToolCalls.add(event.toolCallId);
    if (event.partialResult) {
      this.state.messages = [...this.state.messages.filter((m: any) => !(m.mochiProgress && m.toolCallId === event.toolCallId)),
        { ...event.partialResult, role: "toolResult", toolCallId: event.toolCallId, toolName: event.toolName, timestamp: 0, isError: false, mochiProgress: true }];
    }
    this.emit({ type: "turn_start" });
  }
  async prompt(input: string | any) {
    const message = typeof input === "string" ? input : input.content;
    const images = typeof input === "string" ? [] : (input.attachments ?? []).map((a: any) => {
      if (a.type !== "image") throw new Error("仅支持图片附件");
      return { type: "image", data: a.content.replace(/^data:[^,]*,/, ""), mimeType: a.mimeType };
    });
    this.state.isStreaming = true; this.emit({ type: "agent_start" });
    try { await host("agent_rpc", { type: "prompt", message, images }); await this.sync(); }
    catch (error) { this.state.isStreaming = false; this.emit({ type: "agent_end" }); showError(error); throw error; }
  }
  async abort() { try { this.restore(await host("agent_rpc", { type: "stop" })); } catch (e) { showError(e); } }
}
export function showError(error: unknown) { window.dispatchEvent(new CustomEvent("agent-error", { detail: String(error) })); }
