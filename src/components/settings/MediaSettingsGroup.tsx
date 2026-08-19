import { useEffect, useRef, useState } from "react";
import { Check, Globe2, LoaderCircle, Network, Video } from "lucide-react";
import { open as pickPath } from "@tauri-apps/plugin-dialog";
import { api } from "../../api";
import type { AppSettings, ProxyAuth, ToolComponent, ToolStatus, YtDlpUpdateInfo } from "../../types";
import { formatBytes } from "../../formatters";
import { SettingRow } from "../common/FormComponents";

export function MeteredCheckButton({
  notify,
}: {
  notify: (text: string, kind?: "ok" | "error") => void;
}) {
  const [checking, setChecking] = useState(false);
  const onClick = async () => {
    if (checking) return;
    setChecking(true);
    try {
      const metered = await api.networkCheckMetered();
      notify(
        metered ? "当前为计量网络（按量计费）" : "当前不是计量网络",
        metered ? "error" : "ok"
      );
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setChecking(false);
    }
  };
  return (
    <button
      type="button"
      className="secondary-btn"
      disabled={checking}
      onClick={() => void onClick()}
    >
      {checking ? <LoaderCircle size={11} className="spin" /> : <Network size={11} />}
      <span>{checking ? "检查中…" : "立即检查"}</span>
    </button>
  );
}

export function ProxyTestButton({
  proxyUrl,
  auth,
  notify,
  disabled,
}: {
  proxyUrl: string;
  auth: ProxyAuth | null;
  notify: (text: string, kind?: "ok" | "error") => void;
  disabled?: boolean;
}) {
  const [testing, setTesting] = useState(false);
  const onClick = async () => {
    if (testing) return;
    if (!proxyUrl.trim()) {
      notify("请先填写代理地址", "error");
      return;
    }
    setTesting(true);
    try {
      const result = await api.proxyTest(proxyUrl, auth);
      if (result.success) {
        const ip = result.exit_ip ?? "未知";
        notify(`代理可用 · 出口 IP: ${ip} · 延迟 ${result.latency_ms}ms`, "ok");
      } else {
        notify(result.error ?? "代理测试失败", "error");
      }
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setTesting(false);
    }
  };
  return (
    <button
      type="button"
      className="proxy-test-btn"
      disabled={testing || disabled}
      onClick={() => void onClick()}
    >
      {testing ? <LoaderCircle size={11} className="spin" /> : <Globe2 size={11} />}
      {testing ? "测试中…" : "测试代理"}
    </button>
  );
}

export function MediaPathSettings({
  value,
  onChange,
}: {
  value: AppSettings;
  onChange: (patch: Partial<AppSettings>) => void;
}) {
  const [detecting, setDetecting] = useState(false);
  const [detectionMessage, setDetectionMessage] = useState("");
  const chooseYtDlp = async () => {
    const selected = await pickPath({
      multiple: false,
      filters: [{ name: "yt-dlp", extensions: ["exe"] }],
    });
    if (typeof selected === "string") onChange({ yt_dlp_path: selected });
  };
  const chooseFfmpeg = async () => {
    const selected = await pickPath({
      multiple: false,
      filters: [{ name: "FFmpeg", extensions: ["exe"] }],
    });
    if (typeof selected !== "string") return;
    onChange({
      ffmpeg_path: selected,
      ffprobe_path: selected.replace(/[^\\/]+$/, "ffprobe.exe"),
    });
  };
  const detectSystemTools = async () => {
    setDetecting(true);
    setDetectionMessage("");
    try {
      const detected = await api.detectSystemMediaTools();
      const hasYtDlp = Boolean(detected.yt_dlp_path);
      const hasFfmpegPair = Boolean(detected.ffmpeg_path && detected.ffprobe_path);
      const patch: Partial<AppSettings> = {};
      if (detected.yt_dlp_path) patch.yt_dlp_path = detected.yt_dlp_path;
      if (hasFfmpegPair) {
        patch.ffmpeg_path = detected.ffmpeg_path;
        patch.ffprobe_path = detected.ffprobe_path;
      }
      if (Object.keys(patch).length) onChange(patch);
      if (hasYtDlp && hasFfmpegPair)
        setDetectionMessage("已检测到 yt-dlp、FFmpeg 和 FFprobe，路径已填入下方");
      else if (hasYtDlp)
        setDetectionMessage("已检测到 yt-dlp 并填入路径；未找到完整的 FFmpeg 与 FFprobe");
      else if (hasFfmpegPair)
        setDetectionMessage("已检测到 FFmpeg 与 FFprobe 并填入路径；未找到 yt-dlp");
      else if (detected.ffmpeg_path || detected.ffprobe_path)
        setDetectionMessage(
          "只找到部分 FFmpeg 组件，需要同时存在 ffmpeg.exe 和 ffprobe.exe"
        );
      else
        setDetectionMessage(
          "未在 PATH 或常见独立安装目录中找到媒体组件，可选择本地文件或按需下载"
        );
    } catch (error) {
      setDetectionMessage(`检测失败：${String(error)}`);
    } finally {
      setDetecting(false);
    }
  };
  return (
    <div className="settings-group-content media-path-settings">
      <div className="media-detect-row">
        <div>
          <strong>自动使用系统已有组件</strong>
          <span className="media-detect-desc">扫描系统环境并自动填入对应路径</span>
        </div>
        <button
          className="input-button"
          disabled={detecting}
          onClick={() => void detectSystemTools()}
        >
          {detecting ? "检测中…" : "自动检测"}
        </button>
      </div>
      {detectionMessage && (
        <p className="media-detect-result" role="status">
          {detectionMessage}
        </p>
      )}
      <SettingRow label="自定义 yt-dlp.exe">
        <div className="input-group">
          <input
            value={value.yt_dlp_path}
            onChange={(event) => onChange({ yt_dlp_path: event.target.value })}
            placeholder="留空则自动检测"
          />
          <button className="input-button" onClick={() => void chooseYtDlp()}>
            选择文件
          </button>
          {value.yt_dlp_path && (
            <button className="input-button" onClick={() => onChange({ yt_dlp_path: "" })}>
              清除
            </button>
          )}
        </div>
      </SettingRow>
      <SettingRow label="自定义 ffmpeg.exe">
        <div className="input-group">
          <input
            value={value.ffmpeg_path}
            onChange={(event) => onChange({ ffmpeg_path: event.target.value })}
            placeholder="留空则自动检测"
          />
          <button className="input-button" onClick={() => void chooseFfmpeg()}>
            选择文件
          </button>
          {value.ffmpeg_path && (
            <button
              className="input-button"
              onClick={() => onChange({ ffmpeg_path: "", ffprobe_path: "" })}
            >
              清除
            </button>
          )}
        </div>
      </SettingRow>
      <SettingRow label="YouTube PO Token">
        <div className="input-group">
          <input
            value={value.youtube_po_token || ""}
            onChange={(event) =>
              onChange({ youtube_po_token: event.target.value })
            }
            placeholder="格式如 mweb.gvs+... 留空使用默认回退"
          />
          {value.youtube_po_token && (
            <button
              className="input-button"
              onClick={() => onChange({ youtube_po_token: "" })}
            >
              清除
            </button>
          )}
        </div>
      </SettingRow>
    </div>
  );
}

export function MediaToolsCard({
  status,
  onStatus,
  compact = false,
  required,
  ytUpdate,
}: {
  status: ToolStatus;
  onStatus: (value: ToolStatus) => void;
  compact?: boolean;
  required?: ToolComponent;
  ytUpdate?: YtDlpUpdateInfo | null;
}) {
  const components: ToolComponent[] = required ? [required] : ["yt-dlp", "ffmpeg"];
  return (
    <div className={compact ? "media-tools-stack compact" : "media-tools-stack"}>
      {components.map((component) => (
        <MediaToolComponentCard
          key={component}
          component={component}
          status={status}
          onStatus={onStatus}
          compact={compact}
          ytUpdate={ytUpdate}
        />
      ))}
    </div>
  );
}

function MediaToolComponentCard({
  component,
  status,
  onStatus,
  compact,
  ytUpdate,
}: {
  component: ToolComponent;
  status: ToolStatus;
  onStatus: (value: ToolStatus) => void;
  compact: boolean;
  ytUpdate?: YtDlpUpdateInfo | null;
}) {
  const [successMsg, setSuccessMsg] = useState("");
  const isYtDlp = component === "yt-dlp";
  const available = isYtDlp ? status.yt_dlp_available : status.ffmpeg_available;
  const operationForThis = status.active_component === component;
  const active =
    operationForThis && ["downloading", "verifying", "extracting"].includes(status.state);
  const someInstallActive =
    Boolean(status.active_component) &&
    ["downloading", "verifying", "extracting"].includes(status.state);
  const phase = operationForThis ? status.state : available ? "ready" : "missing";
  const downloadBytes = isYtDlp
    ? ytUpdate?.size_bytes || status.yt_dlp_download_bytes
    : status.ffmpeg_download_bytes;
  const installEstimate = isYtDlp ? downloadBytes : 199 * 1024 * 1024;
  const installedBytes = isYtDlp
    ? status.yt_dlp_installed_bytes
    : status.ffmpeg_installed_bytes;
  const version = isYtDlp ? status.yt_dlp_version : status.ffmpeg_version;
  const source = isYtDlp ? status.yt_dlp_source : status.ffmpeg_source;
  const sourceLabel =
    source === "custom"
      ? isYtDlp
        ? "自定义路径(更新覆盖)"
        : "自定义路径(只读保护)"
      : source === "system"
      ? "系统 PATH(只读保护)"
      : source === "bundled"
      ? "应用安装"
      : "未安装";
  const title = isYtDlp ? "yt-dlp 基础媒体组件" : "FFmpeg 高清合并组件";
  const description = isYtDlp
    ? "媒体分析、单文件视频和音频下载"
    : "最高画质音视频合并、转码与格式处理";
  const progress = status.total_bytes
    ? Math.min(100, (status.downloaded_bytes / status.total_bytes) * 100)
    : 0;
  const isLatest = isYtDlp
    ? Boolean(ytUpdate && !ytUpdate.has_update)
    : available && version.includes("8.1.2");

  const prevActiveRef = useRef(false);
  useEffect(() => {
    if (prevActiveRef.current && !active && !status.error && available) {
      if (isYtDlp && source === "custom") {
        setSuccessMsg("✓ 已成功下载并覆盖更新至自定义路径下的 yt-dlp.exe");
      } else {
        setSuccessMsg("✓ 组件已成功下载安装并投入使用");
      }
    }
    prevActiveRef.current = active;
  }, [active, status.error, available, isYtDlp, source]);

  const install = async () => {
    setSuccessMsg("");
    try {
      await api.installMediaTool(component);
      onStatus(await api.toolStatus());
    } catch (error) {
      onStatus({
        ...status,
        active_component: component,
        state: "failed",
        error: String(error),
      });
    }
  };

  const updateLatest = async () => {
    setSuccessMsg("");
    try {
      await api.updateYtDlp();
      onStatus(await api.toolStatus());
    } catch (error) {
      onStatus({
        ...status,
        active_component: component,
        state: "failed",
        error: String(error),
      });
    }
  };

  const remove = async () => {
    try {
      await api.removeMediaTool(component);
      onStatus(await api.toolStatus());
    } catch (error) {
      onStatus({
        ...status,
        active_component: component,
        state: "failed",
        error: String(error),
      });
    }
  };

  const ytUpdateLabel = ytUpdate?.has_update
    ? `更新到 ${ytUpdate.latest_version}`
    : ytUpdate
    ? "已是最新版 (重新下载)"
    : "更新到最新版";

  return (
    <div className={compact ? "media-tools-card compact" : "media-tools-card"}>
      <div className="media-tools-card-main">
        <div className="tool-summary">
          <span className={`tool-state ${phase}`}>
            {available && !active ? (
              <Check size={14} />
            ) : active ? (
              <LoaderCircle className="spin" size={14} />
            ) : (
              <Video size={14} />
            )}
          </span>
          <div>
            <strong>{title}</strong>
            <small>
              {description} · {version}
              {available
                ? source === "bundled"
                  ? ` · 应用占用 ${formatBytes(installedBytes)}`
                  : ` · 使用${sourceLabel}`
                : ` · 下载约 ${formatBytes(downloadBytes)} · 安装约 ${formatBytes(installEstimate)}`}
            </small>
          </div>
        </div>
        <div className="tool-actions">
          {active ? (
            <button onClick={() => void api.cancelMediaTools()}>取消安装</button>
          ) : available && source === "bundled" ? (
            <>
              <button
                className="danger"
                disabled={someInstallActive}
                onClick={() => void remove()}
              >
                卸载
              </button>
              {isYtDlp ? (
                <button
                  className="primary"
                  disabled={someInstallActive}
                  onClick={() => void updateLatest()}
                >
                  {ytUpdateLabel}
                </button>
              ) : (
                <button
                  className="primary"
                  disabled={someInstallActive}
                  onClick={() => void install()}
                >
                  {isLatest ? "已是最新版 (重新下载)" : "更新组件"}
                </button>
              )}
            </>
          ) : available && isYtDlp && source === "custom" ? (
            ytUpdate?.has_update ? (
              <button
                className="primary"
                disabled={someInstallActive}
                title="发现新版本，点击将下载并覆盖您的自定义路径 yt-dlp.exe"
                onClick={() => void updateLatest()}
              >
                覆盖更新自定义
              </button>
            ) : (
              <button
                className="input-button"
                disabled={someInstallActive}
                title="点击将从 GitHub 官方下载最新版并覆盖自定义文件"
                onClick={() => void updateLatest()}
              >
                {ytUpdate ? "已是最新版本 (点击重新覆盖)" : "更新自定义到最新版"}
              </button>
            )
          ) : available ? (
            <button
              disabled
              title="已使用第三方外部组件，软件将保持原样，不修改外部文件"
            >
              使用外部组件
            </button>
          ) : (
            <button
              className="primary"
              disabled={someInstallActive}
              onClick={() => void install()}
            >
              下载并安装
            </button>
          )}
        </div>
      </div>
      {active && (
        <div className="tool-progress">
          <div>
            <i style={{ width: `${progress}%` }} />
          </div>
          <span>
            {status.state === "verifying"
              ? "正在校验 SHA-256"
              : status.state === "extracting"
              ? "正在安全解压"
              : `${formatBytes(status.downloaded_bytes)} / ${formatBytes(status.total_bytes)}`}
          </span>
        </div>
      )}
      {successMsg && (
        <p
          className="tool-success"
          style={{
            color: "#10b981",
            fontSize: "12px",
            marginTop: "6px",
            fontWeight: 500,
          }}
        >
          {successMsg}
        </p>
      )}
      {operationForThis && status.error && (
        <p className="tool-error">{status.error}</p>
      )}
    </div>
  );
}

export function MediaToolsUpdateRow({
  tools,
  onStatus,
  onYtUpdate,
}: {
  tools: ToolStatus | null | undefined;
  onStatus: (status: ToolStatus) => void;
  onYtUpdate: (info: YtDlpUpdateInfo | null) => void;
}) {
  const [checking, setChecking] = useState(false);
  const [updateMsg, setUpdateMsg] = useState("");

  const handleManualCheck = async () => {
    setChecking(true);
    setUpdateMsg("");
    try {
      const currentStatus = await api.toolStatus();
      onStatus(currentStatus);

      let ytMsg: string;
      try {
        const info = await api.checkYtDlpUpdate();
        onYtUpdate(info);
        const stateTag = info.has_update
          ? "可更新"
          : currentStatus.yt_dlp_available
          ? currentStatus.yt_dlp_source === "custom"
            ? "已是最新版，使用自定义路径"
            : "已是最新版"
          : "未安装";
        ytMsg = `yt-dlp (本地版本: ${info.installed_version || "未安装"} / GitHub 最新: ${info.latest_version} · ${stateTag})`;
      } catch (error) {
        onYtUpdate(null);
        ytMsg = `yt-dlp (本地版本: ${currentStatus.yt_dlp_version || "未安装"} / 在线检查失败：${String(error)})`;
      }

      const ffLocal = currentStatus.ffmpeg_version || "未安装";
      const ffLatest = "8.1.2 essentials";
      const ffIsLatest =
        currentStatus.ffmpeg_available && ffLocal.includes("8.1.2");
      const ffSourceTag =
        currentStatus.ffmpeg_source === "system"
          ? "系统 PATH 自动检测到，已自动复用"
          : currentStatus.ffmpeg_source === "custom"
          ? "自定义路径只读保护"
          : ffIsLatest
          ? "已是最新版"
          : "未安装";
      const ffMsg = `FFmpeg (本地检测: ${ffLocal} / 软件推荐: ${ffLatest} · ${ffSourceTag})`;

      setUpdateMsg(`检测完成：${ytMsg}；${ffMsg}`);
    } catch (error) {
      setUpdateMsg(`检测失败：${String(error)}`);
    } finally {
      setChecking(false);
    }
  };

  return (
    <div
      className="settings-group-content media-path-settings"
      style={{ marginTop: "12px" }}
    >
      <div className="media-detect-row">
        <div>
          <strong>检查媒体组件更新</strong>
          <span className="media-detect-desc">
            联网检查 yt-dlp 官方最新版本并与本地对比，FFmpeg 与软件推荐版本对比
          </span>
        </div>
        <button
          className="input-button primary"
          disabled={checking || Boolean(tools?.active_component)}
          onClick={() => void handleManualCheck()}
        >
          {checking ? "检查中…" : "手动检查是否最新版"}
        </button>
      </div>
      {updateMsg && (
        <p
          className="media-detect-result"
          role="status"
          style={{ width: "100%", marginTop: "8px" }}
        >
          {updateMsg}
        </p>
      )}
    </div>
  );
}
