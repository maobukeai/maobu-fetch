import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AlertCircle,
  AlertTriangle,
  Bookmark,
  Check,
  ChevronDown,
  Download,
  ExternalLink,
  FileText,
  Folder,
  FolderOpen,
  Globe2,
  Info,
  Search,
  ShieldCheck,
  Trash2,
  Video,
  X,
  Zap,
} from "lucide-react";
import { open as pickPath } from "@tauri-apps/plugin-dialog";
import { api } from "../../api";
import { t, useLocale } from "../../i18n";
import type {
  AppSettings,
  BtFileEntry,
  BtTorrentInspectResult,
  CollisionPolicy,
  CompletionAction,
  DownloadPreset,
  DownloadTask,
  DuplicateCheckResult,
  FilenameCleanupRule,
  MediaPlatform,
  MediaProbeResult,
  NewTaskRequest,
  PlatformCompatibility,
  PrecheckResult,
  TaskStatus,
  TaskTemplateTestResult,
  ToolStatus,
  UrlHistoryEntry,
} from "../../types";
import {
  mediaPlatformDisplayName,
  supportLevelColor,
  supportLevelLabel,
} from "../../types";
import {
  applyFilenameCleanup,
  extractDomainForHint,
  extractFileNameFromUrl,
  formatBytes,
  getDuplicateTypeLabel,
  getStatusText,
  isDownloadableUrlForDialog,
  parseMultilineUrls,
  safeDisplayName,
} from "../../formatters";
import { completionActionKind } from "../../types";
import { TASK_PRIORITY_PRESETS } from "../../priority";
import { CompletionActionEditor } from "../CompletionActionEditor";
import { EpisodePicker } from "../EpisodePicker";
import { PrecheckPanel } from "../PrecheckPanel";
import { Select } from "../Select";
import { Modal, ConfirmDialog } from "../common/Modal";
import { Field } from "../common/FormComponents";
import { MediaToolsCard } from "../settings/MediaSettingsGroup";
import { GalleryPicker } from "./GalleryPicker";
import {
  getOrCreateDeviceId,
  inspectPikPakShare,
  isPikPakShareUrl,
  parsePikPakShareUrl,
  resolvePikPakDirectUrl,
  type PikPakShareInfo,
} from "../../services/pikpak";
import { PikPakPicker } from "./PikPakPicker";
import {
  inspectQuarkShare,
  isQuarkUrl,
  parseQuarkUrl,
  resolveQuarkFile,
  type QuarkShareInfo,
} from "../../services/quark";
import { QuarkPicker } from "./QuarkPicker";
import {
  inspectBaiduShare,
  isBaiduUrl,
  parseBaiduUrl,
  resolveBaiduFile,
  type BaiduShareInfo,
} from "../../services/baidupan";
import { BaiduPanPicker } from "./BaiduPanPicker";
import { LanzouPicker } from "./LanzouPicker";
import { Pan123Picker } from "./Pan123Picker";
import type { LanzouShareInfo, Pan123ShareInfo } from "../../types";

const isLanzouUrl = (url: string) => {
  if (!url) return false;
  const lower = url.toLowerCase();
  return lower.includes("lanzou") || lower.includes("lanzo") || lower.includes("baidupan.com.lanzou");
};

const isPan123Url = (url: string) => {
  if (!url) return false;
  const lower = url.toLowerCase();
  return (
    lower.includes("123pan.com") ||
    lower.includes("123pan.cn") ||
    lower.includes("123684.com") ||
    lower.includes("123952.com")
  );
};

export function NewTaskDialog({
  settings,
  allTasks,
  onClose,
  onCreated,
  defaultUrl,
  defaultTorrent,
  onLocateTask,
  notify,
}: {
  settings: AppSettings;
  allTasks?: DownloadTask[];
  onClose: () => void;
  onCreated: (tasks: DownloadTask | DownloadTask[]) => void;
  defaultUrl?: string;
  defaultTorrent?: { name: string; base64: string } | null;
  onLocateTask?: (taskId: string) => void;
  notify?: (text: string, kind?: "ok" | "error") => void;
}) {
  useLocale();
  const [torrentData, setTorrentData] = useState<{ name: string; base64: string } | null>(
    defaultTorrent ?? null
  );
  const [urls, setUrls] = useState(
    defaultTorrent ? defaultTorrent.name : defaultUrl || ""
  );
  const [destination, setDestination] = useState(settings.download_dir);
  const [fileName, setFileName] = useState(() => {
    if (defaultTorrent) {
      return defaultTorrent.name.replace(/\.torrent$/i, "");
    }
    if (defaultUrl) {
      const initLines = defaultUrl
        .split(/\r?\n/)
        .map((l) => l.trim())
        .filter(Boolean);
      if (initLines.length === 1) {
        return extractFileNameFromUrl(initLines[0]);
      }
    }
    return "";
  });
  const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [btStartPaused, setBtStartPaused] = useState(false);
  const [btStreaming, setBtStreaming] = useState(false);
  const [btInspectResult, setBtInspectResult] = useState<BtTorrentInspectResult | null>(null);
  const [btInspecting, setBtInspecting] = useState(false);
  const [selectedBtFileIndices, setSelectedBtFileIndices] = useState<Set<number>>(new Set());
  const [btInspectOpen, setBtInspectOpen] = useState(false);
  const [diskConfirm, setDiskConfirm] = useState<{ override?: string } | null>(null);
  const [schedule, setSchedule] = useState("");
  const [policy, setPolicy] = useState<CollisionPolicy>(
    settings.default_collision_policy
  );
  const [priority, setPriority] = useState(0);
  const [completionAction, setCompletionAction] = useState<CompletionAction>(
    settings.default_completion_action
  );
  const [referer, setReferer] = useState("");
  const [cookie, setCookie] = useState("");
  const [authorization, setAuthorization] = useState("");
  const [checksum, setChecksum] = useState("");
  const [limit, setLimit] = useState(0);
  const [connections, setConnections] = useState(
    settings.connections_per_download
  );
  const [media, setMedia] = useState<MediaProbeResult>();
  const [format, setFormat] = useState("");
  const [subtitleLangs, setSubtitleLangs] = useState<string[]>([]);
  const [selectedImageIds, setSelectedImageIds] = useState<Set<string>>(new Set());
  const [selectedEpisodeIndices, setSelectedEpisodeIndices] = useState<Set<number>>(new Set());
  const [collectionQualityPreference, setCollectionQualityPreference] = useState<string>("best");
  const [toolStatus, setToolStatus] = useState<ToolStatus>();
  const [precheck, setPrecheck] = useState<PrecheckResult>();
  const [precheckLoading, setPrecheckLoading] = useState(false);
  const [precheckError, setPrecheckError] = useState<string>();
  const [ignoreUrlConflict, setIgnoreUrlConflict] = useState(false);
  const [duplicateResult, setDuplicateResult] = useState<DuplicateCheckResult>();
  const [presets, setPresets] = useState<DownloadPreset[]>([]);
  const [selectedPresetId, setSelectedPresetId] = useState<string>("");
  const [templateMatch, setTemplateMatch] = useState<TaskTemplateTestResult>();
  const [detectedPlatform, setDetectedPlatform] = useState<MediaPlatform | null>(null);
  const [platformCompat, setPlatformCompat] = useState<PlatformCompatibility | null>(null);
  const [matchedCredentialDomain, setMatchedCredentialDomain] = useState<string | null>(null);
  const [normalizedUrlPreview, setNormalizedUrlPreview] = useState<string | null>(null);

  // PikPak 分享解析状态
  const [pikpakShareInfo, setPikpakShareInfo] = useState<PikPakShareInfo | null>(null);
  const [pikpakInspecting, setPikpakInspecting] = useState(false);
  const [pikpakSelectedIds, setPikpakSelectedIds] = useState<Set<string>>(new Set());
  const [pikpakOpen, setPikpakOpen] = useState(false);
  const [pikpakPassCodeVerifying, setPikpakPassCodeVerifying] = useState(false);
  const [pikpakPassCodeError, setPikpakPassCodeError] = useState<string>();

  // Quark 分享解析状态
  const [quarkShareInfo, setQuarkShareInfo] = useState<QuarkShareInfo | null>(null);
  const [quarkInspecting, setQuarkInspecting] = useState(false);
  const [quarkSelectedIds, setQuarkSelectedIds] = useState<Set<string>>(new Set());
  const [quarkOpen, setQuarkOpen] = useState(false);
  const [quarkPassCodeVerifying, setQuarkPassCodeVerifying] = useState(false);
  const [quarkPassCodeError, setQuarkPassCodeError] = useState<string>();

  // Baidu 网盘分享解析状态
  const [baiduShareInfo, setBaiduShareInfo] = useState<BaiduShareInfo | null>(null);
  const [baiduInspecting, setBaiduInspecting] = useState(false);
  const [baiduSelectedIds, setBaiduSelectedIds] = useState<Set<string>>(new Set());
  const [baiduOpen, setBaiduOpen] = useState(false);
  const [baiduPassCodeVerifying, setBaiduPassCodeVerifying] = useState(false);
  const [baiduPassCodeError, setBaiduPassCodeError] = useState<string>();

  // Lanzou 分享解析状态
  const [lanzouShareInfo, setLanzouShareInfo] = useState<LanzouShareInfo | null>(null);
  const [lanzouInspecting, setLanzouInspecting] = useState(false);
  const [lanzouSelectedIds, setLanzouSelectedIds] = useState<Set<string>>(new Set());
  const [lanzouOpen, setLanzouOpen] = useState(false);
  const [lanzouPassCodeVerifying, setLanzouPassCodeVerifying] = useState(false);
  const [lanzouPassCodeError, setLanzouPassCodeError] = useState<string>();

  // 123Pan 分享解析状态
  const [pan123ShareInfo, setPan123ShareInfo] = useState<Pan123ShareInfo | null>(null);
  const [pan123Inspecting, setPan123Inspecting] = useState(false);
  const [pan123SelectedIds, setPan123SelectedIds] = useState<Set<string>>(new Set());
  const [pan123Open, setPan123Open] = useState(false);
  const [pan123PassCodeVerifying, setPan123PassCodeVerifying] = useState(false);
  const [pan123PassCodeError, setPan123PassCodeError] = useState<string>();

  const fileNameInputRef = useRef<HTMLInputElement | null>(null);
  const userEditedFileName = useRef(false);
  const userEditedConnections = useRef(false);
  const userEditedDestination = useRef(false);
  const precheckSeqRef = useRef(0);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void api.toolStatus().then(setToolStatus);
    void api.subscribeMediaTools(setToolStatus).then((value) => {
      unlisten = value;
    });
    return () => unlisten?.();
  }, []);

  useEffect(() => {
    void api
      .presetList()
      .then((list) => setPresets(list ?? []))
      .catch(() => setPresets([]));
  }, []);

  const [urlHistory, setUrlHistory] = useState<UrlHistoryEntry[]>([]);
  const [historyOpen, setHistoryOpen] = useState(false);
  const reloadHistory = () => {
    void api
      .urlHistoryList()
      .then(setUrlHistory)
      .catch(() => setUrlHistory([]));
  };
  useEffect(() => {
    reloadHistory();
  }, []);

  const [cleanupRules, setCleanupRules] = useState<FilenameCleanupRule[]>([]);
  useEffect(() => {
    void api
      .filenameCleanupRuleList()
      .then(setCleanupRules)
      .catch(() => setCleanupRules([]));
  }, []);

  useEffect(() => {
    if (!fileName || cleanupRules.length === 0 || userEditedFileName.current) {
      return;
    }
    const cleaned = applyFilenameCleanup(fileName, cleanupRules);
    if (cleaned !== fileName) {
      setFileName(cleaned);
    }
  }, [fileName, cleanupRules]);

  const {
    lines,
    skippedCount,
    duplicateCount,
    sequenceExpanded,
    sequenceError,
  } = parseMultilineUrls(urls);

  const inspectTorrentContent = useCallback(
    async (pathOrMagnet?: string, base64?: string) => {
      const single = lines.length === 1 ? lines[0].trim() : "";
      const isMagnet = single.toLowerCase().startsWith("magnet:");
      const isTorrent =
        !isMagnet &&
        single !== "" &&
        !/^https?:/i.test(single) &&
        /\.torrent$/i.test(single);
      const targetSource =
        pathOrMagnet || (isMagnet ? single : isTorrent ? single : undefined);
      const targetBase64 = base64 || torrentData?.base64;
      if (!targetSource && !targetBase64) return;
      setBtInspecting(true);
      try {
        const result = await api.inspectTorrent({
          source: targetSource,
          torrentPath: isTorrent ? targetSource : undefined,
          torrentBase64: targetBase64,
        });
        setBtInspectResult(result);
        setSelectedBtFileIndices(
          new Set(result.files.map((f: BtFileEntry) => f.index))
        );
        setBtInspectOpen(true);
        if (result.name && !userEditedFileName.current) {
          setFileName(result.name);
        }
      } catch (err) {
        console.warn("解析种子/磁力失败:", err);
        notify?.(String(err), "error");
      } finally {
        setBtInspecting(false);
      }
    },
    [lines, torrentData, notify]
  );

  const inspectPikPakContent = useCallback(
    async (targetUrl?: string, passCode?: string) => {
      const single = targetUrl || (lines.length === 1 ? lines[0].trim() : "");
      if (!isPikPakShareUrl(single)) return;
      setPikpakInspecting(true);
      setPikpakPassCodeError(undefined);
      try {
        const info = await inspectPikPakShare(single, passCode);
        setPikpakShareInfo(info);
        setPikpakOpen(true);
        if (info.passCodeRequired) {
          setPikpakSelectedIds(new Set());
        } else {
          const onlyFiles = info.files.filter((f) => f.kind === "drive#file");
          setPikpakSelectedIds(new Set(onlyFiles.map((f) => f.id)));
          if (info.title && !userEditedFileName.current) {
            setFileName(info.title);
          }
          // PikPak 建议默认 16 连接并发分片下载（实测 16 为 CDN 单 IP 黄金并发上限，0 限流且满速）
          if (!userEditedConnections.current && connections < 16) {
            setConnections(16);
          }
        }
      } catch (err: any) {
        console.warn("解析 PikPak 分享失败:", err);
        const errMsg = err?.message || String(err);
        setPikpakPassCodeError(errMsg);
        notify?.(errMsg, "error");
      } finally {
        setPikpakInspecting(false);
      }
    },
    [lines, connections, notify]
  );

  const inspectQuarkContent = useCallback(
    async (targetUrl?: string, passCode?: string) => {
      const single = targetUrl || (lines.length === 1 ? lines[0].trim() : "");
      if (!isQuarkUrl(single)) return;
      setQuarkInspecting(true);
      setQuarkPassCodeError(undefined);
      try {
        const info = await inspectQuarkShare(single, passCode);
        setQuarkShareInfo(info);
        setQuarkOpen(true);
        if (info.pass_code_required) {
          setQuarkSelectedIds(new Set());
        } else {
          const onlyFiles = info.files.filter((f) => f.kind === "drive#file");
          setQuarkSelectedIds(new Set(onlyFiles.map((f) => f.id)));
          if (info.title && !userEditedFileName.current) {
            setFileName(info.title);
          }
          if (!userEditedConnections.current && connections < 16) {
            setConnections(16);
          }
        }
      } catch (err: any) {
        console.warn("解析夸克分享失败:", err);
        const errMsg = err?.message || String(err);
        setQuarkPassCodeError(errMsg);
        notify?.(errMsg, "error");
      } finally {
        setQuarkInspecting(false);
      }
    },
    [lines, connections, notify]
  );

  const inspectBaiduContent = useCallback(
    async (targetUrl?: string, passCode?: string) => {
      const single = targetUrl || (lines.length === 1 ? lines[0].trim() : "");
      if (!isBaiduUrl(single)) return;
      setBaiduInspecting(true);
      setBaiduPassCodeError(undefined);
      try {
        const info = await inspectBaiduShare(single, passCode);
        setBaiduShareInfo(info);
        setBaiduOpen(true);
        if (info.pass_code_required) {
          setBaiduSelectedIds(new Set());
        } else {
          const onlyFiles = info.files.filter((f) => f.kind === "drive#file");
          setBaiduSelectedIds(new Set(onlyFiles.map((f) => f.id)));
          if (info.title && !userEditedFileName.current) {
            setFileName(info.title);
          }
          if (!userEditedConnections.current && connections < 16) {
            setConnections(16);
          }
        }
      } catch (err: any) {
        console.warn("解析百度网盘分享失败:", err);
        const errMsg = err?.message || String(err);
        setBaiduPassCodeError(errMsg);
        notify?.(errMsg, "error");
      } finally {
        setBaiduInspecting(false);
      }
    },
    [lines, connections, notify]
  );

  const inspectLanzouContent = useCallback(
    async (targetUrl?: string, passCode?: string) => {
      const single = targetUrl || (lines.length === 1 ? lines[0].trim() : "");
      if (!isLanzouUrl(single)) return;
      setLanzouInspecting(true);
      setLanzouPassCodeError(undefined);
      try {
        const info = await api.lanzouInspectShare({ url: single, pass_code: passCode });
        setLanzouShareInfo(info);
        setLanzouOpen(true);
        if (info.requires_password && info.files.length === 0) {
          setLanzouSelectedIds(new Set());
        } else {
          setLanzouSelectedIds(new Set(info.files.map((f) => f.id)));
          if (info.title && !userEditedFileName.current) {
            setFileName(info.title);
          }
          if (!userEditedConnections.current && connections < 32) {
            setConnections(32);
          }
        }
      } catch (err: any) {
        console.warn("解析蓝奏云分享失败:", err);
        const errMsg = err?.message || String(err);
        setLanzouPassCodeError(errMsg);
        notify?.(errMsg, "error");
      } finally {
        setLanzouInspecting(false);
      }
    },
    [lines, connections, notify]
  );

  const inspectPan123Content = useCallback(
    async (targetUrl?: string, passCode?: string) => {
      const single = targetUrl || (lines.length === 1 ? lines[0].trim() : "");
      if (!isPan123Url(single)) return;
      setPan123Inspecting(true);
      setPan123PassCodeError(undefined);
      try {
        const info = await api.pan123InspectShare({ url: single, pass_code: passCode });
        setPan123ShareInfo(info);
        setPan123Open(true);
        if (info.requires_password && info.files.length === 0) {
          setPan123SelectedIds(new Set());
        } else {
          const onlyFiles = info.files.filter((f) => f.kind !== "folder");
          setPan123SelectedIds(new Set(onlyFiles.map((f) => String(f.id))));
          if (info.title && !userEditedFileName.current) {
            setFileName(info.title);
          }
          if (!userEditedConnections.current && connections < 32) {
            setConnections(32);
          }
        }
      } catch (err: any) {
        console.warn("解析123云盘分享失败:", err);
        const errMsg = err?.message || String(err);
        setPan123PassCodeError(errMsg);
        notify?.(errMsg, "error");
      } finally {
        setPan123Inspecting(false);
      }
    },
    [lines, connections, notify]
  );

  useEffect(() => {
    if (defaultTorrent?.base64) {
      void inspectTorrentContent(undefined, defaultTorrent.base64);
    }
  }, [defaultTorrent, inspectTorrentContent]);

  useEffect(() => {
    const firstUrl = lines[0];
    if (!firstUrl || userEditedDestination.current) return;
    let active = true;
    void api
      .categoryRuleApply(firstUrl, fileName || "", precheck?.content_type)
      .then((matched) => {
        if (
          active &&
          matched &&
          matched.trim() &&
          !userEditedDestination.current
        ) {
          setDestination(matched);
        }
      });
    return () => {
      active = false;
    };
  }, [lines, fileName, precheck?.content_type]);

  useEffect(() => {
    const firstUrl = lines[0];
    if (
      !firstUrl ||
      lines.length !== 1 ||
      !isDownloadableUrlForDialog(firstUrl)
    ) {
      setTemplateMatch(undefined);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const result = await api.taskTemplateTest(firstUrl);
        if (!cancelled) setTemplateMatch(result);
      } catch {
        if (!cancelled) setTemplateMatch(undefined);
      }
    }, 300);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [lines]);

  useEffect(() => {
    const firstUrl = lines[0];
    if (
      !firstUrl ||
      lines.length !== 1 ||
      !isDownloadableUrlForDialog(firstUrl)
    ) {
      setDetectedPlatform(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const platform = await api.mediaDetectPlatform(firstUrl);
        if (!cancelled) setDetectedPlatform(platform);
      } catch {
        if (!cancelled) setDetectedPlatform(null);
      }
    }, 300);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [lines]);

  useEffect(() => {
    if (!detectedPlatform || detectedPlatform === "unknown") {
      setPlatformCompat(null);
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const compat = await api.platformCompatibilityGet(detectedPlatform);
        if (!cancelled) setPlatformCompat(compat);
      } catch {
        if (!cancelled) setPlatformCompat(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [detectedPlatform]);

  useEffect(() => {
    const firstUrl = lines[0];
    if (
      !firstUrl ||
      lines.length !== 1 ||
      !isDownloadableUrlForDialog(firstUrl)
    ) {
      setMatchedCredentialDomain(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const domain = extractDomainForHint(firstUrl);
        if (!domain) {
          if (!cancelled) setMatchedCredentialDomain(null);
          return;
        }
        const credential = await api.mediaCredentialGet(domain);
        if (!cancelled) setMatchedCredentialDomain(credential ? domain : null);
      } catch {
        if (!cancelled) setMatchedCredentialDomain(null);
      }
    }, 300);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [lines]);

  useEffect(() => {
    const trimmed = urls.trim();
    if (!trimmed) {
      setNormalizedUrlPreview(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const normalized = await api.mediaNormalizeUrl(trimmed);
        if (cancelled) return;
        setNormalizedUrlPreview(normalized);
      } catch {
        if (!cancelled) setNormalizedUrlPreview(null);
      }
    }, 400);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [urls]);

  useEffect(() => {
    const firstUrl = lines[0];
    setIgnoreUrlConflict(false);
    if (
      !firstUrl ||
      lines.length !== 1 ||
      !isDownloadableUrlForDialog(firstUrl)
    ) {
      setPrecheck(undefined);
      setPrecheckError(undefined);
      setPrecheckLoading(false);
      setDuplicateResult(undefined);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      if (cancelled) return;
      const seq = ++precheckSeqRef.current;
      setPrecheckLoading(true);
      setPrecheckError(undefined);
      setDuplicateResult(undefined);

      const reqHeaders: Record<string, string> = {};
      if (referer) reqHeaders.Referer = referer;
      if (cookie) reqHeaders.Cookie = cookie;
      if (authorization) reqHeaders.Authorization = authorization;

      try {
        const result = await api.precheck({
          url: firstUrl,
          target_directory: destination || undefined,
          suggested_filename: userEditedFileName.current
            ? fileName || undefined
            : undefined,
          headers:
            Object.keys(reqHeaders).length > 0 ? reqHeaders : undefined,
        });
        if (cancelled || seq !== precheckSeqRef.current) return;
        setPrecheck(result);
        if (!userEditedConnections.current && result.suggested_connections) {
          setConnections(result.suggested_connections);
        }
        const finalFileName =
          result.file_name && cleanupRules.length > 0
            ? applyFilenameCleanup(result.file_name, cleanupRules)
            : result.file_name;
        const effectiveFileName = userEditedFileName.current
          ? fileName
          : finalFileName || fileName;
        if (!userEditedFileName.current && finalFileName) {
          setFileName(finalFileName);
        }
        try {
          const sep =
            destination.endsWith("/") || destination.endsWith("\\") ? "" : "/";
          const targetPath = effectiveFileName
            ? `${destination}${sep}${effectiveFileName}`
            : destination;
          const dup = await api.duplicateCheck(firstUrl, targetPath, {
            fileSize: result.file_size,
          });
          if (!cancelled && seq === precheckSeqRef.current)
            setDuplicateResult(dup);
        } catch {
          if (!cancelled && seq === precheckSeqRef.current)
            setDuplicateResult(undefined);
        }
      } catch (err) {
        if (cancelled || seq !== precheckSeqRef.current) return;
        setPrecheckError(String(err));
      } finally {
        if (!cancelled && seq === precheckSeqRef.current)
          setPrecheckLoading(false);
      }
    }, 400);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [urls, destination, referer, cookie, authorization]);

  const runPrecheckNow = useCallback(() => {
    const firstUrl = lines[0];
    if (
      !firstUrl ||
      lines.length !== 1 ||
      !isDownloadableUrlForDialog(firstUrl)
    )
      return;
    const seq = ++precheckSeqRef.current;
    setPrecheckLoading(true);
    setPrecheckError(undefined);

    const reqHeaders: Record<string, string> = {};
    if (referer) reqHeaders.Referer = referer;
    if (cookie) reqHeaders.Cookie = cookie;
    if (authorization) reqHeaders.Authorization = authorization;

    void (async () => {
      try {
        const result = await api.precheck({
          url: firstUrl,
          target_directory: destination || undefined,
          suggested_filename: userEditedFileName.current
            ? fileName || undefined
            : undefined,
          headers:
            Object.keys(reqHeaders).length > 0 ? reqHeaders : undefined,
        });
        if (seq !== precheckSeqRef.current) return;
        setPrecheck(result);
        if (!userEditedConnections.current && result.suggested_connections) {
          setConnections(result.suggested_connections);
        }
        const finalFileName =
          result.file_name && cleanupRules.length > 0
            ? applyFilenameCleanup(result.file_name, cleanupRules)
            : result.file_name;
        const effectiveFileName = userEditedFileName.current
          ? fileName
          : finalFileName || fileName;
        if (!userEditedFileName.current && finalFileName) {
          setFileName(finalFileName);
        }
        try {
          const sep =
            destination.endsWith("/") || destination.endsWith("\\") ? "" : "/";
          const targetPath = effectiveFileName
            ? `${destination}${sep}${effectiveFileName}`
            : destination;
          const dup = await api.duplicateCheck(firstUrl, targetPath, {
            fileSize: result.file_size,
          });
          if (seq === precheckSeqRef.current) setDuplicateResult(dup);
        } catch {
          if (seq === precheckSeqRef.current) setDuplicateResult(undefined);
        }
      } catch (err) {
        if (seq !== precheckSeqRef.current) return;
        setPrecheckError(String(err));
      } finally {
        if (seq === precheckSeqRef.current) setPrecheckLoading(false);
      }
    })();
  }, [lines, destination, referer, cookie, authorization, fileName, cleanupRules]);

  const suffixedFileName = (name: string): string => {
    if (!name) return `download-${Date.now()}.bin`;
    const dotIndex = name.lastIndexOf(".");
    if (dotIndex <= 0) return `${name}-${Date.now()}`;
    return `${name.slice(0, dotIndex)}-${Date.now()}${name.slice(dotIndex)}`;
  };

  const activeConflicts = useMemo(() => {
    if (!precheck?.conflicts?.length) return [];
    const sep =
      destination.endsWith("/") || destination.endsWith("\\") ? "" : "/";
    const currentTargetPath =
      destination && fileName
        ? `${destination}${sep}${fileName}`.toLowerCase()
        : "";

    return precheck.conflicts.filter((conflict) => {
      if (
        ignoreUrlConflict &&
        (conflict.conflict_type === "duplicate-url" ||
          conflict.conflict_type === "duplicate-final-url")
      ) {
        return false;
      }
      if (conflict.conflict_type === "duplicate-target-path") {
        if (!currentTargetPath || !allTasks?.length) return true;
        return allTasks.some((t) => {
          if (t.status === "cancelled") return false;
          const tPath = `${t.destination}${
            t.destination.endsWith("/") || t.destination.endsWith("\\")
              ? ""
              : "/"
          }${t.file_name}`.toLowerCase();
          return tPath === currentTargetPath;
        });
      }
      return true;
    });
  }, [precheck?.conflicts, ignoreUrlConflict, destination, fileName, allTasks]);

  const isPikPak = isPikPakShareUrl(lines.length === 1 ? lines[0].trim() : "");
  const isPikPakActive = Boolean(isPikPak && pikpakShareInfo && pikpakSelectedIds.size > 0);
  const isPikPakWithoutSelection = Boolean(isPikPak && pikpakShareInfo && pikpakSelectedIds.size === 0);

  const isQuark = isQuarkUrl(lines.length === 1 ? lines[0].trim() : "");
  const isQuarkActive = Boolean(isQuark && quarkShareInfo && quarkSelectedIds.size > 0);
  const isQuarkWithoutSelection = Boolean(isQuark && quarkShareInfo && quarkSelectedIds.size === 0);

  const isBaidu = isBaiduUrl(lines.length === 1 ? lines[0].trim() : "");
  const isBaiduActive = Boolean(isBaidu && baiduShareInfo && baiduSelectedIds.size > 0);
  const isBaiduWithoutSelection = Boolean(isBaidu && baiduShareInfo && baiduSelectedIds.size === 0);

  const isLanzou = isLanzouUrl(lines.length === 1 ? lines[0].trim() : "");
  const isLanzouActive = Boolean(isLanzou && lanzouShareInfo && lanzouSelectedIds.size > 0);
  const isLanzouWithoutSelection = Boolean(isLanzou && lanzouShareInfo && lanzouSelectedIds.size === 0);

  const isPan123 = isPan123Url(lines.length === 1 ? lines[0].trim() : "");
  const isPan123Active = Boolean(isPan123 && pan123ShareInfo && pan123SelectedIds.size > 0);
  const isPan123WithoutSelection = Boolean(isPan123 && pan123ShareInfo && pan123SelectedIds.size === 0);

  const isAnyCloudActive = isPikPakActive || isQuarkActive || isBaiduActive || isLanzouActive || isPan123Active;

  const hasConflicts = !isAnyCloudActive && activeConflicts.length > 0;
  const hasDuplicates = !isAnyCloudActive && Boolean(duplicateResult?.matches?.length);
  const isGalleryWithoutSelection =
    (media?.media_type === "gallery" && selectedImageIds.size === 0) ||
    (media?.media_type === "collection" && selectedEpisodeIndices.size === 0);
  const showConflictOptions = hasConflicts && !hasDuplicates;

  const { queueDiskTotal, queueUnknownCount } = useMemo(() => {
    if (!allTasks?.length || !destination)
      return { queueDiskTotal: 0, queueUnknownCount: 0 };
    const getVolume = (p: string) => {
      const norm = p.replace(/\//g, "\\");
      const match = /^([a-zA-Z]:)/.exec(norm);
      return match ? match[1].toUpperCase() : "\\";
    };
    const targetVolume = getVolume(destination);
    let total = 0;
    let unknownCount = 0;
    const activeStatuses = new Set([
      "downloading",
      "queued",
      "scheduled",
      "verifying",
      "paused-by-low-disk",
      "waiting-network",
    ]);

    for (const task of allTasks) {
      if (!activeStatuses.has(task.status)) continue;
      if (getVolume(task.destination) !== targetVolume) continue;
      if (task.total_bytes > 0) {
        const remaining =
          task.total_bytes > task.downloaded_bytes
            ? task.total_bytes - task.downloaded_bytes
            : 0;
        const isMulti = task.connection_count > 1;
        total += isMulti
          ? remaining + task.total_bytes + 100 * 1024 * 1024
          : remaining + 50 * 1024 * 1024;
      } else {
        unknownCount++;
      }
    }
    return { queueDiskTotal: total, queueUnknownCount: unknownCount };
  }, [allTasks, destination]);

  const hhmmToNextDatetimeLocal = (hhmm: string): string => {
    const match = /^(\d{1,2}):(\d{2})$/.exec(hhmm.trim());
    if (!match) return "";
    const hour = Number(match[1]);
    const minute = Number(match[2]);
    if (hour > 23 || minute > 59) return "";
    const now = new Date();
    const target = new Date(
      now.getFullYear(),
      now.getMonth(),
      now.getDate(),
      hour,
      minute,
      0,
      0
    );
    if (target.getTime() <= now.getTime()) {
      target.setDate(target.getDate() + 1);
    }
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${target.getFullYear()}-${pad(target.getMonth() + 1)}-${pad(
      target.getDate()
    )}T${pad(hour)}:${pad(minute)}`;
  };

  const applyPreset = (preset: DownloadPreset | undefined) => {
    if (!preset) {
      setSelectedPresetId("");
      return;
    }
    setSelectedPresetId(preset.id);
    setConnections(preset.connections);
    setLimit(preset.speed_limit ? Math.round(preset.speed_limit / 1024) : 0);
    setCompletionAction(preset.completion_action ?? "none");
    setSchedule(
      preset.scheduled_at ? hhmmToNextDatetimeLocal(preset.scheduled_at) : ""
    );
  };

  const probe = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const result = await api.probeMedia(lines[0], {
        cookie: cookie || undefined,
        referer: referer || undefined,
      });
      if (result.drm)
        throw new Error("检测到 DRM 保护，猫步下载器不处理此内容");
      setMedia(result);
      setSubtitleLangs([]);
      if (result.media_type === "gallery") {
        setFormat("");
        const imageItems = result.formats.filter((item) => item.image_url);
        setSelectedImageIds(new Set(imageItems.map((item) => item.id)));
        if (!fileName) setFileName(safeDisplayName(result.title));
      } else if (result.media_type === "collection") {
        setFormat("");
        const eps = result.episodes || [];
        setSelectedEpisodeIndices(new Set(eps.map((e) => e.index)));
        if (!fileName) setFileName(safeDisplayName(result.title));
      } else if (result.media_type === "audio") {
        const audioFormats = result.formats
          .filter((item) => item.has_audio && !item.has_video)
          .sort((a, b) => (b.file_size ?? 0) - (a.file_size ?? 0));
        const selected = audioFormats[0] ?? result.formats[0];
        setFormat(selected?.id ?? "");
        if (!fileName) setFileName(`${safeDisplayName(result.title)}.m4a`);
      } else {
        const hasFfmpeg = toolStatus?.ffmpeg_available ?? false;
        const directOrVideo = result.formats
          .filter((item) => item.has_video && !item.requires_ffmpeg)
          .sort((a, b) => (b.height ?? 0) - (a.height ?? 0));
        const merged = hasFfmpeg
          ? result.formats
              .filter(
                (item) =>
                  item.has_video && item.has_audio && item.requires_ffmpeg
              )
              .sort((a, b) => (b.height ?? 0) - (a.height ?? 0))
          : [];
        const selected = directOrVideo[0] ?? merged[0] ?? result.formats[0];
        setFormat(selected?.id ?? "");
        if (!fileName) setFileName(`${safeDisplayName(result.title)}.mp4`);
      }
    } catch (reason) {
      const text = String(reason);
      if (text.includes("MEDIA_YT_DLP_MISSING"))
        setToolStatus(await api.toolStatus());
      else setError(text);
    } finally {
      setBusy(false);
    }
  };

  const performSubmit = async (
    overrideFileName?: string,
    ignoreDiskSpace = false
  ) => {
    if (!lines.length) return;
    setBusy(true);
    setError(undefined);
    const activeFileName =
      overrideFileName !== undefined ? overrideFileName : fileName;
    const firstLine = lines[0].trim();
    const isMagnet = firstLine.toLowerCase().startsWith("magnet:");
    const isTorrentFile =
      !isMagnet &&
      !/^https?:/i.test(firstLine) &&
      /\.torrent$/i.test(firstLine);
    if (isMagnet || isTorrentFile) {
      if (lines.length > 1) {
        setError(
          isMagnet
            ? "磁力链接请单独添加，一次一条"
            : "种子文件请单独添加，一次一个"
        );
        setBusy(false);
        return;
      }
      try {
        const task = await api.addBt({
          source: torrentData ? torrentData.name : firstLine,
          source_data_base64: torrentData ? torrentData.base64 : null,
          destination:
            destination && destination.trim() !== "" ? destination : null,
          selected_files:
            selectedBtFileIndices.size > 0
              ? Array.from(selectedBtFileIndices)
              : undefined,
          start_paused: btStartPaused,
          streaming_priority: btStreaming,
          source_tag: "desktop",
        });
        onCreated(task);
      } catch (reason) {
        setError(String(reason));
        setBusy(false);
      }
      return;
    }
    if (
      !ignoreDiskSpace &&
      (precheck?.disk_state === "insufficient" ||
        (precheck && !precheck.disk_ok))
    ) {
      setBusy(false);
      setDiskConfirm({ override: overrideFileName });
      return;
    }
    const headers: Record<string, string> = {};
    if (referer) headers.Referer = referer;
    if (cookie) headers.Cookie = cookie;
    if (authorization) headers.Authorization = authorization;
    const selectedFormat = media?.formats.find((item) => item.id === format);
    if (selectedFormat?.requires_ffmpeg && !toolStatus?.ffmpeg_available) {
      setError("当前最高画质需要先安装 FFmpeg 高清合并组件");
      setBusy(false);
      return;
    }
    if (media?.media_type === "collection") {
      const eps = (media.episodes || []).filter((e) =>
        selectedEpisodeIndices.has(e.index)
      );
      if (eps.length === 0) {
        setError("请至少选择一集再开始下载");
        setBusy(false);
        return;
      }
      const collectionTitleBase = safeDisplayName(
        activeFileName || media.title
      );
      const baseTemplate: Omit<NewTaskRequest, "url" | "file_name"> = {
        destination,
        headers,
        scheduled_at: schedule ? new Date(schedule).getTime() : undefined,
        priority,
        expected_checksum: checksum || undefined,
        source: "desktop",
        per_task_speed_limit: limit * 1024,
        collision_policy: policy,
        completion_action: eps.length > 1 ? "none" : completionAction,
        connection_count: connections,
        media: undefined,
        user_edited_file_name:
          userEditedFileName.current || overrideFileName !== undefined,
      };
      try {
        const results = await Promise.allSettled(
          eps.map((ep) => {
            const epTitle = safeDisplayName(ep.title);
            const itemFileName = `${collectionTitleBase}_P${ep.index}_${epTitle}.mp4`;
            return api.add({ url: ep.url, file_name: itemFileName, ...baseTemplate });
          })
        );
        const fulfilled: DownloadTask[] = [];
        let firstError: string | undefined;
        for (const r of results) {
          if (r.status === "fulfilled") fulfilled.push(r.value);
          else if (!firstError) firstError = String(r.reason);
        }
        void api.urlHistoryAdd(lines[0]).then(reloadHistory).catch(() => {});
        if (fulfilled.length > 0) {
          onCreated(fulfilled.length === 1 ? fulfilled[0] : fulfilled);
        }
        if (firstError) {
          if (fulfilled.length === 0) {
            setError(firstError);
          } else {
            notify?.(
              `部分集数创建失败：${firstError}（成功 ${fulfilled.length}/${eps.length}）`,
              "error"
            );
          }
        }
      } catch (reason) {
        setError(String(reason));
      }
      setBusy(false);
      return;
    }
    if (media?.media_type === "gallery") {
      const imageItems = media.formats.filter(
        (item) => item.image_url && selectedImageIds.has(item.id)
      );
      if (imageItems.length === 0) {
        setError("请至少选择一张图片再开始下载");
        setBusy(false);
        return;
      }
      const titleBase = safeDisplayName(activeFileName || media.title);
      const baseTemplate: Omit<NewTaskRequest, "url" | "file_name"> = {
        destination,
        headers,
        scheduled_at: schedule ? new Date(schedule).getTime() : undefined,
        priority,
        expected_checksum: checksum || undefined,
        source: "desktop",
        per_task_speed_limit: limit * 1024,
        collision_policy: policy,
        completion_action: imageItems.length > 1 ? "none" : completionAction,
        connection_count: connections,
        media: undefined,
        user_edited_file_name:
          userEditedFileName.current || overrideFileName !== undefined,
      };
      try {
        const results = await Promise.allSettled(
          imageItems.map((item, index) => {
            const ext = (item.extension || "jpg")
              .replace(/^\./, "")
              .toLowerCase();
            const itemFileName = `${titleBase}_${index + 1}.${ext}`;
            return api.add({
              url: item.image_url!,
              file_name: itemFileName,
              ...baseTemplate,
            });
          })
        );
        const fulfilled: DownloadTask[] = [];
        let firstError: string | undefined;
        for (const r of results) {
          if (r.status === "fulfilled") fulfilled.push(r.value);
          else if (!firstError) firstError = String(r.reason);
        }
        void api.urlHistoryAdd(lines[0]).then(reloadHistory).catch(() => {});
        if (fulfilled.length > 0) {
          onCreated(fulfilled.length === 1 ? fulfilled[0] : fulfilled);
        }
        if (firstError) {
          if (fulfilled.length === 0) {
            setError(firstError);
          } else {
            notify?.(
              `部分图片创建失败：${firstError}（成功 ${fulfilled.length}/${imageItems.length}）`,
              "error"
            );
          }
        }
      } catch (reason) {
        setError(String(reason));
      }
      setBusy(false);
      return;
    }

    // 处理 PikPak 分享文件批量/单文件直链下载
    if (pikpakShareInfo && pikpakSelectedIds.size > 0) {
      const selectedFiles = pikpakShareInfo.files.filter(
        (f) => f.kind === "drive#file" && pikpakSelectedIds.has(f.id)
      );
      if (selectedFiles.length === 0) {
        setError("请至少勾选一个需要下载的 PikPak 文件");
        setBusy(false);
        return;
      }

      const pikpakHeaders = {
        "User-Agent":
          "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        Referer: "https://mypikpak.com/",
        ...headers,
      };

      const baseTemplate: Omit<NewTaskRequest, "url" | "file_name"> = {
        destination,
        headers: pikpakHeaders,
        scheduled_at: schedule ? new Date(schedule).getTime() : undefined,
        priority,
        expected_checksum: checksum || undefined,
        source: "desktop",
        per_task_speed_limit: limit * 1024,
        collision_policy: policy,
        completion_action:
          selectedFiles.length > 1 ? "none" : completionAction,
        connection_count: connections || 32,
        media: undefined,
        user_edited_file_name:
          userEditedFileName.current || overrideFileName !== undefined,
      };

      try {
        const currentShareId =
          pikpakShareInfo.shareId ||
          (pikpakShareInfo as any).share_id ||
          parsePikPakShareUrl(lines[0])?.shareId ||
          "";
        const currentPassCodeToken =
          pikpakShareInfo.passCodeToken ||
          (pikpakShareInfo as any).pass_code_token;

        const results = await Promise.allSettled(
          selectedFiles.map(async (fileItem) => {
            const { url: directUrl } = await resolvePikPakDirectUrl(
              currentShareId,
              fileItem.id,
              currentPassCodeToken
            );
            const itemFileName =
              selectedFiles.length === 1 && userEditedFileName.current && activeFileName
                ? activeFileName
                : fileItem.name;
            return api.add({
              url: directUrl,
              file_name: itemFileName,
              ...baseTemplate,
              cloud_refresh: {
                platform: "pikpak",
                share_id: currentShareId,
                file_id: fileItem.id,
                pass_code_token: currentPassCodeToken ?? null,
                device_id: getOrCreateDeviceId(),
              },
            });
          })
        );

        const fulfilled: DownloadTask[] = [];
        let firstError: string | undefined;
        for (const r of results) {
          if (r.status === "fulfilled") fulfilled.push(r.value);
          else if (!firstError) firstError = String(r.reason);
        }
        if (lines[0]) {
          void api.urlHistoryAdd(lines[0]).then(reloadHistory).catch(() => {});
        }
        if (fulfilled.length > 0) {
          onCreated(fulfilled.length === 1 ? fulfilled[0] : fulfilled);
        }
        if (firstError) {
          if (fulfilled.length === 0) {
            setError(firstError);
          } else {
            notify?.(
              `部分 PikPak 文件创建失败：${firstError}（成功 ${fulfilled.length}/${selectedFiles.length}）`,
              "error"
            );
          }
        }
      } catch (reason) {
        setError(String(reason));
      }
      setBusy(false);
      return;
    }

    // 处理 Quark 分享文件批量/单文件直链下载
    if (quarkShareInfo && quarkSelectedIds.size > 0) {
      const selectedFiles = quarkShareInfo.files.filter(
        (f) => f.kind === "drive#file" && quarkSelectedIds.has(f.id)
      );
      if (selectedFiles.length === 0) {
        setError("请至少勾选一个需要下载的夸克文件");
        setBusy(false);
        return;
      }

      const baseTemplate: Omit<NewTaskRequest, "url" | "file_name"> = {
        destination,
        headers,
        scheduled_at: schedule ? new Date(schedule).getTime() : undefined,
        priority,
        expected_checksum: checksum || undefined,
        source: "desktop",
        per_task_speed_limit: limit * 1024,
        collision_policy: policy,
        completion_action:
          selectedFiles.length > 1 ? "none" : completionAction,
        connection_count: connections || 32,
        media: undefined,
        user_edited_file_name:
          userEditedFileName.current || overrideFileName !== undefined,
      };

      try {
        const currentPwdId =
          quarkShareInfo.pwd_id ||
          parseQuarkUrl(lines[0])?.pwdId ||
          "";
        const currentStoken = quarkShareInfo.stoken;

        let effectiveCookie = headers?.["Cookie"] || headers?.["cookie"] || cookie;
        if (!effectiveCookie) {
          const storedCred = await api.mediaCredentialGet("pan.quark.cn").catch(() => null);
          if (storedCred?.cookie) {
            effectiveCookie = storedCred.cookie;
          }
        }

        const results = await Promise.allSettled(
          selectedFiles.map(async (fileItem) => {
            const { url: directUrl, headers: directHeaders } = await resolveQuarkFile(
              currentPwdId,
              fileItem.id,
              fileItem.share_fid_token,
              currentStoken,
              effectiveCookie
            );
            const itemFileName =
              selectedFiles.length === 1 && activeFileName
                ? activeFileName
                : fileItem.path || fileItem.name;
            return api.add({
              url: directUrl,
              file_name: itemFileName,
              ...baseTemplate,
              headers: {
                ...directHeaders,
                ...headers,
                ...(effectiveCookie ? { Cookie: effectiveCookie } : {}),
              },
            });
          })
        );

        const fulfilled: DownloadTask[] = [];
        let firstError: string | undefined;
        for (const r of results) {
          if (r.status === "fulfilled") fulfilled.push(r.value);
          else if (!firstError) firstError = String(r.reason);
        }
        if (lines[0]) {
          void api.urlHistoryAdd(lines[0]).then(reloadHistory).catch(() => {});
        }
        if (fulfilled.length > 0) {
          onCreated(fulfilled.length === 1 ? fulfilled[0] : fulfilled);
        }
        if (firstError) {
          if (fulfilled.length === 0) {
            setError(firstError);
          } else {
            notify?.(
              `部分夸克文件创建失败：${firstError}（成功 ${fulfilled.length}/${selectedFiles.length}）`,
              "error"
            );
          }
        }
      } catch (reason) {
        setError(String(reason));
      }
      setBusy(false);
      return;
    }

    if (baiduShareInfo && baiduSelectedIds.size > 0) {
      const selectedFiles = baiduShareInfo.files.filter(
        (f) => f.kind === "drive#file" && baiduSelectedIds.has(f.id)
      );
      if (selectedFiles.length === 0) {
        setError("请至少勾选一个文件再开始下载");
        setBusy(false);
        return;
      }

      const baseTemplate: Omit<NewTaskRequest, "url" | "file_name"> = {
        destination,
        headers,
        scheduled_at: schedule ? new Date(schedule).getTime() : undefined,
        priority,
        expected_checksum: checksum || undefined,
        source: "desktop",
        per_task_speed_limit: limit * 1024,
        collision_policy: policy,
        completion_action:
          selectedFiles.length > 1 ? "none" : completionAction,
        connection_count: connections || 16,
        media: undefined,
        user_edited_file_name:
          userEditedFileName.current || overrideFileName !== undefined,
      };

      try {
        const currentSurl =
          baiduShareInfo.surl ||
          parseBaiduUrl(lines[0])?.surl ||
          "";

        let effectiveCookie = headers?.["Cookie"] || headers?.["cookie"] || cookie;
        if (!effectiveCookie) {
          const storedCred =
            (await api.mediaCredentialGet("pan.baidu.com").catch(() => null)) ||
            (await api.mediaCredentialGet("baidu.com").catch(() => null));
          if (storedCred?.cookie) {
            effectiveCookie = storedCred.cookie;
          }
        }

        const results = await Promise.allSettled(
          selectedFiles.map(async (fileItem) => {
            const { url: directUrl, headers: directHeaders } = await resolveBaiduFile(
              currentSurl,
              fileItem.id,
              baiduShareInfo.shareId || baiduShareInfo.share_id,
              baiduShareInfo.uk,
              baiduShareInfo.sign,
              baiduShareInfo.timestamp,
              baiduShareInfo.seckey,
              baiduShareInfo.randsk,
              effectiveCookie
            );
            const itemFileName =
              selectedFiles.length === 1 && activeFileName
                ? activeFileName
                : fileItem.path || fileItem.name;
            const finalHeaders: Record<string, string> = {
              ...headers,
              ...directHeaders,
            };
            if (!finalHeaders["Cookie"] && !finalHeaders["cookie"] && effectiveCookie) {
              finalHeaders["Cookie"] = effectiveCookie;
            }
            return api.add({
              url: directUrl,
              file_name: itemFileName,
              ...baseTemplate,
              headers: finalHeaders,
            });
          })
        );

        const fulfilled: DownloadTask[] = [];
        let firstError: string | undefined;
        for (const r of results) {
          if (r.status === "fulfilled") fulfilled.push(r.value);
          else if (!firstError) firstError = String(r.reason);
        }
        if (lines[0]) {
          void api.urlHistoryAdd(lines[0]).then(reloadHistory).catch(() => {});
        }
        if (fulfilled.length > 0) {
          onCreated(fulfilled.length === 1 ? fulfilled[0] : fulfilled);
        }
        if (firstError) {
          if (fulfilled.length === 0) {
            setError(firstError);
          } else {
            notify?.(
              `部分百度网盘文件创建失败：${firstError}（成功 ${fulfilled.length}/${selectedFiles.length}）`,
              "error"
            );
          }
        }
      } catch (reason) {
        setError(String(reason));
      }
      setBusy(false);
      return;
    }

    if (lanzouShareInfo && lanzouSelectedIds.size > 0) {
      const selectedFiles = lanzouShareInfo.files.filter(
        (f) => lanzouSelectedIds.has(f.id)
      );
      if (selectedFiles.length === 0) {
        setError("请至少勾选一个文件再开始下载");
        setBusy(false);
        return;
      }

      const baseTemplate: Omit<NewTaskRequest, "url" | "file_name"> = {
        destination,
        headers,
        scheduled_at: schedule ? new Date(schedule).getTime() : undefined,
        priority,
        expected_checksum: checksum || undefined,
        source: "lanzou",
        per_task_speed_limit: limit * 1024,
        collision_policy: policy,
        completion_action:
          selectedFiles.length > 1 ? "none" : completionAction,
        connection_count: connections || 32,
        media: undefined,
        user_edited_file_name:
          userEditedFileName.current || overrideFileName !== undefined,
      };

      try {
        const results = await Promise.allSettled(
          selectedFiles.map(async (fileItem) => {
            const { url: directUrl, headers: directHeaders } = await api.lanzouResolveFile({
              share_url: fileItem.url || lines[0].trim(),
              file_id: fileItem.id,
            });
            const itemFileName =
              selectedFiles.length === 1 && activeFileName
                ? activeFileName
                : fileItem.name;
            return api.add({
              url: directUrl,
              file_name: itemFileName,
              ...baseTemplate,
              headers: { ...headers, ...directHeaders },
            });
          })
        );
        const fulfilled: DownloadTask[] = [];
        let firstError: string | undefined;
        for (const r of results) {
          if (r.status === "fulfilled") fulfilled.push(r.value);
          else if (!firstError) firstError = String(r.reason);
        }
        if (lines[0]) {
          void api.urlHistoryAdd(lines[0]).then(reloadHistory).catch(() => {});
        }
        if (fulfilled.length > 0) {
          onCreated(fulfilled.length === 1 ? fulfilled[0] : fulfilled);
        }
        if (firstError) {
          if (fulfilled.length === 0) {
            setError(firstError);
          } else {
            notify?.(
              `部分蓝奏云文件创建失败：${firstError}（成功 ${fulfilled.length}/${selectedFiles.length}）`,
              "error"
            );
          }
        }
      } catch (reason) {
        setError(String(reason));
      }
      setBusy(false);
      return;
    }

    if (pan123ShareInfo && pan123SelectedIds.size > 0) {
      const selectedFiles = pan123ShareInfo.files.filter(
        (f) => f.kind !== "folder" && pan123SelectedIds.has(String(f.id))
      );
      if (selectedFiles.length === 0) {
        setError("请至少勾选一个文件再开始下载");
        setBusy(false);
        return;
      }

      const baseTemplate: Omit<NewTaskRequest, "url" | "file_name"> = {
        destination,
        headers,
        scheduled_at: schedule ? new Date(schedule).getTime() : undefined,
        priority,
        expected_checksum: checksum || undefined,
        source: "pan123",
        per_task_speed_limit: limit * 1024,
        collision_policy: policy,
        completion_action:
          selectedFiles.length > 1 ? "none" : completionAction,
        connection_count: connections || 32,
        media: undefined,
        user_edited_file_name:
          userEditedFileName.current || overrideFileName !== undefined,
      };

      try {
        const results = await Promise.allSettled(
          selectedFiles.map(async (fileItem) => {
            const { url: directUrl, headers: directHeaders } = await api.pan123ResolveFile({
              share_key: pan123ShareInfo.share_key,
              file_id: fileItem.id,
              s3_key_flag: fileItem.s3_key_flag,
              size: fileItem.size,
              etag: fileItem.etag,
            });
            const itemFileName =
              selectedFiles.length === 1 && activeFileName
                ? activeFileName
                : fileItem.name;
            return api.add({
              url: directUrl,
              file_name: itemFileName,
              ...baseTemplate,
              headers: { ...headers, ...directHeaders },
            });
          })
        );
        const fulfilled: DownloadTask[] = [];
        let firstError: string | undefined;
        for (const r of results) {
          if (r.status === "fulfilled") fulfilled.push(r.value);
          else if (!firstError) firstError = String(r.reason);
        }
        if (lines[0]) {
          void api.urlHistoryAdd(lines[0]).then(reloadHistory).catch(() => {});
        }
        if (fulfilled.length > 0) {
          onCreated(fulfilled.length === 1 ? fulfilled[0] : fulfilled);
        }
        if (firstError) {
          if (fulfilled.length === 0) {
            setError(firstError);
          } else {
            notify?.(
              `部分123云盘文件创建失败：${firstError}（成功 ${fulfilled.length}/${selectedFiles.length}）`,
              "error"
            );
          }
        }
      } catch (reason) {
        setError(String(reason));
      }
      setBusy(false);
      return;
    }

    const template: Omit<NewTaskRequest, "url"> = {
      file_name: activeFileName || undefined,
      destination,
      headers,
      scheduled_at: schedule ? new Date(schedule).getTime() : undefined,
      priority,
      expected_checksum: checksum || undefined,
      source: "desktop",
      per_task_speed_limit: limit * 1024,
      collision_policy: policy,
      completion_action: lines.length > 1 ? "none" : completionAction,
      connection_count: connections,
      media: media
        ? {
            extractor: media.extractor,
            format_id: format,
            format_label: selectedFormat?.label,
            subtitles: subtitleLangs,
            thumbnail: media.thumbnail,
            requires_ffmpeg: selectedFormat?.requires_ffmpeg,
          }
        : undefined,
      user_edited_file_name:
        userEditedFileName.current || overrideFileName !== undefined,
    };
    try {
      if (lines.length === 1) {
        const task = await api.add({ url: lines[0], ...template });
        void api.urlHistoryAdd(lines[0]).then(reloadHistory).catch(() => {});
        onCreated(task);
      } else {
        const tasks = await api.addBatch(lines, template);
        void (async () => {
          for (const url of lines) {
            try {
              await api.urlHistoryAdd(url);
            } catch {}
          }
          reloadHistory();
        })();
        onCreated(tasks);
      }
    } catch (reason) {
      setError(String(reason));
      setBusy(false);
    }
  };

  const submit = () => performSubmit();

  const handleRedownloadDirectly = async () => {
    setBusy(true);
    setError(undefined);
    try {
      const allToId = new Set<string>();
      if (duplicateResult?.matches) {
        for (const m of duplicateResult.matches) {
          allToId.add(m.existing_task_id);
        }
      }
      if (precheck?.conflicts) {
        for (const c of precheck.conflicts) {
          allToId.add(c.existing_task_id);
        }
      }
      if (activeConflicts) {
        for (const c of activeConflicts) {
          allToId.add(c.existing_task_id);
        }
      }

      for (const id of allToId) {
        try {
          await api.remove(id, true);
        } catch (e) {
          console.error("Failed to remove duplicate task:", id, e);
        }
      }

      setDuplicateResult(undefined);
      setPrecheck((prev) => (prev ? { ...prev, conflicts: [] } : undefined));
      setIgnoreUrlConflict(true);

      await performSubmit();
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  };

  const handleRenameAndSubmit = async () => {
    const next = suffixedFileName(fileName || precheck?.file_name || "");
    setFileName(next);
    userEditedFileName.current = true;

    setDuplicateResult(undefined);
    setPrecheck((prev) => (prev ? { ...prev, conflicts: [] } : undefined));
    setIgnoreUrlConflict(true);

    await performSubmit(next);
  };

  return (
    <Modal
      title="新建下载任务"
      onClose={onClose}
      escapeClosable={urls.trim() === ""}
      style={{
        display: "flex",
        flexDirection: "column",
        height: "560px",
        maxHeight: "calc(100vh - 80px)",
        overflow: "hidden",
      }}
    >
      <div
        className="new-task-form"
        style={{
          display: "flex",
          flexDirection: "column",
          flex: 1,
          overflow: "hidden",
        }}
      >
        <div
          className="new-task-scrollable"
          style={{
            flex: 1,
            overflowY: "auto",
            overflowX: "hidden",
            paddingRight: "6px",
            display: "flex",
            flexDirection: "column",
            gap: "11px",
          }}
        >
          {presets.length > 0 && (
            <div className="form-group-row">
              <label className="form-field grow">
                <span>下载预设</span>
                <div className="input-group">
                  <Select
                    value={selectedPresetId}
                    onChange={(val: any) => {
                      const id = String(val);
                      setSelectedPresetId(id);
                      const preset = presets.find((p) => p.id === id);
                      applyPreset(preset);
                    }}
                    options={[
                      { value: "", label: "不使用预设" },
                      ...presets.map((p) => ({
                        value: p.id,
                        label: `${p.name}${p.is_builtin ? "（内置）" : ""} · ${
                          p.connections
                        } 连接${
                          p.speed_limit
                            ? ` · 限速 ${Math.round(p.speed_limit / 1024)} KB/s`
                            : ""
                        }${
                          p.completion_action && p.completion_action !== "none"
                            ? ` · ${
                                p.completion_action === "open-folder"
                                  ? "打开文件夹"
                                  : p.completion_action === "run-file"
                                  ? "运行文件"
                                  : p.completion_action === "shutdown"
                                  ? "完成后关机"
                                  : "完成后休眠"
                              }`
                            : ""
                        }${p.scheduled_at ? ` · 计划 ${p.scheduled_at}` : ""}`,
                      })),
                    ]}
                    ariaLabel="选择下载预设"
                  />
                </div>
              </label>
            </div>
          )}
          <div className="form-section">
            <label className="form-field url-input-field">
              <div className="form-label-bar">
                <span>下载链接（每行一个）</span>
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "8px",
                  }}
                >
                  {(() => {
                    if (!normalizedUrlPreview || lines.length !== 1)
                      return null;
                    const firstUrl = lines[0];
                    const isPurified =
                      normalizedUrlPreview.trim().toLowerCase() !==
                      firstUrl.trim().toLowerCase();
                    return (
                      <span
                        className="normalized-badge"
                        title={
                          isPurified
                            ? `链接已自动净化（双击复制完整链接）：\n${normalizedUrlPreview}`
                            : `链接已成功解析并验证（双击复制完整链接）：\n${normalizedUrlPreview}`
                        }
                        style={{
                          display: "inline-flex",
                          alignItems: "center",
                          gap: "2px",
                          padding: "1px 5px",
                          background: isPurified
                            ? "var(--success-bg, rgba(52, 199, 89, 0.12))"
                            : "rgba(0, 120, 212, 0.08)",
                          color: isPurified
                            ? "var(--success, #34c759)"
                            : "var(--accent, #0078d4)",
                          borderRadius: "3px",
                          fontSize: "9px",
                          fontWeight: "normal",
                          cursor: "pointer",
                          border: isPurified
                            ? "1px solid rgba(52, 199, 89, 0.2)"
                            : "1px solid rgba(0, 120, 212, 0.15)",
                        }}
                        onDoubleClick={(e) => {
                          e.stopPropagation();
                          void navigator.clipboard.writeText(
                            normalizedUrlPreview
                          );
                          notify?.("已复制规范化 URL 到剪贴板", "ok");
                        }}
                      >
                        <Check size={8} strokeWidth={3} />
                        {isPurified ? "链接已净化" : "链接已解析"}
                      </span>
                    );
                  })()}
                  {lines.length > 0 && (
                    <span className="form-label-counter">
                      {lines.length > 1
                        ? t("newTask.urlDetectedMulti", {
                            count: lines.length,
                          })
                        : t("newTask.urlDetectedSingle", {
                            count: lines.length,
                          })}
                    </span>
                  )}
                  {sequenceExpanded > 0 && lines.length > 0 && (
                    <span className="form-label-counter">
                      {t("newTask.sequenceExpandedBadge")}
                    </span>
                  )}
                </div>
              </div>
              <div className="url-input-wrap">
                <textarea
                  autoFocus
                  value={urls}
                  onFocus={() => setHistoryOpen(true)}
                  onBlur={() => {
                    window.setTimeout(() => setHistoryOpen(false), 180);
                  }}
                  onChange={(e) => {
                    const val = e.target.value;
                    setUrls(val);
                    setMedia(undefined);
                    setSelectedImageIds(new Set());
                    userEditedFileName.current = false;
                    setTorrentData((current) =>
                      current && val.trim() === current.name ? current : null
                    );
                    const parsed = parseMultilineUrls(val);
                    if (parsed.lines.length === 1) {
                      const singleUrl = parsed.lines[0];
                      if (isPikPakShareUrl(singleUrl)) {
                        void inspectPikPakContent(singleUrl);
                      } else {
                        setPikpakShareInfo(null);
                        setPikpakSelectedIds(new Set());
                      }
                      if (isQuarkUrl(singleUrl)) {
                        void inspectQuarkContent(singleUrl);
                      } else {
                        setQuarkShareInfo(null);
                        setQuarkSelectedIds(new Set());
                      }
                      if (isBaiduUrl(singleUrl)) {
                        void inspectBaiduContent(singleUrl);
                      } else {
                        setBaiduShareInfo(null);
                        setBaiduSelectedIds(new Set());
                      }
                      if (isLanzouUrl(singleUrl)) {
                        void inspectLanzouContent(singleUrl);
                      } else {
                        setLanzouShareInfo(null);
                        setLanzouSelectedIds(new Set());
                      }
                      if (isPan123Url(singleUrl)) {
                        void inspectPan123Content(singleUrl);
                      } else {
                        setPan123ShareInfo(null);
                        setPan123SelectedIds(new Set());
                      }
                      const name = extractFileNameFromUrl(singleUrl);
                      if (name) {
                        setFileName(name);
                      }
                    } else {
                      setPikpakShareInfo(null);
                      setPikpakSelectedIds(new Set());
                      setQuarkShareInfo(null);
                      setQuarkSelectedIds(new Set());
                      setBaiduShareInfo(null);
                      setBaiduSelectedIds(new Set());
                      setLanzouShareInfo(null);
                      setLanzouSelectedIds(new Set());
                      setPan123ShareInfo(null);
                      setPan123SelectedIds(new Set());
                      if (parsed.lines.length === 0) {
                        setFileName("");
                      }
                    }
                  }}
                  placeholder={t("newTask.urlPlaceholder")}
                  aria-label={t("newTask.urlAriaLabel")}
                />
              </div>
              <div className="url-input-tools-bar">
                <div className="url-input-tools-left">
                  {(() => {
                    const single = lines.length === 1 ? lines[0].trim() : "";
                    const magnet = single
                      .toLowerCase()
                      .startsWith("magnet:");
                    const torrent =
                      !magnet &&
                      single !== "" &&
                      !/^https?:/i.test(single) &&
                      /\.torrent$/i.test(single);
                    if (!magnet && !torrent && !torrentData) return null;
                    return (
                      <>
                        <span
                          className="bt-input-badge"
                          title={
                            magnet
                              ? "将作为 BT 任务下载：元数据获取完成后才显示真实文件名与大小"
                              : "将作为 BT 任务下载该种子"
                          }
                        >
                          <Zap size={10} strokeWidth={2.5} />{" "}
                          {magnet ? "磁力任务" : "种子任务"}
                        </span>
                        <label
                          className="bt-start-paused-toggle"
                          title={t("newTask.btStartPausedHint")}
                        >
                          <input
                            type="checkbox"
                            checked={btStartPaused}
                            onChange={(e) =>
                              setBtStartPaused(e.target.checked)
                            }
                          />
                          {t("newTask.btStartPaused")}
                        </label>
                        <label
                          className="bt-start-paused-toggle"
                          title={t("newTask.btStreamingHint")}
                        >
                          <input
                            type="checkbox"
                            checked={btStreaming}
                            onChange={(e) =>
                              setBtStreaming(e.target.checked)
                            }
                          />
                          {t("newTask.btStreaming")}
                        </label>
                      </>
                    );
                  })()}
                  {(() => {
                    const single = lines.length === 1 ? lines[0].trim() : "";
                    if (!isPikPakShareUrl(single)) return null;
                    return (
                      <span
                        className="bt-input-badge"
                        style={{
                          background: "rgba(0, 120, 212, 0.12)",
                          color: "var(--accent, #0078d4)",
                          borderColor: "rgba(0, 120, 212, 0.3)",
                        }}
                        title="已识别为 PikPak 分享链接，支持免登录解析文件树并开启多连接 Range 加速下载"
                      >
                        <Globe2 size={10} strokeWidth={2.5} /> PikPak 分享
                      </span>
                    );
                  })()}
                  {(() => {
                    const single = lines.length === 1 ? lines[0].trim() : "";
                    if (!isQuarkUrl(single)) return null;
                    return (
                      <span
                        className="bt-input-badge"
                        style={{
                          background: "rgba(245, 158, 11, 0.12)",
                          color: "#d97706",
                          borderColor: "rgba(245, 158, 11, 0.3)",
                        }}
                        title="已识别为夸克网盘分享链接，支持解析目录树并开启多连接 Range 加速下载"
                      >
                        <Globe2 size={10} strokeWidth={2.5} /> 夸克分享
                      </span>
                    );
                  })()}
                  {(() => {
                    const single = lines.length === 1 ? lines[0].trim() : "";
                    if (!isBaiduUrl(single)) return null;
                    return (
                      <span
                        className="bt-input-badge"
                        style={{
                          background: "rgba(37, 99, 235, 0.12)",
                          color: "#2563eb",
                          borderColor: "rgba(37, 99, 235, 0.3)",
                        }}
                        title="已识别为百度网盘分享链接，支持解析目录树与直链下载"
                      >
                        <Globe2 size={10} strokeWidth={2.5} /> 百度网盘
                      </span>
                    );
                  })()}
                </div>
                <div className="url-input-tools-right">
                  {(() => {
                    const single = lines.length === 1 ? lines[0].trim() : "";
                    if (!isPikPakShareUrl(single)) return null;
                    return (
                      <button
                        type="button"
                        className="torrent-pick-button"
                        style={{
                          color: "var(--accent, #0078d4)",
                          borderColor: "rgba(0, 120, 212, 0.3)",
                        }}
                        disabled={pikpakInspecting}
                        onClick={() => {
                          if (pikpakShareInfo) {
                            setPikpakOpen((prev) => !prev);
                          } else {
                            void inspectPikPakContent();
                          }
                        }}
                        title="免登录解析 PikPak 分享文件树，支持勾选特定文件下载"
                      >
                        <Search
                          size={11}
                          className={pikpakInspecting ? "spin" : undefined}
                        />
                        {pikpakInspecting
                          ? "解析分享中..."
                          : pikpakShareInfo
                          ? pikpakOpen
                            ? "收起 PikPak 文件"
                            : "展开 PikPak 文件"
                          : "解析 PikPak 分享"}
                      </button>
                    );
                  })()}
                  {(() => {
                    const single = lines.length === 1 ? lines[0].trim() : "";
                    if (!isQuarkUrl(single)) return null;
                    return (
                      <button
                        type="button"
                        className="torrent-pick-button"
                        style={{
                          color: "#d97706",
                          borderColor: "rgba(245, 158, 11, 0.3)",
                        }}
                        disabled={quarkInspecting}
                        onClick={() => {
                          if (quarkShareInfo) {
                            setQuarkOpen((prev) => !prev);
                          } else {
                            void inspectQuarkContent();
                          }
                        }}
                        title="解析夸克网盘分享文件树，支持勾选特定文件下载"
                      >
                        <Search
                          size={11}
                          className={quarkInspecting ? "spin" : undefined}
                        />
                        {quarkInspecting
                          ? "解析分享中..."
                          : quarkShareInfo
                          ? quarkOpen
                            ? "收起夸克文件"
                            : "展开夸克文件"
                          : "解析夸克分享"}
                      </button>
                    );
                  })()}
                  {(() => {
                    const single = lines.length === 1 ? lines[0].trim() : "";
                    if (!isBaiduUrl(single)) return null;
                    return (
                      <button
                        type="button"
                        className="torrent-pick-button"
                        style={{
                          color: "#2563eb",
                          borderColor: "rgba(37, 99, 235, 0.3)",
                        }}
                        disabled={baiduInspecting}
                        onClick={() => {
                          if (baiduShareInfo) {
                            setBaiduOpen((prev) => !prev);
                          } else {
                            void inspectBaiduContent();
                          }
                        }}
                        title="解析百度网盘分享文件树，支持勾选特定文件下载"
                      >
                        <Search
                          size={11}
                          className={baiduInspecting ? "spin" : undefined}
                        />
                        {baiduInspecting
                          ? "解析分享中..."
                          : baiduShareInfo
                          ? baiduOpen
                            ? "收起百度网盘文件"
                            : "展开百度网盘文件"
                          : "解析百度网盘"}
                      </button>
                    );
                  })()}
                  {(() => {
                    const single = lines.length === 1 ? lines[0].trim() : "";
                    if (!isLanzouUrl(single)) return null;
                    return (
                      <button
                        type="button"
                        className="torrent-pick-button"
                        style={{
                          color: "#0284c7",
                          borderColor: "rgba(2, 132, 199, 0.3)",
                        }}
                        disabled={lanzouInspecting}
                        onClick={() => {
                          if (lanzouShareInfo) {
                            setLanzouOpen((prev) => !prev);
                          } else {
                            void inspectLanzouContent();
                          }
                        }}
                        title="解析蓝奏云分享文件树，支持勾选特定文件下载"
                      >
                        <Search
                          size={11}
                          className={lanzouInspecting ? "spin" : undefined}
                        />
                        {lanzouInspecting
                          ? "解析分享中..."
                          : lanzouShareInfo
                          ? lanzouOpen
                            ? "收起蓝奏云文件"
                            : "展开蓝奏云文件"
                          : "解析蓝奏云"}
                      </button>
                    );
                  })()}
                  {(() => {
                    const single = lines.length === 1 ? lines[0].trim() : "";
                    if (!isPan123Url(single)) return null;
                    return (
                      <button
                        type="button"
                        className="torrent-pick-button"
                        style={{
                          color: "#16a34a",
                          borderColor: "rgba(22, 163, 74, 0.3)",
                        }}
                        disabled={pan123Inspecting}
                        onClick={() => {
                          if (pan123ShareInfo) {
                            setPan123Open((prev) => !prev);
                          } else {
                            void inspectPan123Content();
                          }
                        }}
                        title="解析 123云盘公开分享文件树，支持勾选特定文件下载"
                      >
                        <Search
                          size={11}
                          className={pan123Inspecting ? "spin" : undefined}
                        />
                        {pan123Inspecting
                          ? "解析分享中..."
                          : pan123ShareInfo
                          ? pan123Open
                            ? "收起 123云盘文件"
                            : "展开 123云盘文件"
                          : "解析 123云盘"}
                      </button>
                    );
                  })()}
                  {(() => {
                    const single = lines.length === 1 ? lines[0].trim() : "";
                    const magnet = single
                      .toLowerCase()
                      .startsWith("magnet:");
                    const torrent =
                      !magnet &&
                      single !== "" &&
                      !/^https?:/i.test(single) &&
                      /\.torrent$/i.test(single);
                    if (!magnet && !torrent && !torrentData) return null;
                    return (
                      <button
                        type="button"
                        className="torrent-pick-button"
                        disabled={btInspecting}
                        onClick={() => {
                          if (btInspectResult) {
                            setBtInspectOpen((prev) => !prev);
                          } else {
                            void inspectTorrentContent();
                          }
                        }}
                        title={
                          magnet
                            ? "从 DHT/Trackers 网络获取种子元数据并预览/勾选文件"
                            : "解析并查看种子内的文件列表，勾选需要下载的文件"
                        }
                      >
                        <Search
                          size={11}
                          className={btInspecting ? "spin" : undefined}
                        />
                        {btInspecting
                          ? magnet
                            ? "获取元数据中..."
                            : "解析中..."
                          : btInspectResult
                          ? btInspectOpen
                            ? "收起文件"
                            : "展开文件"
                          : "预览文件"}
                      </button>
                    );
                  })()}
                  <button
                    type="button"
                    className="torrent-pick-button"
                    title="选择本地 .torrent 种子文件（走 BT 内核下载）"
                    onClick={async () => {
                      const picked = await pickPath({
                        multiple: false,
                        filters: [
                          {
                            name: "BitTorrent 种子",
                            extensions: ["torrent"],
                          },
                        ],
                      });
                      if (typeof picked === "string" && picked) {
                        setUrls(picked);
                        setTorrentData(null);
                        setMedia(undefined);
                        userEditedFileName.current = false;
                        const stem =
                          picked
                            .replace(/\\/g, "/")
                            .split("/")
                            .pop()
                            ?.replace(/\.torrent$/i, "") || "";
                        if (stem) setFileName(stem);
                        void inspectTorrentContent(picked);
                      }
                    }}
                  >
                    <FileText size={12} /> 种子文件
                  </button>
                </div>
              </div>
              {(() => {
                const single = lines.length === 1 ? lines[0].trim() : "";
                if (!isPikPakShareUrl(single) || !pikpakShareInfo || !pikpakOpen) return null;
                return (
                  <PikPakPicker
                    shareInfo={pikpakShareInfo}
                    selectedIds={pikpakSelectedIds}
                    onChange={setPikpakSelectedIds}
                    verifyingPassCode={pikpakPassCodeVerifying}
                    passCodeError={pikpakPassCodeError}
                    onVerifyPassCode={async (pwd) => {
                      setPikpakPassCodeVerifying(true);
                      await inspectPikPakContent(undefined, pwd);
                      setPikpakPassCodeVerifying(false);
                    }}
                  />
                );
              })()}
              {(() => {
                const single = lines.length === 1 ? lines[0].trim() : "";
                if (!isQuarkUrl(single) || !quarkShareInfo || !quarkOpen) return null;
                return (
                  <QuarkPicker
                    shareInfo={quarkShareInfo}
                    selectedIds={quarkSelectedIds}
                    onChange={setQuarkSelectedIds}
                    verifyingPassCode={quarkPassCodeVerifying}
                    passCodeError={quarkPassCodeError}
                    onVerifyPassCode={async (pwd) => {
                      setQuarkPassCodeVerifying(true);
                      await inspectQuarkContent(undefined, pwd);
                      setQuarkPassCodeVerifying(false);
                    }}
                  />
                );
              })()}
              {(() => {
                const single = lines.length === 1 ? lines[0].trim() : "";
                if (!isBaiduUrl(single) || !baiduShareInfo || !baiduOpen) return null;
                return (
                  <BaiduPanPicker
                    shareInfo={baiduShareInfo}
                    selectedIds={baiduSelectedIds}
                    onChange={setBaiduSelectedIds}
                    verifyingPassCode={baiduPassCodeVerifying}
                    passCodeError={baiduPassCodeError}
                    onVerifyPassCode={async (pwd) => {
                      setBaiduPassCodeVerifying(true);
                      await inspectBaiduContent(undefined, pwd);
                      setBaiduPassCodeVerifying(false);
                    }}
                  />
                );
              })()}
              {(() => {
                const single = lines.length === 1 ? lines[0].trim() : "";
                if (!isLanzouUrl(single) || !lanzouShareInfo || !lanzouOpen) return null;
                return (
                  <LanzouPicker
                    shareInfo={lanzouShareInfo}
                    selectedIds={lanzouSelectedIds}
                    onChange={setLanzouSelectedIds}
                    verifyingPassCode={lanzouPassCodeVerifying}
                    passCodeError={lanzouPassCodeError}
                    onVerifyPassCode={async (pwd) => {
                      setLanzouPassCodeVerifying(true);
                      await inspectLanzouContent(undefined, pwd);
                      setLanzouPassCodeVerifying(false);
                    }}
                  />
                );
              })()}
              {(() => {
                const single = lines.length === 1 ? lines[0].trim() : "";
                if (!isPan123Url(single) || !pan123ShareInfo || !pan123Open) return null;
                return (
                  <Pan123Picker
                    shareInfo={pan123ShareInfo}
                    selectedIds={pan123SelectedIds}
                    onChange={setPan123SelectedIds}
                    verifyingPassCode={pan123PassCodeVerifying}
                    passCodeError={pan123PassCodeError}
                    onVerifyPassCode={async (pwd) => {
                      setPan123PassCodeVerifying(true);
                      await inspectPan123Content(undefined, pwd);
                      setPan123PassCodeVerifying(false);
                    }}
                  />
                );
              })()}
              {btInspectResult && btInspectOpen && (
                <div className="bt-preview-box">
                  <div className="bt-preview-header">
                    <span
                      style={{
                        fontWeight: 600,
                        display: "inline-flex",
                        alignItems: "center",
                        gap: "4px",
                      }}
                    >
                      <Folder size={13} /> {btInspectResult.name}
                    </span>
                    <div
                      style={{
                        display: "inline-flex",
                        gap: "10px",
                        alignItems: "center",
                      }}
                    >
                      <span
                        style={{ color: "var(--text-secondary, #666)" }}
                      >
                        已选 {selectedBtFileIndices.size} /{" "}
                        {btInspectResult.files.length} 个文件 · 共{" "}
                        {formatBytes(
                          btInspectResult.files
                            .filter((f: BtFileEntry) =>
                              selectedBtFileIndices.has(f.index)
                            )
                            .reduce(
                              (sum: number, f: BtFileEntry) =>
                                sum + f.length_bytes,
                              0
                            )
                        )}
                      </span>
                      <button
                        type="button"
                        className="link-button"
                        style={{
                          fontSize: "11px",
                          color: "var(--accent, #0078d4)",
                          background: "transparent",
                          border: "none",
                          cursor: "pointer",
                        }}
                        onClick={() => {
                          if (
                            selectedBtFileIndices.size ===
                            btInspectResult.files.length
                          ) {
                            setSelectedBtFileIndices(new Set());
                          } else {
                            setSelectedBtFileIndices(
                              new Set(
                                btInspectResult.files.map(
                                  (f: BtFileEntry) => f.index
                                )
                              )
                            );
                          }
                        }}
                      >
                        {selectedBtFileIndices.size ===
                        btInspectResult.files.length
                          ? "全不选"
                          : "全选"}
                      </button>
                    </div>
                  </div>
                  <div className="bt-preview-list">
                    {btInspectResult.files.map((file: BtFileEntry) => {
                      const isChecked = selectedBtFileIndices.has(file.index);
                      return (
                        <label
                          key={file.index}
                          className={`bt-preview-item ${
                            isChecked ? "checked" : ""
                          }`}
                        >
                          <input
                            type="checkbox"
                            checked={isChecked}
                            onChange={(e) => {
                              const next = new Set(selectedBtFileIndices);
                              if (e.target.checked) {
                                next.add(file.index);
                              } else {
                                if (next.size <= 1) {
                                  notify?.(
                                    "至少需要保留一个选中的文件",
                                    "error"
                                  );
                                  return;
                                }
                                next.delete(file.index);
                              }
                              setSelectedBtFileIndices(next);
                            }}
                          />
                          <span
                            className="bt-preview-item-name"
                            title={file.path}
                          >
                            {file.path}
                          </span>
                          <span className="bt-preview-item-size">
                            {formatBytes(file.length_bytes)}
                          </span>
                        </label>
                      );
                    })}
                  </div>
                </div>
              )}
              {historyOpen && urlHistory.length > 0 && (
                <div
                  className="url-history-dropdown"
                  role="listbox"
                  aria-label="最近 URL 历史"
                >
                  <div className="url-history-header">
                    <span>最近 URL</span>
                    <button
                      type="button"
                      className="url-history-clear"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        void api
                          .urlHistoryClear()
                          .then(() => {
                            setUrlHistory([]);
                          })
                          .catch(() => {});
                      }}
                      title="清空全部历史"
                    >
                      <Trash2 size={11} />
                      <span>清空</span>
                    </button>
                  </div>
                  <ul className="url-history-list">
                    {urlHistory.map((entry) => (
                      <li key={entry.url}>
                        <button
                          type="button"
                          className="url-history-item"
                          onMouseDown={(e) => {
                            e.preventDefault();
                            setUrls(entry.url);
                            setHistoryOpen(false);
                            setMedia(undefined);
                            userEditedFileName.current = false;
                            const name = extractFileNameFromUrl(entry.url);
                            if (name) setFileName(name);
                          }}
                          title={entry.url}
                        >
                          <Globe2 size={11} />
                          <span className="url-history-text">
                            {entry.url}
                          </span>
                        </button>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
              {(skippedCount > 0 || duplicateCount > 0 || sequenceError) && (
                <div
                  className="url-parse-hint"
                  style={sequenceError ? { color: "var(--danger)" } : undefined}
                >
                  {sequenceError && <span>{sequenceError}</span>}
                  {sequenceError && skippedCount > 0 && <span> · </span>}
                  {skippedCount > 0 && (
                    <span>
                      {t("newTask.urlParseSkipped", { count: skippedCount })}
                    </span>
                  )}
                  {skippedCount > 0 && duplicateCount > 0 && (
                    <span> · </span>
                  )}
                  {duplicateCount > 0 && (
                    <span>
                      {t("newTask.urlParseDuplicated", {
                        count: duplicateCount,
                      })}
                    </span>
                  )}
                </div>
              )}
              {templateMatch?.matched && (
                <div
                  className="url-template-hint"
                  title="任务模板将自动套用到未由用户显式设置的字段"
                >
                  <Bookmark size={11} />
                  <span>
                    已匹配模板：
                    <code>
                      {templateMatch.matched_template_name ??
                        templateMatch.matched_template_id}
                    </code>
                  </span>
                </div>
              )}
              {matchedCredentialDomain && (
                <div
                  className="url-template-hint"
                  title={`已保存的 ${matchedCredentialDomain} 凭证已应用`}
                  role="status"
                >
                  <ShieldCheck size={11} />
                  <span>
                    已保存的 <code>{matchedCredentialDomain}</code> 凭证已应用
                  </span>
                </div>
              )}
              {detectedPlatform && detectedPlatform !== "unknown" && (
                <div className="url-platform-hint" title="已识别媒体平台">
                  <Globe2 size={11} />
                  <span>
                    检测到：{mediaPlatformDisplayName(detectedPlatform)}
                  </span>
                  {platformCompat && (
                    <span
                      className={`platform-badge platform-badge-${platformCompat.level}`}
                      title={
                        platformCompat.notes ||
                        supportLevelLabel(platformCompat.level)
                      }
                      style={{
                        marginLeft: 6,
                        padding: "1px 6px",
                        borderRadius: 4,
                        fontSize: 11,
                        color: "#fff",
                        backgroundColor: supportLevelColor(platformCompat.level),
                        border: `1px solid ${supportLevelColor(
                          platformCompat.level
                        )}`,
                      }}
                    >
                      {supportLevelLabel(platformCompat.level)}
                    </span>
                  )}
                </div>
              )}
              {platformCompat && platformCompat.level === "unsupported" && (
                <div
                  className="url-platform-hint"
                  title="该平台暂不支持下载，请使用浏览器原生下载"
                  role="alert"
                >
                  <AlertCircle size={11} />
                  <span>该平台暂不支持下载，已禁用下载按钮</span>
                </div>
              )}
              {platformCompat &&
                platformCompat.notes &&
                platformCompat.level !== "unsupported" && (
                  <div
                    className="url-platform-hint"
                    title={platformCompat.notes}
                    role="status"
                  >
                    <Info size={11} />
                    <span>{platformCompat.notes}</span>
                  </div>
                )}
              {detectedPlatform === "twitter" && (
                <div
                  className="url-platform-hint"
                  title="Twitter/X 通常需要登录态才能解析视频与 Spaces 音频"
                >
                  <ShieldCheck size={11} />
                  <span>需要登录态，请使用扩展临时登录态或填写 Cookie</span>
                </div>
              )}
            </label>
          </div>

          <div className="form-group-row">
            <label className="form-field grow">
              <span>保存位置</span>
              <div className="input-group">
                <input
                  value={destination}
                  onChange={(e) => {
                    setDestination(e.target.value);
                    userEditedDestination.current = true;
                  }}
                />
                <button
                  className="input-button primary-border"
                  onClick={async () => {
                    const path = await pickPath({
                      directory: true,
                      multiple: false,
                      defaultPath: destination,
                    });
                    if (typeof path === "string") {
                      setDestination(path);
                      userEditedDestination.current = true;
                    }
                  }}
                >
                  <FolderOpen size={13} />
                  <span>浏览</span>
                </button>
              </div>
            </label>
          </div>

          <div className="form-grid-2">
            <div className="form-field">
              <div className="field-label-row">
                <span>分段连接数</span>
                <span className="field-label-value">
                  <span className="field-label-num">{connections}</span>
                  <span className="field-label-text">路并发</span>
                </span>
              </div>
              <div className="slider-container">
                <input
                  type="range"
                  min="0"
                  max="5"
                  step="1"
                  value={[1, 2, 4, 8, 16, 32].indexOf(connections)}
                  onChange={(e) => {
                    const values = [1, 2, 4, 8, 16, 32];
                    userEditedConnections.current = true;
                    setConnections(values[+e.target.value]);
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
            </div>

            <div className="form-field">
              <div className="field-label-row">
                <span>重名处理</span>
              </div>
              <div className="fluent-segmented-control">
                <button
                  type="button"
                  className={policy === "rename" ? "active" : ""}
                  onClick={() => setPolicy("rename")}
                >
                  重命名
                </button>
                <button
                  type="button"
                  className={policy === "overwrite" ? "active" : ""}
                  onClick={() => setPolicy("overwrite")}
                >
                  覆盖
                </button>
                <button
                  type="button"
                  className={policy === "skip" ? "active" : ""}
                  onClick={() => setPolicy("skip")}
                >
                  跳过
                </button>
              </div>
              <div className="field-helper-text">
                {policy === "rename" &&
                  "当文件名存在冲突时自动追加数字后缀"}
                {policy === "overwrite" &&
                  "直接覆盖同名文件，旧文件将被完全替换"}
                {policy === "skip" &&
                  "跳过该任务的下载，直接保留本地文件"}
              </div>
            </div>
          </div>

          {lines.length === 1 && (
            <div className="form-group-row">
              <label className="form-field grow">
                <span>文件名（可选）</span>
                <div className="input-group">
                  <input
                    ref={fileNameInputRef}
                    value={fileName}
                    onChange={(e) => {
                      userEditedFileName.current = true;
                      setFileName(e.target.value);
                    }}
                    placeholder="保持默认（根据服务器响应解析）"
                  />
                  <button
                    className="input-button media-probe-btn"
                    disabled={busy}
                    onClick={() => void probe()}
                  >
                    <Video size={13} />
                    <span>{busy ? "正在分析..." : "分析媒体"}</span>
                  </button>
                </div>
              </label>
            </div>
          )}

          {toolStatus &&
            (!toolStatus.yt_dlp_available ||
              (media?.formats.find((item) => item.id === format)
                ?.requires_ffmpeg &&
                !toolStatus.ffmpeg_available)) && (
              <MediaToolsCard
                status={toolStatus}
                compact
                required={
                  !toolStatus.yt_dlp_available ? "yt-dlp" : "ffmpeg"
                }
                onStatus={setToolStatus}
              />
            )}

          {media && (
            <div className="media-result-card">
              <div className="media-result-header">
                <span className="media-tag">已探测媒体</span>
                <strong>{media.title}</strong>
              </div>
              {media.media_type === "gallery" ? (
                <GalleryPicker
                  formats={media.formats}
                  thumbnail={media.thumbnail}
                  selectedIds={selectedImageIds}
                  onChange={setSelectedImageIds}
                />
              ) : media.media_type === "collection" ? (
                <EpisodePicker
                  episodes={media.episodes || []}
                  selectedIndices={selectedEpisodeIndices}
                  onChange={setSelectedEpisodeIndices}
                  qualityPreference={collectionQualityPreference}
                  onQualityChange={setCollectionQualityPreference}
                />
              ) : media.media_type === "audio" ? (
                <div className="media-format-select-row">
                  <Select
                    value={format}
                    onChange={(val: any) => setFormat(String(val))}
                    options={media.formats
                      .filter((item) => item.has_audio && !item.has_video)
                      .map((item) => ({
                        value: item.id,
                        label: `${item.label}${
                          item.file_size
                            ? ` (${formatBytes(item.file_size)})`
                            : ""
                        }`,
                      }))}
                    ariaLabel="音频格式选择"
                    style={{ width: "100%" }}
                  />
                  {media.formats.filter(
                    (item) => item.has_audio && !item.has_video
                  ).length === 0 && (
                    <div className="media-empty-hint">
                      未识别到独立音频流，将尝试使用默认格式下载
                    </div>
                  )}
                </div>
              ) : (
                <div className="media-format-select-row">
                  <Select
                    value={format}
                    onChange={(val: any) => setFormat(String(val))}
                    options={media.formats
                      .filter((item) => item.has_video || item.has_audio)
                      .map((item) => ({
                        value: item.id,
                        label: `${item.label}${
                          item.file_size
                            ? ` (${formatBytes(item.file_size)})`
                            : ""
                        }${
                          !item.requires_ffmpeg &&
                          item.has_video &&
                          item.has_audio
                            ? " · 轻量单文件"
                            : ""
                        }`,
                      }))}
                    ariaLabel="视频格式选择"
                    style={{ width: "100%" }}
                  />
                </div>
              )}
              {media.subtitles.length > 0 && (
                <div
                  className="media-format-select-row"
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "6px",
                    flexWrap: "wrap",
                  }}
                >
                  <span
                    style={{
                      fontSize: "11px",
                      color: "var(--muted)",
                      flexShrink: 0,
                    }}
                  >
                    下载字幕：
                  </span>
                  {media.subtitles.slice(0, 10).map((lang) => {
                    const active = subtitleLangs.includes(lang);
                    return (
                      <button
                        type="button"
                        key={lang}
                        aria-pressed={active}
                        title={
                          active
                            ? `点击取消下载 ${lang} 字幕`
                            : `点击下载 ${lang} 字幕`
                        }
                        onClick={() =>
                          setSubtitleLangs((current) =>
                            current.includes(lang)
                              ? current.filter((item) => item !== lang)
                              : [...current, lang]
                          )
                        }
                        style={{
                          height: "20px",
                          padding: "0 9px",
                          fontSize: "10px",
                          cursor: "pointer",
                          borderRadius: "999px",
                          border: active
                            ? "1px solid var(--accent)"
                            : "1px solid var(--border)",
                          background: active
                            ? "var(--accent)"
                            : "var(--bg)",
                          color: active ? "#fff" : "var(--text)",
                        }}
                      >
                        {lang}
                      </button>
                    );
                  })}
                  {media.subtitles.length > 10 && (
                    <span
                      style={{ fontSize: "10px", color: "var(--muted)" }}
                    >
                      等 {media.subtitles.length} 种语言
                    </span>
                  )}
                </div>
              )}
            </div>
          )}

          <div className="advanced-divider">
            <button
              className={
                advanced ? "advanced-toggle active" : "advanced-toggle"
              }
              onClick={() => setAdvanced((value) => !value)}
            >
              <ChevronDown size={13} />
              <span>高级下载选项</span>
            </button>
          </div>

          {advanced && (
            <div className="advanced-options-panel">
              <div className="advanced-grid">
                <Field label="计划开始时间">
                  <input
                    type="datetime-local"
                    value={schedule}
                    onChange={(e) => setSchedule(e.target.value)}
                  />
                </Field>
                <Field label="单任务限速">
                  <div className="input-with-unit">
                    <input
                      type="number"
                      min="0"
                      value={limit}
                      onChange={(e) => setLimit(+e.target.value)}
                      placeholder="0 表示不限制"
                    />
                    <span className="unit-label">KB/s</span>
                  </div>
                </Field>
                <Field label="任务优先级（排队与带宽）">
                  <Select
                    value={priority}
                    onChange={(val: any) => setPriority(+val)}
                    options={[
                      {
                        value: TASK_PRIORITY_PRESETS.high,
                        label: "高优先级",
                      },
                      {
                        value: TASK_PRIORITY_PRESETS.normal,
                        label: "普通",
                      },
                      {
                        value: TASK_PRIORITY_PRESETS.low,
                        label: "低优先级",
                      },
                    ]}
                    ariaLabel="任务优先级"
                    style={{ width: "100%" }}
                  />
                </Field>
                <Field label="自定义 Referer">
                  <input
                    value={referer}
                    onChange={(e) => setReferer(e.target.value)}
                    placeholder="https://..."
                  />
                </Field>
                <Field label="自定义 Cookie">
                  <input
                    value={cookie}
                    onChange={(e) => setCookie(e.target.value)}
                    placeholder="key=value; ..."
                  />
                </Field>
                <Field label="自定义 Authorization 头部">
                  <input
                    value={authorization}
                    onChange={(e) => setAuthorization(e.target.value)}
                    placeholder="Bearer ... 或 Basic ..."
                  />
                </Field>
                <Field
                  className={
                    [
                      "run-command",
                      "copy-to",
                      "move-to",
                    ].includes(completionActionKind(completionAction))
                      ? "wide"
                      : ""
                  }
                  label="预期文件校验和"
                >
                  <input
                    value={checksum}
                    onChange={(e) => setChecksum(e.target.value)}
                    placeholder="MD5(32位) / SHA-1(40位) / SHA-256(64位) 十六进制"
                  />
                </Field>
                <Field
                  className={
                    [
                      "run-command",
                      "copy-to",
                      "move-to",
                    ].includes(completionActionKind(completionAction))
                      ? "wide"
                      : ""
                  }
                  label="下载完成后"
                >
                  <CompletionActionEditor
                    value={completionAction}
                    onChange={setCompletionAction}
                    allowRunFile={lines.length === 1}
                  />
                </Field>
              </div>
            </div>
          )}

          {lines.length === 1 &&
            isDownloadableUrlForDialog(lines[0]) && (
              <PrecheckPanel
                result={precheck}
                loading={precheckLoading}
                error={precheckError}
                queueDiskTotal={queueDiskTotal}
                queueUnknownCount={queueUnknownCount}
                onLocateConflict={(conflict) =>
                  onLocateTask?.(conflict.existing_task_id)
                }
                onRefresh={runPrecheckNow}
              />
            )}

          {error && <div className="inline-error">{error}</div>}
        </div>

        <div
          className="new-task-sticky-footer"
          style={{
            flexShrink: 0,
            marginTop: "12px",
            borderTop: "1px solid var(--border-strong)",
            paddingTop: "12px",
            display: "flex",
            flexDirection: "column",
            gap: "10px",
          }}
        >
          {showConflictOptions && (
            <div
              className="conflict-options"
              role="group"
              aria-label="冲突处理"
            >
              <div className="conflict-options-header">
                <AlertTriangle size={11} />
                <span>检测到与已有任务冲突，请选择处理方式：</span>
              </div>
              <div className="conflict-options-row">
                <button
                  type="button"
                  className="conflict-option"
                  onClick={() => {
                    const first =
                      activeConflicts[0] || precheck?.conflicts?.[0];
                    if (first) onLocateTask?.(first.existing_task_id);
                  }}
                  title="在主列表中选中已有任务"
                >
                  <ExternalLink size={11} />
                  <span>定位已有任务</span>
                </button>
                <button
                  type="button"
                  className="conflict-option"
                  disabled={busy}
                  onClick={handleRedownloadDirectly}
                  title="删除已有的冲突任务与文件，并立即重新开始下载"
                >
                  <Download size={11} />
                  <span>重新下载</span>
                </button>
                <button
                  type="button"
                  className="conflict-option"
                  onClick={handleRenameAndSubmit}
                  title="自动在文件名后增加时间戳后缀以避免冲突，并立即开始下载"
                >
                  <FileText size={11} />
                  <span>改文件名</span>
                </button>
                <button
                  type="button"
                  className="conflict-option secondary"
                  onClick={onClose}
                  title="关闭对话框，不创建任务"
                >
                  <X size={11} />
                  <span>跳过</span>
                </button>
              </div>
            </div>
          )}

          {hasDuplicates && duplicateResult && (
            <div
              className="conflict-options"
              role="group"
              aria-label="重复任务处理"
            >
              <div className="conflict-options-header">
                <AlertTriangle size={11} />
                <span>
                  检测到与已有任务重复（
                  {duplicateResult.matches
                    .map((m) => getDuplicateTypeLabel()[m.duplicate_type])
                    .join("、")}
                  ），请选择处理方式：
                </span>
              </div>
              <ul className="duplicate-match-list">
                {duplicateResult.matches.map((m) => (
                  <li
                    key={`${m.duplicate_type}-${m.existing_task_id}`}
                    className="duplicate-match-item"
                  >
                    <span className="duplicate-match-type">
                      {getDuplicateTypeLabel()[m.duplicate_type]}
                    </span>
                    <span
                      className="duplicate-match-label"
                      title={m.existing_task_label}
                    >
                      {m.existing_task_label}
                    </span>
                    <span className="duplicate-match-status">
                      （
                      {getStatusText()[m.existing_task_status as TaskStatus] ??
                        m.existing_task_status}
                      ）
                    </span>
                  </li>
                ))}
              </ul>
              <div className="conflict-options-row">
                <button
                  type="button"
                  className="conflict-option"
                  onClick={() => {
                    const first = duplicateResult.matches[0];
                    if (first) onLocateTask?.(first.existing_task_id);
                  }}
                  title="在主列表中选中已有任务"
                >
                  <ExternalLink size={11} />
                  <span>定位已有任务</span>
                </button>
                <button
                  type="button"
                  className="conflict-option"
                  disabled={busy}
                  onClick={handleRedownloadDirectly}
                  title="删除已有的冲突任务与文件，并立即重新开始下载"
                >
                  <Download size={11} />
                  <span>重新下载</span>
                </button>
                <button
                  type="button"
                  className="conflict-option"
                  onClick={handleRenameAndSubmit}
                  title="自动在文件名后增加时间戳后缀以避免冲突，并立即开始下载"
                >
                  <FileText size={11} />
                  <span>改文件名</span>
                </button>
                <button
                  type="button"
                  className="conflict-option secondary"
                  onClick={onClose}
                  title="关闭对话框，不创建任务"
                >
                  <X size={11} />
                  <span>跳过</span>
                </button>
              </div>
            </div>
          )}

          <div className="dialog-actions new-task-actions">
            <button className="cancel-btn" onClick={onClose}>
              取消
            </button>
            <button
              className="primary confirm-btn"
              disabled={
                busy ||
                !lines.length ||
                hasConflicts ||
                hasDuplicates ||
                isGalleryWithoutSelection ||
                isPikPakWithoutSelection ||
                isQuarkWithoutSelection ||
                isBaiduWithoutSelection ||
                platformCompat?.level === "unsupported"
              }
              title={
                platformCompat?.level === "unsupported"
                  ? "该平台暂不支持下载，请使用浏览器原生下载"
                  : isPikPakWithoutSelection
                  ? "请至少勾选一个需要下载的 PikPak 文件"
                  : isQuarkWithoutSelection
                  ? "请至少勾选一个需要下载的夸克文件"
                  : isBaiduWithoutSelection
                  ? "请至少勾选一个需要下载的百度网盘文件"
                  : hasConflicts || hasDuplicates
                  ? "存在冲突或重复，请先选择处理方式"
                  : isGalleryWithoutSelection
                  ? "请至少选择一张图片"
                  : undefined
              }
              onClick={() => void submit()}
            >
              {busy ? "正在创建任务..." : "开始下载"}
            </button>
          </div>
        </div>
      </div>
      {diskConfirm && (
        <ConfirmDialog
          title={t("dialogs.diskShortTitle")}
          message={t("dialogs.diskShortMessage", {
            avail: formatBytes(precheck?.available_disk_bytes ?? 0),
            need: formatBytes(precheck?.required_disk_bytes ?? 0),
          })}
          confirmLabel={t("dialogs.diskShortConfirm")}
          cancelLabel={t("common.cancel")}
          danger
          onCancel={() => setDiskConfirm(null)}
          onConfirm={() => {
            const override = diskConfirm.override;
            setDiskConfirm(null);
            void performSubmit(override, true);
          }}
        />
      )}
    </Modal>
  );
}
