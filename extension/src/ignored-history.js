// "被放行的浏览器下载"历史（纯函数，P2-14）。
//
// popup 诊断区展示最近若干条未接管记录，每条带"改用猫步下载器下载"按钮。
// 环形缓冲只保留最新 max 条，防止长驻浏览器无限增长。

export const IGNORED_HISTORY_MAX = 20;

/// 追加一条记录并裁剪到 `max` 条（返回新数组，不修改入参）。
export function pushIgnoredEntry(list, entry, max = IGNORED_HISTORY_MAX) {
  const next = [...(Array.isArray(list) ? list : []), entry];
  return next.length > max ? next.slice(next.length - max) : next;
}
