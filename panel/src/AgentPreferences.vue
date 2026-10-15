<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { call, invoke, listen } from "./bridge";
import Icon from "./Icon.vue";
const props = withDefaults(defineProps<{ initialTab?: string }>(), { initialTab: "models" });
const tab = ref(props.initialTab), runtime = ref<any>({}), result = ref(""), failed = ref(false), working = ref(false);
const providers = ref<any[]>([]), skills = ref<any[]>([]), filter = ref("");
const known = ["anthropic", "openai", "google", "openrouter", "deepseek", "groq", "xai", "mistral", "moonshotai", "zai", "cerebras", "minimax"];
const keyProvider = ref("anthropic"), keyValue = ref(""), configured = ref<string[]>([]);
const provider = ref<any>(null), modelIds = ref(""), providerKey = ref(""), makeDefault = ref(false);
const mcpMode = ref("form"), mcpJson = ref(""), mcpDocument = ref<any>({ mcpServers: {} }), servers = ref<any[]>([]), uninstalling = ref<any>(null);
function message(value: string, error = false) { result.value = value; failed.value = error; }
async function perform(operation: () => Promise<void>) {
  if (working.value) return;
  working.value = true; result.value = "";
  try { await operation(); } catch (error) { message(String(error), true); }
  finally { working.value = false; }
}
function saved() { message(runtime.value.pendingConfiguration ? "已保存，当前任务和交互结束后应用。" : "已保存，下次对话将使用新的配置。"); }
async function refresh() {
  const [status, keys, ids, installed] = await Promise.all([
    call("agent_status"), call<string[]>("agent_storage", { store: "provider-keys", operation: "keys" }),
    call<string[]>("agent_storage", { store: "custom-providers", operation: "keys" }), call<any[]>("agent_skills_list"),
  ]);
  runtime.value = status; configured.value = keys; skills.value = installed;
  providers.value = (await Promise.all(ids.map((key) => call("agent_storage", { store: "custom-providers", operation: "get", key })))).filter(Boolean);
}
function pairs(value: Record<string, string> = {}) { return Object.entries(value).map(([key, value]) => `${key}=${value}`).join("\n"); }
function loadMcp(value: any) {
  if (!value || typeof value !== "object" || Array.isArray(value) || !value.mcpServers || typeof value.mcpServers !== "object" || Array.isArray(value.mcpServers)) throw new Error("MCP 配置需要 mcpServers 对象");
  mcpDocument.value = value;
  servers.value = Object.entries(value.mcpServers).map(([name, entry]: [string, any]) => ({
    id: crypto.randomUUID(), name, transport: entry.url ? "http" : "stdio", command: entry.command ?? "", url: entry.url ?? "",
    args: (entry.args ?? []).join("\n"), env: pairs(entry.env), headers: pairs(entry.headers), enabled: entry.enabled !== false,
    exposure: entry.exposure ?? "direct", extra: JSON.parse(JSON.stringify(entry)),
  }));
  mcpJson.value = JSON.stringify(value, null, 2);
}
function mapping(text: string) {
  const output: Record<string, string> = Object.create(null);
  for (const line of text.split("\n").filter((line) => line.trim())) {
    const separator = line.indexOf("=");
    if (separator <= 0) throw new Error("环境变量和请求头请按每行 名称=值 填写");
    const key = line.slice(0, separator).trim();
    if (Object.hasOwn(output, key)) throw new Error(`重复的名称：${key}`);
    output[key] = line.slice(separator + 1);
  }
  return output;
}
function mcpValue() {
  const mcpServers: Record<string, any> = Object.create(null);
  for (const row of servers.value) {
    const name = row.name.trim();
    if (!name || Object.hasOwn(mcpServers, name)) throw new Error("MCP 服务名称不能为空或重复");
    const entry = { ...row.extra, enabled: row.enabled, exposure: row.exposure };
    if (row.transport === "stdio") { delete entry.url; delete entry.headers; entry.type = "stdio"; entry.command = row.command.trim(); entry.args = row.args === (row.extra.args ?? []).join("\n") ? row.extra.args ?? [] : row.args ? row.args.split("\n") : []; entry.env = row.env === pairs(row.extra.env) ? row.extra.env ?? {} : mapping(row.env); }
    else { delete entry.command; delete entry.args; delete entry.env; delete entry.cwd; entry.type = "http"; entry.url = row.url.trim(); entry.headers = row.headers === pairs(row.extra.headers) ? row.extra.headers ?? {} : mapping(row.headers); }
    mcpServers[name] = entry;
  }
  return { ...mcpDocument.value, mcpServers };
}
function mode(next: string) {
  try {
    if (next === mcpMode.value) return;
    if (next === "json") mcpJson.value = JSON.stringify(mcpValue(), null, 2);
    else loadMcp(JSON.parse(mcpJson.value));
    mcpMode.value = next; result.value = "";
  } catch (error) { message(String(error), true); }
}
function addServer() { servers.value.push({ id: crypto.randomUUID(), name: "", transport: "stdio", command: "", url: "", args: "", env: "", headers: "", enabled: true, exposure: "direct", extra: {} }); }
async function saveMcp() { await perform(async () => { const value = mcpMode.value === "json" ? JSON.parse(mcpJson.value) : mcpValue(); runtime.value = await call("agent_mcp_save", { value }); loadMcp(value); saved(); }); }
function editProvider(item?: any) {
  provider.value = item ? JSON.parse(JSON.stringify(item)) : { id: crypto.randomUUID(), name: "", type: "openai-completions", baseUrl: "", models: [] };
  modelIds.value = (item?.models ?? []).map((model: any) => model.id).join("\n"); providerKey.value = ""; makeDefault.value = false;
}
async function saveProvider() {
  await perform(async () => {
    const item = provider.value;
    if (!item.name.trim()) throw new Error("请填写服务名称");
    const url = new URL(item.baseUrl);
    if (!["http:", "https:"].includes(url.protocol)) throw new Error("接口地址必须使用 HTTP 或 HTTPS");
    const ids = [...new Set(modelIds.value.split(/[\n,]+/).map((v) => v.trim()).filter(Boolean))];
    if (!ids.length) throw new Error("至少填写一个模型 ID");
    const models = ids.map((id) => ({ ...(item.models.find((model: any) => model.id === id) ?? { id, name: id, reasoning: false, input: ["text", "image"], contextWindow: 128000, maxTokens: 8192, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 } }), api: item.type, provider: item.id, baseUrl: item.baseUrl }));
    await call("agent_storage", { store: "custom-providers", operation: "set", key: item.id, value: { ...item, models, ...(providerKey.value.trim() ? { apiKey: providerKey.value } : {}) } });
    if (makeDefault.value) await call("agent_model_default", { provider: item.id, modelId: ids[0] });
    providerKey.value = ""; provider.value = null; await refresh(); saved();
  });
}
async function saveKey(remove = false) { await perform(async () => { await call("agent_storage", { store: "provider-keys", operation: remove ? "delete" : "set", key: keyProvider.value, value: keyValue.value }); keyValue.value = ""; await refresh(); saved(); }); }
async function deleteProvider(item: any) { await perform(async () => { await call("agent_storage", { store: "custom-providers", operation: "delete", key: item.id }); await call("agent_storage", { store: "provider-keys", operation: "delete", key: item.id }); await refresh(); saved(); }); }
async function install(folder: boolean) { await perform(async () => { const path = await invoke<string | null>("choose_agent_skill", { folder }); if (!path) return; const installed = await call<any>("agent_skills_install", { path }); await refresh(); message(`已安装 ${installed.name}。${runtime.value.pendingConfiguration ? "当前任务结束后启用。" : "下次对话时启用。"}`); }); }
async function toggle(skill: any) { await perform(async () => { runtime.value = await call("agent_skills_toggle", { id: skill.id, enabled: !skill.enabled }); await refresh(); saved(); }); }
async function uninstall() { const skill = uninstalling.value; await perform(async () => { await call("agent_skills_uninstall", { id: skill.id }); uninstalling.value = null; await refresh(); message(`已卸载 ${skill.name}。`); }); }
async function open(directory: string) { await perform(async () => { await call("agent_open_directory", { directory }); }); }
let dispose: (() => void) | undefined;
onMounted(() => void perform(async () => { dispose = await listen<any>("agent-event", (event) => { if (event.runtime) runtime.value = event.runtime; }); await refresh(); loadMcp(await call("agent_mcp_get")); }));
onUnmounted(() => dispose?.());
</script>
<template>
  <div class="agent-preferences" :aria-busy="working">
    <fieldset class="agent-settings-fields" :disabled="working">
    <div class="agent-tabs" role="tablist" aria-label="Agent 设置">
      <button v-for="item in [{ id: 'models', title: '模型与服务' }, { id: 'mcp', title: 'MCP 连接' }, { id: 'skills', title: 'Skills' }, { id: 'runtime', title: '运行与目录' }]" :key="item.id" role="tab" :aria-selected="tab === item.id" :class="{ selected: tab === item.id }" @click="tab = item.id; result = ''">{{ item.title }}</button>
    </div>
    <p v-if="result" class="alert" :class="failed ? 'error' : 'subtle'" role="status">{{ result }}</p>
    <section v-if="tab === 'models'" class="card">
      <div class="section-title"><div><h3>模型与服务</h3><p class="field-help">添加服务后，就可以回到首页开始对话。</p></div><button class="button primary" :disabled="working" @click="editProvider()"><Icon name="plus" :size="15" />添加兼容端点</button></div>
      <div v-for="item in providers" :key="item.id" class="agent-list-row"><div><strong>{{ item.name }}</strong><p>{{ item.baseUrl }}</p><small>{{ item.models?.map((m: any) => m.id).join(' · ') }}</small></div><button class="button quiet" :disabled="working" @click="editProvider(item)">编辑</button><button class="button quiet" :disabled="working" @click="deleteProvider(item)">移除</button></div>
      <p v-if="!providers.length && !provider" class="agent-empty">还没有兼容端点。可以添加自己的模型服务，也可以在下方配置服务商密钥。</p>
      <form v-if="provider" class="agent-form agent-form-inset" @submit.prevent="saveProvider">
        <h4 class="wide">{{ providers.some((p) => p.id === provider.id) ? '编辑服务' : '添加服务' }}</h4>
        <label>服务名称<input v-model="provider.name" required placeholder="例如：我的模型服务" /></label>
        <label>接口协议<select v-model="provider.type"><option value="openai-completions">OpenAI Chat Completions</option><option value="openai-responses">OpenAI Responses</option><option value="anthropic-messages">Anthropic Messages</option></select></label>
        <label>接口地址<input v-model="provider.baseUrl" required type="url" placeholder="https://example.com/v1" /></label>
        <label>API Key<input v-model="providerKey" type="password" autocomplete="new-password" :placeholder="configured.includes(provider.id) ? '已配置，留空保留原密钥' : '本地服务可留空'" /></label>
        <label class="wide">模型 ID<textarea v-model="modelIds" required rows="3" placeholder="每行一个模型 ID" /></label>
        <label class="wide check-label"><input v-model="makeDefault" type="checkbox" />将第一个模型设为默认</label>
        <div class="actions wide"><button class="button primary" :disabled="working" type="submit">保存服务</button><button class="button quiet" type="button" @click="provider = null; providerKey = ''">取消</button></div>
      </form>
      <div class="agent-form-inset"><h4>服务商 API Key</h4><p class="field-help">密钥仅保存在本机，保存后不会回显。</p><form class="agent-form" @submit.prevent="saveKey()"><label>服务商<select v-model="keyProvider"><option v-for="name in known" :key="name">{{ name }}</option></select></label><label>API Key<input v-model="keyValue" type="password" autocomplete="new-password" :placeholder="configured.includes(keyProvider) ? '已配置，输入以替换' : '输入密钥'" /></label><div class="actions wide"><button class="button primary" :disabled="working || !keyValue.trim()" type="submit">保存密钥</button><button v-if="configured.includes(keyProvider)" class="button quiet" :disabled="working" type="button" @click="saveKey(true)">移除密钥</button></div></form></div>
    </section>
    <section v-else-if="tab === 'mcp'" class="card">
      <div class="section-title"><div><h3>MCP 连接</h3><p class="field-help">连接本地工具或远程服务。表单和 JSON 编辑同一份配置。</p></div><div class="agent-mode"><button :class="{ selected: mcpMode === 'form' }" @click="mode('form')">表单</button><button :class="{ selected: mcpMode === 'json' }" @click="mode('json')">JSON</button></div></div>
      <template v-if="mcpMode === 'form'">
        <div v-for="server in servers" :key="server.id" class="agent-form-inset mcp-server">
          <div class="section-title"><h4>{{ server.name || '新建连接' }}</h4><div class="actions"><label class="check-label"><input v-model="server.enabled" type="checkbox" />启用</label><button class="button quiet" @click="servers = servers.filter((s) => s.id !== server.id)">移除</button></div></div>
          <div class="agent-form"><label>服务名称<input v-model="server.name" placeholder="例如 filesystem" /></label><label>连接方式<select v-model="server.transport"><option value="stdio">本地进程（stdio）</option><option value="http">远程服务（HTTP）</option></select></label>
            <template v-if="server.transport === 'stdio'"><label class="wide">启动程序<input v-model="server.command" placeholder="程序绝对路径，或 npx / uvx 等命令" /></label><label>启动参数<textarea v-model="server.args" rows="3" placeholder="每行一个参数" /></label><label>环境变量<textarea v-model="server.env" rows="3" placeholder="每行 名称=值" spellcheck="false" /></label></template>
            <template v-else><label class="wide">服务 URL<input v-model="server.url" type="url" placeholder="https://example.com/mcp" /></label><label class="wide">请求头<textarea v-model="server.headers" rows="3" placeholder="每行 名称=值" spellcheck="false" /></label></template>
            <label>工具可见方式<select v-model="server.exposure"><option value="direct">直接提供给模型</option><option value="deferred">按需发现</option><option value="codemode">代码调用</option><option value="codemode-deferred">按需代码调用</option><option value="hidden">隐藏</option></select></label>
          </div>
        </div>
        <p v-if="!servers.length" class="agent-empty">还没有 MCP 连接。</p><button class="button" @click="addServer"><Icon name="plus" :size="15" />添加 MCP 连接</button>
      </template>
      <label v-else class="field-label" for="mcp-config">MCP JSON<textarea id="mcp-config" v-model="mcpJson" class="mcp-editor" spellcheck="false" /></label>
      <div class="actions agent-save"><button class="button primary" :disabled="working" @click="saveMcp">校验并保存 MCP</button><span class="field-help">任务执行期间保存，将在任务结束后应用。</span></div>
    </section>
    <section v-else-if="tab === 'skills'" class="card">
      <div class="section-title"><div><h3>Skills</h3><p class="field-help">把常用工作方式教给麻薯。停用保留文件，卸载删除本机副本。</p></div><div class="actions"><button class="button" :disabled="working" @click="install(true)">从文件夹安装</button><button class="button primary" :disabled="working" @click="install(false)">从 ZIP 安装</button></div></div>
      <div class="agent-skill-search"><input v-model="filter" aria-label="搜索 Skills" placeholder="搜索名称或描述" /><button class="button quiet" :disabled="working" @click="perform(refresh)">刷新</button></div>
      <p v-if="!skills.length" class="agent-empty">还没有安装 Skill。选择一个包含 SKILL.md 的文件夹或 ZIP，即可添加。</p>
      <div v-for="skill in skills.filter((s) => `${s.name} ${s.description}`.toLowerCase().includes(filter.toLowerCase()))" :key="skill.id" class="agent-list-row skill-row"><div><strong>{{ skill.name }}</strong><span class="skill-state" :class="{ enabled: skill.enabled }">{{ skill.error ? '需要修复' : skill.enabled ? '已启用' : '已停用' }}</span><p>{{ skill.description || skill.error }}</p><small>{{ skill.path }}</small></div><button class="button" :disabled="working || !!skill.error" :aria-label="`${skill.enabled ? '停用' : '启用'} ${skill.name}`" @click="toggle(skill)">{{ skill.enabled ? '停用' : '启用' }}</button><button class="button quiet" :disabled="working" :aria-label="`卸载 ${skill.name}`" @click="uninstalling = skill">卸载</button></div>
      <div class="actions agent-save"><button class="button quiet" @click="open('skills')"><Icon name="folder" :size="15" />打开 Skills 目录</button><span class="field-help">安装和启停不打断当前任务；卸载前需先停止任务。</span></div>
    </section>
    <section v-else class="card"><div class="section-title"><h3>运行与目录</h3><span>包内 Pi {{ runtime.version }}</span></div><p class="field-help">{{ runtime.running ? '正在运行' : '按需启动' }}{{ runtime.pendingConfiguration ? ' · 有待应用的配置' : '' }}</p><p class="storage-path field-help">{{ runtime.path }}</p><p v-if="runtime.error" class="alert error">{{ runtime.error }}</p><div class="actions"><button class="button" @click="open('workspace')">打开工作目录</button><button class="button" @click="open('config')">打开配置目录</button><button class="button" @click="open('skills')">打开 Skills 目录</button></div></section>
    <div v-if="uninstalling" class="modal-backdrop" @click.self="uninstalling = null"><section class="agent-confirm card" role="dialog" aria-modal="true" aria-labelledby="uninstall-title"><h3 id="uninstall-title">卸载 {{ uninstalling.name }}？</h3><p>将删除麻薯目录中的 Skill 副本，原始安装文件不会改变。</p><div class="actions"><button class="button quiet" :disabled="working" @click="uninstalling = null">取消</button><button class="button primary" :disabled="working" @click="uninstall">确认卸载</button></div><p v-if="failed" role="alert">{{ result }}</p></section></div>
    </fieldset>
  </div>
</template>
