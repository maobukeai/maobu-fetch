import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  AlertCircle,
  ArrowLeft,
  Bookmark,
  Check,
  Copy,
  Download,
  ExternalLink,
  File,
  FolderOpen,
  Globe2,
  Heart,
  Info,
  Keyboard,
  ListFilter,
  LoaderCircle,
  Magnet,
  MessageCircle,
  Network,
  RefreshCw,
  Settings,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Tag as TagIcon,
  Video,
  Zap,
} from "lucide-react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { open as pickPath, save as savePath } from "@tauri-apps/plugin-dialog";
import { open as openUrl } from "@tauri-apps/plugin-shell";
import contactQr from "../../assets/contact_qr.webp";
import sponsorQr from "../../assets/sponsor_qr.webp";
import { api, isDesktop } from "../../api";
import { setLocale, t, useLocale } from "../../i18n";
import type {
  AppInfo,
  AppSettings,
  ColorScheme,
  ExtensionCompatibilityResult,
  ExtensionUpdateResult,
  PairingInfo,
  PlatformCompatibility,
  ToolStatus,
  UpdateCheckResult,
  UpdateDownloadResult,
  UpdateProgressPayload,
  YtDlpUpdateInfo,
} from "../../types";
import { supportLevelColor, supportLevelLabel } from "../../types";
import {
  DEFAULT_SHORTCUTS,
  formatBytes,
  hhmmToMinutes,
  minutesToHHMM,
} from "../../formatters";
import { BackupRestoreModal } from "../BackupRestoreModal";
import { BtSettingsSection } from "../BtSettingsSection";
import { CompletionActionEditor } from "../CompletionActionEditor";
import { Select } from "../Select";
import { CatDownloadMark } from "../common/EmptyState";
import { Modal } from "../common/Modal";
import { SettingRow, SettingsGroup, Toggle } from "../common/FormComponents";
import {
  applyWindowAppearance,
  usesDarkTheme,
} from "../common/Titlebar";
import { RetryPolicyEditor } from "../details/RetryPolicyEditor";
import { CategoryRulesPanel } from "./CategoryRulesPanel";
import { FilenameCleanupPanel } from "./FilenameCleanupPanel";
import { MediaCredentialsPanel } from "./MediaCredentialsPanel";
import {
  MediaPathSettings,
  MediaToolsCard,
  MediaToolsUpdateRow,
  MeteredCheckButton,
  ProxyTestButton,
} from "./MediaSettingsGroup";
import { PlatformNamingTemplatePanel } from "./PlatformNamingTemplatePanel";
import { PresetsPanel } from "./PresetsPanel";
import { ShortcutSettingsSection } from "./ShortcutSettingsSection";
import { TagManagementPanel } from "./TagManagementPanel";
import { TaskTemplatesPanel } from "./TaskTemplatesPanel";

export type SettingsSection =
  | "general"
  | "download"
  | "network"
  | "bt"
  | "browser"
  | "media"
  | "rules"
  | "filename-cleanup"
  | "naming-template"
  | "presets"
  | "templates"
  | "tags"
  | "credentials"
  | "appearance"
  | "shortcuts"
  | "advanced"
  | "about";

export function SettingsPage({
  value,
  onChange,
  onClose,
  notify,
  totalSpeed = 0,
  activeCount = 0,
}: {
  value: AppSettings;
  onChange: (value: AppSettings) => void;
  onClose: () => void;
  notify: (text: string, kind?: "ok" | "error") => void;
  totalSpeed?: number;
  activeCount?: number;
}) {
  useLocale();
  const appWindow = useMemo(() => (isDesktop() ? getCurrentWindow() : null), []);
  const [draft, setDraft] = useState(value);
  const [section, setSection] = useState<SettingsSection>("general");
  const [pair, setPair] = useState<PairingInfo>();
  const [tools, setTools] = useState<ToolStatus>();
  const [ytUpdate, setYtUpdate] = useState<YtDlpUpdateInfo | null>(null);
  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateResult, setUpdateResult] = useState<UpdateCheckResult | null>(null);
  const [extVersion, setExtVersion] = useState("");
  const [extChecking, setExtChecking] = useState(false);
  const [extResult, setExtResult] = useState<ExtensionCompatibilityResult | null>(null);
  const [appUpdateBusy, setAppUpdateBusy] = useState(false);
  const [appUpdateProgress, setAppUpdateProgress] = useState<UpdateProgressPayload | null>(null);
  const [appUpdateReady, setAppUpdateReady] = useState<UpdateDownloadResult | null>(null);
  const [extUpdateBusy, setExtUpdateBusy] = useState(false);
  const [extUpdateProgress, setExtUpdateProgress] = useState<UpdateProgressPayload | null>(null);
  const [extUpdateResult, setExtUpdateResult] = useState<ExtensionUpdateResult | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void api
      .subscribeUpdateProgress((payload) => {
        if (payload.kind === "app") setAppUpdateProgress(payload);
        else setExtUpdateProgress(payload);
      })
      .then((item) => {
        unlisten = item;
      });
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  useEffect(() => {
    let cancelled = false;
    api
      .appGetInfo()
      .then((info) => {
        if (!cancelled) setAppInfo(info);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  const [platformCompatList, setPlatformCompatList] = useState<PlatformCompatibility[]>([]);
  useEffect(() => {
    let cancelled = false;
    api
      .platformCompatibilityList()
      .then((list) => {
        if (!cancelled) setPlatformCompatList(list ?? []);
      })
      .catch(() => {
        if (!cancelled) setPlatformCompatList([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const checkAppUpdate = async () => {
    setUpdateChecking(true);
    try {
      const result = await api.appCheckUpdate();
      setUpdateResult(result);
      if (result.error) notify(result.error, "error");
      else if (result.has_update) notify("发现新版本，请前往 GitHub 获取更新");
      else notify("当前已是最新版本");
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setUpdateChecking(false);
    }
  };

  const checkExtCompat = async () => {
    const trimmed = extVersion.trim();
    if (!trimmed) {
      notify("请先填写扩展版本号", "error");
      return;
    }
    setExtChecking(true);
    try {
      const result = await api.extensionCheckCompatibility(trimmed);
      setExtResult(result);
      notify(
        result.compatible
          ? "扩展与桌面端版本兼容"
          : result.message || "扩展版本不兼容",
        result.compatible ? "ok" : "error"
      );
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setExtChecking(false);
    }
  };

  const runAppUpdateDownload = async () => {
    setAppUpdateBusy(true);
    setAppUpdateReady(null);
    setAppUpdateProgress({ kind: "app", downloaded: 0, total: 0 });
    try {
      const result = await api.appUpdateDownload();
      setAppUpdateReady(result);
      notify(`v${result.version} 安装包已下载并校验通过`);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setAppUpdateBusy(false);
      setAppUpdateProgress(null);
    }
  };

  const runAppUpdateInstaller = async () => {
    if (!appUpdateReady) return;
    try {
      await api.appUpdateRunInstaller(appUpdateReady.path);
      notify("安装程序已启动，请按提示完成安装");
      setAppUpdateReady(null);
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const runExtensionUpdate = async () => {
    setExtUpdateBusy(true);
    setExtUpdateResult(null);
    setExtUpdateProgress({ kind: "extension", downloaded: 0, total: 0 });
    try {
      const result = await api.extensionUpdateDownload();
      setExtUpdateResult(result);
      notify(`扩展 v${result.version} 已就绪`);
    } catch (error) {
      notify(String(error), "error");
    } finally {
      setExtUpdateBusy(false);
      setExtUpdateProgress(null);
    }
  };

  const exportTasks = async () => {
    try {
      const path = await savePath({
        defaultPath: "maobu-tasks.json",
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!path) return;
      const count = await api.exportTasks(path);
      notify(`已安全导出 ${count} 个任务`);
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const importTasks = async () => {
    try {
      const path = await pickPath({
        multiple: false,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (typeof path !== "string") return;
      const destination = await pickPath({
        directory: true,
        multiple: false,
        title: "选择导入任务的下载目录",
      });
      if (typeof destination !== "string") return;
      const tasks = await api.importTasks(path, destination);
      notify(`已导入 ${tasks.length} 个任务，均保持暂停`);
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const openLogsDir = async () => {
    try {
      await api.openLogsDir();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const exportRecentLogs = async () => {
    try {
      const path = await savePath({
        defaultPath: "maobu-logs.txt",
        filters: [{ name: "日志", extensions: ["log", "txt"] }],
      });
      if (!path) return;
      const count = await api.exportRecentLogs(path);
      notify(`已导出 ${count} 个日志文件（已脱敏）`);
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const [backupOpen, setBackupOpen] = useState(false);
  const [restoreOpen, setRestoreOpen] = useState(false);
  const set = <K extends keyof AppSettings>(key: K, val: AppSettings[K]) =>
    setDraft((item) => ({ ...item, [key]: val }));
  const setColorScheme = (scheme: ColorScheme) =>
    setDraft((item) => ({ ...item, color_scheme: scheme, theme: scheme }));

  const hasSaved = useRef(false);
  const originalSize = useRef<{ width: number; height: number } | null>(null);
  const draftRef = useRef(draft);
  useEffect(() => {
    draftRef.current = draft;
  }, [draft]);

  useEffect(() => {
    if (!appWindow) return;
    void Promise.all([appWindow.outerSize(), appWindow.scaleFactor()]).then(
      ([size, factor]) => {
        originalSize.current = {
          width: Math.round(size.width / factor),
          height: Math.round(size.height / factor),
        };
      }
    );

    return () => {
      const currentDraft = draftRef.current;
      const hasChangedSize =
        currentDraft.window_width !== value.window_width ||
        currentDraft.window_height !== value.window_height;
      if (!hasSaved.current && hasChangedSize && originalSize.current) {
        void appWindow.setSize(
          new LogicalSize(originalSize.current.width, originalSize.current.height)
        );
      }
    };
  }, [appWindow, value.window_width, value.window_height]);

  const applyTemporarySize = (w: number, h: number) => {
    if (appWindow) {
      void appWindow.setSize(new LogicalSize(w, h));
    }
  };

  const changeWidth = (val: number | undefined) => {
    set("window_width", val);
    if (val && draft.window_height) {
      applyTemporarySize(val, draft.window_height);
    }
  };

  const changeHeight = (val: number | undefined) => {
    set("window_height", val);
    if (draft.window_width && val) {
      applyTemporarySize(draft.window_width, val);
    }
  };

  const [cacheSizeBytes, setCacheSizeBytes] = useState<number | null>(null);
  const [cacheSizeLoading, setCacheSizeLoading] = useState(false);
  const [cacheCleaning, setCacheCleaning] = useState(false);
  const [qrModal, setQrModal] = useState<"contact" | "sponsor" | null>(null);

  const handleInspectCache = useCallback(() => {
    setCacheSizeLoading(true);
    api
      .cacheInspect()
      .then((res) => setCacheSizeBytes(res.total_bytes))
      .catch((err) => notify(String(err), "error"))
      .finally(() => setCacheSizeLoading(false));
  }, [notify]);

  const handleClearCache = useCallback(() => {
    setCacheCleaning(true);
    api
      .cacheClear()
      .then((res) => {
        notify(`清理完成，已释放 ${formatBytes(res.freed_bytes)} 磁盘空间`);
        setCacheSizeBytes(0);
      })
      .catch((err) => notify(String(err), "error"))
      .finally(() => setCacheCleaning(false));
  }, [notify]);

  useEffect(() => {
    if (section === "advanced" && cacheSizeBytes === null) {
      handleInspectCache();
    }
  }, [section, cacheSizeBytes, handleInspectCache]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    if (section === "browser") void api.pairing().then(setPair);
    if (section === "media") {
      void api.toolStatus().then(setTools);
      void api.subscribeMediaTools(setTools).then((value) => {
        unlisten = value;
      });
    }
    return () => unlisten?.();
  }, [section]);

  useEffect(() => {
    const applyDraftColorScheme = () => {
      const dark = usesDarkTheme(draft.color_scheme);
      document.documentElement.dataset.theme = dark ? "dark" : "light";
      document.documentElement.dataset.accent = draft.accent_color;
      document.body.classList.toggle("dark", dark);
      document.body.classList.toggle("light", !dark);
      void applyWindowAppearance(draft.frosted_glass, dark).catch((error) => {
        document.documentElement.dataset.windowStyle = "solid";
        notify(`无法预览磨砂玻璃效果：${String(error)}`, "error");
      });
    };
    applyDraftColorScheme();
    if (draft.color_scheme !== "system") return;
    const media = matchMedia("(prefers-color-scheme: dark)");
    media.addEventListener("change", applyDraftColorScheme);
    return () => media.removeEventListener("change", applyDraftColorScheme);
  }, [draft.color_scheme, draft.accent_color, draft.frosted_glass]);

  useEffect(() => {
    document.body.classList.toggle("row-compact", draft.row_compact);
  }, [draft.row_compact]);

  useEffect(() => {
    const applyDraftScale = () => {
      if (draft.auto_scale_ui) {
        const baseWidth = 1024;
        const scale = window.outerWidth / baseWidth;
        const clampedScale = Math.min(Math.max(scale, 0.75), 2.0);
        document.documentElement.style.zoom = String(clampedScale);
      } else {
        document.documentElement.style.zoom = "";
      }
    };
    applyDraftScale();
    window.addEventListener("resize", applyDraftScale);
    return () => {
      window.removeEventListener("resize", applyDraftScale);
    };
  }, [draft.auto_scale_ui]);

  useEffect(() => {
    return () => {
      const finalSettings = hasSaved.current ? draftRef.current : value;
      setLocale(finalSettings.language || "zh-CN");
      const dark = usesDarkTheme(finalSettings.color_scheme);
      document.documentElement.dataset.theme = dark ? "dark" : "light";
      document.documentElement.dataset.accent = finalSettings.accent_color;
      document.body.classList.toggle("dark", dark);
      document.body.classList.toggle("light", !dark);
      document.body.classList.toggle("row-compact", finalSettings.row_compact);
      void applyWindowAppearance(finalSettings.frosted_glass, dark);
      if (finalSettings.auto_scale_ui) {
        const scale = window.outerWidth / 1024;
        const clampedScale = Math.min(Math.max(scale, 0.75), 2.0);
        document.documentElement.style.zoom = String(clampedScale);
      } else {
        document.documentElement.style.zoom = "";
      }
    };
  }, [value]);

  const save = async () => {
    try {
      await api.saveSettings(draft);
      hasSaved.current = true;
      onChange(draft);
      notify(t("toasts.settingsSaved"));
      onClose();
    } catch (error) {
      notify(String(error), "error");
    }
  };

  const items: Array<[SettingsSection, string, typeof Settings]> = [
    ["general", t("settings.sectionGeneral"), Settings],
    ["download", t("settings.sectionDownload"), Download],
    ["network", t("settings.sectionNetwork"), Network],
    ["bt", t("settings.sectionBt"), Magnet],
    ["browser", t("settings.sectionBrowser"), Globe2],
    ["media", t("settings.sectionMedia"), Video],
    ["rules", t("settings.sectionRules"), ListFilter],
    ["filename-cleanup", t("settings.sectionFilenameCleanup"), Sparkles],
    ["naming-template", t("settings.sectionNamingTemplate"), File],
    ["presets", t("settings.sectionPresets"), Zap],
    ["templates", t("settings.sectionTemplates"), Bookmark],
    ["tags", t("settings.sectionTags"), TagIcon],
    ["credentials", t("settings.sectionCredentials"), ShieldCheck],
    ["appearance", t("settings.sectionAppearance"), SlidersHorizontal],
    ["shortcuts", t("settings.sectionShortcuts"), Keyboard],
    ["advanced", t("settings.sectionAdvanced"), Info],
    ["about", t("settings.sectionAbout"), Info],
  ];

  return (
    <div className="settings-page">
      <aside className="nav-pane">
        <div className="brand" data-tauri-drag-region>
          {t("settings.title")}
        </div>
        <div className="settings-nav-list">
          {items.map(([key, label, Icon]) => (
            <button
              key={key}
              className={section === key ? "nav-item active" : "nav-item"}
              onClick={() => setSection(key)}
            >
              <Icon size={15} />
              <span>{label}</span>
            </button>
          ))}
        </div>
        <div className="nav-footer">
          <button
            className="nav-settings"
            onClick={onClose}
            title={t("settings.returnHome")}
          >
            <ArrowLeft size={15} />
            <span>{t("settings.returnHome")}</span>
          </button>
          <div className="nav-status" style={{ cursor: "default" }}>
            <i className={isDesktop() ? "status-dot online" : "status-dot offline"} />
            <span>
              {t("nav.speedFormat", {
                speed: `${formatBytes(totalSpeed)}/s`,
                count: activeCount,
              })}
            </span>
          </div>
        </div>
      </aside>
      <main className="settings-body" data-tauri-drag-region>
        <div className="settings-title" data-tauri-drag-region>
          <h1 data-tauri-drag-region>
            {items.find(([key]) => key === section)?.[1]}
          </h1>
        </div>
        <div className="settings-content">
          {section === "general" && (
            <>
              <SettingsGroup title={t("settings.groupLanguage")}>
                <div className="settings-group-content">
                  <SettingRow label={t("settings.languageLabel")}>
                    <Select
                      value={draft.language || "zh-CN"}
                      onChange={(nextVal) => {
                        const next = String(nextVal);
                        set("language", next);
                        setLocale(next);
                      }}
                      options={[
                        { value: "zh-CN", label: t("settings.languageZhCN") },
                        { value: "en", label: t("settings.languageEn") },
                      ]}
                      ariaLabel={t("settings.languageLabel")}
                    />
                  </SettingRow>
                </div>
                <p className="settings-note">{t("settings.languageHint")}</p>
              </SettingsGroup>
              <SettingsGroup title={t("settings.groupAppBehavior")}>
                <div className="settings-group-content">
                  <Toggle
                    label={t("settings.autoStart")}
                    checked={draft.auto_start}
                    onChange={(v) => set("auto_start", v)}
                  />
                  <Toggle
                    label={t("settings.startMinimized")}
                    checked={draft.start_minimized}
                    onChange={(v) => set("start_minimized", v)}
                  />
                  <Toggle
                    label={t("settings.minimizeToTray")}
                    checked={draft.minimize_to_tray}
                    onChange={(v) => set("minimize_to_tray", v)}
                  />
                  <Toggle
                    label={t("settings.closeToTray")}
                    checked={draft.close_to_tray}
                    onChange={(v) => set("close_to_tray", v)}
                  />
                  <Toggle
                    label={t("settings.notifyComplete")}
                    checked={draft.notifications}
                    onChange={(v) => set("notifications", v)}
                  />
                  <Toggle
                    label={t("settings.monitorClipboard")}
                    checked={draft.clipboard_monitor}
                    onChange={(v) => set("clipboard_monitor", v)}
                  />
                </div>
              </SettingsGroup>
              <SettingsGroup title={t("settings.groupNotifications")}>
                <div className="settings-group-content">
                  <Toggle
                    label={t("settings.notifyOnComplete")}
                    checked={draft.notify_on_complete}
                    onChange={(v) => set("notify_on_complete", v)}
                  />
                  <Toggle
                    label={t("settings.notifyOnFailure")}
                    checked={draft.notify_on_failure}
                    onChange={(v) => set("notify_on_failure", v)}
                  />
                  <Toggle
                    label={t("settings.notifySoundEnabled")}
                    checked={draft.notify_sound_enabled}
                    onChange={(v) => set("notify_sound_enabled", v)}
                  />
                  <Toggle
                    label={t("settings.notifyFailureSoundEnabled")}
                    checked={draft.notify_failure_sound_enabled}
                    onChange={(v) => set("notify_failure_sound_enabled", v)}
                  />
                </div>
                <p className="settings-note">{t("settings.notifySoundDesc")}</p>
              </SettingsGroup>
              <SettingsGroup title={t("settings.groupHistoryArchive")}>
                <div className="settings-group-content">
                  <SettingRow label={t("settings.archiveDaysLabel")}>
                    <input
                      type="number"
                      min="0"
                      max="3650"
                      value={draft.archive_days}
                      onChange={(e) =>
                        set("archive_days", Math.max(0, +e.target.value || 0))
                      }
                    />
                  </SettingRow>
                  <SettingRow label={t("settings.archiveThresholdLabel")}>
                    <input
                      type="number"
                      min="0"
                      max="100000"
                      value={draft.archive_threshold}
                      onChange={(e) =>
                        set(
                          "archive_threshold",
                          Math.max(0, +e.target.value || 0)
                        )
                      }
                    />
                  </SettingRow>
                </div>
                <p className="settings-note">{t("settings.archiveDesc")}</p>
              </SettingsGroup>
            </>
          )}
          {section === "download" && (
            <SettingsGroup title={t("settings.groupSavePerformance")}>
              <div className="settings-group-content">
                <SettingRow label={t("settings.downloadDirLabel")}>
                  <input
                    value={draft.download_dir}
                    onChange={(e) => set("download_dir", e.target.value)}
                  />
                </SettingRow>
                <SettingRow label={t("settings.collisionLabel")}>
                  <div className="fluent-segmented-control settings-segmented">
                    <button
                      type="button"
                      className={
                        draft.default_collision_policy === "rename"
                          ? "active"
                          : ""
                      }
                      onClick={() => set("default_collision_policy", "rename")}
                    >
                      {t("settings.collisionRename")}
                    </button>
                    <button
                      type="button"
                      className={
                        draft.default_collision_policy === "overwrite"
                          ? "active"
                          : ""
                      }
                      onClick={() =>
                        set("default_collision_policy", "overwrite")
                      }
                    >
                      {t("settings.collisionOverwrite")}
                    </button>
                    <button
                      type="button"
                      className={
                        draft.default_collision_policy === "skip"
                          ? "active"
                          : ""
                      }
                      onClick={() => set("default_collision_policy", "skip")}
                    >
                      {t("settings.collisionSkip")}
                    </button>
                  </div>
                </SettingRow>
                <SettingRow label={t("settings.completionDefaultLabel")}>
                  <div className="setting-completion-action">
                    <CompletionActionEditor
                      value={draft.default_completion_action}
                      onChange={(a) => set("default_completion_action", a)}
                    />
                  </div>
                </SettingRow>
                <Toggle
                  label={t("settings.lowMemoryLabel")}
                  checked={draft.low_memory_mode}
                  onChange={(v) => set("low_memory_mode", v)}
                />
                <SettingRow label={t("settings.concurrentLabel")}>
                  <input
                    type="number"
                    min="1"
                    max="16"
                    value={draft.concurrent_downloads}
                    onChange={(e) =>
                      set("concurrent_downloads", +e.target.value)
                    }
                  />
                </SettingRow>
                <SettingRow
                  label={t("settings.connectionsPerTaskLabel", {
                    count: draft.connections_per_download,
                  })}
                >
                  <div className="settings-slider-wrapper">
                    <input
                      type="range"
                      min="0"
                      max="5"
                      step="1"
                      value={[1, 2, 4, 8, 16, 32].indexOf(
                        draft.connections_per_download
                      )}
                      onChange={(e) => {
                        const values = [1, 2, 4, 8, 16, 32];
                        set("connections_per_download", values[+e.target.value]);
                      }}
                      className="fluent-slider"
                    />
                    <div className="slider-ticks">
                      <span>1</span>
                      <span>2</span>
                      <span>4</span>
                      <span>8</span>
                      <span>16</span>
                      <span>32</span>
                    </div>
                  </div>
                </SettingRow>
                <SettingRow label={t("settings.globalLimitLabel")}>
                  <input
                    type="number"
                    min="0"
                    value={draft.speed_limit_kbps}
                    onChange={(e) => set("speed_limit_kbps", +e.target.value)}
                  />
                </SettingRow>
                <Toggle
                  label={t("settings.scheduledLimitLabel")}
                  checked={draft.scheduled_limit?.enabled ?? false}
                  onChange={(v) =>
                    set("scheduled_limit", {
                      enabled: v,
                      start_minutes: draft.scheduled_limit?.start_minutes ?? 540,
                      end_minutes: draft.scheduled_limit?.end_minutes ?? 1080,
                      limit_kbps: draft.scheduled_limit?.limit_kbps ?? 2048,
                    })
                  }
                />
                {draft.scheduled_limit?.enabled && (
                  <>
                    <SettingRow label={t("settings.scheduledLimitRange")}>
                      <div
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: "8px",
                        }}
                      >
                        <input
                          type="time"
                          aria-label={t("settings.scheduledLimitStartAria")}
                          value={minutesToHHMM(
                            draft.scheduled_limit.start_minutes
                          )}
                          onChange={(e) =>
                            set("scheduled_limit", {
                              ...draft.scheduled_limit!,
                              enabled: true,
                              start_minutes: hhmmToMinutes(e.target.value),
                            })
                          }
                        />
                        <span>{t("settings.scheduledLimitRangeTo")}</span>
                        <input
                          type="time"
                          aria-label={t("settings.scheduledLimitEndAria")}
                          value={minutesToHHMM(
                            draft.scheduled_limit.end_minutes
                          )}
                          onChange={(e) =>
                            set("scheduled_limit", {
                              ...draft.scheduled_limit!,
                              enabled: true,
                              end_minutes: hhmmToMinutes(e.target.value),
                            })
                          }
                        />
                      </div>
                    </SettingRow>
                    <SettingRow label={t("settings.scheduledLimitValue")}>
                      <input
                        type="number"
                        min="0"
                        value={draft.scheduled_limit.limit_kbps}
                        onChange={(e) =>
                          set("scheduled_limit", {
                            ...draft.scheduled_limit!,
                            enabled: true,
                            limit_kbps: Math.max(0, +e.target.value || 0),
                          })
                        }
                      />
                    </SettingRow>
                  </>
                )}
                <Toggle
                  label={t("settings.verifyShaLabel")}
                  checked={draft.verify_after_download}
                  onChange={(v) => set("verify_after_download", v)}
                />
              </div>
              <p className="settings-note">{t("settings.lowMemoryNote")}</p>
            </SettingsGroup>
          )}
          {section === "network" && (
            <>
              <SettingsGroup title={t("settings.netProxyGroup")}>
                <div className="settings-group-content">
                  <SettingRow label={t("settings.netProxyMode")}>
                    <Select
                      value={draft.proxy_mode}
                      onChange={(val: any) =>
                        set("proxy_mode", val as AppSettings["proxy_mode"])
                      }
                      options={[
                        {
                          value: "system",
                          label: t("settings.netProxyModeSystem"),
                        },
                        {
                          value: "none",
                          label: t("settings.netProxyModeNone"),
                        },
                        {
                          value: "manual",
                          label: t("settings.netProxyModeManual"),
                        },
                      ]}
                      ariaLabel={t("settings.netProxyMode")}
                    />
                  </SettingRow>
                  {draft.proxy_mode === "manual" && (
                    <>
                      <SettingRow label={t("settings.netProxyAddressLabel")}>
                        <input
                          value={draft.proxy_url}
                          onChange={(e) => set("proxy_url", e.target.value)}
                          placeholder={t(
                            "settings.netProxyAddressPlaceholder"
                          )}
                        />
                      </SettingRow>
                      <SettingRow label={t("settings.netProxyUsername")}>
                        <input
                          value={draft.proxy_username}
                          onChange={(e) =>
                            set("proxy_username", e.target.value)
                          }
                          placeholder={t(
                            "settings.netProxyUsernamePlaceholder"
                          )}
                        />
                      </SettingRow>
                      <SettingRow label={t("settings.netProxyPassword")}>
                        <input
                          type="password"
                          value={draft.proxy_password}
                          onChange={(e) =>
                            set("proxy_password", e.target.value)
                          }
                          placeholder={t(
                            "settings.netProxyPasswordPlaceholder"
                          )}
                        />
                      </SettingRow>
                      <SettingRow label={t("settings.netTestConnectivity")}>
                        <ProxyTestButton
                          proxyUrl={draft.proxy_url}
                          auth={
                            draft.proxy_username || draft.proxy_password
                              ? {
                                  username: draft.proxy_username,
                                  password: draft.proxy_password,
                                }
                              : null
                          }
                          notify={notify}
                        />
                      </SettingRow>
                    </>
                  )}
                  <SettingRow label={t("settings.netPacLabel")}>
                    <input
                      value={draft.pac_script_path ?? ""}
                      onChange={(e) =>
                        set("pac_script_path", e.target.value || null)
                      }
                      placeholder={t("settings.netPacPlaceholder")}
                    />
                  </SettingRow>
                </div>
                <p className="settings-note">{t("settings.netProxyNote")}</p>
              </SettingsGroup>
              <SettingsGroup title={t("settings.netRetryGroup")}>
                <div className="retry-policy-grid">
                  <RetryPolicyEditor
                    value={draft.default_retry_policy}
                    onChange={(p) => set("default_retry_policy", p)}
                    compact
                  />
                </div>
                <p className="settings-note">{t("settings.netRetryNote")}</p>
              </SettingsGroup>
              <SettingsGroup title={t("settings.netAwareGroup")}>
                <div className="settings-group-content">
                  <Toggle
                    label={t("settings.netMeteredToggle")}
                    checked={draft.metered_auto_pause}
                    onChange={(v) => set("metered_auto_pause", v)}
                  />
                  <SettingRow label={t("settings.netMeteredCheck")}>
                    <MeteredCheckButton notify={notify} />
                  </SettingRow>
                </div>
                <p className="settings-note">{t("settings.netMeteredNote")}</p>
              </SettingsGroup>
            </>
          )}
          {section === "bt" && (
            <BtSettingsSection
              settings={draft}
              toolStatus={tools}
              onUpdate={(patch) =>
                setDraft((current) => ({ ...current, ...patch }))
              }
              notify={notify}
            />
          )}
          {section === "browser" && (
            <>
              <SettingsGroup title={t("settings.groupDownloadIntercept")}>
                <div className="settings-group-content">
                  <Toggle
                    label={t("settings.browserInterceptToggle")}
                    checked={draft.intercept_browser_downloads}
                    onChange={(v) => set("intercept_browser_downloads", v)}
                  />
                  <SettingRow label={t("settings.browserMinSize")}>
                    <input
                      type="number"
                      min="0"
                      value={draft.min_file_size_mb}
                      onChange={(e) => set("min_file_size_mb", +e.target.value)}
                    />
                  </SettingRow>
                </div>
              </SettingsGroup>
              <SettingsGroup title={t("settings.groupSecurityPairing")}>
                {pair ? (
                  <div className="pair-card">
                    <p>{t("settings.browserPairHint")}</p>
                    <div className="pair-code-wrapper">
                      <code>{pair.code}</code>
                      <button
                        className="copy-code-btn"
                        onClick={() => {
                          void navigator.clipboard.writeText(pair.code);
                          notify(t("settings.browserPairCopied"));
                        }}
                        title={t("settings.browserCopyTitle")}
                      >
                        <Copy size={13} />
                        <span>{t("settings.browserCopy")}</span>
                      </button>
                    </div>
                    {pair.paired_extension && (
                      <p>
                        {t("settings.browserPaired", {
                          id: pair.paired_extension.slice(0, 16),
                        })}
                      </p>
                    )}
                    <div className="maintenance">
                      <button
                        onClick={() =>
                          void api.rotatePairing().then(setPair)
                        }
                      >
                        {t("settings.browserRotate")}
                      </button>
                      {pair.paired_extension && (
                        <button
                          onClick={() =>
                            void api
                              .revokePairing()
                              .then(() => api.pairing().then(setPair))
                          }
                        >
                          {t("settings.browserRevoke")}
                        </button>
                      )}
                    </div>
                  </div>
                ) : (
                  <LoaderCircle className="spin" />
                )}
              </SettingsGroup>
            </>
          )}
          {section === "media" && (
            <SettingsGroup title="媒体组件">
              <p className="settings-note">
                按“自定义路径 → 应用安装 → Windows PATH”顺序查找组件。外部组件只会被引用，猫步下载器不会复制、更新或删除它们。
              </p>
              {tools ? (
                <MediaToolsCard
                  status={tools}
                  onStatus={setTools}
                  ytUpdate={ytUpdate}
                />
              ) : (
                <LoaderCircle className="spin" />
              )}
              <MediaPathSettings
                value={draft}
                onChange={(patch) =>
                  setDraft((current) => ({ ...current, ...patch }))
                }
              />
              <MediaToolsUpdateRow
                tools={tools}
                onStatus={setTools}
                onYtUpdate={setYtUpdate}
              />
            </SettingsGroup>
          )}
          {section === "rules" && <CategoryRulesPanel notify={notify} />}
          {section === "filename-cleanup" && (
            <FilenameCleanupPanel notify={notify} />
          )}
          {section === "naming-template" && (
            <PlatformNamingTemplatePanel notify={notify} />
          )}
          {section === "presets" && <PresetsPanel notify={notify} />}
          {section === "templates" && <TaskTemplatesPanel notify={notify} />}
          {section === "tags" && <TagManagementPanel notify={notify} />}
          {section === "credentials" && (
            <MediaCredentialsPanel notify={notify} />
          )}
          {section === "appearance" && (
            <>
              <SettingsGroup title="主题与紧凑度">
                <div className="settings-group-content">
                  <SettingRow label="颜色方案">
                    <div className="fluent-segmented-control settings-segmented">
                      <button
                        type="button"
                        className={
                          draft.color_scheme === "system" ? "active" : ""
                        }
                        onClick={() => setColorScheme("system")}
                      >
                        跟随系统
                      </button>
                      <button
                        type="button"
                        className={
                          draft.color_scheme === "light" ? "active" : ""
                        }
                        onClick={() => setColorScheme("light")}
                      >
                        浅色
                      </button>
                      <button
                        type="button"
                        className={
                          draft.color_scheme === "dark" ? "active" : ""
                        }
                        onClick={() => setColorScheme("dark")}
                      >
                        深色
                      </button>
                    </div>
                  </SettingRow>
                  <SettingRow label="行高">
                    <div className="fluent-segmented-control settings-segmented">
                      <button
                        type="button"
                        className={!draft.row_compact ? "active" : ""}
                        onClick={() => set("row_compact", false)}
                      >
                        标准 (36px)
                      </button>
                      <button
                        type="button"
                        className={draft.row_compact ? "active" : ""}
                        onClick={() => set("row_compact", true)}
                      >
                        紧凑 (32px)
                      </button>
                    </div>
                  </SettingRow>
                  <Toggle
                    label="详情栏默认折叠（切换任务时）"
                    checked={draft.detail_default_collapsed}
                    onChange={(v) => set("detail_default_collapsed", v)}
                  />
                  <SettingRow label="强调色">
                    <Select
                      value={draft.accent_color}
                      onChange={(val: any) =>
                        set(
                          "accent_color",
                          val as AppSettings["accent_color"]
                        )
                      }
                      options={[
                        { value: "system", label: "跟随 Windows" },
                        { value: "blue", label: "猫步蓝" },
                        { value: "cyan", label: "青色" },
                        { value: "green", label: "绿色" },
                        { value: "purple", label: "紫色" },
                        { value: "orange", label: "橙色" },
                      ]}
                      ariaLabel="强调色"
                    />
                  </SettingRow>
                  <Toggle
                    label="磨砂玻璃"
                    checked={draft.frosted_glass}
                    onChange={(v) => set("frosted_glass", v)}
                  />
                </div>
                <p className="settings-note">
                  在此设置颜色方案、行高大小、强调色以及详情栏折叠与磨砂玻璃等外观偏好。
                </p>
              </SettingsGroup>
              <SettingsGroup title="窗口大小">
                <div className="settings-group-content">
                  <SettingRow label="窗口大小">
                    <div className="window-size-setting-row">
                      <input
                        type="number"
                        placeholder="宽度 (如 800)"
                        value={draft.window_width || ""}
                        onChange={(e) =>
                          changeWidth(
                            e.target.value ? +e.target.value : undefined
                          )
                        }
                        className="window-size-input"
                      />
                      <span>×</span>
                      <input
                        type="number"
                        placeholder="高度 (如 600)"
                        value={draft.window_height || ""}
                        onChange={(e) =>
                          changeHeight(
                            e.target.value ? +e.target.value : undefined
                          )
                        }
                        className="window-size-input"
                      />
                      <Select
                        value={
                          draft.window_width && draft.window_height
                            ? `${draft.window_width}x${draft.window_height}`
                            : ""
                        }
                        onChange={(val: any) => {
                          if (!val) return;
                          const [w, h] = String(val).split("x").map(Number);
                          set("window_width", w);
                          set("window_height", h);
                          applyTemporarySize(w, h);
                        }}
                        options={[
                          { value: "", label: "选择常用预设..." },
                          {
                            value: "800x600",
                            label: "800 × 600 (迷你紧凑)",
                          },
                          {
                            value: "960x640",
                            label: "960 × 640 (精致比例)",
                          },
                          {
                            value: "1024x720",
                            label: "1024 × 720 (默认标准)",
                          },
                          {
                            value: "1120x760",
                            label: "1120 × 760 (舒适格局)",
                          },
                          {
                            value: "1280x800",
                            label: "1280 × 800 (高效宽屏)",
                          },
                          {
                            value: "1440x900",
                            label: "1440 × 900 (专业超宽)",
                          },
                        ]}
                        ariaLabel="预设窗口大小"
                        className="window-size-preset-select"
                      />
                    </div>
                  </SettingRow>
                  <Toggle
                    label="自适应缩放"
                    checked={draft.auto_scale_ui || false}
                    onChange={(v) => set("auto_scale_ui", v)}
                  />
                </div>
                <p className="settings-note">
                  磨砂玻璃使用 Windows 10/11 原生 Acrylic 材质；自适应缩放根据窗口宽度自动放大 UI。
                </p>
              </SettingsGroup>
            </>
          )}
          {section === "shortcuts" && (
            <SettingsGroup title={t("shortcuts.title")}>
              <ShortcutSettingsSection
                value={draft.shortcut_keys || DEFAULT_SHORTCUTS}
                onChange={(val) => set("shortcut_keys", val)}
                notify={notify}
              />
              <p className="settings-note">{t("shortcuts.fixedBindings")}</p>
            </SettingsGroup>
          )}
          {section === "advanced" && (
            <>
              <SettingsGroup title={t("settings.groupTaskMigration")}>
                <div className="maintenance">
                  <button onClick={() => void exportTasks()}>
                    {t("settings.advExportTasks")}
                  </button>
                  <button onClick={() => void importTasks()}>
                    {t("settings.advImportTasks")}
                  </button>
                </div>
                <p className="settings-note">{t("settings.advMigrationNote")}</p>
              </SettingsGroup>
              <SettingsGroup title={t("settings.groupBackupRestore")}>
                <div className="maintenance">
                  <button onClick={() => setBackupOpen(true)}>
                    {t("settings.advCreateBackup")}
                  </button>
                  <button onClick={() => setRestoreOpen(true)}>
                    {t("settings.advRestoreBackup")}
                  </button>
                </div>
                <p className="settings-note">{t("settings.advBackupNote")}</p>
              </SettingsGroup>
              <SettingsGroup title={t("settings.groupLogs")}>
                <div className="maintenance">
                  <button onClick={() => void openLogsDir()}>
                    {t("settings.advOpenLogs")}
                  </button>
                  <button onClick={() => void exportRecentLogs()}>
                    {t("settings.advExportLogs")}
                  </button>
                </div>
                <p className="settings-note">{t("settings.advLogsNote")}</p>
              </SettingsGroup>
              <SettingsGroup title={t("settings.groupMaintenance")}>
                <div
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    gap: "10px",
                  }}
                >
                  <div className="maintenance">
                    <button
                      onClick={() =>
                        void api
                          .clearHistory(false)
                          .then(() =>
                            notify(t("settings.advClearedCancelled"))
                          )
                      }
                    >
                      {t("settings.advClearCancelled")}
                    </button>
                    <button
                      onClick={() =>
                        void api
                          .clearHistory(true)
                          .then(() =>
                            notify(t("settings.advHistoryCleared"))
                          )
                      }
                    >
                      {t("settings.advClearFinished")}
                    </button>
                  </div>
                  <div
                    style={{
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "space-between",
                      paddingTop: "10px",
                      borderTop: "1px solid var(--border)",
                    }}
                  >
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: "8px",
                        fontSize: "12px",
                        color: "var(--text)",
                      }}
                    >
                      <span>{t("settings.advCacheLabel")}</span>
                      <strong style={{ color: "var(--primary)" }}>
                        {cacheSizeLoading
                          ? t("settings.advCacheCalculating")
                          : cacheSizeBytes !== null
                          ? formatBytes(cacheSizeBytes)
                          : "—"}
                      </strong>
                    </div>
                    <div className="maintenance" style={{ marginTop: 0 }}>
                      <button
                        onClick={() => void handleInspectCache()}
                        disabled={cacheSizeLoading || cacheCleaning}
                      >
                        {t("settings.advInspectCache")}
                      </button>
                      <button
                        onClick={() => void handleClearCache()}
                        disabled={
                          cacheCleaning ||
                          cacheSizeBytes === 0 ||
                          cacheSizeBytes === null
                        }
                      >
                        {cacheCleaning
                          ? t("settings.advCacheCleaning")
                          : t("settings.advClearCache")}
                      </button>
                    </div>
                  </div>
                </div>
              </SettingsGroup>
            </>
          )}
          <BackupRestoreModal
            notify={notify}
            backupOpen={backupOpen}
            setBackupOpen={setBackupOpen}
            restoreOpen={restoreOpen}
            setRestoreOpen={setRestoreOpen}
          />
          {section === "about" && (
            <SettingsGroup title={t("settings.groupAboutMaobu")}>
              <div
                style={{
                  display: "flex",
                  flexDirection: "column",
                  gap: "16px",
                  padding: "10px 0",
                }}
              >
                <div style={{ display: "flex", alignItems: "center", gap: "16px" }}>
                  <div style={{ width: "64px", height: "64px", flexShrink: 0 }}>
                    <CatDownloadMark />
                  </div>
                  <div>
                    <h2
                      style={{
                        margin: 0,
                        fontSize: "16px",
                        fontWeight: 700,
                        color: "var(--text)",
                      }}
                    >
                      猫步下载器 (Maobu Fetch)
                    </h2>
                    <p
                      style={{
                        margin: "4px 0 0",
                        fontSize: "11px",
                        color: "var(--muted)",
                      }}
                    >
                      版本 {appInfo?.version || "0.6.9"}
                    </p>
                  </div>
                </div>

                {appInfo?.portable_mode && (
                  <div
                    role="status"
                    style={{
                      display: "flex",
                      alignItems: "flex-start",
                      gap: "8px",
                      padding: "10px 12px",
                      borderRadius: "6px",
                      border: "1px solid var(--accent)",
                      background: "rgba(59,130,246,0.08)",
                      color: "var(--text)",
                      fontSize: "11px",
                      lineHeight: 1.5,
                    }}
                  >
                    <ShieldCheck
                      size={14}
                      color="var(--accent)"
                      style={{ flexShrink: 0, marginTop: "1px" }}
                    />
                    <div>
                      <strong style={{ color: "var(--accent)" }}>
                        便携模式已启用
                      </strong>
                      <div style={{ marginTop: "2px", color: "var(--muted)" }}>
                        数据存储于 EXE 同目录的{" "}
                        <code
                          style={{
                            fontSize: "11px",
                            padding: "1px 4px",
                            borderRadius: "3px",
                            background: "var(--bg-alt, rgba(0,0,0,0.04))",
                            border: "1px solid var(--border)",
                          }}
                        >
                          data/
                        </code>{" "}
                        文件夹，不写入系统{" "}
                        <code
                          style={{
                            fontSize: "11px",
                            padding: "1px 4px",
                            borderRadius: "3px",
                            background: "var(--bg-alt, rgba(0,0,0,0.04))",
                            border: "1px solid var(--border)",
                          }}
                        >
                          %APPDATA%
                        </code>
                        。可将整个程序目录复制到任意位置或设备使用。
                      </div>
                    </div>
                  </div>
                )}

                <div
                  style={{
                    borderTop: "1px solid var(--border)",
                    paddingTop: "14px",
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "space-between",
                    gap: "12px",
                    flexWrap: "wrap",
                  }}
                >
                  <div style={{ display: "flex", alignItems: "center", gap: "12px" }}>
                    <span
                      style={{
                        fontSize: "12px",
                        fontWeight: 600,
                        color: "var(--muted)",
                      }}
                    >
                      作者 / 开发团队
                    </span>
                    <div
                      style={{
                        display: "inline-flex",
                        alignItems: "center",
                        gap: "6px",
                      }}
                    >
                      <span
                        style={{
                          fontSize: "12px",
                          fontWeight: 600,
                          color: "var(--text)",
                        }}
                      >
                        猫步可爱
                      </span>
                      <span style={{ fontSize: "11px", color: "var(--subtle)" }}>
                        (maobukeai)
                      </span>
                    </div>
                  </div>
                  <div
                    style={{
                      display: "inline-flex",
                      alignItems: "center",
                      gap: "8px",
                    }}
                  >
                    <button
                      type="button"
                      className="input-button"
                      onClick={() => setQrModal("contact")}
                      style={{
                        display: "inline-flex",
                        alignItems: "center",
                        gap: "5px",
                        height: "26px",
                        padding: "0 10px",
                        fontSize: "11px",
                        fontWeight: 500,
                        borderRadius: "6px",
                        cursor: "pointer",
                        border: "1px solid var(--border-strong)",
                        background: "var(--control)",
                        color: "var(--text)",
                      }}
                    >
                      <MessageCircle size={12} color="#07c160" />
                      <span>联系方式</span>
                    </button>
                    <button
                      type="button"
                      className="input-button"
                      onClick={() => setQrModal("sponsor")}
                      style={{
                        display: "inline-flex",
                        alignItems: "center",
                        gap: "5px",
                        height: "26px",
                        padding: "0 10px",
                        fontSize: "11px",
                        fontWeight: 500,
                        borderRadius: "6px",
                        cursor: "pointer",
                        border: "1px solid var(--border-strong)",
                        background: "var(--control)",
                        color: "var(--text)",
                      }}
                    >
                      <Heart size={12} color="#e11d48" />
                      <span>赞助支持</span>
                    </button>
                  </div>
                </div>

                {qrModal === "contact" && (
                  <Modal
                    title="联系作者 · 微信二维码"
                    onClose={() => setQrModal(null)}
                    style={{ width: "360px" }}
                  >
                    <div
                      style={{
                        display: "flex",
                        flexDirection: "column",
                        alignItems: "center",
                        gap: "12px",
                        padding: "12px 0 4px",
                      }}
                    >
                      <img
                        src={contactQr}
                        alt="猫步可爱 微信二维码"
                        style={{
                          width: "260px",
                          height: "auto",
                          borderRadius: "8px",
                          border: "1px solid var(--border)",
                          boxShadow: "0 4px 16px rgba(0,0,0,0.08)",
                        }}
                      />
                      <div style={{ textAlign: "center" }}>
                        <div
                          style={{
                            fontSize: "13px",
                            fontWeight: 600,
                            color: "var(--text)",
                          }}
                        >
                          猫步可爱 (鲤蓝)
                        </div>
                        <div
                          style={{
                            fontSize: "11px",
                            color: "var(--muted)",
                            marginTop: "4px",
                          }}
                        >
                          扫二维码，添加我为微信好友
                        </div>
                      </div>
                    </div>
                  </Modal>
                )}

                {qrModal === "sponsor" && (
                  <Modal
                    title="赞助支持 · 赞赏码"
                    onClose={() => setQrModal(null)}
                    style={{ width: "360px" }}
                  >
                    <div
                      style={{
                        display: "flex",
                        flexDirection: "column",
                        alignItems: "center",
                        gap: "12px",
                        padding: "12px 0 4px",
                      }}
                    >
                      <img
                        src={sponsorQr}
                        alt="猫步可爱 赞赏码"
                        style={{
                          width: "260px",
                          height: "auto",
                          borderRadius: "8px",
                          border: "1px solid var(--border)",
                          boxShadow: "0 4px 16px rgba(0,0,0,0.08)",
                        }}
                      />
                      <div style={{ textAlign: "center" }}>
                        <div
                          style={{
                            fontSize: "13px",
                            fontWeight: 600,
                            color: "var(--text)",
                          }}
                        >
                          “新年快乐”
                        </div>
                        <div
                          style={{
                            fontSize: "11px",
                            color: "var(--muted)",
                            marginTop: "4px",
                          }}
                        >
                          猫步可爱 (鲤蓝) 的赞赏码 · 感谢支持！
                        </div>
                      </div>
                    </div>
                  </Modal>
                )}

                <div
                  style={{
                    borderTop: "1px solid var(--border)",
                    paddingTop: "14px",
                  }}
                >
                  <h3
                    style={{
                      margin: "0 0 6px",
                      fontSize: "12px",
                      fontWeight: 600,
                      color: "var(--text)",
                    }}
                  >
                    软件技术架构
                  </h3>
                  <p
                    style={{
                      margin: 0,
                      fontSize: "11px",
                      color: "var(--muted)",
                      lineHeight: 1.6,
                    }}
                  >
                    猫步下载器采用现代化、高性能且低开销的桌面端产品架构：
                  </p>
                  <ul
                    style={{
                      margin: "6px 0 0",
                      paddingLeft: "16px",
                      fontSize: "11px",
                      color: "var(--muted)",
                      lineHeight: 1.6,
                    }}
                  >
                    <li>
                      <strong>前端展示层 (Frontend)</strong>: 基于 React 19 +
                      TypeScript + Vite，配合极致轻量的 Vanilla CSS 实现，无第三方重型组件库，界面精细紧凑。
                    </li>
                    <li>
                      <strong>桌面后端层 (Backend)</strong>: 基于 Rust 核心与 Tauri v2
                      框架，保证极高的执行性能与近乎为零的待机内存开销。
                    </li>
                    <li>
                      <strong>数据持久层 (Database)</strong>: 使用嵌入式 SQLite
                      关系型数据库，安全快速地持久化下载任务队列与用户偏好。
                    </li>
                    <li>
                      <strong>多线程下载引擎 (Engine)</strong>: 高并发 HTTP Range
                      切片下载，支持动态断点续传与速度限制，按需支持 yt-dlp 与 FFmpeg 媒体源分析。
                    </li>
                  </ul>
                </div>

                <div
                  style={{
                    borderTop: "1px solid var(--border)",
                    paddingTop: "14px",
                    display: "grid",
                    gridTemplateColumns: "repeat(3, 1fr)",
                    gap: "16px",
                  }}
                >
                  <div
                    style={{
                      display: "flex",
                      flexDirection: "column",
                      gap: "8px",
                    }}
                  >
                    <h3
                      style={{
                        margin: "0",
                        fontSize: "12px",
                        fontWeight: 600,
                        color: "var(--text)",
                      }}
                    >
                      应用更新检查
                    </h3>
                    <p
                      style={{
                        margin: 0,
                        fontSize: "11px",
                        color: "var(--muted)",
                        lineHeight: 1.4,
                      }}
                    >
                      查询 Releases 最新版本并手动校验。
                    </p>
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: "8px",
                        flexWrap: "wrap",
                        marginTop: "auto",
                        paddingTop: "4px",
                      }}
                    >
                      <button
                        className="input-button"
                        disabled={updateChecking}
                        onClick={() => void checkAppUpdate()}
                        style={{
                          display: "inline-flex",
                          alignItems: "center",
                          gap: "6px",
                          height: "28px",
                          padding: "0 12px",
                          fontSize: "11px",
                          fontWeight: 500,
                          cursor: updateChecking ? "default" : "pointer",
                          borderRadius: "6px",
                          border: "1px solid var(--border)",
                          background: "var(--accent)",
                          color: "white",
                        }}
                      >
                        {updateChecking ? (
                          <LoaderCircle size={12} className="spin" />
                        ) : (
                          <RefreshCw size={12} />
                        )}
                        {updateChecking ? "检查中…" : "检查更新"}
                      </button>
                      <span style={{ fontSize: "11px", color: "var(--muted)" }}>
                        v{appInfo?.version || "0.6.9"}
                      </span>
                    </div>
                    {updateResult && !updateResult.error && (
                      <div
                        style={{
                          marginTop: "6px",
                          fontSize: "11px",
                          lineHeight: 1.5,
                          color: "var(--muted)",
                          padding: "8px 10px",
                          background: "var(--bg-alt, rgba(0,0,0,0.03))",
                          borderRadius: "6px",
                          border: "1px solid var(--border)",
                        }}
                      >
                        {updateResult.has_update && updateResult.latest ? (
                          <div
                            style={{
                              display: "flex",
                              flexDirection: "column",
                              gap: "6px",
                            }}
                          >
                            <div
                              style={{
                                display: "flex",
                                alignItems: "center",
                                gap: "6px",
                              }}
                            >
                              <AlertCircle size={12} color="var(--accent)" />
                              <strong style={{ color: "var(--text)" }}>
                                发现新版本 v{updateResult.latest.version}
                              </strong>
                              {updateResult.latest.release_date && (
                                <span style={{ color: "var(--muted)" }}>
                                  · {updateResult.latest.release_date}
                                </span>
                              )}
                            </div>
                            {updateResult.latest.release_notes && (
                              <div
                                style={{
                                  maxHeight: "120px",
                                  overflowY: "auto",
                                  whiteSpace: "pre-wrap",
                                  fontSize: "11px",
                                  color: "var(--muted)",
                                  borderTop: "1px solid var(--border)",
                                  paddingTop: "6px",
                                }}
                              >
                                {updateResult.latest.release_notes}
                              </div>
                            )}
                            {updateResult.latest.sha256 && (
                              <div
                                style={{
                                  fontSize: "10px",
                                  color: "var(--muted)",
                                  wordBreak: "break-all",
                                }}
                              >
                                SHA-256: {updateResult.latest.sha256}
                              </div>
                            )}
                            <div
                              style={{
                                display: "flex",
                                gap: "6px",
                                flexWrap: "wrap",
                                alignItems: "center",
                              }}
                            >
                              <button
                                className="input-button"
                                disabled={appUpdateBusy}
                                onClick={() => void runAppUpdateDownload()}
                                style={{
                                  display: "inline-flex",
                                  alignItems: "center",
                                  gap: "6px",
                                  height: "26px",
                                  padding: "0 12px",
                                  fontSize: "11px",
                                  fontWeight: 500,
                                  cursor: appUpdateBusy ? "default" : "pointer",
                                  borderRadius: "6px",
                                  border: "1px solid var(--accent)",
                                  background: "var(--accent)",
                                  color: "white",
                                }}
                              >
                                {appUpdateBusy ? (
                                  <LoaderCircle size={11} className="spin" />
                                ) : (
                                  <Download size={11} />
                                )}
                                {appUpdateBusy ? "下载中…" : "一键更新"}
                              </button>
                              <button
                                className="input-button"
                                onClick={() =>
                                  void openUrl(
                                    updateResult.latest?.download_url ||
                                      "https://github.com/maobukeai/maobu-fetch/releases"
                                  ).catch((err) => notify(String(err), "error"))
                                }
                                style={{
                                  display: "inline-flex",
                                  alignItems: "center",
                                  gap: "6px",
                                  height: "26px",
                                  padding: "0 12px",
                                  fontSize: "11px",
                                  fontWeight: 500,
                                  cursor: "pointer",
                                  borderRadius: "6px",
                                  border: "1px solid var(--accent)",
                                  background: "transparent",
                                  color: "var(--accent)",
                                }}
                              >
                                <ExternalLink size={11} />
                                前往下载页
                              </button>
                            </div>
                          </div>
                        ) : (
                          <div
                            style={{
                              display: "flex",
                              alignItems: "center",
                              gap: "6px",
                              flexWrap: "wrap",
                            }}
                          >
                            <Check size={12} color="#22c55e" />
                            <span style={{ flex: 1, minWidth: 0 }}>
                              已是最新版
                              {updateResult.latest
                                ? ` (v${updateResult.latest.version})`
                                : ""}
                            </span>
                            <button
                              className="input-button"
                              disabled={appUpdateBusy}
                              onClick={() => void runAppUpdateDownload()}
                              title="重新下载当前版本安装包（可用于修复安装）"
                              style={{
                                display: "inline-flex",
                                alignItems: "center",
                                gap: "6px",
                                height: "24px",
                                padding: "0 10px",
                                fontSize: "11px",
                                fontWeight: 500,
                                cursor: appUpdateBusy ? "default" : "pointer",
                                borderRadius: "6px",
                                border: "1px solid var(--accent)",
                                background: "transparent",
                                color: "var(--accent)",
                                flexShrink: 0,
                              }}
                            >
                              {appUpdateBusy ? (
                                <LoaderCircle size={11} className="spin" />
                              ) : (
                                <Download size={11} />
                              )}
                              {appUpdateBusy ? "下载中…" : "重新下载安装包"}
                            </button>
                          </div>
                        )}
                        {appUpdateBusy && appUpdateProgress && (
                          <div
                            style={{
                              display: "flex",
                              flexDirection: "column",
                              gap: "4px",
                            }}
                          >
                            <div
                              style={{
                                height: "4px",
                                borderRadius: "2px",
                                background: "var(--bg-alt, rgba(0,0,0,0.08))",
                                overflow: "hidden",
                              }}
                            >
                              <div
                                style={{
                                  height: "100%",
                                  width: `${
                                    appUpdateProgress.total > 0
                                      ? Math.min(
                                          100,
                                          Math.round(
                                            (appUpdateProgress.downloaded /
                                              appUpdateProgress.total) *
                                              100
                                          )
                                        )
                                      : 0
                                  }%`,
                                  background: "var(--accent)",
                                  transition: "width 0.15s",
                                }}
                              />
                            </div>
                            <span style={{ fontSize: "10px", color: "var(--muted)" }}>
                              {formatBytes(appUpdateProgress.downloaded)}
                              {appUpdateProgress.total > 0
                                ? ` / ${formatBytes(appUpdateProgress.total)}`
                                : ""}
                            </span>
                          </div>
                        )}
                        {appUpdateReady && (
                          <div
                            style={{
                              display: "flex",
                              alignItems: "center",
                              gap: "6px",
                              padding: "6px 8px",
                              borderRadius: "6px",
                              border: "1px solid rgba(34,197,94,0.3)",
                              background: "rgba(34,197,94,0.08)",
                            }}
                          >
                            <Check size={12} color="#22c55e" />
                            <span
                              style={{
                                fontSize: "11px",
                                color: "var(--muted)",
                                flex: 1,
                                minWidth: 0,
                              }}
                            >
                              v{appUpdateReady.version} 安装包已校验（
                              {formatBytes(appUpdateReady.size)}）
                            </span>
                            <button
                              className="input-button"
                              onClick={() => void runAppUpdateInstaller()}
                              style={{
                                display: "inline-flex",
                                alignItems: "center",
                                gap: "5px",
                                height: "24px",
                                padding: "0 10px",
                                fontSize: "11px",
                                fontWeight: 500,
                                cursor: "pointer",
                                borderRadius: "6px",
                                border: "1px solid var(--accent)",
                                background: "var(--accent)",
                                color: "white",
                                flexShrink: 0,
                              }}
                            >
                              <Zap size={11} />
                              立即安装
                            </button>
                          </div>
                        )}
                      </div>
                    )}
                    {updateResult?.error && (
                      <div
                        style={{
                          marginTop: "6px",
                          fontSize: "11px",
                          color: "var(--danger, #ef4444)",
                          padding: "8px 10px",
                          background: "rgba(239,68,68,0.08)",
                          borderRadius: "6px",
                          border: "1px solid rgba(239,68,68,0.2)",
                          display: "flex",
                          alignItems: "center",
                          gap: "6px",
                        }}
                      >
                        <AlertCircle size={12} />
                        <span>检查失败：{updateResult.error}</span>
                      </div>
                    )}
                  </div>

                  <div
                    style={{
                      display: "flex",
                      flexDirection: "column",
                      gap: "8px",
                    }}
                  >
                    <h3
                      style={{
                        margin: "0",
                        fontSize: "12px",
                        fontWeight: 600,
                        color: "var(--text)",
                      }}
                    >
                      浏览器扩展
                    </h3>
                    <p
                      style={{
                        margin: 0,
                        fontSize: "11px",
                        color: "var(--muted)",
                        lineHeight: 1.4,
                      }}
                    >
                      一键拉取 GitHub 最新扩展（与桌面端版本同步），自动校验并解压到本地托管目录。
                    </p>
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: "6px",
                        flexWrap: "wrap",
                      }}
                    >
                      <button
                        className="input-button"
                        disabled={extUpdateBusy}
                        onClick={() => void runExtensionUpdate()}
                        style={{
                          display: "inline-flex",
                          alignItems: "center",
                          gap: "6px",
                          height: "28px",
                          padding: "0 12px",
                          fontSize: "11px",
                          fontWeight: 500,
                          cursor: extUpdateBusy ? "default" : "pointer",
                          borderRadius: "6px",
                          border: "1px solid var(--accent)",
                          background: "var(--accent)",
                          color: "white",
                        }}
                      >
                        {extUpdateBusy ? (
                          <LoaderCircle size={11} className="spin" />
                        ) : (
                          <Download size={11} />
                        )}
                        {extUpdateBusy ? "更新中…" : "一键更新扩展"}
                      </button>
                    </div>
                    {extUpdateBusy && extUpdateProgress && (
                      <div
                        style={{
                          display: "flex",
                          flexDirection: "column",
                          gap: "4px",
                        }}
                      >
                        <div
                          style={{
                            height: "4px",
                            borderRadius: "2px",
                            background: "var(--bg-alt, rgba(0,0,0,0.08))",
                            overflow: "hidden",
                          }}
                        >
                          <div
                            style={{
                              height: "100%",
                              width: `${
                                extUpdateProgress.total > 0
                                  ? Math.min(
                                      100,
                                      Math.round(
                                        (extUpdateProgress.downloaded /
                                          extUpdateProgress.total) *
                                          100
                                      )
                                    )
                                  : 0
                              }%`,
                              background: "var(--accent)",
                              transition: "width 0.15s",
                            }}
                          />
                        </div>
                        <span style={{ fontSize: "10px", color: "var(--muted)" }}>
                          {formatBytes(extUpdateProgress.downloaded)}
                          {extUpdateProgress.total > 0
                            ? ` / ${formatBytes(extUpdateProgress.total)}`
                            : ""}
                        </span>
                      </div>
                    )}
                    {extUpdateResult && (
                      <div
                        style={{
                          display: "flex",
                          flexDirection: "column",
                          gap: "6px",
                          fontSize: "11px",
                          lineHeight: 1.5,
                          padding: "8px 10px",
                          borderRadius: "6px",
                          border: "1px solid rgba(34,197,94,0.3)",
                          background: "rgba(34,197,94,0.08)",
                          color: "var(--muted)",
                        }}
                      >
                        <div
                          style={{
                            display: "flex",
                            alignItems: "center",
                            gap: "6px",
                          }}
                        >
                          <Check size={12} color="#22c55e" />
                          <strong style={{ color: "var(--text)" }}>
                            扩展 v{extUpdateResult.version} 已就绪
                          </strong>
                          <button
                            className="input-button"
                            onClick={() =>
                              void api
                                .extensionUpdateOpenFolder()
                                .catch((e) => notify(String(e), "error"))
                            }
                            style={{
                              display: "inline-flex",
                              alignItems: "center",
                              gap: "5px",
                              height: "24px",
                              padding: "0 10px",
                              fontSize: "11px",
                              fontWeight: 500,
                              cursor: "pointer",
                              borderRadius: "6px",
                              border: "1px solid var(--accent)",
                              background: "transparent",
                              color: "var(--accent)",
                              marginLeft: "auto",
                              flexShrink: 0,
                            }}
                          >
                            <FolderOpen size={11} />
                            打开目录
                          </button>
                        </div>
                        <div>
                          首次使用：在浏览器扩展管理页选择"加载已解压的扩展程序"，指向上述目录；已从该目录加载过：在扩展页点击"刷新"即可生效。
                        </div>
                        <div
                          style={{
                            fontSize: "10px",
                            wordBreak: "break-all",
                            color: "var(--muted)",
                          }}
                        >
                          {extUpdateResult.folder}
                        </div>
                      </div>
                    )}
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: "6px",
                        flexWrap: "wrap",
                        marginTop: "auto",
                        paddingTop: "4px",
                        borderTop: "1px dashed var(--border)",
                      }}
                    >
                      <input
                        value={extVersion}
                        onChange={(e) => setExtVersion(e.target.value)}
                        placeholder="如 0.6.9"
                        style={{
                          height: "28px",
                          padding: "0 8px",
                          fontSize: "11px",
                          borderRadius: "6px",
                          border: "1px solid var(--border)",
                          background: "var(--bg)",
                          color: "var(--text)",
                          width: "85px",
                        }}
                      />
                      <button
                        className="input-button"
                        disabled={extChecking}
                        onClick={() => void checkExtCompat()}
                        style={{
                          display: "inline-flex",
                          alignItems: "center",
                          gap: "6px",
                          height: "28px",
                          padding: "0 10px",
                          fontSize: "11px",
                          fontWeight: 500,
                          cursor: extChecking ? "default" : "pointer",
                          borderRadius: "6px",
                          border: "1px solid var(--border)",
                          background: "var(--bg)",
                          color: "var(--text)",
                        }}
                      >
                        {extChecking ? (
                          <LoaderCircle size={11} className="spin" />
                        ) : (
                          <ShieldCheck size={11} />
                        )}
                        {extChecking ? "检查中…" : "检查兼容性"}
                      </button>
                    </div>
                    {extResult && (
                      <div
                        style={{
                          marginTop: "6px",
                          fontSize: "11px",
                          lineHeight: 1.5,
                          padding: "8px 10px",
                          borderRadius: "6px",
                          border: extResult.compatible
                            ? "1px solid rgba(34,197,94,0.3)"
                            : "1px solid rgba(239,68,68,0.3)",
                          background: extResult.compatible
                            ? "rgba(34,197,94,0.08)"
                            : "rgba(239,68,68,0.08)",
                          color: "var(--muted)",
                        }}
                      >
                        <div
                          style={{
                            display: "flex",
                            alignItems: "center",
                            gap: "6px",
                            marginBottom: "4px",
                          }}
                        >
                          {extResult.compatible ? (
                            <Check size={12} color="#22c55e" />
                          ) : (
                            <AlertCircle size={12} color="#ef4444" />
                          )}
                          <strong style={{ color: "var(--text)" }}>
                            {extResult.compatible ? "兼容" : "不兼容"}
                          </strong>
                          <span style={{ color: "var(--muted)" }}>
                            · 扩展 v{extResult.extension_version} / 桌面 v
                            {extResult.app_version}
                          </span>
                        </div>
                        {extResult.message && <div>{extResult.message}</div>}
                      </div>
                    )}
                  </div>

                  <div
                    style={{
                      display: "flex",
                      flexDirection: "column",
                      gap: "8px",
                    }}
                  >
                    <h3
                      style={{
                        margin: "0",
                        fontSize: "12px",
                        fontWeight: 600,
                        color: "var(--text)",
                      }}
                    >
                      开源项目主页
                    </h3>
                    <p
                      style={{
                        margin: 0,
                        fontSize: "11px",
                        color: "var(--muted)",
                        lineHeight: 1.4,
                      }}
                    >
                      访问 GitHub 仓库获取最新源码与参与贡献。
                    </p>
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: "8px",
                        marginTop: "auto",
                        paddingTop: "4px",
                      }}
                    >
                      <button
                        className="input-button"
                        style={{
                          display: "inline-flex",
                          alignItems: "center",
                          gap: "6px",
                          height: "28px",
                          padding: "0 12px",
                          fontSize: "11px",
                          fontWeight: 500,
                          cursor: "pointer",
                          borderRadius: "6px",
                          border: "1px solid var(--border)",
                          background: "var(--accent)",
                          color: "white",
                        }}
                        onClick={async () => {
                          try {
                            await openUrl(
                              "https://github.com/maobukeai/maobu-fetch"
                            );
                          } catch (err) {
                            notify(String(err), "error");
                          }
                        }}
                      >
                        <ExternalLink size={12} />
                        访问 GitHub
                      </button>
                    </div>
                  </div>
                </div>

                <div
                  style={{
                    borderTop: "1px solid var(--border)",
                    paddingTop: "14px",
                    display: "flex",
                    flexDirection: "column",
                    gap: "10px",
                  }}
                >
                  <h3
                    style={{
                      margin: "0",
                      fontSize: "12px",
                      fontWeight: 600,
                      color: "var(--text)",
                    }}
                  >
                    平台兼容性
                  </h3>
                  <p
                    style={{
                      margin: 0,
                      fontSize: "11px",
                      color: "var(--muted)",
                      lineHeight: 1.5,
                    }}
                  >
                    以下是各媒体平台的支持级别，新建任务时将根据匹配到的平台展示对应状态徽章。
                  </p>
                  {platformCompatList.length === 0 ? (
                    <div
                      style={{
                        fontSize: "11px",
                        color: "var(--muted)",
                        padding: "8px 10px",
                        background: "var(--bg-alt, rgba(0,0,0,0.03))",
                        borderRadius: "6px",
                        border: "1px solid var(--border)",
                      }}
                    >
                      暂无平台兼容性数据
                    </div>
                  ) : (
                    <div
                      style={{
                        display: "grid",
                        gridTemplateColumns:
                          "repeat(auto-fill, minmax(220px, 1fr))",
                        gap: "8px",
                      }}
                    >
                      {platformCompatList.map((item) => {
                        const platformLabel = (() => {
                          switch (item.platform) {
                            case "douyin":
                              return "抖音";
                            case "tiktok":
                              return "TikTok";
                            case "twitter":
                              return "Twitter/X";
                            case "youtube":
                              return "YouTube";
                            case "bilibili":
                              return "哔哩哔哩";
                            case "weibo":
                              return "微博";
                            default:
                              return item.platform;
                          }
                        })();
                        return (
                          <div
                            key={item.platform}
                            style={{
                              display: "flex",
                              flexDirection: "column",
                              gap: "4px",
                              padding: "8px 10px",
                              borderRadius: "6px",
                              border: "1px solid var(--border)",
                              background: "var(--bg-alt, rgba(0,0,0,0.02))",
                            }}
                          >
                            <div
                              style={{
                                display: "flex",
                                alignItems: "center",
                                gap: "6px",
                                flexWrap: "wrap",
                              }}
                            >
                              <Globe2 size={12} color="var(--muted)" />
                              <strong
                                style={{
                                  fontSize: "11px",
                                  color: "var(--text)",
                                }}
                              >
                                {platformLabel}
                              </strong>
                              <span
                                title={supportLevelLabel(item.level)}
                                style={{
                                  marginLeft: "auto",
                                  padding: "1px 6px",
                                  borderRadius: 4,
                                  fontSize: 10,
                                  color: "#fff",
                                  backgroundColor: supportLevelColor(
                                    item.level
                                  ),
                                  border: `1px solid ${supportLevelColor(
                                    item.level
                                  )}`,
                                }}
                              >
                                {supportLevelLabel(item.level)}
                              </span>
                            </div>
                            {item.notes && (
                              <p
                                style={{
                                  margin: 0,
                                  fontSize: "10px",
                                  color: "var(--muted)",
                                  lineHeight: 1.4,
                                }}
                              >
                                {item.notes}
                              </p>
                            )}
                            {item.known_issues &&
                              item.known_issues.length > 0 && (
                                <ul
                                  style={{
                                    margin: "2px 0 0",
                                    paddingLeft: "14px",
                                    fontSize: "10px",
                                    color: "var(--muted)",
                                    lineHeight: 1.4,
                                  }}
                                >
                                  {item.known_issues.map((issue, idx) => (
                                    <li key={idx}>{issue}</li>
                                  ))}
                                </ul>
                              )}
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
              </div>
            </SettingsGroup>
          )}
          <div className="dialog-actions settings-actions">
            <button onClick={onClose}>{t("common.cancel")}</button>
            <button className="primary" onClick={() => void save()}>
              {t("settings.saveSettings")}
            </button>
          </div>
        </div>
      </main>
    </div>
  );
}
