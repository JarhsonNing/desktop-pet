// Exercise the bundled chat components in a real Tauri/WebView2, without model calls.
import { chromium } from "@playwright/test";
import { spawn } from "node:child_process";
import { promises as fs } from "node:fs";
import { createInterface } from "node:readline";
import path from "node:path";
import assert from "node:assert/strict";

const root = path.resolve(import.meta.dirname, "../..");
const output = path.resolve(process.env.MOCHI_AGENT_TEST_OUTPUT ?? path.join(root, ".artifacts/agent-tools"));
const data = path.join(output, `data-${Date.now()}`);
await fs.mkdir(data, { recursive: true });
await fs.writeFile(path.join(data, "config.json"), JSON.stringify({ clipboard_enabled: false, explorer_enabled: false }));
const executable = process.env.MOCHI_TEST_BINARY ?? path.join(root, "target/release/desktop-pet.exe");
const child = spawn(executable, ["--data-dir", data, "--settings", "--no-shell", "--control-stdin", "--panel-debug-port", "9925", "--exit-after", "120"], { stdio: ["pipe", "pipe", "pipe"], env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview2-test") }, windowsHide: true });
let stderr = "", serial = 10000, browser;
child.stderr.on("data", chunk => stderr += chunk);
const pending = new Map();
createInterface({ input: child.stdout }).on("line", line => { try { const reply = JSON.parse(line); pending.get(reply.id)?.(reply); } catch {} });
async function host(command, args = {}) {
  const id = ++serial;
  const reply = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`Timeout ${command}`)); }, 30000);
    pending.set(id, value => { clearTimeout(timer); pending.delete(id); resolve(value); });
    child.stdin.write(JSON.stringify({ id, command, args }) + "\n");
  });
  assert(!reply.error, reply.error); return reply.value;
}
async function wait(fn) {
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) { const value = await fn(); if (value) return value; await new Promise(resolve => setTimeout(resolve, 100)); }
  throw new Error(`Timed out\n${stderr}`);
}
const report = { passed: false, checks: [] };
async function check(name, fn) { await fn(); report.checks.push(name); console.log(`PASS ${name}`); }
try {
  browser = await wait(() => chromium.connectOverCDP("http://127.0.0.1:9925").catch(() => null));
  const page = browser.contexts()[0].pages()[0];
  const frame = page.frameLocator('iframe[title="Pi 对话"]');
  await wait(async () => await frame.locator("message-editor textarea").count() && !(await frame.locator("message-editor textarea").isDisabled()));
  const command = "printf '%s\\n' '中文命令 <tag>'\nprintf '%s\\n' 'second line'";
  const result = (text, isError = false) => ({ role: "toolResult", toolCallId: "bash-fixture", toolName: "bash", content: [{ type: "text", text }], isError, timestamp: 2 });
  async function present(output, args = { command }, pending = false) {
    await frame.locator("agent-interface").evaluate((chat, { output, args, pending }) => {
      chat.session.restore({ generation: 0, sequence: 0, runtime: { running: false, busy: pending }, state: { sessionId: "bash-test" }, partial: null, interactions: [], toolProgress: pending ? [{ type: "tool_execution_start", toolCallId: "bash-fixture", toolName: "bash" }] : [], messages: [
        { role: "assistant", content: [{ type: "toolCall", id: "bash-fixture", name: "bash", arguments: args }], stopReason: "toolUse", timestamp: 1 },
        ...(output ? [output] : []),
      ] });
    }, { output, args, pending });
  }
  const tool = frame.locator("tool-message .tool-disclosure");
  await check("Bash 历史结果默认折叠，展开显示多行命令与完整文本输出", async () => {
    await present(result("中文输出\n<img src=x onerror=alert(1)>\nsecond line"));
    await tool.getByText("已完成", { exact: true }).waitFor();
    assert.equal(await tool.getAttribute("open"), null);
    await tool.locator("summary").click();
    await tool.locator(".bash-command code-block code").waitFor();
    assert.equal(await tool.locator(".bash-command code-block code").textContent(), command);
    assert.equal(await tool.locator(".bash-output code-block code").textContent(), "中文输出\n<img src=x onerror=alert(1)>\nsecond line");
    assert.equal(await tool.locator(".bash-output img").count(), 0);
    assert.equal(await tool.locator("console-block").count(), 0);
    assert(!(await tool.textContent()).includes("Running command..."));
    await page.screenshot({ path: path.join(output, "bash-expanded.png") });
  });
  await check("Bash 运行中展示部分输出，完成更新后保留展开和命令", async () => {
    await present({ ...result("first chunk"), mochiProgress: true }, { command }, true);
    await tool.getByText("执行中", { exact: true }).waitFor();
    await tool.locator(".bash-output code-block code").filter({ hasText: "first chunk" }).waitFor();
    await present(result("first chunk\nfinished"));
    await tool.getByText("已完成", { exact: true }).waitFor();
    assert.equal(await tool.getAttribute("open"), "");
    assert.equal(await tool.locator(".bash-output code-block code").textContent(), "first chunk\nfinished");
  });
  await check("Bash 失败仍显示命令和错误输出", async () => {
    await present(result("command not found: missing-command", true));
    await tool.getByText("失败", { exact: true }).waitFor();
    assert.equal(await tool.locator(".bash-command code-block code").textContent(), command);
    await tool.locator(".bash-output code-block code").filter({ hasText: "command not found" }).waitFor();
  });
  await check("Bash 空输出与缺失命令参数明确展示，结果不被丢弃", async () => {
    await present(result("")); await tool.getByText("（无输出）", { exact: true }).waitFor();
    await present(result("output without recorded args"), {});
    await tool.getByText("未记录命令参数", { exact: true }).waitFor();
    await tool.locator(".bash-output code-block code").filter({ hasText: "output without recorded args" }).waitFor();
  });
  await check("Bash 输出原样保留 Base64 外形的文本与长 Unicode 输出", async () => {
    for (const output of ["YWJj", "中文🙂\n".repeat(12000)]) {
      await present(result(output));
      await wait(async () => await tool.locator(".bash-output code-block code").textContent() === output);
    }
  });
  assert.equal((await host("agent_status")).running, false);
  report.passed = true;
} finally {
  child.stdin.write(JSON.stringify({ id: ++serial, command: "quit", args: {} }) + "\n");
  await Promise.race([new Promise(resolve => child.on("exit", resolve)), new Promise(resolve => setTimeout(resolve, 6000))]);
  if (child.exitCode === null) child.kill();
  if (browser) await browser.close().catch(() => {});
  report.stderr = stderr;
  await fs.writeFile(path.join(output, "report.json"), JSON.stringify(report, null, 2));
}
