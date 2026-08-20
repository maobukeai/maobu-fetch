import { useState, useEffect } from "react";
import type { Pan123ShareInfo } from "../../types";
import { CloudSharePicker } from "./CloudSharePicker";
import { api } from "../../api";

export function Pan123Picker({
  shareInfo,
  selectedIds,
  onChange,
  onVerifyPassCode,
  verifyingPassCode,
  passCodeError,
}: {
  shareInfo: Pan123ShareInfo;
  selectedIds: Set<string>;
  onChange: (next: Set<string>) => void;
  onVerifyPassCode?: (passCode: string) => void;
  verifyingPassCode?: boolean;
  passCodeError?: string;
}) {
  const [showCredForm, setShowCredForm] = useState(false);
  const [credToken, setCredToken] = useState("");
  const [hasStoredCred, setHasStoredCred] = useState(false);
  const [savedMsg, setSavedMsg] = useState("");

  useEffect(() => {
    void api.mediaCredentialGet(".123pan.com").then((c) => {
      if (c && c.cookie && c.cookie.trim()) {
        setHasStoredCred(true);
        setCredToken(c.cookie.trim());
      }
    }).catch(() => {});
  }, []);

  const handleSaveCred = async () => {
    if (!credToken.trim()) return;
    try {
      await api.mediaCredentialSave({
        domain: ".123pan.com",
        cookie: credToken.trim(),
      });
      setHasStoredCred(true);
      setSavedMsg("凭证已保存！再次点击下载将自动携带鉴权");
      setTimeout(() => setSavedMsg(""), 4000);
    } catch (e) {
      setSavedMsg(`保存失败：${e}`);
    }
  };

  const totalSize = shareInfo.files.reduce((acc, f) => acc + (f.size || 0), 0);
  const fileCount = shareInfo.files.filter((f) => f.kind !== "folder").length;
  const folderCount = shareInfo.files.filter((f) => f.kind === "folder").length;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <CloudSharePicker
        platform="pan123"
        platformDisplayName="123云盘"
        themeColor="#16a34a"
        shareInfo={{
          title: shareInfo.title,
          files: shareInfo.files.map((f) => ({
            id: String(f.id),
            name: f.name,
            kind: f.kind === "folder" ? "drive#folder" : "drive#file",
            size: f.size,
            path: f.name,
            extension: f.name.includes(".") ? f.name.split(".").pop() : undefined,
            category: "file",
            mimeType: "application/octet-stream",
          })),
          fileCount,
          folderCount,
          totalSize,
          passCodeRequired: shareInfo.requires_password,
        }}
        selectedIds={selectedIds}
        onChange={onChange}
        onVerifyPassCode={onVerifyPassCode}
        verifyingPassCode={verifyingPassCode}
        passCodeError={passCodeError}
        tipText="💡 123云盘公开分享可免登录高速下载；若遇受限分享（5112），可展开下方配置凭据。"
      />

      <div
        style={{
          border: "1px solid var(--border-color, #e5e7eb)",
          borderRadius: 6,
          padding: "8px 12px",
          background: "var(--bg-subtle, rgba(0,0,0,0.02))",
          fontSize: 12,
        }}
      >
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
            cursor: "pointer",
            userSelect: "none",
          }}
          onClick={() => setShowCredForm((v) => !v)}
        >
          <span style={{ color: "var(--text-secondary, #6b7280)", display: "flex", alignItems: "center", gap: 4 }}>
            <span>🔑</span>
            <span>123云盘登录凭据配置（解决 5112 受限下载）</span>
            {hasStoredCred && (
              <span style={{ color: "#16a34a", fontSize: 11, background: "rgba(22,163,74,0.1)", padding: "1px 6px", borderRadius: 4 }}>
                已配置
              </span>
            )}
          </span>
          <span style={{ color: "var(--text-tertiary, #9ca3af)", fontSize: 11 }}>
            {showCredForm ? "收起 ▲" : "展开配置 ▼"}
          </span>
        </div>

        {showCredForm && (
          <div style={{ marginTop: 8, display: "flex", flexDirection: "column", gap: 6 }}>
            <div style={{ color: "var(--text-tertiary, #6b7280)", lineHeight: 1.4 }}>
              提示：在 123云盘 网页登录后，按 F12 在控制台或 Application/Cookie 中复制 <code>token</code>（或完整 Cookie），粘贴到下方即可解锁受限文件下载：
            </div>
            <div style={{ display: "flex", gap: 8 }}>
              <input
                type="text"
                value={credToken}
                onChange={(e) => setCredToken(e.target.value)}
                placeholder="粘贴 123云盘 Token (如 eyJhbGciOi...) 或完整 Cookie"
                style={{
                  flex: 1,
                  padding: "4px 8px",
                  borderRadius: 4,
                  border: "1px solid var(--border-color, #d1d5db)",
                  background: "var(--bg-input, #fff)",
                  color: "var(--text-primary, #111827)",
                  fontSize: 12,
                }}
              />
              <button
                type="button"
                onClick={handleSaveCred}
                disabled={!credToken.trim()}
                style={{
                  padding: "4px 12px",
                  borderRadius: 4,
                  background: "#16a34a",
                  color: "#fff",
                  border: "none",
                  cursor: credToken.trim() ? "pointer" : "not-allowed",
                  fontSize: 12,
                  whiteSpace: "nowrap",
                }}
              >
                保存凭据
              </button>
            </div>
            {savedMsg && (
              <div style={{ color: savedMsg.includes("失败") ? "#ef4444" : "#16a34a", fontSize: 11 }}>
                {savedMsg}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
