import type { PlatformCompatibility } from "../../types";
import { compatibilityBadge, verificationScopeLabel } from "./platform-compatibility";

const names: Record<string, string> = { bilibili: "哔哩哔哩", douyin: "抖音", tiktok: "TikTok", twitter: "Twitter/X", youtube: "YouTube", weibo: "微博", pikpak: "PikPak", quark: "夸克网盘", baidupan: "百度网盘", baidu: "百度网盘", lanzou: "蓝奏云", pan123: "123云盘", "123pan": "123云盘", xiaohongshu: "小红书", kuaishou: "快手", xigua: "西瓜视频" };
const cell = { padding: "7px 8px", borderBottom: "1px solid var(--border)", verticalAlign: "top" } as const;

export function PlatformCompatibilitySection({ items, appVersion }: { items: PlatformCompatibility[]; appVersion?: string }) {
  return <section style={{ borderTop: "1px solid var(--border)", paddingTop: 14, display: "grid", gap: 8 }}>
    <h3 style={{ margin: 0, fontSize: 12, fontWeight: 600 }}>平台兼容性</h3>
    <p style={{ margin: 0, fontSize: 11, color: "var(--muted)", lineHeight: 1.5 }}>
      能力说明不代表当前可用保证。仅当前软件版本的完整下载样本通过，才标记为已验证；解析、登录、合并或播放检查单独列出。不会后台联网扫描。
    </p>
    {!items.length ? <p style={{ color: "var(--muted)", fontSize: 11 }}>暂无兼容性记录</p> :
      <div style={{ overflowX: "auto" }}><table style={{ width: "100%", borderCollapse: "collapse", fontSize: 11, textAlign: "left", lineHeight: 1.5 }}>
        <thead><tr><th style={cell}>平台</th><th style={cell}>验证状态</th><th style={cell}>能力说明与限制</th><th style={cell}>验证记录</th></tr></thead>
        <tbody>{items.map(item => {
          const badge = compatibilityBadge(item, appVersion);
          const record = item.verification;
          return <tr key={item.platform}>
            <th scope="row" style={{ ...cell, minWidth: 70, fontWeight: 600 }}>{names[item.platform] || item.platform}</th>
            <td style={{ ...cell, minWidth: 92 }}><span style={{ color: "var(--text)", borderLeft: `3px solid ${badge.color}`, paddingLeft: 5 }}>{badge.label}</span></td>
            <td style={{ ...cell, minWidth: 190 }}>
              <p style={{ margin: 0 }}>{item.notes || "暂无能力说明"}</p>
              {!!item.known_issues?.length && <ul style={{ margin: "3px 0 0", paddingLeft: 16, color: "var(--muted)" }}>{item.known_issues.map((issue, index) => <li key={index}>{issue}</li>)}</ul>}
            </td>
            <td style={{ ...cell, minWidth: 130, maxWidth: 220, overflowWrap: "anywhere" }}>
              {!record ? <span style={{ color: "var(--muted)" }}>{item.last_tested_at ? `${item.last_tested_at} · 缺少样本与版本记录` : "未记录测试证据"}</span> :
                <details><summary style={{ cursor: "pointer", outlineColor: "var(--accent)", outlineOffset: 2 }}>{item.last_tested_at || "日期未记录"}<br />{verificationScopeLabel(record.scope)}</summary>
                  <dl style={{ margin: "5px 0 0", color: "var(--muted)" }}>
                    <dt>软件版本</dt><dd style={{ marginLeft: 8 }}>{record.app_version || "未记录"}</dd>
                    <dt>工具版本</dt><dd style={{ marginLeft: 8 }}>{record.tool_version || "未记录"}</dd>
                    <dt>样本</dt><dd style={{ marginLeft: 8, userSelect: "text" }}>{record.sample || "未记录"}</dd>
                    <dt>结果</dt><dd style={{ marginLeft: 8 }}>{record.result === "passed" ? "该层级样本通过" : record.result === "failed" ? "未通过" : "未记录"}</dd>
                  </dl>
                </details>}
            </td>
          </tr>;
        })}</tbody>
      </table></div>}
  </section>;
}
