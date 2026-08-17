// 链接识别与提取（纯函数，无 Chrome API 依赖）。
//
// 用途：
//   - content script 拦截页面内 magnet: 链接点击（BT-08）；
//   - 右键"下载选中文字中的链接"：从用户选中文本提取 magnet/http(s) 链接。
// 按位与（&）等操作不涉及页面数据采集，仅处理用户显式提供的文本（AGENTS.md §5）。

/// 判断是否为 magnet: 链接（大小写不敏感；URL 解析失败时退化为前缀判断）。
export function isMagnetUrl(value) {
  const text = String(value || "").trim();
  if (!text) return false;
  try {
    return new URL(text).protocol === "magnet:";
  } catch {
    return /^magnet:/i.test(text);
  }
}

/// 磁力链接至少要包含 infohash 参数才算可下载；
/// 只有 magnet: 前缀而无 xt=urn:btih 的字符串交给桌面端只会报错，
/// 这里提前识别，让调用方走"不是磁力"的分支。
export function isDownloadableMagnet(value) {
  const text = String(value || "").trim();
  return /^magnet:\?[^#]*xt=urn:btih:/i.test(text);
}

// 剥离句尾标点：ASCII 与中文全角（。，、；：！？）都常见于正文粘贴。
const TRAILING_PUNCTUATION = /[.,;:!?)\]}>》」』"'。，、；：！？）】〉”』]+$/;

function cleanCandidate(value) {
  return value.replace(TRAILING_PUNCTUATION, "");
}

/// 从任意文本提取下载链接。
///
/// 返回 `{ magnets: string[], urls: string[] }`，各自去重、保序、最多 `max` 条。
/// - magnet：`magnet:?xt=urn:btih:…`（必须带 infohash）
/// - urls：`http(s)://…`（排除常见结尾标点）
/// 纯文本中的链接由用户选中文本触发提取，不做全页面扫描。
// 提取正则的排除字符：ASCII 空白/引号/括号 + 中文全角标点（，。；：！？、）。
// 全角标点不可能出现在合法 URL 中，却是中文正文最常见的分隔符，
// 不排除会把“链接，还有…”整段吞进候选。
const URL_BOUNDARY = String.raw`[^\s"'<>()\[\]{}，。；：！？、]`;

export function extractLinksFromText(text, max = 10) {
  const source = String(text || "");
  const magnets = [];
  const urls = [];
  const pushUnique = (list, value) => {
    if (list.length >= max || list.includes(value)) return;
    list.push(value);
  };
  for (const match of source.matchAll(new RegExp(`magnet:\\?${URL_BOUNDARY}+`, "gi"))) {
    const candidate = cleanCandidate(match[0]);
    if (isDownloadableMagnet(candidate)) pushUnique(magnets, candidate);
  }
  for (const match of source.matchAll(new RegExp(`https?:\\/\\/${URL_BOUNDARY}+`, "gi"))) {
    pushUnique(urls, cleanCandidate(match[0]));
  }
  return { magnets, urls };
}
