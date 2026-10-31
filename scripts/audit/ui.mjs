import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import puppeteer from "puppeteer-core";
import { createAuditServer } from "./ui-server.mjs";
import { verifyCompletionTimers } from "./completion-timers.mjs";

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
  // Independent temporary browser windows keep each timer active while the matrix runs.
  const extendedTimers=matrixOnly?Promise.resolve(null):verifyCompletionTimers(base,executablePath,record).then(()=>null,error=>error);
  browser = await puppeteer.launch({ executablePath, headless: true });
  const page = await browser.newPage();
  page.setDefaultTimeout(30000);
  const errors = [];
  page.on("pageerror", error => { errors.push(error.message); console.error("Page error:", error.message); });
  page.on("console", message => { if (message.type() === "error") console.error("Browser:", message.text()); });
  page.on("requestfailed", request => console.error("Request failed:", request.url(), request.failure()?.errorText));
  if(!matrixOnly) {
  await page.goto(base + "?mode=notice&duration=30");
  await page.waitForFunction(() => Boolean(window.audit?.emit));
  await page.evaluate(() => { window.audit.emit(); window.audit.emit("audit-1"); });
  await page.waitForSelector(".completion-toast");
  await delay(4000);
  assert.ok(await page.$(".completion-toast"), "超过旧通用提示的 3.5 秒后仍保留");
  await page.hover(".completion-toast");
  await delay(31000);
  assert.ok(await page.$(".completion-toast"), "悬停期间不消失");
  record("30 秒提醒与悬停暂停", "实际等待 35 秒");
  await page.evaluate(() => document.querySelector('.completion-toast .toast-close-btn').click());
  await page.waitForFunction(() => !document.querySelector('.completion-toast')?.textContent.includes('另有'));
  assert.ok(await page.$(".completion-toast"), "关闭后出现排队的第二条");
  await page.evaluate(() => document.querySelector('.completion-toast .toast-action-btn').click());
  await page.waitForFunction(() => !document.querySelector('.completion-toast'));
  assert.deepEqual(await page.evaluate(() => window.audit.calls[0]), ["openFile", "audit-1"]);
  record("排队和打开文件快捷操作");

  await page.goto(base + "?mode=notice&duration=0");
  await page.waitForFunction(() => Boolean(window.audit?.emit));
  await page.evaluate(() => { window.audit.failOpen = true; window.audit.emit(); });
  await page.waitForSelector(".completion-toast");
  await page.click(".completion-toast .toast-action-btn");
  await page.waitForFunction(() => window.audit.calls.some(call => call[0] === "error"));
  assert.ok(await page.$(".completion-toast"), "打开失败保留提醒供再次操作");
  await page.click(".completion-toast .toast-actions button:nth-child(2)");
  await page.waitForFunction(() => !document.querySelector('.completion-toast'));
  assert.deepEqual(await page.evaluate(() => window.audit.calls.at(-1)), ["openFolder", "audit-0"]);
  record("手动关闭模式、打开失败反馈与打开文件夹");

  await page.goto(base + "?mode=notice&duration=15");
  await page.waitForFunction(() => Boolean(window.audit?.emit));
  await page.evaluate(() => window.audit.emit());
  await page.waitForSelector(".completion-toast");
  await page.mouse.move(0, 0);
  await page.waitForSelector(".completion-toast", { hidden: true, timeout: 20000 });
  record("15 秒自动关闭", "真实浏览器计时");
  }

  for (const [width, height] of [[1280, 720], [1440, 900], [780, 540]]) {
    for (const scale of [1, 1.25]) {
      for (const theme of ["light", "dark"]) {
        await page.setViewport({ width, height, deviceScaleFactor: scale });
        await page.emulateMediaFeatures([{ name: "prefers-color-scheme", value: theme }]);
        await page.goto(base + "?count=500");
        await page.waitForSelector(".task-row");
        await page.waitForSelector("#splash-screen", { hidden: true });
        await page.evaluate(scaleValue => { document.documentElement.style.zoom = String(scaleValue); window.audit.emit(); }, scale);
        await page.waitForSelector(".completion-toast");
        const bounds = await page.$eval(".completion-toast", el => {
          const rect = el.getBoundingClientRect();
          return { x: rect.x, y: rect.y, right: rect.right, bottom: rect.bottom, width: innerWidth, height: innerHeight };
        });
        assert.ok(bounds.x >= 0 && bounds.y >= 0 && bounds.right <= bounds.width + 1 && bounds.bottom <= bounds.height + 1,
          "完成提醒必须在视口内 " + JSON.stringify(bounds));
        assert.ok(await page.$eval(".completion-toast", el => {
          const r = el.getBoundingClientRect();
          return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
        }), "完成提醒必须可见且没有被启动遮罩覆盖");
        const name = width + "x" + height + "-" + scale + "-" + theme;
        await page.screenshot({ path: path.join(output, name + ".png") });
        record(name, "500 条测试任务、长文件名、完成提醒；CSS缩放及设备缩放");
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
  await page.type(".search-box input", "不存在的搜索内容");
  await page.evaluate(() => window.audit.emit());
  await page.waitForSelector(".completion-toast");
  await page.click(".completion-toast .toast-actions button:nth-child(3)");
  await page.waitForSelector(".details-pane");
  assert.equal(await page.$eval(".search-box input", input => input.value), "");
  assert.equal(await page.$eval('.task-row[data-task-id="audit-0"]', row => row.classList.contains("selected")), true);
  record("完成提醒定位任务、展开详情并清除搜索过滤");
  await page.click(".nav-settings");
  await page.waitForSelector('select[aria-label="完成提醒停留时间"]');
  await page.select('select[aria-label="完成提醒停留时间"]', "0");
  await page.click(".settings-actions button.primary");
  await page.waitForFunction(() => window.audit.saved?.completion_notice_seconds === 0);
  record("完成提醒设置保存（API替身）", "真实数据库持久化由Rust serde/Store测试另验");
  await page.goto(base);
  await page.waitForSelector("#splash-screen", { hidden: true });
  await page.waitForFunction(() => document.body.textContent.includes("暂无") || document.body.textContent.includes("还没有"));
  record("空任务页面");
  const timerError=await extendedTimers;
  if(timerError)throw timerError;
  assert.deepEqual(errors, [], "浏览器不应产生未捕获异常");
  console.log("Output:", output);
} finally {
  await writeFile(path.join(output, "results.json"), JSON.stringify({ date: new Date().toISOString(), results }, null, 2));
  if (browser) await browser.close();
  await server.close();
}
