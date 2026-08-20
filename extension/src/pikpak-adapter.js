// 猫步下载器 · PikPak 网页专属适配器（经典脚本/挂载在 window.MaobuPikPak 上）。
//
// 职责：
//   1. 识别 mypikpak.com / mypikpak.net 网页环境（分享页、播放页、文件列表页）；
//   2. 自动捕获网页播放器或文件 API 中的 1080P/4K 原画直链（https://dl-*.mypikpak.com/...）；
//   3. 统一将状态和直链提供给猫步标准 FAB 悬浮球，保持界面克制纯粹，杜绝多余重复按钮；
//   4. 支持一键下发至猫步桌面端并开启 16/32 线程并发 Range 下载与防盗链 Header。

(() => {
  if (window.MaobuPikPak) return;

  const isPikPakHost = (host) => {
    const h = String(host || window.location.hostname || "").toLowerCase();
    return h === "mypikpak.com" || h.endsWith(".mypikpak.com") ||
           h === "mypikpak.net" || h.endsWith(".mypikpak.net");
  };

  if (!isPikPakHost()) return;

  const isPikPakSharePage = () => {
    const p = window.location.pathname + window.location.hash;
    return p.includes("/s/") || /mypikpak\.(?:com|net)\/s\/[a-zA-Z0-9_-]+/i.test(window.location.href);
  };

  // 捕获到的 PikPak 媒体直链缓存：Map<url, { url, name, resolution, size }>
  const capturedMedia = new Map();

  function recordDirectLink(url, name, resolution = "Original") {
    if (!url || typeof url !== "string") return;
    const cleanUrl = url.trim();
    if (!cleanUrl.startsWith("http://") && !cleanUrl.startsWith("https://")) return;
    if (!capturedMedia.has(cleanUrl)) {
      capturedMedia.set(cleanUrl, {
        url: cleanUrl,
        name: name || document.title.replace(/\s*-\s*PikPak.*$/i, "").trim() || "PikPak_Video.mp4",
        resolution,
        timestamp: Date.now(),
      });
    }
  }

  // 1. 拦截页面内的 fetch 响应，解析 PikPak 返回的 medias 直链
  function hookFetch() {
    const originalFetch = window.fetch;
    if (!originalFetch) return;

    window.fetch = async function (...args) {
      const resp = await originalFetch.apply(this, args);
      try {
        const url = typeof args[0] === "string" ? args[0] : args[0]?.url || "";
        if (
          url.includes("mypikpak.com") &&
          (url.includes("/file_info") || url.includes("/share/detail") || url.includes("/download"))
        ) {
          const clone = resp.clone();
          clone.json().then((data) => {
            if (data?.file_info) {
              const file = data.file_info;
              const fileName = file.name || "";
              if (Array.isArray(file.medias)) {
                for (const m of file.medias) {
                  if (m?.link?.url) {
                    recordDirectLink(m.link.url, fileName, m.resolution_name || m.media_name || "1080P");
                  }
                }
              }
              if (file.web_content_link) {
                recordDirectLink(file.web_content_link, fileName, "Direct");
              }
            } else if (Array.isArray(data?.files)) {
              for (const f of data.files) {
                if (f.web_content_link) {
                  recordDirectLink(f.web_content_link, f.name, "Direct");
                }
              }
            }
          }).catch(() => {});
        }
      } catch {}
      return resp;
    };
  }

  // 2. 观察页面中的 <video> 标签
  function inspectVideos() {
    const videos = document.querySelectorAll("video");
    videos.forEach((video) => {
      const src = video.currentSrc || video.src;
      if (src && (src.includes("mypikpak.com") || src.includes(".mypikpak."))) {
        recordDirectLink(src, document.title, "Video Player");
      }
    });
  }

  // 初始化
  try {
    hookFetch();
    setInterval(inspectVideos, 1500);

    // 监听来自 Main World (pikpak-injected.js) 的直链广播
    window.addEventListener("message", (event) => {
      if (event.data?.source === "maobu-pikpak-injected" && event.data?.type === "PIKPAK_MEDIA_FOUND") {
        const medias = event.data.medias || [];
        for (const m of medias) {
          if (m?.url) {
            recordDirectLink(m.url, m.name, m.resolution);
          }
        }
      }
    });
  } catch {}

  window.MaobuPikPak = {
    isPikPakHost,
    isPikPakSharePage,
    recordDirectLink,
    getCapturedMedia: () => [...capturedMedia.values()],
  };
})();
