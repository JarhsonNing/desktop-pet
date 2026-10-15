// A real packaged Pi + Tauri/WebView2 integration, with a local OpenAI-compatible fixture.
import { chromium } from "@playwright/test";
import { spawn, execFileSync } from "node:child_process";
import { createServer } from "node:http";
import { createInterface } from "node:readline";
import { promises as fs } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";

const root = path.resolve(import.meta.dirname, "../..");
const output = path.resolve(process.env.MOCHI_AGENT_TEST_OUTPUT ?? path.join(root, ".artifacts/agent-e2e"));
const data = path.join(output, `data-${Date.now()}`);
await fs.mkdir(path.join(data, "agent/config"), { recursive: true });
const fixture = path.join(data, "agent/workspace/fixture.txt");
await fs.mkdir(path.dirname(fixture), { recursive: true });
await fs.writeFile(fixture, "Mochi tool fixture 中文");
await fs.writeFile(path.join(data, "config.json"), JSON.stringify({ clipboard_enabled: false, explorer_enabled: false }));
let modelRequests = 0, toolResults = 0, imageRequests = 0, mcpResults = 0, sawSkill = false, lastSkillAdvertised = false;
let confirmPartial;
const partialObserved = new Promise((resolve) => { confirmPartial = resolve; });
const mcpPids = path.join(data, "mcp-pids.txt");
const server = createServer(async (request, response) => {
  let text = "";
  for await (const chunk of request) text += chunk;
  const body = JSON.parse(text);
  modelRequests++;
  lastSkillAdvertised = text.includes("mochi-skill-fixture");
  sawSkill ||= lastSkillAdvertised;
  if (body.messages.some((m) => JSON.stringify(m).includes("data:image/"))) imageRequests++;
  const last = body.messages.at(-1);
  const slow = JSON.stringify(last).includes("慢速");
  response.writeHead(200, { "Content-Type": "text/event-stream", "Cache-Control": "no-cache" });
  const send = (delta, finish = null) => response.write(`data: ${JSON.stringify({ id: "chatcmpl-mochi", object: "chat.completion.chunk", created: Math.floor(Date.now() / 1000), model: body.model, choices: [{ index: 0, delta, finish_reason: finish }] })}\n\n`);
  send({ role: "assistant", content: "" });
  if (slow) await new Promise((resolve) => setTimeout(resolve, 5000));
  if (response.destroyed) return;
  if (last.role === "tool") {
    toolResults++;
    assert(/Mochi (tool|MCP) fixture/.test(JSON.stringify(last)));
    if (JSON.stringify(last).includes("MCP fixture")) mcpResults++;
    for (const chunk of ["## 麻薯回复\n\n", "读取成功。中文 **流式** 回复。\n\n", "```js\nconsole.log('mochi');\n```\n"]) { send({ content: chunk }); await new Promise((r) => setTimeout(r, 600)); }
    // Keep the first response open until its partial content is observed by the UI.
    // This avoids depending on machine speed to prove streaming before completion.
    if (toolResults === 1) await Promise.race([partialObserved, new Promise((resolve) => setTimeout(resolve, 20000))]);
    send({}, "stop");
  } else {
    const mcp = JSON.stringify(last).includes("MCP 测试");
    const toolName = mcp ? body.tools.find((t) => t.function.name.includes("fixture") && t.function.name.includes("echo"))?.function.name : "read";
    assert(toolName, "MCP direct tool must be advertised by Pi");
    send({ tool_calls: [{ index: 0, id: `call_${modelRequests}`, type: "function", function: { name: toolName, arguments: JSON.stringify(mcp ? {} : { path: fixture }) } }] });
    send({}, "tool_calls");
  }
  response.end("data: [DONE]\n\n");
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const endpoint = `http://127.0.0.1:${server.address().port}/v1`;
const executable = path.resolve(process.env.MOCHI_TEST_BINARY ?? path.join(root, "target/release/desktop-pet.exe"));
// An isolated WebView2 profile keeps a running user panel from swallowing the debug port.
const child = spawn(executable, ["--data-dir", data, "--settings", "--panel-debug-port", "9922", "--no-shell", "--control-stdin", "--exit-after", "300"], { cwd: root, stdio: ["pipe", "pipe", "pipe"], env: { ...process.env, PATH: `${process.env.SystemRoot}\\System32;${process.env.SystemRoot}`, WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview2-test") } });
let stderr = "";
child.stderr.on("data", (chunk) => stderr += chunk);
let serial = 100000;
const waiting = new Map();
createInterface({ input: child.stdout }).on("line", (line) => { try { const message = JSON.parse(line); waiting.get(message.id)?.(message); } catch {} });
async function parent(command, args = {}) {
  const id = ++serial;
  const result = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => { waiting.delete(id); reject(new Error(`Timeout ${command}`)); }, 45000);
    waiting.set(id, (value) => { clearTimeout(timer); waiting.delete(id); resolve(value); });
    child.stdin.write(JSON.stringify({ id, command, args }) + "\n");
  });
  assert(!result.error, result.error); return result.value;
}
let browser;
const report = { checks: [], modelRequests: 0, toolResults: 0, errors: [], passed: false };
function memory() {
  const powershell = path.join(process.env.SystemRoot, "System32/WindowsPowerShell/v1.0/powershell.exe");
  const script = `$pet = Get-Process -Id ${child.pid}; $agents = @(Get-CimInstance Win32_Process -Filter "Name = 'pi.exe'" | Where-Object ParentProcessId -eq ${child.pid} | ForEach-Object { Get-Process -Id $_.ProcessId }); @{ petWorkingSet = $pet.WorkingSet64; piWorkingSet = ($agents | Measure-Object WorkingSet64 -Sum).Sum; piPids = @($agents.Id) } | ConvertTo-Json -Compress`;
  return JSON.parse(execFileSync(powershell, ["-NoProfile", "-NonInteractive", "-Command", script], { encoding: "utf8", windowsHide: true }));
}
async function check(name, fn) { await fn(); report.checks.push(name); console.log(`PASS ${name}`); }
async function waitFor(fn, description, timeout = 30000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) { const value = await fn(); if (value) return value; await new Promise((r) => setTimeout(r, 100)); }
  throw new Error(`Timeout: ${description}`);
}
try {
  browser = await waitFor(() => chromium.connectOverCDP("http://127.0.0.1:9922").catch(() => null), "WebView2");
  let page = browser.contexts()[0].pages()[0];
  page.on("pageerror", (error) => { report.errors.push(String(error)); console.log("PAGEERROR", error.stack); });
  page.on("console", (message) => { if (message.type() === "error") console.log("WEB", message.text().slice(0, 300)); });
  let frame = page.frameLocator('iframe[title="Pi 对话"]');
  await frame.locator("message-editor textarea").waitFor();
  await check("首页读取状态不启动 Pi，iframe 无 Tauri 入口", async () => {
    assert.equal((await parent("agent_status")).running, false);
    report.beforeAiMemory = memory();
    assert.equal(await page.locator(".page-header").count(), 0);
    assert.equal(await frame.getByRole("button", { name: "开始对话" }).count(), 0);
    await frame.getByRole("heading", { name: "有什么想聊的？" }).waitFor();
    await frame.locator("#pet-menu summary").click();
    await waitFor(() => frame.locator('[data-action="chase"]').isDisabled(), "chase authorization");
    assert(await frame.locator('[data-action="hide"]').isDisabled());
    await frame.locator("#pet-menu summary").click();
    const iframe = page.frames().find((f) => f.url().endsWith("agent.html"));
    assert.equal(await iframe.evaluate(() => !!window.__TAURI__), false);
    await page.screenshot({ path: path.join(output, "home-before.png") });
  });
  await check("本地设置组件展示兼容端点，密钥不回显", async () => {
    await parent("agent_storage", { store: "custom-providers", operation: "set", key: "mochi-fixture", value: { id: "mochi-fixture", name: "本地验收", type: "openai-completions", baseUrl: endpoint, apiKey: "test-only-secret", models: [{ id: "mochi-test", name: "麻薯测试模型", api: "openai-completions", provider: "mochi-fixture", baseUrl: endpoint, reasoning: false, input: ["text", "image"], contextWindow: 32000, maxTokens: 4096, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 } }] } });
    assert.equal((await parent("agent_status")).running, false);
    assert.equal(await parent("agent_storage", { store: "provider-keys", operation: "get", key: "mochi-fixture" }), "configured");
    assert(!JSON.stringify(await parent("agent_status")).includes("test-only-secret"));
    await frame.getByRole("button", { name: "模型配置" }).click();
    await page.getByRole("tab", { name: "模型与服务" }).waitFor();
    await page.getByRole("button", { name: "编辑", exact: true }).click();
    await page.getByLabel("服务名称", { exact: true }).fill("本地验收服务");
    await page.getByRole("button", { name: "保存服务", exact: true }).click();
    await page.getByText("已保存，下次对话将使用新的配置。", { exact: true }).waitFor();
    assert.equal((await parent("agent_status")).running, false);
    assert(!(await page.locator('input[type="password"]').first().inputValue()).includes("test-only-secret"));
    await page.screenshot({ path: path.join(output, "model-settings.png") });
    await page.locator("nav button").nth(0).click();
  });
  await check("MCP 表单与 JSON 往返保留高级字段，保存不启动 Pi", async () => {
    const mcpExecutable = path.resolve(process.env.MOCHI_TEST_MCP_BINARY ?? path.join(root, "target/release/examples/agent_mcp_fixture.exe"));
    await fs.access(mcpExecutable);
    await page.getByRole("button", { name: "Agent 设置", exact: true }).click();
    await page.getByRole("tab", { name: "MCP 连接", exact: true }).click();
    await page.getByRole("button", { name: "添加 MCP 连接", exact: true }).click();
    await page.getByLabel("服务名称", { exact: true }).fill("fixture");
    await page.getByLabel("启动程序", { exact: true }).fill(mcpExecutable);
    await page.getByLabel("环境变量", { exact: true }).fill(`MOCHI_MCP_PID_FILE=${mcpPids}`);
    await page.getByRole("button", { name: "JSON", exact: true }).click();
    const editor = page.locator("#mcp-config");
    const value = JSON.parse(await editor.inputValue()); value.mcpServers.fixture.timeout = 42; value.mcpServers.fixture.toolExposure = { echo: "direct" }; value.extra = { preserved: true };
    await editor.fill(JSON.stringify(value, null, 2));
    await page.getByRole("button", { name: "表单", exact: true }).click();
    await page.getByRole("button", { name: "校验并保存 MCP", exact: true }).click();
    await page.getByText("已保存，下次对话将使用新的配置。", { exact: true }).waitFor();
    const saved = await parent("agent_mcp_get"); assert.equal(saved.mcpServers.fixture.timeout, 42); assert(saved.extra.preserved); assert.equal(saved.mcpServers.fixture.toolExposure.echo, "direct");
    await page.screenshot({ path: path.join(output, "mcp-form.png") });
    await parent("agent_model_default", { provider: "mochi-fixture", modelId: "mochi-test" });
    assert.equal((await parent("agent_status")).running, false);
  });
  await check("Skills 文件夹和 ZIP 安装、启停、卸载及目录隔离", async () => {
    const source = path.join(data, "imports/local-skill"), second = path.join(data, "imports/zip-skill"), zip = path.join(data, "import-skill.zip");
    for (const [folder, name] of [[source, "mochi-skill-fixture"], [second, "zip-skill-fixture"]]) { await fs.mkdir(path.join(folder, "references"), { recursive: true }); await fs.writeFile(path.join(folder, "SKILL.md"), `---\nname: ${name}\ndescription: Local native test skill.\n---\n# Test\n`); await fs.writeFile(path.join(folder, "references/example.txt"), "asset"); }
    const quote = (value) => "'" + value.replaceAll("'", "''") + "'";
    execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", `Compress-Archive -LiteralPath ${quote(second)} -DestinationPath ${quote(zip)}`], { windowsHide: true });
    await page.evaluate(([source, zip]) => { const original = window.__TAURI__.core.invoke; window.__TAURI__.core.invoke = (command, args) => command === "choose_agent_skill" ? Promise.resolve(args.folder ? source : zip) : original(command, args); }, [source, zip]);
    await page.getByRole("tab", { name: "Skills", exact: true }).click();
    await page.getByRole("button", { name: "从文件夹安装", exact: true }).click();
    await page.getByRole("button", { name: "停用 mochi-skill-fixture", exact: true }).waitFor();
    await page.getByRole("button", { name: "从 ZIP 安装", exact: true }).click();
    await page.getByRole("button", { name: "停用 zip-skill-fixture", exact: true }).waitFor();
    await page.getByRole("button", { name: "停用 mochi-skill-fixture", exact: true }).click();
    await page.getByRole("button", { name: "启用 mochi-skill-fixture", exact: true }).waitFor();
    assert(!(await parent("agent_skills_list")).find((s) => s.name === "mochi-skill-fixture").enabled);
    await page.getByRole("button", { name: "启用 mochi-skill-fixture", exact: true }).click();
    await page.getByRole("button", { name: "停用 mochi-skill-fixture", exact: true }).waitFor();
    await page.getByRole("button", { name: "卸载 zip-skill-fixture", exact: true }).click();
    await page.getByRole("button", { name: "确认卸载", exact: true }).click();
    await page.getByRole("button", { name: "卸载 zip-skill-fixture", exact: true }).waitFor({ state: "detached" });
    assert.equal((await parent("agent_skills_list")).length, 1);
    await fs.access(path.join(second, "SKILL.md")); await fs.access(path.join(data, "agent/skills/mochi-skill-fixture/references/example.txt"));
    assert.equal((await parent("agent_status")).running, false);
    await page.screenshot({ path: path.join(output, "skills.png") });
    await page.locator("nav button").nth(0).click();
    frame = page.frameLocator('iframe[title="Pi 对话"]');
    await frame.locator("message-editor textarea").waitFor();
  });
  await check("中文组合输入 Enter 不发送", async () => {
    const input = frame.locator("message-editor textarea");
    await input.fill("中文测试");
    await input.dispatchEvent("compositionstart");
    await input.dispatchEvent("keydown", { key: "Enter", isComposing: true });
    assert.equal(modelRequests, 0);
    assert.equal(await input.inputValue(), "中文测试");
    await input.dispatchEvent("compositionend");
  });
  await check("流式 Markdown、代码块与真实 read 工具调用", async () => {
    const started = Date.now();
    await frame.locator("message-editor textarea").press("Enter");
    await frame.getByRole("heading", { name: "麻薯回复" }).waitFor();
    report.firstReplyMs = Date.now() - started;
    assert((await parent("agent_status")).busy, "partial content must be visible before Pi settles");
    await frame.locator("#history-menu summary").click();
    await frame.locator(".history-open").first().waitFor();
    assert(await frame.locator(".history-open").first().isDisabled());
    assert(await frame.locator(".history-delete").first().isDisabled());
    await frame.locator("#history-menu summary").click();
    confirmPartial();
    await waitFor(async () => { const snapshot = await parent("agent_snapshot"); if (snapshot.runtime.error) throw new Error(snapshot.runtime.error); return !snapshot.runtime.busy && snapshot.messages.some((m) => m.role === "toolResult"); }, "tool completed");
    await frame.getByRole("heading", { name: "麻薯回复" }).waitFor();
    assert(toolResults >= 1);
    assert(sawSkill, "enabled local Skill must be advertised to the real Pi model");
    await frame.locator('code-block[language="js"] code').waitFor();
    const tool = frame.locator("tool-message .tool-disclosure").first();
    assert.equal(await tool.getAttribute("open"), null);
    assert.equal(await tool.locator(".tool-content").count(), 0);
    assert((await tool.locator("summary").textContent()).includes("已完成"));
    await tool.locator("summary").click();
    await tool.locator(".tool-content code-block code").first().waitFor();
    assert((await tool.locator(".tool-content").textContent()).includes("Mochi tool fixture"));
    await tool.locator("summary").press("Enter");
    await tool.locator(".tool-content").waitFor({ state: "detached" });
    await page.screenshot({ path: path.join(output, "chat-tool-markdown.png") });
  });
  await check("工具失败和中止摘要可见，展开状态随进度更新保留", async () => {
    await frame.locator("#agent-root").evaluate((root) => {
      const tool = document.createElement("tool-message"); tool.id = "tool-state-fixture";
      tool.toolCall = { type: "toolCall", id: "state-fixture", name: "test_tool", arguments: { path: "fixture.txt" } };
      tool.pending = true; root.append(tool);
    });
    const tool = frame.locator("#tool-state-fixture .tool-disclosure");
    await tool.getByText("执行中", { exact: true }).waitFor();
    assert.equal(await tool.getAttribute("open"), null);
    await tool.locator("summary").click();
    await tool.locator(".tool-content").waitFor();
    await frame.locator("#tool-state-fixture").evaluate((element) => {
      element.pending = false;
      element.result = { role: "toolResult", toolCallId: "state-fixture", toolName: "test_tool", isError: true, timestamp: Date.now(), content: [{ type: "text", text: "fixture error details" }] };
    });
    await tool.getByText("失败", { exact: true }).waitFor();
    assert.equal(await tool.getAttribute("open"), "");
    assert((await tool.locator(".tool-content").textContent()).includes("fixture error details"));
    await tool.locator("summary").click();
    await frame.locator("#tool-state-fixture").evaluate((element) => { element.result = undefined; element.aborted = true; });
    await tool.getByText("已中止", { exact: true }).waitFor();
    assert.equal(await tool.getAttribute("open"), null);
    await tool.locator("summary").click();
    await frame.locator("#tool-state-fixture").evaluate((element) => {
      element.toolCall = { type: "toolCall", id: "another-call", name: "another_tool", arguments: {} };
      element.aborted = false; element.pending = true;
    });
    await tool.getByText("another_tool", { exact: true }).waitFor();
    assert.equal(await tool.getAttribute("open"), null);
    await frame.locator("#tool-state-fixture").evaluate((element) => element.remove());
  });
  await check("图片附件进入 Pi，恢复后使用本地图片预览", async () => {
    const png = await fs.readFile(path.join(root, "skins/png/images/mochi.png"));
    await frame.locator('message-editor input[type="file"]').setInputFiles({ name: "fixture.png", mimeType: "image/png", buffer: png });
    await frame.locator('attachment-tile img').waitFor();
    await frame.locator("message-editor textarea").fill("图片测试");
    await frame.locator("message-editor textarea").press("Enter");
    await waitFor(async () => !(await parent("agent_status")).busy && imageRequests > 0, "image prompt completed");
    await frame.locator("user-message attachment-tile img").waitFor();
    assert(await frame.locator("user-message attachment-tile img").first().evaluate((image) => image.complete && image.naturalWidth > 0));
    await frame.locator("user-message attachment-tile img").first().click();
    await frame.locator("attachment-overlay img").waitFor();
    assert(await frame.locator("attachment-overlay img").evaluate((image) => image.complete && image.naturalWidth > 0));
    await frame.locator("attachment-overlay").press("Escape");
    await frame.locator("attachment-overlay").waitFor({ state: "detached" });
  });
  await check("执行中关闭面板，重开恢复且不重发", async () => {
    await frame.locator("message-editor textarea").fill("慢速测试");
    await frame.locator("message-editor textarea").press("Enter");
    await waitFor(async () => (await parent("agent_status")).busy, "busy");
    const count = modelRequests;
    const generation = (await parent("agent_status")).generation;
    const pending = await parent("agent_skills_toggle", { id: "mochi-skill-fixture", enabled: false });
    assert(pending.pendingConfiguration); assert.equal(pending.generation, generation);
    await page.getByRole("button", { name: "收起面板" }).click();
    await new Promise((r) => setTimeout(r, 1000));
    assert.equal((await parent("agent_status")).running, true);
    await parent("open_panel", { page: 0 });
    browser = await waitFor(() => chromium.connectOverCDP("http://127.0.0.1:9922").catch(() => null), "reopened WebView2");
    await waitFor(async () => { const pages = browser.contexts()[0].pages(); page = pages.find((p) => !p.isClosed()); return page; }, "reopened panel");
    frame = page.frameLocator('iframe[title="Pi 对话"]');
    await frame.locator("message-editor textarea").waitFor();
    await waitFor(async () => !(await parent("agent_status")).busy, "settled after close");
    assert(modelRequests <= count + 2, "panel reopen must not replay a prompt");
    const snapshot = await parent("agent_snapshot");
    assert(snapshot.messages.filter((m) => m.role === "user").length === 3);
    await waitFor(async () => !(await parent("agent_status")).pendingConfiguration, "deferred Skill configuration applied");
  });
  await check("原生 MCP 服务调用不依赖 Node.js", async () => {
    await parent("agent_rpc", { type: "prompt", message: "MCP 测试" });
    await waitFor(async () => !(await parent("agent_status")).busy && mcpResults > 0, "MCP result");
    assert.equal(lastSkillAdvertised, false, "disabled Skill must no longer be advertised after restart");
  });
  await check("停止与新建、切换 JSONL 会话", async () => {
    const previous = (await parent("agent_snapshot")).state.sessionFile;
    await parent("agent_rpc", { type: "prompt", message: "慢速停止测试" });
    await parent("agent_rpc", { type: "stop" });
    assert.equal((await parent("agent_status")).busy, false);
    const fresh = await parent("agent_rpc", { type: "new_session" });
    assert.equal(fresh.messages.length, 0);
    const restored = await parent("agent_rpc", { type: "switch_session", sessionPath: previous });
    assert(restored.messages.length > 0);
    assert((await parent("agent_sessions")).length >= 1);
  });
  report.idleAiMemory = memory();
  await check("关闭面板空闲 60 秒回收，重新打开恢复 JSONL", async () => {
    const previous = (await parent("agent_snapshot")).state.sessionId;
    await page.getByRole("button", { name: "收起面板" }).click();
    await waitFor(async () => !(await parent("agent_status")).running, "60 second idle shutdown", 75000);
    report.afterIdleMemory = memory();
    const requests = modelRequests;
    await parent("open_panel", { page: 0 });
    browser = await waitFor(() => chromium.connectOverCDP("http://127.0.0.1:9922").catch(() => null), "restore after idle");
    page = browser.contexts()[0].pages()[0];
    frame = page.frameLocator('iframe[title="Pi 对话"]');
    await frame.locator("message-editor textarea").waitFor();
    const restored = await waitFor(async () => { const snapshot = await parent("agent_snapshot"); return snapshot.state.sessionId === previous && snapshot.messages.length ? snapshot : null; }, "authoritative session restored after idle");
    assert.equal(restored.state.sessionId, previous);
    assert(restored.messages.length > 0);
    assert.equal(modelRequests, requests);
  });
  await check("历史搜索、删除确认、删除其他会话及当前会话，重开不复活", async () => {
    const original = (await parent("agent_snapshot")).state.sessionFile;
    await frame.locator("#new-session").click();
    await frame.getByRole("heading", { name: "有什么想聊的？" }).waitFor();
    await waitFor(async () => !(await frame.locator("message-editor textarea").isDisabled()), "new conversation ready for input");
    await frame.locator("message-editor textarea").fill("用于删除的临时会话");
    await frame.locator("message-editor textarea").press("Enter");
    await frame.getByRole("heading", { name: "麻薯回复" }).waitFor();
    await waitFor(async () => !(await parent("agent_status")).busy, "temporary conversation completed");
    const temporary = (await parent("agent_snapshot")).state.sessionFile;
    await frame.locator("#history-menu summary").click();
    await frame.getByRole("button", { name: "打开对话：中文测试", exact: true }).click();
    await waitFor(async () => (await parent("agent_snapshot")).state.sessionFile === original, "switched through history UI");
    await frame.locator("#history-menu summary").click();
    await frame.getByRole("searchbox", { name: "搜索历史对话" }).fill("用于删除");
    assert.equal(await frame.locator(".history-row").count(), 1);
    await frame.getByRole("button", { name: "删除对话：用于删除的临时会话", exact: true }).click();
    await frame.getByRole("button", { name: "取消", exact: true }).click();
    await fs.access(temporary);
    await frame.locator("#history-menu summary").click();
    await frame.getByRole("button", { name: "删除对话：用于删除的临时会话", exact: true }).click();
    await frame.getByRole("button", { name: "删除对话", exact: true }).click();
    await frame.locator("#delete-session-dialog").waitFor({ state: "hidden" });
    assert(!(await parent("agent_sessions")).some((item) => item.title === "用于删除的临时会话"));
    assert.equal((await parent("agent_snapshot")).state.sessionFile, original);
    await assert.rejects(fs.access(temporary));
    await frame.locator("#history-menu summary").click();
    await frame.getByRole("searchbox", { name: "搜索历史对话" }).fill("");
    await frame.getByRole("button", { name: "打开对话：中文测试", exact: true }).waitFor();
    assert.equal(await frame.locator('.history-open[aria-current="true"]').count(), 1);
    await page.screenshot({ path: path.join(output, "history-menu.png") });
    await frame.getByRole("button", { name: "删除对话：中文测试", exact: true }).click();
    const viewport = await page.locator('iframe[title="Pi 对话"]').boundingBox();
    const confirmation = await frame.locator("#delete-session-dialog").boundingBox();
    assert(Math.abs(confirmation.x + confirmation.width / 2 - viewport.x - viewport.width / 2) < 2, "confirmation must be horizontally centered");
    assert(Math.abs(confirmation.y + confirmation.height / 2 - viewport.y - viewport.height / 2) < 2, "confirmation must be vertically centered");
    await page.screenshot({ path: path.join(output, "delete-conversation.png") });
    await frame.getByRole("button", { name: "删除对话", exact: true }).click();
    await frame.getByRole("heading", { name: "有什么想聊的？" }).waitFor();
    assert.equal((await parent("agent_status")).running, false);
    await assert.rejects(fs.access(original));
    await assert.rejects(fs.access(path.join(data, "agent/current.json")));
    await page.getByRole("button", { name: "收起面板" }).click();
    await new Promise((resolve) => setTimeout(resolve, 1000));
    await parent("open_panel", { page: 0 });
    browser = await waitFor(() => chromium.connectOverCDP("http://127.0.0.1:9922").catch(() => null), "reopen after deleting current conversation");
    page = browser.contexts()[0].pages()[0];
    frame = page.frameLocator('iframe[title="Pi 对话"]');
    await frame.getByRole("heading", { name: "有什么想聊的？" }).waitFor();
    assert.equal((await parent("agent_status")).running, false);
    assert.deepEqual((await parent("agent_snapshot")).messages, []);
  });
  assert.deepEqual(report.errors, []);
  report.passed = true;
} finally {
  report.modelRequests = modelRequests; report.toolResults = toolResults; report.stderr = stderr;
  await fs.writeFile(path.join(output, "report.json"), JSON.stringify(report, null, 2));
  if (browser) await browser.close().catch(() => {});
  child.stdin.write(JSON.stringify({ id: ++serial, command: "quit", args: {} }) + "\n");
  await Promise.race([new Promise((r) => child.on("exit", r)), new Promise((r) => setTimeout(r, 6000))]);
  if (child.exitCode === null) child.kill();
  const pids = await fs.readFile(mcpPids, "utf8").catch(() => "");
  for (const pid of pids.trim().split(/\s+/).filter(Boolean).map(Number)) {
    let alive = true; try { process.kill(pid, 0); } catch { alive = false; }
    assert(!alive, `MCP descendant ${pid} survived application shutdown`);
  }
  server.closeAllConnections(); server.close();
}
