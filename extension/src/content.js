const found = new Map();
function scan() {
  document.querySelectorAll("video, audio, source").forEach(node => {
    const src = node.currentSrc || node.src;
    if (src && /^https?:/.test(src)) found.set(src, { url: src, type: node.tagName.toLowerCase(), title: document.title });
  });
  performance.getEntriesByType("resource").forEach(entry => {
    if (/\.(m3u8|mpd|mp4|webm|mp3|m4a)(\?|$)/i.test(entry.name)) found.set(entry.name, { url: entry.name, type: "media", title: document.title });
  });
  chrome.runtime.sendMessage({ type: "media", items: [...found.values()].slice(-20) }).catch(() => {});
}
scan();
new MutationObserver(() => scan()).observe(document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ["src"] });
setInterval(scan, 5000);

