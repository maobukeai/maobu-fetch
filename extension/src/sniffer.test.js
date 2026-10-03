import test from "node:test";
import assert from "node:assert/strict";
import {
  isSniffableMediaUrl, sniffedKind, sniffedName, pushSniffedItem,
  hostMatchesList, pickFabTarget, toggleSniffHost, attachSniffer,
} from "./sniffer.js";

// ---- URL 识别（纯函数）----

test("isSniffableMediaUrl: 匹配媒体扩展名（含查询串），拒绝非媒体与非 http(s)", () => {
  assert.equal(isSniffableMediaUrl("https://v.example.com/hls/index.m3u8?token=1"), true);
  assert.equal(isSniffableMediaUrl("https://v.example.com/video/720.mp4"), true);
  assert.equal(isSniffableMediaUrl("http://v.example.com/podcast.m4a"), true);
  assert.equal(isSniffableMediaUrl("https://example.com/page?fmt=mp4"), false, "查询参数不算扩展名");
  assert.equal(isSniffableMediaUrl("https://example.com/page.html"), false);
  assert.equal(isSniffableMediaUrl("blob:https://example.com/uuid"), false);
  assert.equal(isSniffableMediaUrl(""), false);
});

test("sniffedKind: 区分流/视频/音频", () => {
  assert.equal(sniffedKind("https://v.example.com/index.m3u8"), "stream");
  assert.equal(sniffedKind("https://v.example.com/seg-1.ts"), "stream");
  assert.equal(sniffedKind("https://v.example.com/movie.mp4"), "video");
  assert.equal(sniffedKind("https://v.example.com/podcast.mp3"), "audio");
  assert.equal(sniffedKind("not a url"), "media");
});

test("sniffedName: 取路径最后一段并解码", () => {
  assert.equal(sniffedName("https://v.example.com/a/b/%E8%A7%86%E9%A2%91.mp4"), "视频.mp4");
  assert.equal(sniffedName("https://v.example.com/index.m3u8"), "index.m3u8");
});

// ---- 记录列表维护（纯函数）----

test("pushSniffedItem: 同 URL 去重并移动到末尾，超容量裁剪最旧条目", () => {
  let list = pushSniffedItem([], "https://v.example.com/a.mp4");
  list = pushSniffedItem(list, "https://v.example.com/b.mp4");
  list = pushSniffedItem(list, "https://v.example.com/a.mp4");
  assert.equal(list.length, 2);
  assert.equal(list[1].url, "https://v.example.com/a.mp4", "重复 URL 应更新为最新位置");

  for (let i = 0; i < 40; i += 1) list = pushSniffedItem(list, `https://v.example.com/${i}.ts`);
  assert.equal(list.length, 30, "默认容量 30");
  assert.equal(list[0].url, "https://v.example.com/10.ts", "最旧条目被裁剪");
});

// ---- 站点开关（纯函数）----

test("hostMatchesList: 精确与子域命中，后缀相似不误判", () => {
  assert.equal(hostMatchesList("www.example.com", ["example.com"]), true);
  assert.equal(hostMatchesList("example.com", ["example.com"]), true);
  assert.equal(hostMatchesList("notexample.com", ["example.com"]), false);
  assert.equal(hostMatchesList("", ["example.com"]), false);
});

test("toggleSniffHost: 开启去重追加；关闭移除自身与覆盖它的父域规则", () => {
  assert.deepEqual(toggleSniffHost(["a.com"], "b.com", true), ["a.com", "b.com"]);
  assert.deepEqual(toggleSniffHost(["a.com", "b.com"], "b.com", true), ["a.com", "b.com"]);
  assert.deepEqual(toggleSniffHost(["a.com", "b.com"], "b.com", false), ["a.com"]);
  assert.deepEqual(toggleSniffHost(["b.com"], "www.b.com", false), [], "在子域上关闭应移除覆盖它的父域规则");
});

// ---- FAB 直连目标挑选（纯函数）----

test("pickFabTarget: 只直连 video/audio；stream 类不作为直连目标", () => {
  assert.equal(pickFabTarget([
    { kind: "audio", url: "a.mp3" },
    { kind: "stream", url: "s.m3u8" },
    { kind: "video", url: "v1.mp4" },
    { kind: "video", url: "v2.mp4" },
  ]), "v2.mp4");
  assert.equal(pickFabTarget([{ kind: "audio", url: "a.mp3" }, { kind: "stream", url: "s.m3u8" }]), "a.mp3");
  // 仅有流地址时返回空（调用方退回 page 模式走媒体解析）。
  assert.equal(pickFabTarget([{ kind: "stream", url: "s.m3u8" }, { kind: "stream", url: "seg-1.ts" }]), "");
  assert.equal(pickFabTarget([]), "");
});

// ---- SW 接线（模拟 Chrome API）----

test("attachSniffer: 仅记录已开启站点的媒体 URL 并推送给对应标签页", async () => {
  const listeners = {};
  const sent = [];
  const fakeChrome = {
    webRequest: {
      onBeforeRequest: { addListener: (fn) => { listeners.before = fn; }, removeListener: () => {} },
    },
    tabs: {
      sendMessage: async (tabId, msg) => { sent.push([tabId, msg]); },
      onRemoved: { addListener: () => {} },
    },
    storage: {
      local: { get: async () => ({ snifferHosts: ["video.example"] }) },
      onChanged: { addListener: () => {} },
    },
  };
  const bridge = attachSniffer({ chrome: fakeChrome });
  assert.ok(bridge, "webRequest 可用时应成功接线");
  await new Promise((resolve) => setTimeout(resolve, 10)); // 等待域名表异步加载

  listeners.before({ tabId: 3, url: "https://video.example/hls/index.m3u8?tok=1" });
  listeners.before({ tabId: 3, url: "https://cdn.othercdn.com/seg.m4s", initiator: "https://video.example" }); // 开启站点的跨域 CDN 媒体流
  listeners.before({ tabId: 3, url: "https://other.example/clip.mp4" }); // 未开启嗅探的站点
  listeners.before({ tabId: 3, url: "https://video.example/page.html" }); // 非媒体扩展名
  listeners.before({ tabId: -1, url: "https://video.example/a.mp4" }); // 非页面请求

  assert.equal(sent.length, 2, "开启站点的主机直连或跨域 CDN 媒体 URL 均会推送");
  assert.equal(sent[0][0], 3);
  assert.equal(sent[0][1].type, "sniffed-media");
  assert.equal(sent[0][1].items.at(-1).url, "https://video.example/hls/index.m3u8?tok=1");
  assert.equal(sent[1][1].items.at(-1).url, "https://cdn.othercdn.com/seg.m4s");

  const items = bridge.getItems(3);
  assert.equal(items.length, 2);
  assert.equal(items[0].kind, "stream");
  assert.equal(bridge.isHostEnabled("www.video.example"), true);
  assert.equal(bridge.isHostEnabled("other.example"), false);
  bridge.stop();
});

test("attachSniffer: webRequest 不可用时返回 null（功能整体降级）", () => {
  assert.equal(attachSniffer({ chrome: { tabs: { sendMessage: async () => {} } } }), null);
  assert.equal(attachSniffer({}), null);
});
