// Exercise the actual Tauri/WebView2 panel attached to an isolated native pet.
import { chromium } from "@playwright/test";
import { spawn, execFile } from "node:child_process";
import { promises as fs } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { createInterface } from "node:readline";
const root = path.resolve(import.meta.dirname, "../..");
const data = path.resolve(
  process.argv[2] ?? path.join(root, ".artifacts/content-ui-final"),
);
const output = path.resolve(
  process.env.MOCHI_TEST_OUTPUT ?? path.join(root, ".artifacts/tauri-e2e"),
);
await fs.mkdir(output, { recursive: true });
const executable = path.resolve(
  process.env.MOCHI_TEST_BINARY ??
    path.join(root, "target/release/desktop-pet.exe"),
);
const debugPort = process.env.MOCHI_TEST_PORT ?? "9921";
const child = spawn(
  executable,
  [
    "--data-dir",
    data,
    "--settings-page",
    "home",
    "--panel-debug-port",
    debugPort,
    "--no-shell",
    "--control-stdin",
    "--exit-after",
    "240",
  ],
  {
    cwd: root, stdio: ["pipe", "pipe", "pipe"],
    // Independent profiles prevent a running panel's WebView2 process from
    // swallowing this test's debug-port argument. No production profile changes.
    env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview2-test") },
  },
);
let serial = 1000;
const waiting = new Map();
const lines = createInterface({ input: child.stdout });
lines.on("line", (line) => {
  try {
    const message = JSON.parse(line);
    waiting.get(message.id)?.(message);
  } catch {}
});
async function parent(command, args = {}) {
  const id = ++serial;
  const message = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      waiting.delete(id);
      reject(new Error("Parent timeout"));
    }, 15000);
    waiting.set(id, (value) => {
      clearTimeout(timer);
      waiting.delete(id);
      resolve(value);
    });
    child.stdin.write(JSON.stringify({ id, command, args }) + "\n");
  });
  assert(!message.error, message.error);
  return message.value;
}
let stderr = "";
child.stderr.on("data", (data) => {
  stderr += data.toString();
});
let browser;
const report = {
  passed: false,
  checks: [],
  screenshots: [],
  petPid: child.pid,
};
async function check(name, fn) {
  await fn();
  report.checks.push(name);
  console.log(`PASS ${name}`);
}
async function host(page, command, args = {}) {
  return page.evaluate(
    ({ command, args }) =>
      window.__TAURI__.core.invoke("host", { command, args }),
    { command, args },
  );
}
async function waitState(page, predicate, timeout = 30000) {
  const deadline = Date.now() + timeout;
  let value;
  do {
    value = await host(page, "snapshot");
    if (predicate(value)) return value;
    await new Promise(resolve => setTimeout(resolve, 100));
  } while (Date.now() < deadline);
  throw new Error(`State did not settle: ${JSON.stringify(value?.content.delivery)}`);
}
let panelWindowHandle = 0;
async function windowState(mode = "state") {
  const output = await new Promise((resolve, reject) => execFile("powershell.exe", [
    "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File",
    path.join(root, "panel/tests/window-state.ps1"), "-PetId", String(child.pid), "-Mode", mode,
    "-WindowHandle", String(mode === "minimize" ? 0 : panelWindowHandle),
  ], { windowsHide: true, timeout: 10000 }, (error, stdout) => error ? reject(error) : resolve(stdout)));
  const state = JSON.parse(output);
  panelWindowHandle = state.windowHandle;
  return state;
}
try {
  const deadline = Date.now() + 60000;
  while (Date.now() < deadline) {
    try {
      browser = await chromium.connectOverCDP(`http://127.0.0.1:${debugPort}`);
      break;
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 350));
    }
  }
  assert(browser, "Tauri WebView2 debug target did not start");
  const context = browser.contexts()[0];
  let page =
    context.pages().find((page) => /tauri/.test(page.url())) ??
    context.pages()[0];
  assert(page);
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  await page.frameLocator('iframe[title="Pi 对话"]').getByRole("heading", { name: "有什么想聊的？" }).waitFor();
  await page.waitForFunction(() => !!document.querySelector(".sidebar-pet img"));
  await page.evaluate(() => document.fonts.ready);
  await check("真实 Tauri 窗口与七页导航", async () => {
    assert.equal(await page.locator("nav button").count(), 7);
    assert((await host(page, "snapshot")).config);
  });
  for (let index = 0; index < 6; index++) {
    await page.locator("nav button").nth(index).click();
    await page.waitForTimeout(500);
    await page.evaluate(() => document.fonts.ready);
    const file = path.join(
      output,
      ["home", "preferences", "journal", "skin", "clipboard", "transfer"][
        index
      ] + ".png",
    );
    await page.screenshot({ path: file });
    report.screenshots.push(file);
  }
  await check("相同页面再次打开时恢复最小化窗口并前置聚焦", async () => {
    const before = await windowState("minimize");
    let state = await windowState();
    assert(state.minimized, `Panel did not minimize: ${JSON.stringify({ before, state })}`);
    // Shares the Settings action with pet right-click. Page 5 is already open,
    // so navigation cannot accidentally mask a missing focus request.
    await parent("open_panel", { page: 5 });
    const deadline = Date.now() + 10000;
    do {
      state = await windowState();
      if (!state.minimized && state.foreground) break;
    } while (Date.now() < deadline);
    assert.equal(state.panelPid, before.panelPid);
    assert(state.visible && !state.minimized && state.foreground, JSON.stringify(state));
  });
  await check("昵称中文输入与配置持久化", async () => {
    await page.locator("nav button").nth(1).click();
    const nickname = page.locator("#nickname");
    await nickname.fill("");
    await nickname.dispatchEvent("compositionstart");
    await nickname.evaluate((element) => {
      element.value = "拼音预编辑";
      element.dispatchEvent(
        new InputEvent("input", { bubbles: true, isComposing: true }),
      );
      element.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "Enter",
          bubbles: true,
          isComposing: true,
        }),
      );
    });
    assert.equal(await nickname.inputValue(), "拼音预编辑");
    await nickname.evaluate((element) => {
      element.value = "曦玥珩璟團團𠮷";
      element.dispatchEvent(
        new CompositionEvent("compositionend", {
          bubbles: true,
          data: element.value,
        }),
      );
    });
    await page.getByRole("button", { name: "保存昵称" }).click();
    await waitState(page, value => value.config.name === "曦玥珩璟團團𠮷");
    assert(
      await page.evaluate(() =>
        document.fonts.check('14px "Mochi Sans"', "𠮷"),
      ),
    );
  });
  await check("宠物互动调用原生引擎", async () => {
    const before = (await host(page, "snapshot")).config.feeds;
    await page.locator("nav button").nth(0).click();
    const chat = page.frameLocator('iframe[title="Pi 对话"]');
    await chat.locator("#pet-menu summary").click();
    await chat.getByRole("button", { name: "投喂一块饼干" }).click();
    await waitState(page, value => value.config.feeds === before + 1);
  });
  await check("单独授权提示保持安全默认值", async () => {
    await page.locator("nav button").nth(1).click();
    await page.getByRole("button", { name: "了解并授权" }).click();
    assert(await page.getByRole("button", { name: "确认授权" }).isDisabled());
    await page.getByRole("button", { name: "暂时不要" }).click();
    assert.equal((await host(page, "snapshot")).config.allow_hide_icons, false);
  });
  await check("剪切板全文搜索与原生导出准备", async () => {
    await page.locator("nav button").nth(4).click();
    await page.getByRole("textbox", { name: "搜索记录" }).fill("工作笔记");
    await page.waitForTimeout(600);
    assert.equal(await page.locator(".record").count(), 1);
    await page.locator(".record").first().click();
    await page.waitForFunction(() =>
      document.querySelector(".preview-area")?.textContent.includes("生僻字"),
    );
    await page.getByRole("button", { name: "准备拖到其他应用" }).click();
    await page.locator(".drag-handle").waitFor();
    assert.equal(
      (await host(page, "snapshot")).config.clipboard_enabled,
      false,
    );
    await page.getByRole("textbox", { name: "搜索记录" }).fill("不存在的内容");
    await page.getByRole("heading", { name: "没有找到匹配内容" }).waitFor();
  });
  await check("中转站大图与收藏", async () => {
    await page.locator("nav button").nth(5).click();
    await page.locator(".record").filter({ hasText: "图片" }).click();
    await page.locator(".image-preview").waitFor();
    await page.locator(".image-preview").click();
    assert(await page.locator(".image-modal").isVisible());
    await page.getByRole("button", { name: "关闭大图" }).click();
    await page.getByRole("button", { name: "收藏", exact: true }).click();
    await page.getByRole("button", { name: "取消收藏", exact: true }).waitFor();
    await page.getByRole("button", { name: "取消收藏", exact: true }).click();
  });
  await check("皮肤重载接口与独立接收区域", async () => {
    await page.locator("nav button").nth(3).click();
    assert(await page.locator(".skin-drop-zone").isVisible());
    const manifest = (await host(page, "snapshot")).skin;
    assert(
      (await host(page, "skin_preview")).startsWith(
        "data:image/svg+xml;base64,",
      ),
    );
    assert(manifest.id);
    const loaded = await host(page, "load_skin", {
      path: path.join(root, "skins/blue"),
    });
    assert.equal(loaded.id, "mochi-blue");
    assert.equal((await host(page, "reload_skin")).id, "mochi-blue");
    await assert.rejects(
      host(page, "load_skin", {
        path: path.join(root, "skins/missing-fixture"),
      }),
    );
    assert.equal((await host(page, "snapshot")).skin.id, "mochi-blue");
    assert.equal((await host(page, "default_skin")).id, manifest.id);
  });
  await check("多批拖入按文件累加默认选择，删除原件仍能取用", async () => {
    const input = path.join(data, "delivery-input");
    await fs.mkdir(input, { recursive: true });
    const files = ["第一份.txt", "第二份.txt", "第三份.txt"].map((name) =>
      path.join(input, name),
    );
    for (const file of files) await fs.writeFile(file, path.basename(file));
    async function drop(paths, count) {
      await host(page, "import", {
        payloads: [{ Files: paths }],
        auto_select: true,
      });
      await waitState(page, ({ content }) => content.delivery.count === count && content.delivery.ready);
    }
    await drop(files.slice(0, 2), 2);
    await drop(files.slice(2), 3);
    const { content } = await host(page, "snapshot");
    assert.equal(
      content.delivery.choices.filter(
        (choice) => choice.selected && choice.recent,
      ).length,
      3,
      JSON.stringify({ delivery: content.delivery, entries: content.records.transfer }),
    );
    const ids = [
      ...new Set(
        content.delivery.choices
          .filter((choice) => choice.selected)
          .map((choice) => choice.pick.entry),
      ),
    ];
    for (const file of files) await fs.unlink(file);
    const exported = await host(page, "prepare", { ids });
    assert.equal(exported.files.length, 3);
    for (const file of exported.files)
      assert.equal(await fs.readFile(file, "utf8"), path.basename(file));
  });
  await check("待发送选择器支持逐文件修改、收藏与清空", async () => {
    await page.locator("nav button").nth(5).click();
    await page.getByRole("button", { name: /选择待发送文件/ }).click();
    const dialog = page.getByRole("dialog", { name: "选择待发送文件" });
    await dialog.waitFor();
    assert.equal(
      await dialog.locator('input[type="checkbox"]:checked').count(),
      3,
    );
    await page.screenshot({ path: path.join(output, "delivery-picker.png") });
    report.screenshots.push(path.join(output, "delivery-picker.png"));
    await dialog.locator('input[type="checkbox"]').first().uncheck();
    await dialog.getByRole("button", { name: "确认待发送" }).click();
    await dialog.waitFor({ state: "hidden" });
    assert.equal((await host(page, "snapshot")).content.delivery.count, 2);
    await host(page, "favorite", { ids: ["demo-files"], value: true });
    await page.getByRole("button", { name: /选择待发送文件/ }).click();
    await dialog.getByRole("button", { name: "我的收藏", exact: true }).click();
    await dialog
      .locator(".delivery-choice")
      .filter({ hasText: "明天的计划.txt" })
      .waitFor();
    await dialog.getByRole("button", { name: "清空选择" }).click();
    await dialog.getByRole("button", { name: "确认待发送" }).click();
    await dialog.waitFor({ state: "hidden" });
    assert.equal((await host(page, "snapshot")).content.delivery.count, 0);
    await page.getByRole("button", { name: /选择待发送文件/ }).click();
    await dialog
      .locator(".delivery-choice")
      .filter({ hasText: "明天的计划.txt" })
      .locator("input")
      .check();
    await dialog.getByRole("button", { name: "确认待发送" }).click();
    await dialog.waitFor({ state: "hidden" });
    await waitState(page, value => value.content.delivery.ready);
    assert.equal((await host(page, "snapshot")).content.delivery.count, 1);
  });
  await check("原生窗口拒绝无按键的过期拖动，取消后保留选择", async () => {
    const before = JSON.parse(
      await fs.readFile(path.join(data, "delivery-state.json"), "utf8"),
    );
    // Exercise the native posted-message guard without moving the real cursor.
    // Cross-application OLE acceptance still needs a physical drag compatibility check.
    await parent("delivery_begin");
    await waitState(page, value => !value.content.delivery.active);
    const after = JSON.parse(
      await fs.readFile(path.join(data, "delivery-state.json"), "utf8"),
    );
    assert.equal(after.epoch, before.epoch + 1);
    assert.equal(after.recent.length, 0);
    assert.equal((await host(page, "snapshot")).content.delivery.count, 1);
  });
  await check("此电脑文件箱展示独立副本，目录新增不写回记录", async () => {
    await page.locator("nav button").nth(1).click();
    await page.getByRole("checkbox", { name: "在此电脑显示文件箱" }).check();
    await waitState(page, ({ content }) => content.explorer.bytes > 0 && !content.progress["explorer-view"]);
    const { content } = await host(page, "snapshot");
    assert.equal(content.explorer.error, null);
    const view = content.explorer.root;
    assert((await fs.readdir(path.join(view, "中转站"))).length >= 5);
    assert((await fs.readdir(path.join(view, "我的收藏"))).length >= 1);
    await fs.writeFile(path.join(view, "本地文件不导入.txt"), "仅在展示目录中");
    await page.waitForTimeout(700);
    assert.equal(
      (await host(page, "snapshot")).content.records.transfer.length,
      content.records.transfer.length,
    );
    await page
      .locator(".card")
      .filter({ hasText: "此电脑 · Mochi 文件箱" })
      .scrollIntoViewIfNeeded();
    await page.screenshot({
      path: path.join(output, "filebox-preferences.png"),
    });
    report.screenshots.push(path.join(output, "filebox-preferences.png"));
    await page.getByRole("checkbox", { name: "在此电脑显示文件箱" }).uncheck();
    assert.equal((await host(page, "snapshot")).config.explorer_enabled, false);
    assert((await fs.stat(view)).isDirectory());
  });
  await check("系统窗口关闭保存昵称草稿", async () => {
    await page.locator("nav button").nth(1).click();
    await page.locator("#nickname").fill("曦玥團團𠮷");
    const closed = page.waitForEvent("close", { timeout: 10000 });
    await page.evaluate(() =>
      window.__TAURI__.core.invoke("plugin:window|close", { label: "main" }),
    );
    await closed;
    assert.equal((await parent("snapshot")).config.name, "曦玥團團𠮷");
    assert.equal((await parent("snapshot")).content.delivery.count, 1);
    await browser.close().catch(() => {});
    await parent("open_panel", { page: 0 });
    browser = null;
    const deadline = Date.now() + 30000;
    while (Date.now() < deadline) {
      try {
        browser = await chromium.connectOverCDP(`http://127.0.0.1:${debugPort}`);
        break;
      } catch {
        await new Promise((resolve) => setTimeout(resolve, 350));
      }
    }
    assert(browser, "Panel did not reopen");
    page = browser.contexts()[0].pages()[0];
    await page.frameLocator('iframe[title="Pi 对话"]').getByRole("heading", { name: "有什么想聊的？" }).waitFor();
  });
  await check("窗口关闭后宠物继续运行", async () => {
    await host(page, "receiving", {
      id: "fixture-panel-receive",
      label: "模拟附件进度",
      bytes: 1,
    });
    await page.getByRole("button", { name: "收起面板" }).click();
    await page.waitForEvent("close", { timeout: 10000 });
    await new Promise((resolve) => setTimeout(resolve, 1500));
    assert.equal(child.exitCode, null);
    assert(
      !Object.hasOwn(
        (await parent("snapshot")).content.progress,
        "fixture-panel-receive",
      ),
    );
  });
  assert.deepEqual(errors, []);
  report.passed = true;
} catch (error) {
  report.error = String(error);
  report.stderr = stderr;
  throw error;
} finally {
  if (browser) await browser.close().catch(() => {});
  // Stop only our isolated pet through its normal shutdown/recovery path.
  if (child.exitCode === null) {
    try {
      await parent("update_config", { explorer_enabled: false });
      await parent("quit");
    } catch {
      child.kill();
    }
  }
  lines.close();
  await fs.writeFile(
    path.join(output, "report.json"),
    JSON.stringify(report, null, 2),
  );
  console.log(JSON.stringify(report, null, 2));
}
