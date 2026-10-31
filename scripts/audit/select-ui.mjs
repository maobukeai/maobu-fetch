import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import puppeteer from "puppeteer-core";
import { createAuditServer } from "./ui-server.mjs";

const output = await mkdtemp(path.join(tmpdir(), "maobu-select-regression-"));
const server = await createAuditServer(output);
let browser;
try {
  await server.listen();
  browser = await puppeteer.launch({ headless: true, executablePath: [
    "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
  ].find(existsSync) });
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", error => errors.push(String(error)));
  await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/__audit.html?mode=select`);
  await page.waitForSelector('[role="combobox"]');
  await page.focus('[role="combobox"]');
  await page.keyboard.press("ArrowDown");
  await page.waitForSelector('[role="listbox"]');
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => window.audit.selected === "third");
  assert.equal(await page.$eval('[role="combobox"]', el => el.getAttribute("aria-expanded")), "false");
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("Home");
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => window.audit.selected === "first");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("End");
  await page.keyboard.press("Escape");
  await page.waitForSelector('[role="listbox"]', { hidden: true });
  assert.equal(await page.evaluate(() => window.audit.selected), "first");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Tab");
  assert.equal(await page.evaluate(() => document.activeElement.id), "after-select");
  await page.waitForSelector('[role="listbox"]', { hidden: true });
  assert.deepEqual(errors, []);
  await writeFile(path.join(output, "results.json"), JSON.stringify({ passed: true,
    cases: ["箭头打开", "跳过禁用选项", "Enter提交", "Home/End导航", "Escape不提交并关闭", "Tab关闭且移动焦点", "listbox/option语义"] }, null, 2));
  console.log("Select keyboard regression passed:", output);
} finally {
  await browser?.close();
  await server.close();
}
