import path from "node:path";
import fs from "node:fs/promises";
import http from "node:http";
import { createHash, createHmac } from "node:crypto";
import puppeteer from "puppeteer-core";
import { lanDiskPageHtml, probeLanDisk } from "./extension-landisk-browser.mjs";
import { fileURLToPath } from "node:url";

const SYNTHETIC_TOKEN = "maobu-local-audit-synthetic-token";
const BROWSERS = [
  ["Edge", "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"],
  ["Chrome", "C:/Program Files/Google/Chrome/Application/chrome.exe"],
];

function echoServer() {
  return http.createServer(async (req, res) => {
    if (req.url === "/landisk") {
      res.setHeader("Content-Type", "text/html; charset=utf-8");
      res.end(lanDiskPageHtml);
      return;
    }
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const body = Buffer.concat(chunks).toString();
    const timestamp = req.headers["x-luma-timestamp"];
    const key = createHash("sha256").update(SYNTHETIC_TOKEN).digest();
    const expected = timestamp ? createHmac("sha256", key).update(`${timestamp}\n${body}`).digest("hex") : null;
    res.setHeader("Content-Type", "application/json");
    res.end(JSON.stringify({ method: req.method, extensionOriginPresent: /^chrome-extension:\/\//.test(req.headers.origin || ""), signatureValid: expected !== null && expected === req.headers["x-luma-signature"] }));
  });
}

async function inspectExtensionPage({ echo, source, token }) {
  const tabs = await chrome.tabs.query({});
  const { getDownloadAuthHeaders } = await import("./interceptor.js");
  const { signedFetch, signedGet } = await import("./protocol.js");
  const normalTab = tabs.find((tab) => tab.url === source && !tab.incognito);
  const privateTab = tabs.find((tab) => tab.url === source && tab.incognito);
  const normal = await getDownloadAuthHeaders({ url: echo }, { tab: normalTab, url: echo });
  const priv = privateTab ? await getDownloadAuthHeaders({ url: source }, { tab: privateTab, url: source }) : {};
  await chrome.storage.local.set({ bridgeToken: token });
  let protocol;
  try {
    protocol = [await (await signedGet(echo)).json(), await (await signedFetch(echo, { audit: "synthetic" })).json()];
  } finally {
    await chrome.storage.local.remove("bridgeToken");
  }
  return {
    protocol,
    normalTabFound: Boolean(normalTab), privateTabFound: Boolean(privateTab),
    privateTabHasCookieStoreId: Boolean(privateTab && Object.hasOwn(privateTab, "cookieStoreId")),
    pageCookieForwardedToOtherHost: normal.Cookie === "audit_session=NORMAL_SYNTHETIC",
    privateHeaderUsesNormalStore: priv.Cookie === "audit_session=NORMAL_SYNTHETIC",
    privateHeaderUsesPrivateStore: priv.Cookie === "audit_session=PRIVATE_SYNTHETIC",
  };
}

async function probeBrowser(name, executablePath, root, auditDir, echo) {
  let browser;
  try {
    await fs.access(executablePath);
    browser = await puppeteer.launch({
      executablePath, headless: true, pipe: true, enableExtensions: true,
      userDataDir: path.join(auditDir, `profile-${name}`),
      args: ["--enable-unsafe-extension-debugging", "--disable-background-networking", "--no-first-run"],
    });
    const extensionId = await browser.installExtension(fileURLToPath(new URL("extension/", root)), { enabledInIncognito: true });
    await browser.waitForTarget((target) => target.type() === "service_worker" && target.url().includes(extensionId), { timeout: 15_000 });
    const extensionPage = await browser.newPage();
    // Options has no automatic native-bridge health/recent-task requests.
    await extensionPage.goto(`chrome-extension://${extensionId}/src/options.html`);
    const source = echo.replace("127.0.0.1", "localhost");
    await browser.defaultBrowserContext().setCookie({ name: "audit_session", value: "NORMAL_SYNTHETIC", url: source });
    const normalPage = await browser.newPage();
    await normalPage.goto(source);
    // Chromium 扩展只能读取标准主无痕 store，CDP 自建的额外 OTR context 不是用户无痕窗口。
    await extensionPage.evaluate(async (url) => {
      const window = await chrome.windows.create({ incognito: true, url });
      const tabId = window.tabs[0].id;
      const stores = await chrome.cookies.getAllCookieStores();
      const store = stores.find((item) => item.tabIds.includes(tabId));
      if (!store) throw new Error("Native private window store was not exposed");
      await chrome.cookies.set({ name: "audit_session", value: "PRIVATE_SYNTHETIC", url, storeId: store.id });
    }, source);
    await extensionPage.waitForFunction(async (url) => (await chrome.tabs.query({})).some((tab) => tab.url === url && tab.incognito), {}, source);
    const observations = await extensionPage.evaluate(inspectExtensionPage, { echo, source, token: SYNTHETIC_TOKEN });
    observations.lanDiskApproval = await probeLanDisk(browser, extensionId, echo, SYNTHETIC_TOKEN, auditDir, name);
    return { name, status: "executed", browser: await browser.version(), observations };
  } catch (error) {
    return { name, status: "unverified", error: String(error.message) };
  } finally {
    if (browser) await browser.close();
  }
}

export async function runBrowserProbes(root, auditDir) {
  const server = echoServer();
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  try {
    const echo = `http://127.0.0.1:${server.address().port}/echo`;
    const results = [];
    for (const [name, executablePath] of BROWSERS) results.push(await probeBrowser(name, executablePath, root, auditDir, echo));
    return results;
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
}
