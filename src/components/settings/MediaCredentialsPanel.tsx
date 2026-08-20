import { useEffect, useState } from "react";
import {
  AlertCircle,
  CheckCircle2,
  HelpCircle,
  Info,
  LoaderCircle,
  Plus,
  RefreshCw,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { api } from "../../api";
import type { MediaCredential, MediaCredentialCheckResult } from "../../types";
import { Modal } from "../common/Modal";
import { Field, SettingsGroup } from "../common/FormComponents";

function MediaCredentialsGuideModal({ onClose }: { onClose: () => void }) {
  return (
    <Modal
      title="凭证获取指引与平台关键 Key 说明"
      onClose={onClose}
      style={{ width: "620px" }}
    >
      <div className="media-cred-guide-container">
        <div className="media-cred-guide-section">
          <h3>📌 如何通过浏览器开发者工具 (F12) 获取凭证</h3>
          <ol className="media-cred-guide-steps">
            <li>
              在 <b>Chrome / Edge</b> 浏览器中打开目标网站（如 B站、抖音、Twitter、YouTube）并登录您的账号。
            </li>
            <li>
              按键盘 <kbd>F12</kbd> 打开<b>开发者工具</b>，切换到 <b>Network (网络)</b> 标签页。
            </li>
            <li>
              <b>刷新页面</b>或播放网页视频，在左侧请求列表中选中顶部任意主页面请求或 API 请求。
            </li>
            <li>
              在右侧 <b>Headers (请求头)</b> 区域找到 <code>Cookie</code>、<code>Referer</code> 和 <code>User-Agent</code> 字段。右键复制其完整值并粘贴至本软件。
            </li>
          </ol>
        </div>

        <div className="media-cred-guide-section">
          <h3>🔑 四大平台核心凭证 Key 校验指南</h3>
          <div className="media-cred-guide-platforms">
            <div className="platform-guide-card">
              <h4>
                哔哩哔哩 <code>bilibili.com</code>
              </h4>
              <p>
                必须包含 <code>SESSDATA</code>、<code>bili_jct</code>、<code>DedeUserID</code>
              </p>
              <span className="tip">支持获取 1080P+ 高清画质及大会员专属音轨。</span>
            </div>
            <div className="platform-guide-card">
              <h4>
                抖音 <code>douyin.com</code>
              </h4>
              <p>
                必须包含 <code>sessionid</code> (或 <code>sessionid_ss</code>)、<code>passport_csrf_token</code>、<code>ttwid</code>
              </p>
              <span className="tip">支持获取 4K/2K 无水印视频及高级高清源。</span>
            </div>
            <div className="platform-guide-card">
              <h4>
                Twitter / X <code>twitter.com / x.com</code>
              </h4>
              <p>
                必须包含 <code>auth_token</code> 和 <code>ct0</code>
              </p>
              <span className="tip">
                <code>ct0</code> 用于 x-csrf-token 鉴权，缺一不可。
              </span>
            </div>
            <div className="platform-guide-card">
              <h4>
                YouTube <code>youtube.com</code>
              </h4>
              <p>
                必须包含 <code>LOGIN_INFO</code>、<code>SID</code>、<code>HSID</code>、<code>SSID</code>、<code>APISID</code>、<code>SAPISID</code>
              </p>
              <span className="tip">用于突破年龄限制及会员专属视频下载。</span>
            </div>
            <div className="platform-guide-card">
              <h4>
                百度网盘 <code>pan.baidu.com</code>
              </h4>
              <p>
                必须包含 <code>BDUSS</code>、<code>STOKEN</code>、<code>BAIDUID</code>
              </p>
              <span className="tip">用于百度网盘 SVIP 高速直链解析与自动转存。</span>
            </div>
            <div className="platform-guide-card">
              <h4>
                夸克网盘 <code>pan.quark.cn</code>
              </h4>
              <p>
                必须包含 <code>cookie</code>（建议通过扩展一键同步）
              </p>
              <span className="tip">用于夸克 VIP 4K/大文件极速直链下载。</span>
            </div>
          </div>
        </div>

        <div className="media-cred-guide-section">
          <h3>🧩 扩展程序自动同步说明</h3>
          <p className="settings-note" style={{ margin: 0 }}>
            若您已安装并配对<b>猫步下载器浏览器扩展</b>，在浏览器访问目标网页时，扩展也可自动捕获当前页面的凭证进行透传，无需频繁手动复制。
          </p>
        </div>

        <div className="dialog-actions">
          <button className="primary" onClick={onClose}>
            知道了
          </button>
        </div>
      </div>
    </Modal>
  );
}

export function MediaCredentialsPanel({
  notify,
}: {
  notify: (text: string, kind?: "ok" | "error") => void;
}) {
  const [credentials, setCredentials] = useState<MediaCredential[]>([]);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<MediaCredential | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [guideOpen, setGuideOpen] = useState(false);

  const [checkingDomains, setCheckingDomains] = useState<Record<string, boolean>>({});
  const [checkResults, setCheckResults] = useState<Record<string, MediaCredentialCheckResult>>({});
  const [checkingAll, setCheckingAll] = useState(false);

  const [editingCheckResult, setEditingCheckResult] = useState<MediaCredentialCheckResult | null>(null);
  const [editingChecking, setEditingChecking] = useState(false);

  const reload = async () => {
    setLoading(true);
    try {
      const list = await api.mediaCredentialList();
      setCredentials(list);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  const startAdd = () => {
    setEditing({ domain: "", cookie: "", referer: null, user_agent: null, updated_at: "" });
    setIsNew(true);
    setEditingCheckResult(null);
  };

  const startEdit = (cred: MediaCredential) => {
    setEditing({ ...cred });
    setIsNew(false);
    setEditingCheckResult(null);
  };

  const checkCredential = async (domain: string) => {
    setCheckingDomains((prev) => ({ ...prev, [domain]: true }));
    try {
      const res = await api.mediaCredentialCheck(domain);
      setCheckResults((prev) => ({ ...prev, [domain]: res }));
      if (res.valid) {
        notify(res.message);
      } else {
        notify(res.message, "error");
      }
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setCheckingDomains((prev) => ({ ...prev, [domain]: false }));
    }
  };

  const checkAllCredentials = async () => {
    if (credentials.length === 0) return;
    setCheckingAll(true);
    notify("开始在线检测已保存的媒体凭证...");
    try {
      await Promise.all(
        credentials.map(async (c) => {
          setCheckingDomains((prev) => ({ ...prev, [c.domain]: true }));
          try {
            const res = await api.mediaCredentialCheck(c.domain);
            setCheckResults((prev) => ({ ...prev, [c.domain]: res }));
          } catch {
            // ignore
          } finally {
            setCheckingDomains((prev) => ({ ...prev, [c.domain]: false }));
          }
        })
      );
      notify("所有媒体凭证检测完毕");
    } finally {
      setCheckingAll(false);
    }
  };

  const checkEditingCredential = async () => {
    if (!editing) return;
    const domain = editing.domain.trim();
    if (!domain) {
      notify("域名不能为空", "error");
      return;
    }
    setEditingChecking(true);
    setEditingCheckResult(null);
    try {
      const res = await invoke<MediaCredentialCheckResult>("media_credential_check", { domain });
      setEditingCheckResult(res);
      if (res.valid) {
        notify(res.message);
      } else {
        notify(res.message, "error");
      }
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setEditingChecking(false);
    }
  };

  const onDomainChange = (val: string) => {
    if (!editing) return;
    const next = { ...editing, domain: val };
    const lower = val.trim().toLowerCase();
    if (!editing.referer) {
      if (lower.includes("bilibili.com")) next.referer = "https://www.bilibili.com/";
      else if (lower.includes("douyin.com")) next.referer = "https://www.douyin.com/";
      else if (lower.includes("twitter.com") || lower.includes("x.com")) next.referer = "https://x.com/";
      else if (lower.includes("youtube.com")) next.referer = "https://www.youtube.com/";
    }
    setEditing(next);
  };

  const saveEdit = async () => {
    if (!editing) return;
    const domain = editing.domain.trim();
    if (!domain) {
      notify("域名不能为空", "error");
      return;
    }
    if (/\s/.test(domain) || /^https?:\/\//i.test(domain) || /[\/\\]/.test(domain)) {
      notify("域名格式无效（应为裸域名，如 example.com）", "error");
      return;
    }
    const toSave: MediaCredential = {
      domain,
      cookie: editing.cookie?.trim() ?? "",
      referer: editing.referer?.trim() ? editing.referer!.trim() : null,
      user_agent: editing.user_agent?.trim() ? editing.user_agent!.trim() : null,
      updated_at: new Date().toISOString(),
    };
    try {
      await api.mediaCredentialSave(toSave);
      notify(isNew ? "已保存媒体凭证" : "已更新媒体凭证");
      setEditing(null);
      await reload();
      void checkCredential(domain);
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const removeCredential = async (domain: string) => {
    if (!confirm(`确定删除域名 ${domain} 的凭证？`)) return;
    try {
      await api.mediaCredentialDelete(domain);
      notify("已删除媒体凭证");
      setCheckResults((prev) => {
        const next = { ...prev };
        delete next[domain];
        return next;
      });
      await reload();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const fmtUpdated = (v?: string): string => {
    if (!v) return "—";
    try {
      const d = new Date(v);
      if (Number.isNaN(d.getTime())) return v;
      return d.toLocaleString();
    } catch {
      return v;
    }
  };

  const maskCookie = (cookie?: string): string => {
    if (!cookie) return "—";
    if (cookie.length <= 12) return "已保存（较短）";
    return `已保存（${cookie.length} 字符）`;
  };

  return (
    <SettingsGroup title="媒体凭证管理">
      <p className="settings-note">
        按域名保存 Cookie 和 Referer 等凭证以在下载时自动附带。Cookie 使用 Windows DPAPI 加密存储。深度支持 Bilibili / 抖音 / Twitter(X) / YouTube 在线有效性检测。
      </p>
      <div className="category-rules-toolbar">
        <button className="input-button" onClick={startAdd}>
          <Plus size={13} />
          <span>新增凭证</span>
        </button>
        <button className="input-button" onClick={() => setGuideOpen(true)}>
          <HelpCircle size={13} />
          <span>凭证获取指引</span>
        </button>
        <button className="input-button" onClick={() => void reload()}>
          <RefreshCw size={13} />
          <span>刷新</span>
        </button>
        <button
          className="input-button"
          disabled={credentials.length === 0 || checkingAll}
          onClick={() => void checkAllCredentials()}
        >
          {checkingAll ? <LoaderCircle size={13} className="spin" /> : <ShieldCheck size={13} />}
          <span>{checkingAll ? "检测中..." : "批量检测"}</span>
        </button>
      </div>
      {loading ? (
        <LoaderCircle className="spin" />
      ) : credentials.length === 0 ? (
        <p className="settings-note">暂无已保存的媒体凭证。</p>
      ) : (
        <div className="category-rules-list" role="table">
          <div className="category-rule-row media-credential-row category-rule-row-header" role="row">
            <span className="category-rule-name">域名</span>
            <span className="category-rule-pattern">Cookie</span>
            <span>Referer</span>
            <span>User-Agent</span>
            <span>更新时间</span>
            <span className="category-rule-actions">操作</span>
          </div>
          {credentials.map((cred) => {
            const res = checkResults[cred.domain];
            const isChecking = !!checkingDomains[cred.domain];
            return (
              <div key={cred.domain} style={{ display: "flex", flexDirection: "column" }}>
                <div className="category-rule-row media-credential-row" role="row">
                  <span className="category-rule-name" role="cell" title={cred.domain}>
                    <code>{cred.domain}</code>
                  </span>
                  <span className="category-rule-pattern" role="cell">
                    {maskCookie(cred.cookie)}
                  </span>
                  <span role="cell" title={cred.referer ?? ""}>
                    {cred.referer ? "已设置" : "—"}
                  </span>
                  <span role="cell" title={cred.user_agent ?? ""}>
                    {cred.user_agent ? "已设置" : "—"}
                  </span>
                  <span role="cell">{fmtUpdated(cred.updated_at)}</span>
                  <span className="category-rule-actions" role="cell">
                    <button
                      title="在线检测凭证有效性"
                      disabled={isChecking}
                      onClick={() => void checkCredential(cred.domain)}
                    >
                      {isChecking ? <LoaderCircle size={12} className="spin" /> : <ShieldCheck size={12} />}
                      <span>检测</span>
                    </button>
                    <button title="编辑" onClick={() => startEdit(cred)}>
                      编辑
                    </button>
                    <button
                      title="删除"
                      className="danger"
                      onClick={() => void removeCredential(cred.domain)}
                    >
                      <Trash2 size={12} />
                    </button>
                  </span>
                </div>
                {res && (
                  <div className={`media-cred-result-box ${res.valid ? "valid" : "invalid"}`}>
                    {res.valid ? <CheckCircle2 size={13} /> : <AlertCircle size={13} />}
                    <span>{res.message}</span>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
      {guideOpen && <MediaCredentialsGuideModal onClose={() => setGuideOpen(false)} />}
      {editing && (
        <Modal
          title={isNew ? "新增媒体凭证" : "编辑媒体凭证"}
          onClose={() => setEditing(null)}
          style={{ width: "560px" }}
        >
          <div className="category-rule-edit-form">
            <Field label="域名">
              <input
                value={editing.domain}
                onChange={(e) => onDomainChange(e.target.value)}
                placeholder="如 bilibili.com (裸域名，不含 http:// 或 https:// 协议前缀)"
                disabled={!isNew}
              />
              {isNew && (
                <div style={{ display: "flex", gap: "6px", flexWrap: "wrap", marginTop: "4px" }}>
                  {[
                    { name: "百度网盘", domain: "pan.baidu.com" },
                    { name: "夸克网盘", domain: "pan.quark.cn" },
                    { name: "PikPak", domain: "mypikpak.com" },
                    { name: "B站", domain: "bilibili.com" },
                    { name: "抖音", domain: "douyin.com" },
                    { name: "YouTube", domain: "youtube.com" },
                    { name: "Twitter/X", domain: "twitter.com" },
                  ].map((p) => (
                    <button
                      key={p.domain}
                      type="button"
                      className="input-button"
                      style={{ fontSize: "11px", padding: "1px 6px" }}
                      onClick={() => onDomainChange(p.domain)}
                    >
                      {p.name}
                    </button>
                  ))}
                </div>
              )}
            </Field>
            <Field label="Cookie">
              <textarea
                rows={5}
                value={editing.cookie ?? ""}
                onChange={(e) => setEditing({ ...editing, cookie: e.target.value })}
                placeholder="输入或粘贴 Cookie 键值对内容 (多行 name=value 形式，留空表示清除)"
                style={{ width: "100%", fontFamily: "monospace" }}
              />
            </Field>
            <Field label="Referer">
              <input
                value={editing.referer ?? ""}
                onChange={(e) => setEditing({ ...editing, referer: e.target.value || null })}
                placeholder="https://example.com/ (选填，留空表示不设置)"
              />
            </Field>
            <Field label="User-Agent">
              <input
                value={editing.user_agent ?? ""}
                onChange={(e) => setEditing({ ...editing, user_agent: e.target.value || null })}
                placeholder="Mozilla/5.0 ... (选填，使用自定义 User-Agent；留空表示使用软件默认)"
              />
              <button
                type="button"
                className="input-button"
                style={{
                  fontSize: "11px",
                  padding: "2px 8px",
                  alignSelf: "flex-start",
                  marginTop: "2px",
                }}
                onClick={() =>
                  setEditing({
                    ...editing,
                    user_agent:
                      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
                  })
                }
              >
                <span>填充默认 UA</span>
              </button>
            </Field>

            <details className="category-rule-test-details" style={{ marginTop: "4px" }}>
              <summary>
                <Info size={12} />
                <span>如何获取 Cookie 与四大平台关键 Key 提示</span>
              </summary>
              <div className="category-rule-test-body" style={{ fontSize: "11px", lineHeight: "1.5" }}>
                <p style={{ margin: 0 }}>
                  <b>F12 抓包方法</b>：打开 Chrome/Edge → 登录网站 → F12 → Network 页签 → 刷新页面点任意请求 → 复制 Headers 里的 Cookie 值。
                </p>
                {editing.domain.includes("bilibili") && (
                  <p style={{ margin: "4px 0 0", color: "var(--accent)" }}>
                    💡 <b>B站关键 Key</b>：请确保包含 <code>SESSDATA</code>、<code>bili_jct</code>、<code>DedeUserID</code>。
                  </p>
                )}
                {editing.domain.includes("douyin") && (
                  <p style={{ margin: "4px 0 0", color: "var(--accent)" }}>
                    💡 <b>抖音关键 Key</b>：请确保包含 <code>sessionid</code> (或 <code>sessionid_ss</code>) 和 <code>ttwid</code>。
                  </p>
                )}
                {(editing.domain.includes("twitter") || editing.domain.includes("x.com")) && (
                  <p style={{ margin: "4px 0 0", color: "var(--accent)" }}>
                    💡 <b>Twitter/X 关键 Key</b>：请确保包含 <code>auth_token</code> 和 <code>ct0</code>。
                  </p>
                )}
                {editing.domain.includes("youtube") && (
                  <p style={{ margin: "4px 0 0", color: "var(--accent)" }}>
                    💡 <b>YouTube 关键 Key</b>：请确保包含 <code>LOGIN_INFO</code> 和 <code>SID</code>。
                  </p>
                )}
              </div>
            </details>

            {editingCheckResult && (
              <div className={`media-cred-result-box ${editingCheckResult.valid ? "valid" : "invalid"}`}>
                {editingCheckResult.valid ? <CheckCircle2 size={13} /> : <AlertCircle size={13} />}
                <span>{editingCheckResult.message}</span>
              </div>
            )}
            <div className="dialog-actions">
              <button
                type="button"
                disabled={editingChecking || !editing.domain.trim()}
                onClick={() => void checkEditingCredential()}
              >
                {editingChecking ? <LoaderCircle size={13} className="spin" /> : <ShieldCheck size={13} />}
                <span>检测凭证</span>
              </button>
              <button onClick={() => setEditing(null)}>取消</button>
              <button className="primary" onClick={() => void saveEdit()}>
                保存
              </button>
            </div>
          </div>
        </Modal>
      )}
    </SettingsGroup>
  );
}
