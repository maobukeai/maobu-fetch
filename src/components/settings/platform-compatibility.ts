import type { PlatformCompatibility } from "../../types";

export function verificationScopeLabel(scope?: string): string {
  return ({ metadata: "元数据解析", download: "完整下载", merge: "音视频合并", login: "登录与权限", playback: "播放检查" } as Record<string, string>)[scope || ""] || "未记录层级";
}

function validDate(value: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}(?:T\d{2}:\d{2}:\d{2}Z)?$/.test(value)) return false;
  const timestamp = Date.parse(value.length === 10 ? `${value}T00:00:00Z` : value);
  return Number.isFinite(timestamp) && timestamp <= Date.now()
    && new Date(timestamp).toISOString().slice(0, 10) === value.slice(0, 10);
}

export function compatibilityBadge(item: PlatformCompatibility, appVersion?: string): { label: string; color: string } {
  const warning = "var(--warning-color, #b57408)";
  if (item.level === "unsupported") return { label: "不支持", color: "var(--danger-color, #c94444)" };
  const record = item.verification;
  if (!record) return { label: "未验证", color: "var(--muted)" };
  const complete = !!record.app_version.trim() && !!record.tool_version.trim() && !!record.sample.trim()
    && validDate(item.last_tested_at || "") && ["metadata", "download", "merge", "login", "playback"].includes(record.scope);
  if (!complete) return { label: "证据不完整", color: warning };
  if (record.result === "failed") return { label: "验证未通过", color: "var(--danger-color, #c94444)" };
  if (record.result !== "passed") return { label: "未验证", color: "var(--muted)" };
  if (!appVersion || record.app_version !== appVersion) return { label: "历史样本通过", color: warning };
  if (record.scope === "download" && item.level === "verified") return { label: "下载样本已验证", color: "var(--success-color, #16814c)" };
  return { label: `仅${verificationScopeLabel(record.scope)}通过`, color: warning };
}
