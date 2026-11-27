import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import puppeteer from "puppeteer-core";
import { createAuditServer } from "./ui-server.mjs";
import { assertNoInternalCompletion, checkCompletionEvents, checkSystemNotificationFocus } from "./notification-ui-checks.mjs";

const output = process.env.MAOBU_AUDIT_OUTPUT || await mkdtemp(path.join(tmpdir(), "maobu-ui-audit-"));
await mkdir(output, { recursive: true });
const executablePath = [
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
  "C:/Program Files/Google/Chrome/Application/chrome.exe",
].find(existsSync);
assert.ok(executablePath, "需要已安装的 Chrome 或 Edge；测试不下载浏览器");
const server = await createAuditServer(output);
let browser;
const results = [];
const matrixOnly=process.argv.includes('--matrix-only');
const record = (name, detail) => { results.push({ name, detail, passed: true }); console.log("PASS", name); };
try {
  await server.listen();
  const address = server.httpServer.address();
  const base = "http://127.0.0.1:" + address.port + "/__audit.html";
  browser = await puppeteer.launch({ executablePath, headless: true });
  const page = await browser.newPage();
  page.setDefaultTimeout(30000);
  const errors = [];
  page.on("pageerror", error => { errors.push(error.message); console.error("Page error:", error.message); });
  page.on("console", message => { if (message.type() === "error") console.error("Browser:", message.text()); });
  page.on("requestfailed", request => console.error("Request failed:", request.url(), request.failure()?.errorText));
  const settings = matrixOnly ? null : await checkCompletionEvents(page, base, record);

  for (const [width, height] of [[1280, 720], [1440, 900], [780, 540]]) {
    for (const scale of [1, 1.25]) {
      for (const theme of ["light", "dark"]) {
        await page.setViewport({ width, height, deviceScaleFactor: scale });
        await page.emulateMediaFeatures([{ name: "prefers-color-scheme", value: theme }]);
        await page.goto(base + "?count=500");
        await page.waitForSelector(".task-row");
        await page.waitForSelector("#splash-screen", { hidden: true });
        await page.evaluate(scaleValue => { document.documentElement.style.zoom = String(scaleValue); window.audit.emit(); }, scale);
        await assertNoInternalCompletion(page);
        const name = width + "x" + height + "-" + scale + "-" + theme;
        await page.screenshot({ path: path.join(output, name + ".png") });
        record(name, "500 条测试任务、长文件名；CSS缩放及设备缩放；完成事件不生成内部卡片");
      }
    }
  }
  await page.setViewport({ width: 1280, height: 720, deviceScaleFactor: 1 });
  await page.goto(base + "?count=500");
  await page.waitForSelector(".task-row");
  await page.waitForSelector("#splash-screen", { hidden: true });
  assert.equal(await page.$$eval(".task-row", rows => rows.length), 500);
  await page.click('.task-row[data-task-id="audit-0"] label');
  await page.click('.task-row[data-task-id="audit-1"] label');
  assert.equal(await page.$$eval(".task-row.selected", rows => rows.length), 2);
  assert.equal(await page.$eval(".table-header input", input => input.indeterminate), true);
  await page.click(".table-header input");
  assert.equal(await page.$$eval(".task-row.selected", rows => rows.length), 500);
  await page.click(".table-header input");
  record("500 条可见任务、多选、半选及全选");
  await page.click(".table-header > span:first-of-type");
  const ascending = await page.$$eval(".task-row", rows => rows.map(row => row.dataset.taskId));
  await page.click(".table-header > span:first-of-type");
  const descending = await page.$$eval(".task-row", rows => rows.map(row => row.dataset.taskId));
  assert.deepEqual(descending, ascending.toReversed());
  record("任务文件名双向排序");
  if (settings) await checkSystemNotificationFocus(page, base, settings, record);
  await page.goto(base);
  await page.waitForSelector("#splash-screen", { hidden: true });
  await page.waitForFunction(() => document.body.textContent.includes("暂无") || document.body.textContent.includes("还没有"));
  record("空任务页面");
  assert.deepEqual(errors, [], "浏览器不应产生未捕获异常");
  console.log("Output:", output);
} finally {
  await writeFile(path.join(output, "results.json"), JSON.stringify({ date: new Date().toISOString(), results }, null, 2));
  if (browser) await browser.close();
  await server.close();
}
