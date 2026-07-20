import { useState } from "react";
import {
  AlertCircle, Check, CheckCircle2, ChevronDown, ChevronRight, ExternalLink,
  HardDriveDownload, Info, LoaderCircle, Network, RefreshCw, ShieldAlert, Zap,
} from "lucide-react";
import type { PrecheckConflict, PrecheckConflictType, PrecheckResult, RedirectHop } from "../types";

const conflictTypeLabel: Record<PrecheckConflictType, string> = {
  "duplicate-url": "URL 冲突",
  "duplicate-final-url": "最终地址冲突",
  "duplicate-target-path": "目标文件冲突",
};

/** 字节转人类可读字符串，复用 App.tsx 中的算法。 */
function formatBytes(value: number): string {
  if (!value) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  return `${(value / 1024 ** index).toFixed(index ? 1 : 0)} ${units[index]}`;
}

/** 截断超长字符串到指定长度（按 Unicode 码点计数，避免半截多字节字符）。 */
function truncate(value: string, max: number): string {
  if (value.length <= max) return value;
  return `${value.slice(0, max)}…`;
}

/** 折叠/展开型条目：标签 + 单行值，点击可展开查看完整内容。 */
function CollapsibleRow({ label, value }: { label: string; value: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="precheck-row collapsible">
      <button type="button" className="precheck-collapsible-toggle" onClick={() => setOpen((v) => !v)} aria-expanded={open}>
        {open ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
        <span className="precheck-label">{label}</span>
      </button>
      <div className={`precheck-collapsible-value ${open ? "open" : ""}`} title={value}>
        {open ? value : truncate(value, 64)}
      </div>
    </div>
  );
}

function PlainRow({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <div className="precheck-row">
      <span className="precheck-label">{label}</span>
      <span className="precheck-value" title={title ?? value}>{value}</span>
    </div>
  );
}

function RedirectChainRow({ hops }: { hops: RedirectHop[] }) {
  const [open, setOpen] = useState(false);
  if (!hops.length) {
    return <PlainRow label="重定向链" value="无重定向" />;
  }
  return (
    <div className="precheck-row collapsible">
      <button type="button" className="precheck-collapsible-toggle" onClick={() => setOpen((v) => !v)} aria-expanded={open}>
        {open ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
        <span className="precheck-label">重定向链（{hops.length} 跳）</span>
      </button>
      {open && (
        <ol className="precheck-redirect-list">
          {hops.map((hop, idx) => (
            <li key={idx} className="precheck-redirect-item">
              <span className="precheck-redirect-status">{hop.status}</span>
              <span className="precheck-redirect-url" title={hop.url}>{hop.url}</span>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}

function ConflictsList({ conflicts, onLocate }: { conflicts: PrecheckConflict[]; onLocate?: (conflict: PrecheckConflict) => void }) {
  if (!conflicts.length) return null;
  return (
    <div className="precheck-conflicts">
      <div className="precheck-conflicts-header">
        <ShieldAlert size={12} />
        <span>检测到 {conflicts.length} 个冲突</span>
      </div>
      <ul className="precheck-conflicts-list">
        {conflicts.map((conflict, idx) => (
          <li key={`${conflict.existing_task_id}-${idx}`} className="precheck-conflict-item">
            <span className="precheck-conflict-type">{conflictTypeLabel[conflict.conflict_type]}</span>
            <span className="precheck-conflict-label" title={conflict.existing_task_label}>{conflict.existing_task_label}</span>
            {onLocate && (
              <button type="button" className="precheck-conflict-locate" onClick={() => onLocate(conflict)} title="定位已有任务">
                <ExternalLink size={11} />
                定位
              </button>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}

export interface PrecheckPanelProps {
  result?: PrecheckResult;
  loading: boolean;
  error?: string;
  /** 在新建对话框中点击“定位已有任务”冲突时回调（详情面板中无该按钮）。 */
  onLocateConflict?: (conflict: PrecheckConflict) => void;
  /** 在详情面板中显示“重新预检”按钮时回调。 */
  onRefresh?: () => void;
  /** 紧凑模式（详情面板中更窄）。 */
  compact?: boolean;
  /** 同一磁盘卷中已有队列任务所需的预估总空间 */
  queueDiskTotal?: number;
  /** 同一磁盘卷中大小未知的排队/下载任务数 */
  queueUnknownCount?: number;
}

/**
 * 预检结果面板：展示原 URL、重定向链、最终文件名、大小、ETag/Last-Modified、
 * Content-Type、建议连接数、磁盘空间、冲突列表、警告列表。
 *
 * 用于 NewTaskDialog（9.1）和 Details 的“预检结果”标签页（9.4）。
 */
export function PrecheckPanel({
  result,
  loading,
  error,
  onLocateConflict,
  onRefresh,
  compact,
  queueDiskTotal,
  queueUnknownCount,
}: PrecheckPanelProps) {
  const [showTechDetails, setShowTechDetails] = useState(false);

  if (loading) {
    return (
      <div className={`precheck-panel ${compact ? "compact" : ""} precheck-loading`}>
        <LoaderCircle className="spin" size={14} />
        <span>正在预检...</span>
      </div>
    );
  }
  if (error) {
    return (
      <div className={`precheck-panel ${compact ? "compact" : ""} precheck-error-state`}>
        <AlertCircle size={14} />
        <div className="precheck-error-text">
          <strong>预检失败</strong>
          <span title={error}>{truncate(error, 120)}</span>
        </div>
        <p className="precheck-error-hint">你可以修正链接后重试，或直接强制创建任务。</p>
      </div>
    );
  }
  if (!result) {
    return (
      <div className={`precheck-panel ${compact ? "compact" : ""} precheck-empty`}>
        <Info size={14} />
        <span>输入下载链接后将自动预检。</span>
      </div>
    );
  }

  const connectionHint = result.suggested_connections > 1
    ? `建议 ${result.suggested_connections} 连接`
    : result.accepts_ranges
      ? "建议单连接（文件较小）"
      : "不支持多连接，将使用单连接";

  const diskState = result.disk_state ?? (result.disk_ok ? "sufficient" : "insufficient");
  const diskStatusClass = diskState === "sufficient" ? "ok" : diskState === "unknown" ? "info" : "warn";
  const diskStatusText = diskState === "sufficient"
    ? `可用 ${formatBytes(result.available_disk_bytes)} · 本任务需 ${formatBytes(result.required_disk_bytes)}`
    : diskState === "unknown"
      ? `可用 ${formatBytes(result.available_disk_bytes)} · 大小未知（保留 ${formatBytes(result.required_disk_bytes)} 余量）`
      : `空间不足：可用 ${formatBytes(result.available_disk_bytes)} · 本任务需 ${formatBytes(result.required_disk_bytes)}`;

  return (
    <div className={`precheck-panel ${compact ? "compact" : ""}`}>
      <div className="precheck-panel-header">
        <div className="precheck-panel-title">
          <Network size={12} />
          <span>预检结果</span>
        </div>
        {onRefresh && (
          <button type="button" className="precheck-refresh" onClick={onRefresh} title="重新预检">
            <RefreshCw size={11} />
          </button>
        )}
      </div>

      <div className="precheck-rows">
        {/* 主要下载属性（永远显示） */}
        <PlainRow label="最终文件名" value={result.file_name || "（待解析）"} />
        <PlainRow label="文件大小" value={result.file_size ? formatBytes(result.file_size) : "未知"} />
        
        <div className="precheck-row precheck-connection">
          <span className="precheck-label">
            {result.suggested_connections > 1 ? <Zap size={11} /> : <Network size={11} />}
            建议连接
          </span>
          <span className="precheck-value">{connectionHint}</span>
        </div>
        <div className={`precheck-row precheck-disk ${diskStatusClass}`}>
          <span className="precheck-label">
            <HardDriveDownload size={11} />
            磁盘空间
          </span>
          <span className="precheck-value">{diskStatusText}</span>
        </div>
        {queueDiskTotal !== undefined && queueDiskTotal > 0 && (
          <PlainRow
            label="同卷队列占用"
            value={`预计 ${formatBytes(queueDiskTotal)}${queueUnknownCount ? `（另有 ${queueUnknownCount} 个任务大小未知）` : ""}`}
          />
        )}
        {result.supports_resume ? (
          <div className="precheck-row precheck-resume ok">
            <CheckCircle2 size={11} />
            <span>支持断点续传</span>
          </div>
        ) : (
          <div className="precheck-row precheck-resume warn">
            <Info size={11} />
            <span>{result.accepts_ranges ? "支持 Range，文件大小未知" : "不支持断点续传"}</span>
          </div>
        )}

        {/* 可折叠的技术性网络细节 */}
        <button
          type="button"
          className="precheck-tech-toggle"
          onClick={() => setShowTechDetails(!showTechDetails)}
          aria-expanded={showTechDetails}
        >
          {showTechDetails ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
          <span>{showTechDetails ? "隐藏技术细节" : "查看技术细节"}</span>
        </button>

        {showTechDetails && (
          <div className="precheck-tech-content" style={{ display: "flex", flexDirection: "column", gap: "2px", marginTop: "4px", borderTop: "1px dashed var(--border)", paddingTop: "6px" }}>
            <CollapsibleRow label="原始地址" value={result.original_url || "（空）"} />
            {result.final_url && result.final_url !== result.original_url && (
              <CollapsibleRow label="最终地址" value={result.final_url} />
            )}
            <RedirectChainRow hops={result.redirect_chain} />
            {result.etag && <PlainRow label="ETag" value={truncate(result.etag, 16)} title={result.etag} />}
            {result.last_modified && <PlainRow label="Last-Modified" value={truncate(result.last_modified, 16)} title={result.last_modified} />}
            {result.content_type && <PlainRow label="Content-Type" value={result.content_type} />}
          </div>
        )}
      </div>

      <ConflictsList conflicts={result.conflicts} onLocate={onLocateConflict} />

      {result.warnings.length > 0 && (
        <div className="precheck-warnings">
          <div className="precheck-warnings-header">
            <AlertCircle size={12} />
            <span>{result.warnings.length} 条警告</span>
          </div>
          <ul className="precheck-warnings-list">
            {result.warnings.map((warning, idx) => (
              <li key={idx} className="precheck-warning-item">{warning}</li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
