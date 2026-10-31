import assert from "node:assert/strict";
import { setTimeout as delay } from "node:timers/promises";
import path from "node:path";

export const lanDiskPageHtml = `<!doctype html><html><head><title>Synthetic LanDisk</title></head><body>
<div id="apple-floating-batch-bar"><button id="btn-batch-zip">下载打包文件</button></div>
<script>
window.__nativeFallback = 0;
window.FileBatchManager = class { constructor(){this.selectedFiles=new Set(['report.txt']);} getPin(){return '';} getApiUrl(url){return url;} downloadZip(){window.__nativeFallback++;} };
window.fileBatchManager = new FileBatchManager();
document.getElementById('btn-batch-zip').onclick = () => window.fileBatchManager.downloadZip();
</script></body></html>`;

async function approvalPage(browser, extensionId) {
  const target = await browser.waitForTarget((item) => item.type() === "page"
    && item.url().startsWith(`chrome-extension://${extensionId}/src/landisk-approval.html#`), { timeout: 10000 });
  const page = await target.page();
  await page.waitForFunction(() => !document.getElementById("accept").disabled);
  return page;
}

/** 浏览器真实确认流程；仅桥接 fetch 为替身，绝不访问正在运行的 17433。 */
export async function probeLanDisk(browser, extensionId, echo, token, outputDirectory, browserName) {
  const workerTarget = await browser.waitForTarget((item) => item.type() === "service_worker" && item.url().includes(extensionId));
  const worker = await workerTarget.worker();
  await worker.evaluate(async (syntheticToken) => {
    globalThis.__maobuAuditTasks = [];
    globalThis.__maobuAuditOriginalFetch = globalThis.fetch;
    globalThis.fetch = async (url, options = {}) => {
      if (!String(url).startsWith("http://127.0.0.1:17433/")) return globalThis.__maobuAuditOriginalFetch(url, options);
      if (String(url).endsWith("/v1/health")) return new Response(JSON.stringify({ takeover_enabled: true, min_file_size_mb: 0 }), { status: 200 });
      if (String(url).endsWith("/v1/tasks")) {
        globalThis.__maobuAuditTasks.push(JSON.parse(options.body));
        return new Response(JSON.stringify({ id: "synthetic-lan-task" }), { status: 201 });
      }
      return new Response("{}", { status: 200 });
    };
    await chrome.storage.local.set({ bridgeToken: syntheticToken, intercept: true, bypassUntil: 0 });
  }, token);
  const page = await browser.newPage();
  const source = new URL("/landisk", echo).href;
  const apiUrl = new URL("/api/download/batch", echo).href;
  await browser.defaultBrowserContext().setCookie({ name: "audit_lan_session", value: "LAN_SYNTHETIC", url: source });
  try {
    await page.goto(source);
    await delay(1200); // 原网页的动态 FileBatchManager 安装由现有钩子识别。
    await page.evaluate((url) => window.postMessage({
      source: "maobu-landisk-main", type: "SEND_LANDISK_BATCH_TASK", reqId: "forged-page",
      payload: { url, fileName: "很长的打包文件名".repeat(18) + ".zip",
        body: new URLSearchParams([["folderName", "Documents"], ...Array.from({ length: 5 }, (_, i) => ["files", "较长的文件路径/".repeat(8) + i + ".txt"])]).toString(),
        headers: { Authorization: "UNTRUSTED" } },
    }, location.origin), apiUrl);
    const declined = await approvalPage(browser, extensionId);
    for (const theme of ["light", "dark"]) {
      for (const scale of [1, 1.25]) {
        await declined.setViewport({ width: 440, height: 300 });
        await declined.emulateMediaFeatures([{ name: "prefers-color-scheme", value: theme }]);
        await declined.evaluate((value) => { document.documentElement.style.zoom = String(value); }, scale);
        assert.ok(await declined.$$eval("button", (buttons) => buttons.every((button) => {
          const rect = button.getBoundingClientRect();
          return rect.x >= 0 && rect.y >= 0 && rect.bottom <= innerHeight + 1 && rect.right <= innerWidth + 1;
        })), "approval controls must remain inside small popup viewport");
        await declined.screenshot({ path: path.join(outputDirectory, `${browserName}-landisk-${theme}-${scale}.png`) });
      }
    }
    assert.equal(await worker.evaluate(() => globalThis.__maobuAuditTasks.length), 0);
    await declined.evaluate(() => document.getElementById("accept").click());
    await delay(150);
    assert.equal(await worker.evaluate(() => globalThis.__maobuAuditTasks.length), 0, "synthetic clicks must not approve");
    await declined.click("#decline");
    await delay(150);

    await page.click("#btn-batch-zip");
    const accepted = await approvalPage(browser, extensionId);
    await accepted.click("#accept");
    await worker.evaluate(async () => {
      const deadline = Date.now() + 3000;
      while (!globalThis.__maobuAuditTasks.length && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 50));
    });
    const tasks = await worker.evaluate(() => globalThis.__maobuAuditTasks);
    assert.equal(tasks.length, 1);
    assert.equal(tasks[0].url, apiUrl);
    assert.equal(tasks[0].method, "POST");
    assert.match(tasks[0].body, /files=report.txt/);
    assert.match(tasks[0].headers.Cookie, /audit_lan_session=LAN_SYNTHETIC/);
    assert.equal(tasks[0].headers.Authorization, undefined);
    assert.equal(await page.evaluate(() => window.__nativeFallback), 0);
    return { confirmedPopupOrigin: true, pageMessageCannotAutoDownload: true, syntheticClickRejected: true, nativeBatchApproved: true, boundCookieContext: true, popupVisualChecks: true };
  } finally {
    await page.close();
    await worker.evaluate(async () => {
      globalThis.fetch = globalThis.__maobuAuditOriginalFetch;
      await chrome.storage.local.remove("bridgeToken");
    });
  }
}
