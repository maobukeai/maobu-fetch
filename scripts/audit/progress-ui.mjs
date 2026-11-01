import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import puppeteer from "puppeteer-core";
import { createAuditServer } from "./ui-server.mjs";
import { progressUiFixture } from "./progress-ui-fixture.mjs";
import { checkCompact, checkSummaryActivation, checkControls, checkRevealRaces, checkRecovery, checkSettings, checkMatrix, checkEntryScope } from "./progress-ui-checks.mjs";
import { checkCompletionLifetime, checkCompletionActions, checkCompletionBatches } from "./progress-completion-checks.mjs";
import { checkCompletionMatrix } from "./progress-completion-matrix.mjs";
import { checkCompletionMountRecovery } from "./progress-completion-recovery.mjs";
import { checkCompletionPolish } from "./progress-completion-polish.mjs";

const output = await mkdtemp(path.join(tmpdir(), "maobu-compact-ui-"));
const server = await createAuditServer(output);
server.middlewares.use(async (req, res, next) => {
  if (!req.url?.startsWith("/__progress-audit.html") || req.url.includes("html-proxy")) return next();
  res.setHeader("Content-Type", "text/html; charset=utf-8");
  res.end(await server.transformIndexHtml("/__progress-audit.html", progressUiFixture));
});
server.middlewares.stack.unshift(server.middlewares.stack.pop());
let browser;
const results = [], errors = [];
const record = name => { results.push({ name, passed: true }); console.log("PASS", name); };
const recoveryOnly = process.argv.includes('--completion-recovery-only');
try {
  await server.listen();
  const executablePath = ["C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe", "C:/Program Files/Google/Chrome/Application/chrome.exe"].find(existsSync);
  assert.ok(executablePath, "Use an existing browser without installing software");
  browser = await puppeteer.launch({ executablePath, headless: true });
  const page = await browser.newPage();
  page.on("pageerror", error => errors.push(error.message));
  await page.setViewport({ width: 1280, height: 720, deviceScaleFactor: 1 });
  const base = `http://127.0.0.1:${server.httpServer.address().port}/__progress-audit.html`;
  const context = { page, base, output, record };
  await checkCompletionMountRecovery(context);
  if (!recoveryOnly) {
  await checkCompact(context);
  await checkSummaryActivation(context);
  await checkControls(context);
  await checkRevealRaces(context);
  await checkRecovery(context);
  await checkCompletionLifetime(context);
  await checkCompletionActions(context);
  await checkCompletionBatches(context);
  await checkSettings(context);
  await checkMatrix(context);
  await checkCompletionMatrix(context);
  await checkCompletionPolish(context);
  await checkEntryScope(context);
  }
  assert.deepEqual(errors, []);
} finally {
  await writeFile(path.join(output, "results.json"), JSON.stringify({
    scope: "Actual React in an isolated browser; native HWND sizing and visibility are simulated with 250ms delay. Completion expiry uses an explicit virtual 180000ms backend model, not real native time. Rust tests validate the real retention state. Native transparency/topmost/focus/file opening remain unverified here.",
    results, errors,
  }, null, 2));
  await browser?.close();
  await server.close();
  console.log("Evidence:", output);
}
