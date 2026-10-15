// Real Tauri/IPC/disk migration checks. Folder paths are supplied through IPC;
// the OS folder picker itself remains an interactive rfd dialog.
import { chromium } from "@playwright/test";
import { spawn, execFile } from "node:child_process";
import { createInterface } from "node:readline";
import { promises as fs } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
const root = path.resolve(import.meta.dirname, "../..");
const data = path.resolve(process.argv[2]);
const binary = path.resolve(process.env.MOCHI_TEST_BINARY ?? "target/release/desktop-pet.exe");
const port = process.env.MOCHI_TEST_PORT ?? "9935";
const output = path.join(root, ".artifacts/storage-e2e");
await fs.mkdir(output, { recursive: true });
let child, lines, browser, page, serial = 800000, stderr = "";
const pending = new Map();
const report = { passed: false, checks: [] };
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
async function parent(command, args = {}) {
  const id = ++serial;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`Parent timeout: ${command}`)); }, 12000);
    pending.set(id, message => {
      clearTimeout(timer); pending.delete(id);
      message.error ? reject(new Error(message.error)) : resolve(message.value);
    });
    child.stdin.write(JSON.stringify({ id, command, args }) + "\n");
  });
}
async function host(command, args = {}) {
  return page.evaluate(({ command, args }) => window.__TAURI__.core.invoke("host", { command, args }), { command, args });
}
async function wait(predicate) {
  const deadline = Date.now() + 30000;
  do {
    const snapshot = await host("snapshot");
    if (predicate(snapshot)) return snapshot;
    await pause(100);
  } while (Date.now() < deadline);
  throw new Error("Storage state did not settle");
}
async function start() {
  child = spawn(binary, ["--data-dir", data, "--settings-page", "preferences", "--panel-debug-port", port, "--no-shell", "--control-stdin", "--exit-after", "150"], {
    cwd: root, stdio: ["pipe", "pipe", "pipe"], windowsHide: true,
    env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview2-test") },
  });
  lines = createInterface({ input: child.stdout });
  lines.on("line", line => { try { const message = JSON.parse(line); pending.get(message.id)?.(message); } catch {} });
  child.stderr.on("data", bytes => { stderr += bytes; });
  browser = null;
  const deadline = Date.now() + 40000;
  while (Date.now() < deadline) {
    try { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`); break; }
    catch { await pause(300); }
  }
  assert(browser, "Isolated WebView2 did not start");
  page = browser.contexts()[0].pages()[0];
  await page.locator("#nickname").waitFor();
}
async function stop() {
  if (browser) { await browser.close().catch(() => {}); browser = null; }
  if (child && child.exitCode === null) {
    const exited = new Promise(resolve => child.once("exit", resolve));
    try { await parent("quit"); } catch { child.kill(); }
    await Promise.race([exited, pause(10000)]);
    if (child.exitCode === null) child.kill();
  }
  lines?.close();
}
async function check(name, fn) {
  await fn(); report.checks.push(name); console.log(`PASS ${name}`);
}
let library, filebox, expectedIds;
try {
  await start();
  await check("原生目录选择器属于当前面板，取消后目录不变", async () => {
    const before = (await host("snapshot")).content.storage;
    await page.getByRole("button", { name: "选择中转站存储目录", exact: true }).click();
    await pause(700);
    await new Promise((resolve, reject) => execFile("powershell.exe", [
      "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File",
      path.join(root, "panel/tests/cancel-storage-dialog.ps1"), "-PetId", String(child.pid),
    ], { windowsHide: true, timeout: 10000 }, (error, stdout) => error ? reject(error) : resolve(stdout)));
    await wait(value => !value.content.storage.busy);
    assert.equal((await host("snapshot")).content.storage.content, before.content);
    await page.getByRole("button", { name: "选择中转站存储目录", exact: true }).waitFor();
    await page.waitForFunction(() => !document.querySelector('button[aria-label="选择中转站存储目录"]').disabled);
  });
  await check("偏好中可分别选择两个目录，内容库迁移保留记录和独立副本", async () => {
    assert(await page.getByRole("button", { name: "选择中转站存储目录", exact: true }).isEnabled());
    assert(await page.getByRole("button", { name: "选择 Mochi 文件箱目录", exact: true }).isEnabled());
    const initial = await host("snapshot");
    expectedIds = initial.content.records.transfer.map(entry => entry.id).sort();
    const old = initial.content.storage.content;
    assert.equal(path.resolve(old), path.join(data, "content"));
    library = path.join(data, "自选内容库");
    await fs.mkdir(library);
    await host("storage_move", { kind: "content", path: library });
    const snapshot = await wait(value => value.content.storage.content === library && !value.content.storage.busy);
    assert.deepEqual(snapshot.content.records.transfer.map(entry => entry.id).sort(), expectedIds);
    assert((await fs.stat(path.join(old, "index.sqlite"))).isFile());
    // Both checked paths belong to this disposable fixture; keep the old copy.
    assert(path.resolve(old).startsWith(data + path.sep));
    await fs.rename(old, path.join(data, "原内容库备份"));
    const exported = await host("prepare", { ids: ["demo-files"] });
    assert.equal(exported.files.length, 2);
    assert(exported.files.every(file => file.startsWith(library + path.sep) || file.startsWith("\\\\?\\" + library + path.sep)));
    assert.equal(await fs.readFile(path.join(exported.files[0], "说明.txt"), "utf8"), "独立中转副本保留目录层级。");
  });
  await check("非空目录切换被拒绝，用户文件与当前库不变", async () => {
    const occupied = path.join(data, "已有内容"); await fs.mkdir(occupied);
    await fs.writeFile(path.join(occupied, "user.txt"), "不要覆盖");
    await assert.rejects(host("storage_move", { kind: "content", path: occupied }), /空文件夹/);
    const snapshot = await wait(value => !value.content.storage.busy);
    assert.equal(snapshot.content.storage.content, library);
    assert.equal(await fs.readFile(path.join(occupied, "user.txt"), "utf8"), "不要覆盖");
  });
  await check("文件箱目录独立迁移，旧副本保留且后续记录只更新新位置", async () => {
    await page.getByRole("checkbox", { name: "在此电脑显示文件箱" }).check();
    const before = await wait(value => value.content.explorer.bytes > 0 && !value.content.progress["explorer-view"]);
    const old = before.content.storage.explorer;
    await fs.writeFile(path.join(old, "用户附加.txt"), "保留本地内容");
    filebox = path.join(data, "自选文件箱"); await fs.mkdir(filebox);
    await host("storage_move", { kind: "explorer", path: filebox });
    await wait(value => value.content.storage.explorer === filebox && !value.content.progress["explorer-view"]);
    assert.equal(await fs.readFile(path.join(filebox, "用户附加.txt"), "utf8"), "保留本地内容");
    const oldIndex = await fs.readFile(path.join(old, ".mochi-view.json"), "utf8");
    await host("import", { payloads: [{ Text: { plain: "切换后新收纳的内容", html: null, rtf: null } }] });
    await wait(value => value.content.records.transfer.some(entry => entry.preview === "切换后新收纳的内容") && !value.content.progress["explorer-view"]);
    assert.equal(await fs.readFile(path.join(old, ".mochi-view.json"), "utf8"), oldIndex);
    const view = JSON.parse(await fs.readFile(path.join(filebox, ".mochi-view.json"), "utf8"));
    assert(Object.keys(view.records).length > Object.keys(JSON.parse(oldIndex).records).length);
    await page.locator(".storage-path").filter({ hasText: filebox }).waitFor();
    await page.locator(".card").filter({ hasText: "此电脑 · Mochi 文件箱" }).scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(output, "selected-directories.png") });
  });
  await check("重启后两个自选目录及历史仍有效", async () => {
    await stop(); await start();
    const snapshot = await wait(value => value.content.records.transfer.length >= expectedIds.length);
    assert.equal(snapshot.content.storage.content, library);
    assert.equal(snapshot.content.storage.explorer, filebox);
    const entries = await host("search", { source: "Clipboard", query: "工作笔记" });
    assert.equal(entries.length, 1);
    await page.getByRole("button", { name: "选择中转站存储目录", exact: true }).scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(output, "selected-library.png") });
  });
  report.passed = true;
} catch (error) {
  report.error = String(error); report.stderr = stderr; throw error;
} finally {
  // Only unregister the isolated fixture's own This PC entry.
  if (child?.exitCode === null) { try { await parent("update_config", { explorer_enabled: false }); } catch {} }
  await stop();
  await fs.writeFile(path.join(output, "report.json"), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
}
