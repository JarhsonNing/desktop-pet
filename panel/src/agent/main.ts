import { AgentInterface } from "./ui/components/AgentInterface.js";
import { ModelSelector } from "./ui/dialogs/ModelSelector.js";
import { translations } from "./ui/utils/i18n.js";
import "@mariozechner/mini-lit/dist/MarkdownBlock.js";
import "@mariozechner/mini-lit/dist/CodeBlock.js";
import "./ui/styles/chat.css";
import { RpcAgent, showError } from "./adapter";
import { host } from "./transport";
import "./page.css";

(translations as Record<string, Record<string, string>>).zh = { ...translations.en, "Type a message...": "输入消息…", "Select Model": "选择模型", "No session set": "开始对话", "Search models...": "搜索模型…", "Send": "发送", "Settings": "设置", "Cancel": "取消", "Save": "保存", "Add Provider": "添加兼容端点", "Edit Provider": "编辑兼容端点", "Provider Name": "名称", "Provider Type": "协议", "Base URL": "接口地址", "API Key (Optional)": "API Key（可选）", "Enter API key": "输入 API Key", "Providers & Models": "模型配置" };
// The upstream setLanguage reloads unconditionally; set the presentation preference before rendering.
localStorage.setItem("language", "zh");
const root = document.querySelector<HTMLDivElement>("#agent-root")!;
root.innerHTML = `<div id="chat-mount"></div>`;
const extras = document.createElement("div");
extras.className = "composer-extras";
extras.innerHTML = `<div class="composer-actions"><button id="new-session" title="新建对话">＋ 新对话</button><details id="history-menu"><summary>历史对话</summary><section class="history-panel" aria-label="历史对话"><header><strong>历史对话</strong><span id="history-count"></span></header><input id="history-search" type="search" aria-label="搜索历史对话" placeholder="搜索对话…"><div id="history-list"></div></section></details><button id="settings">模型配置</button><button id="skills">Skills</button><details id="pet-menu"><summary>陪伴 ···</summary><div class="pet-menu-content"><button data-action="pet">摸摸</button><button data-action="feed" aria-label="投喂一块饼干">投喂</button><button data-action="play">玩耍</button><button data-action="busy">忙碌</button><button data-action="home">归窝</button><button data-action="wake">叫醒它</button><button data-action="chase">体验光标追逐</button><button data-action="hide">体验图标捉迷藏</button><button data-action="restore">恢复桌面 / 停止捣蛋</button></div></details></div><div id="agent-status" role="status"></div><div id="interactions"></div><dialog id="delete-session-dialog" aria-labelledby="delete-session-title"><h2 id="delete-session-title">删除对话？</h2><p id="delete-session-name"></p><p>将删除这条对话的全部消息，无法撤销。</p><div class="dialog-actions"><button id="cancel-delete">取消</button><button id="confirm-delete" class="danger">删除对话</button></div><p id="delete-session-error" role="alert"></p></dialog>`;
const agent = new RpcAgent();
const chat = new AgentInterface();
chat.session = agent;
chat.composerExtras = extras;
chat.enableThinkingSelector = false;
chat.onModelSelect = () => void chooseModel().catch(showError);
chat.onBeforeSend = async () => {
  if (loading) throw new Error("正在连接，请稍候…");
  loading = true; update();
  try {
    if (!agent.runtime.running) await agent.sync("agent_connect");
    const { models } = await host("agent_rpc", { type: "get_available_models" });
    if (!models.length) { await host("open_agent_settings", { tab: "models" }); throw new Error("请先添加模型服务，再发送消息。"); }
    if (!models.some((model: any) => model.id === agent.state.model?.id && model.provider === agent.state.model?.provider)) {
      if (models.length === 1) { await host("agent_rpc", { type: "set_model", provider: models[0].provider, modelId: models[0].id }); await agent.sync(); }
      else { ModelSelector.rpcModels = models; await ModelSelector.open(null, (model) => { void host("agent_rpc", { type: "set_model", provider: model.provider, modelId: model.id }).then(() => agent.sync()).catch(showError); }); throw new Error("选择模型后即可发送，输入内容已保留。"); }
    }
  } finally { loading = false; update(); }
};
document.querySelector("#chat-mount")!.append(chat);
chat.setInput(sessionStorage.getItem("mochi-chat-draft") ?? "");
chat.addEventListener("input", (event) => { if (event.target instanceof HTMLTextAreaElement) sessionStorage.setItem("mochi-chat-draft", event.target.value); });
chat.addEventListener("composer-sent", () => sessionStorage.removeItem("mochi-chat-draft"));
let loading = true;
const status = extras.querySelector<HTMLElement>("#agent-status")!;
type Session = { file: string; title: string; modified: number; messageCount: number };
let history: Session[] = [];
let deleteTarget: Session | undefined;
const historyMenu = extras.querySelector<HTMLDetailsElement>("#history-menu")!;
const historySearch = extras.querySelector<HTMLInputElement>("#history-search")!;
const deleteDialog = extras.querySelector<HTMLDialogElement>("#delete-session-dialog")!;
function sessionKey(file = "") {
  const key = file.replace(/^\\\\\?\\/, "").replace(/\\/g, "/");
  return /^[a-z]:\//i.test(key) ? key.toLowerCase() : key;
}
async function sessions() {
  history = await host<Session[]>("agent_sessions");
  renderHistory();
}
function locked() { return agent.state.isStreaming || agent.interactions.length > 0 || loading; }
function renderHistory() {
  const list = extras.querySelector<HTMLElement>("#history-list")!;
  list.replaceChildren();
  extras.querySelector("#history-count")!.textContent = `${history.length} 条`;
  const query = historySearch.value.trim().toLocaleLowerCase();
  const items = history.filter((item) => item.title.toLocaleLowerCase().includes(query));
  if (!items.length) {
    const empty = document.createElement("p"); empty.className = "history-empty";
    empty.textContent = query ? "没有匹配的对话" : "还没有历史对话"; list.append(empty);
  }
  for (const item of items) {
    const row = document.createElement("div"); row.className = "history-row";
    const current = sessionKey(item.file) === sessionKey(agent.state.sessionFile);
    row.classList.toggle("is-current", current);
    const open = document.createElement("button"); open.className = "history-open";
    open.disabled = locked(); open.setAttribute("aria-label", `打开对话：${item.title}`);
    if (current) open.setAttribute("aria-current", "true");
    const title = document.createElement("span"); title.className = "history-title"; title.textContent = item.title; title.title = item.title;
    const meta = document.createElement("span"); meta.className = "history-meta";
    meta.textContent = `${current ? "当前对话 · " : ""}${new Date(item.modified * 1000).toLocaleDateString("zh-CN")} · ${item.messageCount} 条消息`;
    open.append(title, meta);
    open.onclick = () => void (async () => {
      loading = true; update();
      try { await agent.sync("agent_rpc", { type: "switch_session", sessionPath: item.file }); historyMenu.open = false; await sessions(); }
      finally { loading = false; update(); }
    })().catch(showError);
    const remove = document.createElement("button"); remove.className = "history-delete";
    remove.textContent = "删除"; remove.setAttribute("aria-label", `删除对话：${item.title}`); remove.disabled = locked();
    remove.onclick = () => {
      deleteTarget = item;
      extras.querySelector("#delete-session-name")!.textContent = item.title;
      extras.querySelector("#delete-session-error")!.textContent = "";
      historyMenu.open = false; deleteDialog.showModal();
      extras.querySelector<HTMLButtonElement>("#cancel-delete")!.focus();
    };
    row.append(open, remove); list.append(row);
  }
}
let previouslyBusy = false;
function update() {
  chat.composerBusy = loading;
  (extras.querySelector("#new-session") as HTMLButtonElement).disabled = locked();
  for (const button of extras.querySelectorAll<HTMLButtonElement>(".history-row button, #confirm-delete, #cancel-delete")) button.disabled = locked();
  status.textContent = agent.runtime.error || (agent.runtime.pendingConfiguration ? "配置将在任务结束后应用" : loading ? "正在连接…" : agent.state.isStreaming ? "正在回复…" : "");
  renderInteractions();
  chat.requestUpdate();
  if (previouslyBusy && !agent.state.isStreaming) void sessions().catch(showError);
  previouslyBusy = agent.state.isStreaming;
}
function renderInteractions() {
  const container = extras.querySelector<HTMLElement>("#interactions")!;
  const ids = agent.interactions.map((r) => r.id).join();
  if (container.dataset.ids === ids) return;
  container.dataset.ids = ids; container.replaceChildren();
  for (const request of agent.interactions) {
    const block = document.createElement("div"); block.className = "interaction";
    const title = document.createElement("p"); title.textContent = request.title || request.message || "Pi 请求输入"; block.append(title);
    const respond = async (value: unknown, cancelled = false) => {
      try { await host("agent_rpc", { type: "extension_ui_response", id: request.id, ...(request.method === "confirm" ? { confirmed: value } : { value }), cancelled }); agent.interactions = agent.interactions.filter((r) => r.id !== request.id); update(); }
      catch (e) { showError(e); }
    };
    if (request.method === "select") for (const option of request.options ?? []) { const button = document.createElement("button"); button.textContent = option; button.onclick = () => void respond(option); block.append(button); }
    else if (request.method === "confirm") for (const [text, value] of [["确认", true], ["拒绝", false]] as const) { const button = document.createElement("button"); button.textContent = text; button.onclick = () => void respond(value); block.append(button); }
    else { const input = request.method === "editor" ? document.createElement("textarea") : document.createElement("input"); input.value = request.prefill ?? ""; input.setAttribute("aria-label", "Pi 交互输入"); block.append(input); const button = document.createElement("button"); button.textContent = "提交"; button.onclick = () => void respond(input.value); block.append(button); }
    const cancel = document.createElement("button"); cancel.textContent = "取消"; cancel.onclick = () => void respond(undefined, true); block.append(cancel); container.append(block);
  }
}
agent.onChange = update;
window.addEventListener("agent-error", ((event: CustomEvent) => { status.textContent = event.detail; }) as EventListener);
async function chooseModel() {
  await agent.sync("agent_connect");
  const { models } = await host("agent_rpc", { type: "get_available_models" });
  if (!models.length) { await host("open_agent_settings", { tab: "models" }); return; }
  Object.assign(ModelSelector, { rpcModels: models });
  await ModelSelector.open(agent.state.model ?? null, (model) => {
    void host("agent_rpc", { type: "set_model", provider: model.provider, modelId: model.id }).then(() => agent.sync()).catch(showError);
  });
}
extras.querySelector<HTMLButtonElement>("#settings")!.onclick = () => void host("open_agent_settings", { tab: "models" }).catch(showError);
extras.querySelector<HTMLButtonElement>("#skills")!.onclick = () => void host("open_agent_settings", { tab: "skills" }).catch(showError);
extras.querySelector<HTMLButtonElement>("#new-session")!.onclick = () => void (async () => {
  loading = true; update();
  try { if (agent.runtime.running || agent.state.messages.length) { await agent.sync("agent_rpc", { type: "new_session" }); await sessions(); } historyMenu.open = false; chat.setInput(""); sessionStorage.removeItem("mochi-chat-draft"); }
  finally { loading = false; update(); }
})().catch(showError);
extras.querySelector<HTMLDetailsElement>("#pet-menu")!.addEventListener("toggle", () => {
  if (!(extras.querySelector("#pet-menu") as HTMLDetailsElement).open) return;
  void host("pet_context").then((pet) => { (extras.querySelector('[data-action="chase"]') as HTMLButtonElement).disabled = !pet.canChase; (extras.querySelector('[data-action="hide"]') as HTMLButtonElement).disabled = !pet.canHide; }).catch(showError);
});
for (const button of extras.querySelectorAll<HTMLButtonElement>("[data-action]")) button.onclick = () => { (extras.querySelector("#pet-menu") as HTMLDetailsElement).open = false; void host("pet_action", { action: button.dataset.action }).catch(showError); };
historySearch.oninput = renderHistory;
historyMenu.addEventListener("toggle", () => { if (historyMenu.open) { void sessions().catch(showError); historySearch.focus(); } });
document.addEventListener("click", (event) => { if (event.target instanceof Node && !historyMenu.contains(event.target)) historyMenu.open = false; });
document.addEventListener("keydown", (event) => { if (event.key === "Escape" && historyMenu.open) { historyMenu.open = false; historyMenu.querySelector("summary")!.focus(); } });
extras.querySelector<HTMLButtonElement>("#cancel-delete")!.onclick = () => deleteDialog.close();
deleteDialog.addEventListener("cancel", (event) => { if (loading) event.preventDefault(); });
deleteDialog.addEventListener("close", () => { deleteTarget = undefined; historyMenu.querySelector("summary")!.focus(); });
extras.querySelector<HTMLButtonElement>("#confirm-delete")!.onclick = () => void (async () => {
  if (!deleteTarget || locked()) return;
  loading = true; update();
  try {
    await agent.sync("agent_session_delete", { sessionPath: deleteTarget.file });
    await sessions(); deleteDialog.close();
  } catch (error) { extras.querySelector("#delete-session-error")!.textContent = String(error); }
  finally { loading = false; update(); }
})();
// Reading state does not launch Pi. A live process is restored automatically after panel reopen.
void agent.sync("agent_restore").then(sessions).catch(showError).finally(() => { loading = false; update(); });
update();
