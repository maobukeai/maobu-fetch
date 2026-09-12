import { useState, useEffect } from "react";
import { Modal } from "./common/Modal";
import { api } from "../api";
import { formatBytes } from "../formatters";
import type { UpdateInfo, UpdateProgressPayload } from "../types";
import {
  Download,
  ExternalLink,
  LoaderCircle,
  Check,
  AlertCircle,
  Sparkles,
} from "lucide-react";
import { open as openUrl } from "@tauri-apps/plugin-shell";

interface UpdateModalProps {
  open: boolean;
  updateInfo: UpdateInfo;
  onClose: () => void;
}

export function UpdateModal({ open, updateInfo, onClose }: UpdateModalProps) {
  const [status, setStatus] = useState<"idle" | "downloading" | "installing" | "error">("idle");
  const [progress, setProgress] = useState<UpdateProgressPayload | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  useEffect(() => {
    if (!open) {
      setStatus("idle");
      setProgress(null);
      setErrorMessage(null);
      return;
    }

    let unlisten: (() => void) | undefined;
    void api
      .subscribeUpdateProgress((payload) => {
        if (payload.kind === "app") {
          setProgress(payload);
        }
      })
      .then((fn) => {
        unlisten = fn;
      });

    return () => {
      if (unlisten) unlisten();
    };
  }, [open]);

  if (!open) return null;

  const handleStartUpdate = async () => {
    setStatus("downloading");
    setErrorMessage(null);
    setProgress({ kind: "app", downloaded: 0, total: 0 });

    try {
      // 1. 下载并强制校验 SHA-256
      const result = await api.appUpdateDownload();

      // 2. 校验通过，立即进入极速静默安装并自动重启流程（无需人为点击）
      setStatus("installing");

      // 稍作停顿向用户展示已校验状态并提醒即将自动重启
      setTimeout(async () => {
        try {
          await api.appUpdateRunInstaller(result.path, true);
        } catch (err) {
          setStatus("error");
          setErrorMessage(`启动安装器失败：${String(err)}`);
        }
      }, 800);
    } catch (err) {
      setStatus("error");
      setErrorMessage(String(err));
    }
  };

  const handleDismiss = () => {
    if (status === "downloading" || status === "installing") return;
    try {
      sessionStorage.setItem("maobu_dismissed_update", updateInfo.version);
    } catch {}
    onClose();
  };

  const percent =
    progress && progress.total > 0
      ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100))
      : 0;

  return (
    <Modal
      title="发现新版本"
      onClose={handleDismiss}
      escapeClosable={status !== "downloading" && status !== "installing"}
      style={{ width: "480px", maxWidth: "90vw" }}
    >
      <div style={{ display: "flex", flexDirection: "column", gap: "12px", padding: "4px 0" }}>
        {/* 版本概要头 */}
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            padding: "10px 12px",
            background: "var(--bg-alt, rgba(0,0,0,0.03))",
            borderRadius: "8px",
            border: "1px solid var(--border)",
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
            <div
              style={{
                display: "inline-flex",
                alignItems: "center",
                justifyContent: "center",
                width: "28px",
                height: "28px",
                borderRadius: "6px",
                background: "var(--accent-subtle, rgba(59,130,246,0.12))",
                color: "var(--accent)",
              }}
            >
              <Sparkles size={16} />
            </div>
            <div>
              <div style={{ fontWeight: 600, fontSize: "13px", color: "var(--text)" }}>
                猫步下载器 v{updateInfo.version}
              </div>
              <div style={{ fontSize: "11px", color: "var(--muted)" }}>
                {updateInfo.release_date ? `发布于 ${updateInfo.release_date}` : "官方最新版本"}
              </div>
            </div>
          </div>
          <button
            className="input-button"
            onClick={() => {
              const targetUrl = updateInfo.download_url || "https://github.com/maobukeai/maobu-fetch/releases";
              void openUrl(targetUrl).catch(() => {});
            }}
            title="在浏览器中查看 GitHub Release 页面"
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: "4px",
              height: "24px",
              padding: "0 8px",
              fontSize: "11px",
              background: "transparent",
              color: "var(--muted)",
              border: "1px solid var(--border)",
              borderRadius: "4px",
              cursor: "pointer",
            }}
          >
            <ExternalLink size={11} />
            网页说明
          </button>
        </div>

        {/* 更新日志 */}
        {updateInfo.release_notes ? (
          <div>
            <div style={{ fontSize: "11px", fontWeight: 500, color: "var(--text)", marginBottom: "4px" }}>
              更新日志：
            </div>
            <div
              style={{
                maxHeight: "150px",
                overflowY: "auto",
                whiteSpace: "pre-wrap",
                fontSize: "11px",
                lineHeight: 1.6,
                color: "var(--text)",
                padding: "8px 10px",
                background: "var(--card-bg, rgba(0,0,0,0.02))",
                borderRadius: "6px",
                border: "1px solid var(--border)",
              }}
            >
              {updateInfo.release_notes}
            </div>
          </div>
        ) : null}

        {/* 错误提示 */}
        {status === "error" && errorMessage && (
          <div
            style={{
              display: "flex",
              alignItems: "flex-start",
              gap: "6px",
              padding: "8px 10px",
              borderRadius: "6px",
              background: "rgba(239, 68, 68, 0.08)",
              border: "1px solid rgba(239, 68, 68, 0.2)",
              color: "#ef4444",
              fontSize: "11px",
            }}
          >
            <AlertCircle size={14} style={{ flexShrink: 0, marginTop: "1px" }} />
            <div style={{ flex: 1, wordBreak: "break-all" }}>{errorMessage}</div>
          </div>
        )}

        {/* 下载中进度 */}
        {status === "downloading" && (
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              gap: "6px",
              padding: "10px",
              background: "var(--bg-alt, rgba(0,0,0,0.03))",
              borderRadius: "6px",
              border: "1px solid var(--border)",
            }}
          >
            <div style={{ display: "flex", justifyContent: "space-between", fontSize: "11px" }}>
              <span style={{ display: "inline-flex", alignItems: "center", gap: "6px", color: "var(--text)" }}>
                <LoaderCircle size={12} className="spin" />
                正在下载更新安装包...
              </span>
              <span style={{ fontWeight: 600, color: "var(--accent)" }}>{percent}%</span>
            </div>
            <div
              style={{
                height: "6px",
                borderRadius: "3px",
                background: "var(--border)",
                overflow: "hidden",
              }}
            >
              <div
                style={{
                  height: "100%",
                  width: `${percent}%`,
                  background: "var(--accent)",
                  transition: "width 0.2s ease-out",
                }}
              />
            </div>
            <div style={{ display: "flex", justifyContent: "space-between", fontSize: "10px", color: "var(--muted)" }}>
              <span>
                {progress ? formatBytes(progress.downloaded) : "0 B"}
                {progress && progress.total > 0 ? ` / ${formatBytes(progress.total)}` : ""}
              </span>
              <span>下载完成后将自动静默安装并重启</span>
            </div>
          </div>
        )}

        {/* 安装中状态 */}
        {status === "installing" && (
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "8px",
              padding: "10px 12px",
              borderRadius: "6px",
              background: "rgba(34, 197, 94, 0.08)",
              border: "1px solid rgba(34, 197, 94, 0.25)",
              color: "#16a34a",
              fontSize: "12px",
              fontWeight: 500,
            }}
          >
            <Check size={16} style={{ flexShrink: 0 }} />
            <div>安装包 SHA-256 校验通过，正在极速安装并自动重启...</div>
          </div>
        )}

        {/* 操作按钮栏 */}
        <div
          style={{
            display: "flex",
            justifyContent: "flex-end",
            gap: "8px",
            marginTop: "6px",
            paddingTop: "10px",
            borderTop: "1px solid var(--border)",
          }}
        >
          {status === "idle" || status === "error" ? (
            <>
              <button
                className="input-button"
                onClick={handleDismiss}
                style={{
                  height: "28px",
                  padding: "0 14px",
                  fontSize: "12px",
                  borderRadius: "6px",
                  border: "1px solid var(--border)",
                  background: "transparent",
                  color: "var(--text)",
                  cursor: "pointer",
                }}
              >
                稍后提醒
              </button>
              <button
                className="input-button"
                onClick={() => void handleStartUpdate()}
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  gap: "6px",
                  height: "28px",
                  padding: "0 16px",
                  fontSize: "12px",
                  fontWeight: 500,
                  borderRadius: "6px",
                  border: "1px solid var(--accent)",
                  background: "var(--accent)",
                  color: "white",
                  cursor: "pointer",
                }}
              >
                <Download size={13} />
                极速更新（无需点击）
              </button>
            </>
          ) : status === "downloading" ? (
            <button
              className="input-button"
              disabled
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: "6px",
                height: "28px",
                padding: "0 16px",
                fontSize: "12px",
                fontWeight: 500,
                borderRadius: "6px",
                border: "1px solid var(--border)",
                background: "var(--bg-alt, rgba(0,0,0,0.05))",
                color: "var(--muted)",
                cursor: "not-allowed",
              }}
            >
              <LoaderCircle size={12} className="spin" />
              正在更新中…
            </button>
          ) : (
            <button
              className="input-button"
              disabled
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: "6px",
                height: "28px",
                padding: "0 16px",
                fontSize: "12px",
                fontWeight: 500,
                borderRadius: "6px",
                border: "1px solid rgba(34,197,94,0.3)",
                background: "rgba(34,197,94,0.1)",
                color: "#16a34a",
                cursor: "wait",
              }}
            >
              正在重启…
            </button>
          )}
        </div>
      </div>
    </Modal>
  );
}
