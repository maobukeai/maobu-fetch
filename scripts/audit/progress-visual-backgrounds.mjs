import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import puppeteer from "puppeteer-core";
import { createAuditServer } from "./ui-server.mjs";
import { progressUiFixture } from "./progress-ui-fixture.mjs";

const cases = [
  { key: "light-on-light", backdrop: "light", theme: "light", scale: 1 },
  { key: "dark-on-light", backdrop: "light", theme: "dark", scale: 1.25 },
  { key: "dark-on-dark", backdrop: "dark", theme: "dark", scale: 1 },
  { key: "light-on-dark", backdrop: "dark", theme: "light", scale: 1.25 },
  { key: "document-light", backdrop: "document", theme: "light", scale: 1 },
  { key: "document-dark", backdrop: "document", theme: "dark", scale: 1.25, expanded: true },
];

async function setBackdrop(page, kind) {
  await page.evaluate(backdrop => {
    const host = document.getElementById("audit-backdrop");
    host.replaceChildren();
    host.style.background = backdrop === "dark" ? "#20242a" : "#f2f3f5";
    document.getElementById("audit-external-focus").hidden = true;
    if (backdrop !== "document") return;
    const article = document.createElement("article");
    Object.assign(article.style, {
      position: "absolute", inset: "16px", padding: "16px 20px", overflow: "hidden",
      background: "#fff", color: "#424a54", border: "1px solid #d9dde3",
      font: '12px "Segoe UI", sans-serif', lineHeight: "1.4",
    });
    const title = document.createElement("h2");
    title.textContent = "项目资料";
    Object.assign(title.style, { margin: "0 0 12px", fontSize: "15px", fontWeight: "600" });
    article.append(title);
    const names = ["调研记录", "安装日志", "任务说明", "备份索引", "接口文档", "测试清单"];
    for (let index = 0; index < 15; index++) {
      const row = document.createElement("div");
      Object.assign(row.style, {
        display: "flex", justifyContent: "space-between", height: "25px", alignItems: "center",
        borderBottom: "1px solid #eceef2", fontVariantNumeric: "tabular-nums",
      });
      const name = document.createElement("span");
      name.textContent = `${names[index % names.length]} ${String(index + 1).padStart(2, "0")}`;
      const info = document.createElement("span");
      info.textContent = `${24 + index * 7} KB　　已保存`;
      info.style.color = "#717984";
      row.append(name, info);
      article.append(row);
    }
    host.append(article);
  }, kind);
}

async function measure(page) {
  return page.$eval(".download-progress-window", element => {
    const rectangle = element.getBoundingClientRect();
    const strip = element.querySelector(".progress-window-summary").getBoundingClientRect();
    const controls = [...element.querySelectorAll("button")].map(button => button.getBoundingClientRect());
    return {
      width: rectangle.width, height: rectangle.height, stripY: strip.y, stripHeight: strip.height,
      buttons: controls.length, rows: element.querySelectorAll(".progress-window-task").length,
      revealed: element.dataset.revealed === "true",
      detailsMounted: Boolean(element.querySelector("#progress-window-details")),
      controlsFit: controls.every(button => button.width > 0 && button.left >= rectangle.left
        && button.right <= rectangle.right + 1 && button.top >= rectangle.top - 1
        && button.bottom <= rectangle.bottom + 1),
      inside: rectangle.x >= 0 && rectangle.y >= 0 && rectangle.right <= innerWidth && rectangle.bottom <= innerHeight,
      transparentDocument: [document.documentElement, document.body, document.getElementById("root")]
        .every(node => getComputedStyle(node).backgroundColor === "rgba(0, 0, 0, 0)"),
    };
  });
}

async function checkCase(page, base, output, scenario) {
  await page.setViewport({ width: 640, height: 400, deviceScaleFactor: scenario.scale });
  await page.emulateMediaFeatures([{ name: "prefers-color-scheme", value: scenario.theme }]);
  await page.mouse.move(0, 0);
  await page.goto(`${base}?count=8&known=1&theme=${scenario.theme}`);
  await page.waitForFunction(() => window.audit?.initialCalled
    && document.querySelector(".progress-window-summary")?.getAttribute("aria-label").includes("8"));
  await setBackdrop(page, scenario.backdrop);
  const compact = await measure(page);
  assert.deepEqual([compact.width, compact.height, compact.stripHeight], [224, 28, 28]);
  assert.ok(compact.transparentDocument && compact.inside && compact.controlsFit);
  assert.equal(compact.buttons, 1);
  assert.equal(compact.detailsMounted, false);
  const compactImage = `${scenario.key}-${scenario.scale}-compact.png`;
  await page.screenshot({ path: path.join(output, compactImage) });
  await page.hover(".progress-window-summary");
  await page.waitForFunction(() => document.querySelector(".download-progress-window")?.dataset.revealed === "true"
    && document.getElementById("root").getBoundingClientRect().height === 190);
  if (scenario.expanded) {
    await page.click(".progress-window-fold");
    await page.waitForFunction(() => document.getElementById("root").getBoundingClientRect().height === 298);
  }
  const details = await measure(page);
  assert.deepEqual([details.width, details.height, details.rows], [224, scenario.expanded ? 298 : 190, scenario.expanded ? 6 : 3]);
  assert.equal(details.stripY, compact.stripY);
  assert.ok(details.transparentDocument && details.inside && details.controlsFit);
  const detailsImage = `${scenario.key}-${scenario.scale}-details.png`;
  await page.screenshot({ path: path.join(output, detailsImage) });
  return { ...scenario, passed: true, compact, details, images: [compactImage, detailsImage] };
}

const output = await mkdtemp(path.join(tmpdir(), "maobu-progress-backgrounds-"));
const profile = path.join(output, "browser-profile");
await mkdir(profile);
const server = await createAuditServer(output);
server.middlewares.use(async (request, response, next) => {
  if (!request.url?.startsWith("/__progress-backgrounds.html") || request.url.includes("html-proxy")) return next();
  response.setHeader("Content-Type", "text/html; charset=utf-8");
  response.end(await server.transformIndexHtml("/__progress-backgrounds.html", progressUiFixture));
});
server.middlewares.stack.unshift(server.middlewares.stack.pop());
const results = [], errors = [], blockedHosts = [];
let browser;
try {
  await server.listen();
  const executablePath = ["C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe", "C:/Program Files/Google/Chrome/Application/chrome.exe"].find(existsSync);
  assert.ok(executablePath, "An existing Chrome/Edge is required; no browser is downloaded");
  browser = await puppeteer.launch({ executablePath, headless: true, userDataDir: profile });
  const page = await browser.newPage();
  page.setDefaultTimeout(15000);
  page.on("pageerror", error => errors.push(error.message));
  await page.setRequestInterception(true);
  page.on("request", request => {
    const url = new URL(request.url());
    if (["http:", "https:"].includes(url.protocol) && url.hostname !== "127.0.0.1") {
      blockedHosts.push(url.hostname);
      void request.abort();
    } else void request.continue();
  });
  const base = `http://127.0.0.1:${server.httpServer.address().port}/__progress-backgrounds.html`;
  for (const scenario of cases) {
    results.push(await checkCase(page, base, output, scenario));
    console.log("PASS", scenario.key, scenario.scale);
  }
  assert.deepEqual(errors, []);
  assert.deepEqual(blockedHosts, []);
} finally {
  await writeFile(path.join(output, "results.json"), JSON.stringify({
    scope: "Production React progress UI over isolated synthetic host backgrounds. API and delayed HWND resizing use the existing loopback fixture; native transparency, topmost and focus are not proved. A new headless browser profile is used; external HTTP(S) requests are blocked.",
    results, errors, blockedHosts,
  }, null, 2));
  await browser?.close();
  await server.close();
  console.log("Evidence:", output);
}
