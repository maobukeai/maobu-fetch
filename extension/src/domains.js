// 媒体平台域名表（纯函数）。
//
// background.js 的 Cookie 自动同步与 popup.js 打开时的同步共用这一份表；
// 此前两处各复制一份，新增平台时容易漏改（P1-7 修复）。

/// 已知媒体平台域名 → 归一化基域。Cookie 按基域同步给桌面端凭证库。
const MEDIA_DOMAIN_ALIASES = new Map([
  ["weibo.cn", "weibo.com"],
  ["x.com", "twitter.com"],
]);

export const MEDIA_SYNC_DOMAINS = [
  "douyin.com",
  "tiktok.com",
  "bilibili.com",
  "weibo.com",
  "weibo.cn",
  "youtube.com",
  "twitter.com",
  "x.com",
];

/// 判断 hostname 是否属于已知媒体平台；命中返回归一化基域，否则返回 null。
/// 匹配规则与接管规则的 host 匹配一致：精确等于或子域（`.example.com`）。
/// `extraDomains`：用户在扩展选项中自定义追加的同步域名（options 维护）。
export function matchMediaDomain(hostname, extraDomains = []) {
  const host = String(hostname || "").toLowerCase();
  if (!host) return null;
  const extras = Array.isArray(extraDomains) ? extraDomains : [];
  for (const domain of [...MEDIA_SYNC_DOMAINS, ...extras]) {
    const rule = String(domain || "").toLowerCase();
    if (!rule) continue;
    if (host === rule || host.endsWith(`.${rule}`)) {
      return MEDIA_DOMAIN_ALIASES.get(rule) || rule;
    }
  }
  return null;
}
