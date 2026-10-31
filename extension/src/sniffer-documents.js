// 只保存当前页面的 documentId/frameId，不保存 URL 或浏览历史。
export function createSnifferDocuments() {
  const tabs = new Map();
  return {
    commit(details) {
      let frames = tabs.get(details.tabId);
      if (frames?.get(details.frameId)?.id === details.documentId) return;
      if (!frames || details.frameId === 0) frames = new Map();
      // 子 frame 导航时同时丢弃其旧子树，避免晚到的旧媒体污染新页面。
      const removed = new Set([details.frameId]);
      let changed = true;
      while (changed) {
        changed = false;
        for (const [id, frame] of frames) {
          if (removed.has(id) || removed.has(frame.parent)) {
            frames.delete(id);
            if (!removed.has(id)) { removed.add(id); changed = true; }
          }
        }
      }
      frames.set(details.frameId, { id: details.documentId, parent: details.parentFrameId });
      tabs.set(details.tabId, frames);
    },
    matches(details) {
      const frames = tabs.get(details.tabId);
      if (!frames || !details.documentId) return true;
      const frameId = Number.isInteger(details.frameId) ? details.frameId : 0;
      if (frameId >= 0) return frames.get(frameId)?.id === details.documentId;
      return [...frames.values()].some((frame) => frame.id === details.documentId);
    },
    remove(tabId) { tabs.delete(tabId); },
    clear() { tabs.clear(); },
  };
}
