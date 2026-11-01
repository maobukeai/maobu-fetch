import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import puppeteer from "puppeteer-core";

const executablePath = [process.env.PUPPETEER_EXECUTABLE_PATH,
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
  "C:/Program Files/Google/Chrome/Application/chrome.exe", "/usr/bin/chromium", "/usr/bin/google-chrome",
].find((path) => path && existsSync(path));
const sources = ["douyin-media.js", "content.js"].map((name) => readFileSync(new URL(name, import.meta.url), "utf8"));
const browserTest = (name, run) => test(name, { skip: executablePath ? false : "未找到本地 Chromium，需设置 PUPPETEER_EXECUTABLE_PATH" }, run);
let browser;
before(async () => {
  if (executablePath) browser = await puppeteer.launch({ executablePath, headless: true, args: ["--no-first-run", "--no-default-browser-check"] });
});
after(async () => { await browser?.close(); });

async function fixture(t, { url = "https://www.douyin.com/video/7702744149895712063", html = "", hidden = [] } = {}) {
  const page = await browser.newPage();
  t.after(() => page.close());
  await page.setViewport({ width: 1280, height: 720 });
  await page.setRequestInterception(true);
  page.on("request", (request) => request.respond({ status: 200, contentType: "text/html", body: `<!doctype html><title>测试作品 - 抖音</title>${html}` }));
  await page.goto(url);
  await page.evaluate((hidden) => {
    window.__messages = []; window.__listeners = []; window.__badges = [];
    window.MaobuUi = { showBadge: (...args) => window.__badges.push(args), showFabMenu: (_fab, _items, callback) => { window.__menuCallback = callback; } };
    window.chrome = {
      runtime: { id: "fixture", lastError: null, onMessage: { addListener: (callback) => window.__listeners.push(callback) },
        sendMessage: (payload, callback) => { window.__messages.push(payload); callback?.({ ok: true }); } },
      storage: { local: { get: async () => ({ fabHiddenHosts: hidden }), set: async () => {} }, onChanged: { addListener: (callback) => { window.__storageListener = callback; } } },
    };
  }, hidden);
  for (const source of sources) await page.addScriptTag({ content: source });
  return page;
}

const active = (page) => page.evaluate(() => window.MaobuDouyinMedia.extract());
const messages = (page) => page.evaluate(() => window.__messages.filter((message) => ["send", "download-page-media"].includes(message.type)));

browserTest("Douyin: 作品页尚未加载播放器时也显示入口并传递具体作品 URL", async (t) => {
  const page = await fixture(t);
  assert.equal((await active(page)).url, "https://www.douyin.com/video/7702744149895712063");
  await page.click("#maobu-fetch-media-fab");
  await page.waitForFunction(() => window.__messages.some((message) => message.type === "download-page-media"));
  assert.deepEqual((await messages(page)).map(({ type, url }) => ({ type, url })), [{ type: "download-page-media", url: "https://www.douyin.com/video/7702744149895712063" }]);
});

browserTest("Douyin: 可见激活卡片优先于不可见视频和全页推荐链接", async (t) => {
  const page = await fixture(t, { url: "https://www.douyin.com/", html: '<a href="/video/7000000000000000001">推荐视频</a><video style="display:none" src="https://example.test/old.mp4"></video><div data-e2e="feed-active-video" data-aweme-id="7702744149895712063"><video style="width:300px;height:300px" src="blob:https://www.douyin.com/current"></video></div>' });
  assert.equal((await active(page)).awemeId, "7702744149895712063");
  assert.equal((await active(page)).isDirect, false);
});

browserTest("Douyin: 推荐流未知作品保留提示入口，不下载旧嗅探流或推荐链接", async (t) => {
  const page = await fixture(t, { url: "https://www.douyin.com/", html: '<a href="/video/7000000000000000001">推荐视频</a><video style="width:300px;height:300px" src="blob:https://www.douyin.com/current"></video>' });
  await page.evaluate(() => window.__listeners.forEach((callback) => callback({ type: "sniffed-media", items: [{ kind: "video", url: "https://old.douyinvod.com/old.mp4" }] }, {}, () => {})));
  assert.equal((await active(page)).unavailable, true);
  await page.click("#maobu-fetch-media-fab");
  await page.waitForFunction(() => window.__badges.some(([, text]) => text.includes("作品详情页")));
  assert.deepEqual(await messages(page), []);
});

browserTest("Douyin: SPA 点击时重新绑定当前作品，右键 32 连接下载不发生作用域错误", async (t) => {
  const page = await fixture(t, { html: '<div data-e2e="feed-active-video" data-aweme-id="7702744149895712063"><video style="width:300px;height:300px" src="https://media.example.test/first.mp4"></video></div>' });
  await page.evaluate(() => {
    document.querySelector("[data-aweme-id]").setAttribute("data-aweme-id", "7702744149895712064");
    document.querySelector("video").src = "https://media.example.test/second.mp4";
    history.pushState({}, "", "/video/7702744149895712064");
    document.getElementById("maobu-fetch-media-fab").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    window.__menuCallback("send-32");
  });
  const [sent] = await messages(page);
  assert.equal(sent.url, "https://media.example.test/second.mp4");
  assert.equal(sent.extra.connection_count, 32);
  assert.match(sent.fileName, /7702744149895712064\.mp4$/);
});

browserTest("Douyin: 图文作品有播放器时仍解析整篇图文", async (t) => {
  const page = await fixture(t, { url: "https://www.douyin.com/note/7702744149895712063", html: '<div data-aweme-id="7702744149895712063"><video style="width:300px;height:300px" src="https://media.example.test/music.mp4"></video></div>' });
  const target = await active(page);
  assert.equal(target.type, "gallery"); assert.equal(target.isDirect, false);
  assert.equal(target.url, "https://www.douyin.com/note/7702744149895712063");
});

browserTest("Douyin: 播放器外层作者或频道 data-id 不能覆盖当前作品", async (t) => {
  const page = await fixture(t, { html: '<div data-id="7000000000000000001"><div data-e2e="video-player"><video style="width:300px;height:300px" src="blob:https://www.douyin.com/current"></video></div></div>' });
  assert.equal((await active(page)).awemeId, "7702744149895712063");
});

browserTest("Douyin: 播放列表和分片不能作为完整视频直链发送", async (t) => {
  const page = await fixture(t, { html: '<video style="width:300px;height:300px" src="https://media.example.test/list.m3u8"></video>' });
  assert.equal((await active(page)).isDirect, false);
});

browserTest("Douyin: 状态反馈结束恢复可见底色，媒体刷新保留当前反馈", async (t) => {
  const page = await fixture(t);
  await page.evaluate(() => {
    const original = window.setTimeout;
    window.setTimeout = (callback, duration, ...args) => original(callback, duration === 3500 ? 150 : duration, ...args);
    document.getElementById("maobu-fetch-media-fab").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
  });
  await page.waitForFunction(() => typeof window.__menuCallback === "function");
  await page.evaluate(() => window.__menuCallback("send-current"));
  await page.waitForFunction(() => document.getElementById("maobu-fetch-media-fab").textContent.startsWith("✓"));
  await page.evaluate(() => window.__listeners.forEach((callback) => callback({ type: "sniffed-media", items: [] }, {}, () => {})));
  assert.match(await page.$eval("#maobu-fetch-media-fab", (element) => element.textContent), /^✓/);
  await page.waitForFunction(() => document.getElementById("maobu-fetch-media-fab").textContent === "⬇ 猫步下载");
  assert.deepEqual(await page.$eval("#maobu-fetch-media-fab", (element) => ({ background: element.style.background, color: element.style.color })), { background: "rgba(29, 29, 31, 0.88)", color: "rgb(245, 245, 247)" });
});

browserTest("Douyin: 尊重站点隐藏设置，恢复与重复注入不重复创建入口", async (t) => {
  const page = await fixture(t, { hidden: ["douyin.com"] });
  assert.equal(await page.$("#maobu-fetch-media-fab"), null);
  await page.evaluate(() => window.__storageListener({ fabHiddenHosts: { newValue: [] } }, "local"));
  for (const source of sources) await page.addScriptTag({ content: source });
  assert.equal(await page.$$eval("#maobu-fetch-media-fab", (elements) => elements.length), 1);
  assert.equal(await page.evaluate(() => window.__listeners.length), 1);
});

browserTest("Douyin: 拒绝相似域名与外站推荐链接", async (t) => {
  const page = await fixture(t);
  assert.deepEqual(await page.evaluate(() => [window.MaobuDouyinMedia.isHost("www.douyin.com.evil.test"), window.MaobuDouyinMedia.parseWorkUrl("https://evil.test/video/7702744149895712063")]), [false, null]);
});
