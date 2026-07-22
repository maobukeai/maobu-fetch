import { useState } from "react";
import { KeyRound, Globe, FileText, Check, AlertCircle, RefreshCw } from "lucide-react";
import { Modal } from "../App";
import { api } from "../api";

export interface YouTubeCredentialsModalProps {
  taskId?: string;
  onClose: () => void;
  notify: (text: string, kind?: "ok" | "error") => void;
  onSuccessRetry?: () => void;
}

export function YouTubeCredentialsModal({ taskId, onClose, notify, onSuccessRetry }: YouTubeCredentialsModalProps) {
  const [cookieText, setCookieText] = useState("");
  const [poToken, setPoToken] = useState("");
  const [saving, setSaving] = useState(false);

  const handleSaveAndRetry = async () => {
    if (!cookieText.trim() && !poToken.trim()) {
      notify("请填写 Cookie 或 PO Token", "error");
      return;
    }
    setSaving(true);
    try {
      if (cookieText.trim()) {
        await api.mediaCredentialSave({
          domain: "youtube.com",
          cookie: cookieText.trim(),
          referer: "https://www.youtube.com/",
          user_agent: null,
          updated_at: new Date().toISOString(),
        });
      }
      if (poToken.trim()) {
        const currentSettings = await api.settings();
        await api.saveSettings({
          ...currentSettings,
          youtube_po_token: poToken.trim(),
        });
      }
      notify("已成功保存 YouTube 凭证！", "ok");
      if (taskId) {
        await api.action(taskId, "retry");
        notify("已开始重新尝试下载 YouTube 视频", "ok");
      }
      onSuccessRetry?.();
      onClose();
    } catch (err) {
      notify(`保存凭证失败：${String(err)}`, "error");
    } finally {
      setSaving(false);
    }
  };

  const handleOpenFile = (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = (e) => {
      const content = e.target?.result;
      if (typeof content === "string") {
        setCookieText(content);
        notify("已读取 cookie 文件内容", "ok");
      }
    };
    reader.readAsText(file);
  };

  const handleOpenBrowser = async () => {
    try {
      await api.openExternalUrl("https://www.youtube.com");
      notify("已在浏览器中打开 YouTube。若已安装猫步插件并登录，凭证将自动同步！", "ok");
    } catch (err) {
      notify(String(err), "error");
    }
  };

  return (
    <Modal title="YouTube 凭证与反爬虫配置" onClose={onClose} style={{ width: "560px" }}>
      <div className="settings-note" style={{ marginBottom: "16px", lineHeight: "1.6" }}>
        <p style={{ display: "flex", alignItems: "center", gap: "6px", fontWeight: 600, color: "var(--fg)" }}>
          <AlertCircle size={15} color="var(--accent)" />
          为什么 YouTube 需要提供凭证？
        </p>
        <p style={{ marginTop: "4px" }}>
          YouTube 强制要求机器人校验（PO Token / 403）。提供登录 Cookie 或凭证后可恢复正常下载。
        </p>
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: "14px" }}>
        <div>
          <label style={{ display: "block", marginBottom: "6px", fontSize: "13px", fontWeight: 600 }}>
            方式一：直接粘贴 Cookie 或导入 cookies.txt 文件（推荐）
          </label>
          <textarea
            value={cookieText}
            onChange={(e) => setCookieText(e.target.value)}
            placeholder="粘贴浏览器 F12 中的 Cookie 字符串（如 VISITOR_INFO1_LIVE=...; YSC=...）或 Netscape cookies.txt 内容"
            rows={5}
            style={{
              width: "100%",
              fontFamily: "monospace",
              fontSize: "12px",
              padding: "8px 10px",
              borderRadius: "6px",
              border: "1px solid var(--border)",
              background: "var(--bg-input, rgba(0,0,0,0.03))",
              color: "var(--fg)",
              resize: "vertical",
            }}
          />
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginTop: "6px" }}>
            <label className="input-button" style={{ cursor: "pointer", display: "inline-flex", alignItems: "center", gap: "4px" }}>
              <FileText size={13} />
              选择本地 cookies.txt 文件
              <input type="file" accept=".txt" onChange={handleOpenFile} style={{ display: "none" }} />
            </label>
            {cookieText && (
              <button className="input-button" onClick={() => setCookieText("")}>
                清空 Cookie
              </button>
            )}
          </div>
        </div>

        <div style={{ borderTop: "1px solid var(--border)", paddingTop: "12px" }}>
          <label style={{ display: "block", marginBottom: "6px", fontSize: "13px", fontWeight: 600 }}>
            方式二：使用猫步浏览器扩展自动同步
          </label>
          <p style={{ fontSize: "12px", color: "var(--fg-subtle)", marginBottom: "8px" }}>
            在 Chrome/Edge 中登录 YouTube 首页，扩展会自动将凭证无缝同步至下载器：
          </p>
          <button className="input-button" onClick={() => void handleOpenBrowser()} style={{ display: "inline-flex", alignItems: "center", gap: "6px" }}>
            <Globe size={13} />
            在浏览器中打开 YouTube 首页
          </button>
        </div>

        <div style={{ borderTop: "1px solid var(--border)", paddingTop: "12px" }}>
          <label style={{ display: "block", marginBottom: "4px", fontSize: "13px", fontWeight: 600 }}>
            方式三：手动配置 YouTube PO Token（高级）
          </label>
          <input
            type="text"
            value={poToken}
            onChange={(e) => setPoToken(e.target.value)}
            placeholder="如 mweb.gvs+...（留空使用 Cookie）"
            style={{
              width: "100%",
              padding: "6px 10px",
              borderRadius: "6px",
              border: "1px solid var(--border)",
              background: "var(--bg-input, rgba(0,0,0,0.03))",
              color: "var(--fg)",
              fontSize: "12px",
            }}
          />
        </div>
      </div>

      <div className="modal-actions" style={{ marginTop: "20px", display: "flex", justifyContent: "flex-end", gap: "8px" }}>
        <button className="input-button" onClick={onClose}>
          取消
        </button>
        <button className="input-button primary" disabled={saving} onClick={() => void handleSaveAndRetry()}>
          {saving ? "保存中…" : taskId ? "保存凭证并一键重试下载" : "保存凭证"}
        </button>
      </div>
    </Modal>
  );
}
