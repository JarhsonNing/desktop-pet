export const CHANNEL = "mochi-pi-v1";
let serial = 0;
const pending = new Map<number, { resolve: (v: any) => void; reject: (e: Error) => void; timer: ReturnType<typeof setTimeout> }>();
const listeners = new Set<(event: any) => void>();
window.addEventListener("message", (message) => {
  if (message.source !== window.parent || message.origin !== window.location.origin || message.data?.channel !== CHANNEL) return;
  const data = message.data;
  if (data.kind === "event") { for (const callback of listeners) callback(data.value); return; }
  if (data.kind !== "response" || !Number.isSafeInteger(data.id)) return;
  const request = pending.get(data.id);
  if (!request) return;
  clearTimeout(request.timer); pending.delete(data.id);
  if (data.error) request.reject(new Error(data.error)); else request.resolve(data.value);
});
export function host<T = any>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  return new Promise((resolve, reject) => {
    const id = ++serial;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error("Agent 响应超时；不会自动重发")); }, 45000);
    pending.set(id, { resolve, reject, timer });
    window.parent.postMessage({ channel: CHANNEL, kind: "request", id, command, args }, window.location.origin);
  });
}
export function subscribe(callback: (event: any) => void) { listeners.add(callback); return () => listeners.delete(callback); }
