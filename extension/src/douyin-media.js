// 经典内容脚本：仅从当前作品 URL 或可见播放器所属卡片识别作品。
// 不读取页面私有状态、不扫描推荐链接，也不把旧嗅探缓存当成当前作品。
(() => {
  if (window.MaobuDouyinMedia) return;

  const isHost = (hostname) => /(^|\.)douyin\.com$/i.test(String(hostname || ""));
  const validId = (value) => /^\d{10,}$/.test(String(value || ""));
  const CARD_SELECTOR = '[data-e2e="feed-active-video"], .swiper-slide-active, [data-aweme-id], [data-item-id], [data-e2e="feed-video"], [data-e2e="aweme-item"], [data-e2e="video-player"]';
  const ACTIVE_SELECTOR = '[data-e2e="feed-active-video"], .swiper-slide-active';

  function parseWorkUrl(value, base) {
    try {
      const url = new URL(value, base);
      if (!/^https?:$/.test(url.protocol) || !isHost(url.hostname)) return null;
      const match = url.pathname.match(/^\/(?:share\/)?(video|note)\/(\d{10,})(?:\/|$)/);
      const id = match?.[2] || ["modal_id", "aweme_id", "vid"].map((key) => url.searchParams.get(key)).find(validId);
      if (!id) return null;
      return { id, type: match?.[1] === "note" ? "gallery" : "video" };
    } catch { return null; }
  }

  function visibleArea(element, view) {
    const rect = element.getBoundingClientRect();
    const width = Math.max(0, Math.min(rect.right, view.innerWidth) - Math.max(rect.left, 0));
    const height = Math.max(0, Math.min(rect.bottom, view.innerHeight) - Math.max(rect.top, 0));
    const style = view.getComputedStyle?.(element);
    if (style?.display === "none" || style?.visibility === "hidden" || style?.opacity === "0") return 0;
    return width * height;
  }

  function findActiveVideo(doc, view) {
    const candidates = Array.from(doc.querySelectorAll("video"))
      .map((video) => ({ video, area: visibleArea(video, view) }))
      .filter(({ area }) => area > 0);
    candidates.sort((a, b) => {
      const activeDifference = Number(Boolean(b.video.closest(ACTIVE_SELECTOR))) - Number(Boolean(a.video.closest(ACTIVE_SELECTOR)));
      const playingDifference = Number(!b.video.paused) - Number(!a.video.paused);
      return activeDifference || playingDifference || b.area - a.area;
    });
    return candidates[0]?.video || null;
  }

  function findCardWork(card, base) {
    if (!card) return null;
    for (let node = card, depth = 0; node && depth < 5 && node.tagName !== "BODY"; node = node.parentElement, depth += 1) {
      // 外层 data-id 也可能属于作者或频道，只接受当前卡片自身的通用 ID。
      const attributes = node === card ? ["data-aweme-id", "data-item-id", "data-id"] : ["data-aweme-id", "data-item-id"];
      for (const attribute of attributes) {
        const id = node.getAttribute(attribute);
        if (validId(id)) return { id, type: "video" };
      }
    }
    for (const anchor of card.querySelectorAll('a[href*="/video/"], a[href*="/note/"], a[href*="modal_id="]')) {
      const target = parseWorkUrl(anchor.href, base);
      if (target) return target;
    }
    return null;
  }

  function directSource(video) {
    // 换作品的同一帧 currentSrc 可能仍指向上一段媒体，显式 src 已是新目标。
    const sources = [video?.src || video?.currentSrc, ...Array.from(video?.querySelectorAll("source") || []).map((source) => source.src)];
    for (const value of sources) {
      try {
        const url = new URL(value);
        if (/^https?:$/.test(url.protocol) && !/\.(m3u8|mpd|m4s|ts)(?:$|[?#])/i.test(url.href)) return url.href;
      } catch { /* blob 与未加载的媒体交由具体作品页解析。 */ }
    }
    return "";
  }

  function extract({ doc = document, loc = location, view = window } = {}) {
    if (!isHost(loc.hostname)) return null;
    const video = findActiveVideo(doc, view);
    const card = video?.closest(CARD_SELECTOR) || Array.from(doc.querySelectorAll(ACTIVE_SELECTOR)).find((node) => visibleArea(node, view) > 0);
    const pageWork = parseWorkUrl(loc.href, loc.href);
    const cardWork = findCardWork(card, loc.href);
    const work = cardWork && cardWork.id !== pageWork?.id ? cardWork : pageWork || cardWork;
    const title = card?.querySelector('[data-e2e="video-desc"], [data-e2e="video-title"], .title, h1')?.textContent?.trim()
      || doc.title.replace(/\s*-\s*抖音.*$/i, "").trim() || "抖音视频";
    // 图文即使有背景音乐播放器也必须进入作品解析，不能变成单个音视频任务。
    const direct = work?.type === "gallery" ? "" : directSource(video);
    if (direct) {
      const cleanTitle = title.replace(/[\\/:*?"<>|\r\n\t]/g, "_").replace(/\s+/g, " ").trim().slice(0, 120) || "抖音视频";
      return { url: direct, title, type: "video", isDirect: true, awemeId: work?.id || "", fileName: `${cleanTitle}${work?.id ? `_${work.id}` : ""}.mp4` };
    }
    if (work) return { url: `https://www.douyin.com/${work.type === "gallery" ? "note" : "video"}/${work.id}`, title, type: work.type, isDirect: false, awemeId: work.id };
    // 推荐流的 blob 播放器暂未暴露作品 ID 时保留入口，明确提示用户打开作品页。
    return video ? { url: "", title, unavailable: true, isDirect: false } : null;
  }

  window.MaobuDouyinMedia = Object.freeze({ isHost, parseWorkUrl, extract });
})();
