// 猫步下载器 · PikPak Main World 拦截器（运行在页面宿主 JS 环境中）。
//
// 职责：
//   1. 拦截 PikPak 网页端自身发起的 fetch 与 XHR 请求；
//   2. 捕获 /drive/v1/share/detail、/drive/v1/share/file_info、/file_info 中的 1080P/4K 原画直链；
//   3. 通过 window.postMessage 安全广播给 Isolated World 的 content.js。

(() => {
  if (window.__MAOBU_PIKPAK_INJECTED__) return;
  window.__MAOBU_PIKPAK_INJECTED__ = true;

  function broadcastMedia(mediaList) {
    if (!Array.isArray(mediaList) || mediaList.length === 0) return;
    try {
      window.postMessage({
        source: "maobu-pikpak-injected",
        type: "PIKPAK_MEDIA_FOUND",
        medias: mediaList,
      }, "*");
    } catch {}
  }

  function extractFromPikPakJson(data) {
    const results = [];
    if (!data || typeof data !== "object") return results;

    // 单文件详情
    const file = data.file_info || (data.kind === "drive#file" ? data : null);
    if (file) {
      const fileName = file.name || "";
      if (Array.isArray(file.medias)) {
        for (const m of file.medias) {
          if (m?.link?.url) {
            results.push({
              url: m.link.url,
              name: fileName,
              resolution: m.resolution_name || m.media_name || "1080P",
              size: Number(file.size || 0),
            });
          }
        }
      }
      if (file.web_content_link) {
        results.push({
          url: file.web_content_link,
          name: fileName,
          resolution: "Original",
          size: Number(file.size || 0),
        });
      }
    }

    // 分享列表 / 目录树
    if (Array.isArray(data.files)) {
      for (const f of data.files) {
        if (f.web_content_link) {
          results.push({
            url: f.web_content_link,
            name: f.name || "PikPak_File",
            resolution: "Direct",
            size: Number(f.size || 0),
          });
        }
      }
    }

    return results;
  }

  // 1. Hook Fetch
  const originalFetch = window.fetch;
  if (originalFetch) {
    window.fetch = async function (...args) {
      const resp = await originalFetch.apply(this, args);
      try {
        const url = typeof args[0] === "string" ? args[0] : args[0]?.url || "";
        if (
          url.includes("mypikpak.com") &&
          (url.includes("/file_info") || url.includes("/share/detail") || url.includes("/download") || url.includes("/share/"))
        ) {
          const clone = resp.clone();
          clone.json().then((data) => {
            const extracted = extractFromPikPakJson(data);
            if (extracted.length > 0) {
              broadcastMedia(extracted);
            }
          }).catch(() => {});
        }
      } catch {}
      return resp;
    };
  }

  // 2. Hook XMLHttpRequest
  const originalXhrOpen = XMLHttpRequest.prototype.open;
  const originalXhrSend = XMLHttpRequest.prototype.send;

  XMLHttpRequest.prototype.open = function (method, url, ...rest) {
    this._maobu_url = url;
    return originalXhrOpen.apply(this, [method, url, ...rest]);
  };

  XMLHttpRequest.prototype.send = function (...args) {
    this.addEventListener("load", () => {
      try {
        const url = String(this._maobu_url || "");
        if (
          url.includes("mypikpak.com") &&
          (url.includes("/file_info") || url.includes("/share/detail") || url.includes("/download") || url.includes("/share/"))
        ) {
          const data = JSON.parse(this.responseText);
          const extracted = extractFromPikPakJson(data);
          if (extracted.length > 0) {
            broadcastMedia(extracted);
          }
        }
      } catch {}
    });
    return originalXhrSend.apply(this, args);
  };
})();
