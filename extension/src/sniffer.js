// 流嗅探（网络媒体直链捕获）——按站点开关、仅内存、不上传（AGENTS.md §5）。
//
// 背景：MSE 站点（B 站/YouTube 等）的 <video> src 是 blob:，语义元素探测拿不到
// 真实直链，只能整页交给桌面端 yt-dlp 分析（60 秒级）。本模块在用户显式为某站点
// 开启嗅探后，观察该站点页面自身发起的媒体类请求 URL，供 FAB 直连下载与弹窗展示。
//
// 合规边界：
//   - 只记录媒体扩展名匹配的 http(s) URL，不读请求体/响应体/Cookie；
//   - 只在用户开启嗅探的站点记录，默认全局关闭；
//   - 仅存 Service Worker 内存（随 SW 回收清空），不写 storage、不上传。

export const SNIFF_LIST_MAX = 30;

// 扩展名匹配基于路径（去掉查询串），避免 ?format=mp4 之类的误报。
const SNIFF_EXT_PATTERN = /\.(m3u8|mpd|mp4|webm|flv|ts|m4s|mp3|m4a|aac|ogg|opus|wav)$/i;

const KIND_BY_EXT = {
  m3u8: "stream", mpd: "stream", ts: "stream", m4s: "stream", flv: "stream",
  mp4: "video", webm: "video",
  mp3: "audio", m4a: "audio", aac: "audio", ogg: "audio", opus: "audio", wav: "audio",
};

function pathExtension(url) {
  try {
    const name = new URL(url).pathname.split("/").pop() || "";
    const match = name.match(/\.([a-z0-9]{1,5})$/i);
    return match ? match[1].toLowerCase() : "";
  } catch {
    return "";
  }
}

/// URL 是否为可嗅探的媒体直链（http/https + 媒体扩展名）。
export function isSniffableMediaUrl(url) {
  const text = String(url || "");
  if (!/^https?:/i.test(text)) return false;
  try {
    return SNIFF_EXT_PATTERN.test(new URL(text).pathname.split("/").pop() || "");
  } catch {
    return false;
  }
}

/// 媒体类别（stream/video/audio），用于展示图标与 FAB 挑选优先级。
export function sniffedKind(url) {
  return KIND_BY_EXT[pathExtension(url)] || "media";
}

/// 从 URL 路径取展示名（最后一段，尽量解码）。
export function sniffedName(url) {
  try {
    const raw = new URL(url).pathname.split("/").pop() || "";
    try { return decodeURIComponent(raw); } catch { return raw; }
  } catch {
    return "";
  }
}

/// 追加一条嗅探记录：同 URL 去重（更新时间）、按容量裁剪最旧的。返回新数组。
export function pushSniffedItem(list, url, max = SNIFF_LIST_MAX) {
  const entry = { url: String(url || ""), kind: sniffedKind(url), name: sniffedName(url), at: Date.now() };
  const next = (Array.isArray(list) ? list : []).filter((item) => item?.url !== entry.url);
  next.push(entry);
  return next.length > max ? next.slice(next.length - max) : next;
}

/// hostname 是否命中域名规则列表（精确或子域，与接管规则一致）。
export function hostMatchesList(hostname, hosts) {
  const host = String(hostname || "").toLowerCase();
  if (!host) return false;
  return (Array.isArray(hosts) ? hosts : []).some((rule) => {
    const value = String(rule || "").toLowerCase();
    return value && (host === value || host.endsWith(`.${value}`));
  });
}

/// FAB 直连目标挑选：仅使用可被 HTTP 内核直接下载的 video/audio 文件直链
/// （mp4/webm/mp3 等）。stream 类（m3u8/mpd/ts 分片）不走直连——桌面端只对
/// 已知媒体平台的 URL 走 yt-dlp 管道，裸 m3u8 直连会被当成文本文件下载；
/// 流地址应由 popup 的"解析下载"交给 /v1/media/probe（yt-dlp 原生支持 HLS）。
export function pickFabTarget(items) {
  const list = Array.isArray(items) ? items : [];
  for (const kind of ["video", "audio"]) {
    for (let i = list.length - 1; i >= 0; i -= 1) {
      if (list[i]?.kind === kind) return list[i].url;
    }
  }
  return "";
}

/// 站点开关列表更新（纯函数）：开启时追加去重；关闭时移除该 host 自身与
/// 覆盖它的父域规则（用户在子域页面关闭时，父域规则也一并失效）。
export function toggleSniffHost(hosts, host, enabled) {
  const list = (Array.isArray(hosts) ? hosts : []).map((item) => String(item || "").toLowerCase()).filter(Boolean);
  const target = String(host || "").toLowerCase();
  if (!target) return [...new Set(list)];
  if (enabled) return [...new Set([...list, target])];
  return list.filter((rule) => rule !== target && !target.endsWith(`.${rule}`));
}

/// SW 侧接线：注册 webRequest 观察监听，按站点开关记录媒体 URL 并推送 content script。
///
/// 依赖注入 `chrome`（默认 globalThis.chrome）便于模拟测试；webRequest/tabs 任一
/// 不可用时返回 null（旧浏览器/测试环境下降级为无嗅探，不影响其它功能）。
/// 返回 `{ getItems(tabId), isHostEnabled(hostname), stop() }`：
///   - getItems：该标签页的嗅探记录，最新在前；
///   - isHostEnabled：站点开关状态（popup 展示用）；
///   - stop：移除监听（测试清理用）。
export function attachSniffer(deps = {}) {
  const chromeLike = deps.chrome || globalThis.chrome;
  const webRequest = chromeLike?.webRequest;
  const tabsApi = chromeLike?.tabs;
  const storage = chromeLike?.storage;
  if (!webRequest?.onBeforeRequest?.addListener || !tabsApi?.sendMessage) return null;

  const sniffed = new Map();
  let enabledHosts = [];
  const loadHosts = async () => {
    try {
      const stored = await storage.local.get("snifferHosts");
      enabledHosts = Array.isArray(stored.snifferHosts)
        ? stored.snifferHosts.map((item) => String(item || "").toLowerCase()).filter(Boolean)
        : [];
    } catch {
      enabledHosts = [];
    }
  };
  void loadHosts();
  try {
    storage?.onChanged?.addListener?.((changes, area) => {
      if (area === "local" && changes.snifferHosts) void loadHosts();
    });
  } catch { /* onChanged 不可用时开关需重启 SW 生效，可接受。 */ }

  const onBeforeRequest = (details) => {
    if (!details || !(details.tabId >= 0)) return;
    if (!isSniffableMediaUrl(details.url)) return;

    let initiatorHost = "";
    if (details.initiator) {
      try { initiatorHost = new URL(details.initiator).hostname.toLowerCase(); } catch {}
    }
    let requestHost = "";
    try { requestHost = new URL(details.url).hostname.toLowerCase(); } catch {}

    const isMatch = (initiatorHost && hostMatchesList(initiatorHost, enabledHosts))
      || (requestHost && hostMatchesList(requestHost, enabledHosts));
    if (!isMatch) return;

    const list = pushSniffedItem(sniffed.get(details.tabId) || [], details.url);
    sniffed.set(details.tabId, list);
    // 推送给 content script（FAB 直连用最新 10 条）；页面无接收方时静默忽略。
    void Promise.resolve(tabsApi.sendMessage(details.tabId, { type: "sniffed-media", items: list.slice(-10) }))
      .catch(() => {});
  };
  try {
    webRequest.onBeforeRequest.addListener(onBeforeRequest, { urls: ["http://*/*", "https://*/*"] });
    tabsApi.onRemoved?.addListener?.((tabId) => { sniffed.delete(tabId); });
  } catch {
    return null;
  }

  return {
    getItems: (tabId) => (sniffed.get(tabId) || []).slice().reverse(),
    isHostEnabled: (hostname) => hostMatchesList(hostname, enabledHosts),
    stop: () => { try { webRequest.onBeforeRequest.removeListener(onBeforeRequest); } catch {} },
  };
}
