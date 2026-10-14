// Exercise the actual Tauri/WebView2 panel attached to an isolated native pet.
import { chromium } from "@playwright/test";
import { spawn } from "node:child_process";
import { promises as fs } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { createInterface } from "node:readline";
const root = path.resolve(import.meta.dirname, "../..");
const data = path.resolve(
  process.argv[2] ?? path.join(root, ".artifacts/content-ui-final"),
);
const output = path.join(root, ".artifacts/tauri-e2e");
await fs.mkdir(output, { recursive: true });
const executable = path.resolve(
  process.env.MOCHI_TEST_BINARY ??
    path.join(root, "target/release/desktop-pet.exe"),
);
const child = spawn(
  executable,
  [
    "--data-dir",
    data,
    "--settings-page",
    "home",
    "--panel-debug-port",
    "9921",
    "--no-shell",
    "--control-stdin",
    "--exit-after",
    "150",
  ],
  { cwd: root, stdio: ["pipe", "pipe", "pipe"] },
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
try {
  const deadline = Date.now() + 60000;
  while (Date.now() < deadline) {
    try {
      browser = await chromium.connectOverCDP("http://127.0.0.1:9921");
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
  await page.getByRole("heading", { name: "今天也有我陪着你" }).waitFor();
  await page.waitForFunction(() => !!document.querySelector(".hero-art img"));
  await page.evaluate(() => document.fonts.ready);
  await check("真实 Tauri 窗口与六页导航", async () => {
    assert.equal(await page.locator("nav button").count(), 6);
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
  await check("昵称中文输入与配置持久化", async () => {
    await page.locator("nav button").nth(1).click();
    const nickname = page.locator("#nickname");
    await nickname.fill("");
    await nickname.dispatchEvent("compositionstart");
    await nickname.evaluate(element => {
      element.value = "拼音预编辑";
      element.dispatchEvent(new InputEvent("input", { bubbles: true, isComposing: true }));
      element.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, isComposing: true }));
    });
    assert.equal(await nickname.inputValue(), "拼音预编辑");
    await nickname.evaluate(element => {
      element.value = "曦玥珩璟團團𠮷";
      element.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: element.value }));
    });
    await page.getByRole("button", { name: "保存昵称" }).click();
    await page.waitForFunction(async () => {
      const value = await window.__TAURI__.core.invoke("host", {
        command: "snapshot",
        args: {},
      });
      return value.config.name === "曦玥珩璟團團𠮷";
    });
    assert(
      await page.evaluate(() =>
        document.fonts.check('14px "Mochi Sans"', "𠮷"),
      ),
    );
  });
  await check("宠物互动调用原生引擎", async () => {
    const before = (await host(page, "snapshot")).config.feeds;
    await page.locator("nav button").nth(0).click();
    await page.getByRole("button", { name: "投喂一块饼干" }).click();
    assert.equal((await host(page, "snapshot")).config.feeds, before + 1);
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
  await check("系统窗口关闭保存昵称草稿", async () => {
    await page.locator("nav button").nth(1).click();
    await page.locator("#nickname").fill("曦玥團團𠮷");
    const closed = page.waitForEvent("close", { timeout: 10000 });
    await page.evaluate(() =>
      window.__TAURI__.core.invoke("plugin:window|close", { label: "main" }),
    );
    await closed;
    assert.equal((await parent("snapshot")).config.name, "曦玥團團𠮷");
    await browser.close().catch(() => {});
    await parent("open_panel", { page: 0 });
    browser = null;
    const deadline = Date.now() + 30000;
    while (Date.now() < deadline) {
      try {
        browser = await chromium.connectOverCDP("http://127.0.0.1:9921");
        break;
      } catch {
        await new Promise((resolve) => setTimeout(resolve, 350));
      }
    }
    assert(browser, "Panel did not reopen");
    page = browser.contexts()[0].pages()[0];
    await page.getByRole("heading", { name: "今天也有我陪着你" }).waitFor();
  });
  await check("窗口关闭后宠物继续运行", async () => {
    await host(page, "receiving", { id: "fixture-panel-receive", label: "模拟附件进度", bytes: 1 });
    await page.getByRole("button", { name: "收起面板" }).click();
    await page.waitForEvent("close", { timeout: 10000 });
    await new Promise((resolve) => setTimeout(resolve, 1500));
    assert.equal(child.exitCode, null);
    assert(!Object.hasOwn((await parent("snapshot")).content.progress, "fixture-panel-receive"));
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
