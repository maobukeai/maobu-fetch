export type TaskStatus = "queued" | "downloading" | "paused" | "completed" | "failed" | "cancelled" | "scheduled" | "verifying" | "waiting-network" | "remote-changed" | "interrupted" | "paused-by-low-disk" | "paused-by-metered";
export type CollisionPolicy = "overwrite" | "skip" | "rename";
/**
 * 下载完成动作（Task 17 扩展）。
 *
 * 旧变体（none/open-folder/run-file/shutdown/hibernate）为字符串；
 * 新增 quit 同样为字符串。
 * RunCommand/CopyTo/MoveTo 带结构化数据，使用 serde 外部标签格式序列化：
 * `{ "run-command": { command, args, working_dir } }` 等。
 */
export type CompletionAction =
  | "none"
  | "open-folder"
  | "run-file"
  | "shutdown"
  | "hibernate"
  | "quit"
  | { "run-command": { command: string; args: string[]; working_dir: string | null } }
  | { "copy-to": { target_directory: string; rename_pattern: string | null } }
  | { "move-to": { target_directory: string; rename_pattern: string | null } };

/** RunCommand 变体的数据结构。 */
export interface RunCommandData {
  command: string;
  args: string[];
  working_dir: string | null;
}

/** CopyTo / MoveTo 变体共用的数据结构。 */
export interface CopyMoveData {
  target_directory: string;
  rename_pattern: string | null;
}

/** 模板变量列表（用于 UI 提示）。 */
export const TEMPLATE_VARIABLES: ReadonlyArray<{ token: string; desc: string }> = [
  { token: "$FILE", desc: "完整文件路径" },
  { token: "$FILENAME", desc: "仅文件名" },
  { token: "$DIR", desc: "文件所在目录" },
  { token: "$URL", desc: "下载 URL" },
  { token: "$TITLE", desc: "任务标题或文件名" },
];

/** 获取 CompletionAction 的种类字符串（用于 <select> value）。 */
export function completionActionKind(action: CompletionAction | null | undefined): string {
  if (action == null) return "none";
  if (typeof action === "string") return action;
  return Object.keys(action)[0];
}

/** 从 CompletionAction 提取 RunCommand 数据；非 RunCommand 返回默认空值。 */
export function getRunCommandData(action: CompletionAction | null | undefined): RunCommandData {
  if (action && typeof action === "object" && "run-command" in action) {
    return action["run-command"];
  }
  return { command: "", args: [], working_dir: null };
}

/** 从 CompletionAction 提取 CopyTo 数据；非 CopyTo 返回默认空值。 */
export function getCopyToData(action: CompletionAction | null | undefined): CopyMoveData {
  if (action && typeof action === "object" && "copy-to" in action) {
    return action["copy-to"];
  }
  return { target_directory: "", rename_pattern: null };
}

/** 从 CompletionAction 提取 MoveTo 数据；非 MoveTo 返回默认空值。 */
export function getMoveToData(action: CompletionAction | null | undefined): CopyMoveData {
  if (action && typeof action === "object" && "move-to" in action) {
    return action["move-to"];
  }
  return { target_directory: "", rename_pattern: null };
}

/** 根据 kind 字符串和数据构造 CompletionAction。 */
export function makeCompletionAction(
  kind: string,
  data?: Partial<RunCommandData & CopyMoveData>,
): CompletionAction {
  switch (kind) {
    case "run-command":
      return {
        "run-command": {
          command: data?.command ?? "",
          args: data?.args ?? [],
          working_dir: data?.working_dir ?? null,
        },
      };
    case "copy-to":
      return {
        "copy-to": {
          target_directory: data?.target_directory ?? "",
          rename_pattern: data?.rename_pattern ?? null,
        },
      };
    case "move-to":
      return {
        "move-to": {
          target_directory: data?.target_directory ?? "",
          rename_pattern: data?.rename_pattern ?? null,
        },
      };
    default:
      return kind as CompletionAction;
  }
}
export type PowerAction = "none" | "shutdown" | "hibernate";
export type PowerActionPhase = "idle" | "armed" | "countdown" | "blocked";

// ===== 任务级重试策略（Task 14）=====
/** 退避策略：固定间隔或指数退避。 */
export type BackoffStrategy = "fixed" | "exponential";

/**
 * 重试策略：单连接超时、总任务超时、最大重试次数、退避策略和初始/最大间隔。
 * - `connection_timeout_secs`：单连接超时（秒），传递给 reqwest connect_timeout。
 * - `task_timeout_secs`：任务总超时（秒），`null` 表示不限制。优先于连接重试。
 * - `max_retries`：每条连接独立计数的最大重试次数。
 * - `initial_backoff_ms` / `max_backoff_ms`：退避初始值与指数退避上限（毫秒）。
 */
export interface RetryPolicy {
  connection_timeout_secs: number;
  task_timeout_secs: number | null;
  max_retries: number;
  backoff: BackoffStrategy;
  initial_backoff_ms: number;
  max_backoff_ms: number;
}

export interface LowDiskPayload {
  task_id: string;
  available_bytes: number;
  required_bytes: number;
}

export interface PowerActionState {
  action: PowerAction;
  phase: PowerActionPhase;
  remaining_seconds: number;
  target_count: number;
  message?: string;
}

export interface MediaSelection {
  extractor?: string;
  format_id?: string;
  format_label?: string;
  subtitles: string[];
  thumbnail?: string;
  requires_ffmpeg?: boolean;
}

export interface DownloadTask {
  id: string;
  url: string;
  file_name: string;
  destination: string;
  total_bytes: number;
  downloaded_bytes: number;
  speed: number;
  eta_seconds?: number;
  status: TaskStatus;
  error?: string;
  created_at: number;
  completed_at?: number;
  scheduled_at?: number;
  category: string;
  queue_position: number;
  priority: number;
  retry_count: number;
  max_retries: number;
  checksum_sha256?: string;
  expected_checksum?: string;
  source: string;
  etag?: string;
  last_modified?: string;
  final_url?: string;
  response_status?: number;
  content_type?: string;
  accepts_ranges?: boolean;
  headers: Record<string, string>;
  media?: MediaSelection;
  per_task_speed_limit: number;
  collision_policy: CollisionPolicy;
  completion_action: CompletionAction;
  connection_count: number;
  active_connections: number;
  segments: DownloadSegment[];
  /** 任务级重试策略覆盖（Task 14）。`undefined`/`null` 表示使用全局默认。 */
  retry_policy_override?: RetryPolicy | null;
  /**
   * Task 31：任务级代理 URL 覆盖。
   * - `undefined`/`null`：使用全局 `AppSettings.proxy_mode`/`proxy_url`。
   * - `""`：显式禁用代理（即使全局是 manual）。
   * - 非空字符串：使用指定代理 URL（http/https/socks5/socks5h）。
   */
  proxy_override?: string | null;
  /**
   * Task 31：任务级代理认证。仅当 `proxy_override` 为非空 URL 时生效。
   * `password` 在内存中为明文（前端输入），后端保存时由 DPAPI 加密为密文落库。
   * 反序列化时由后端解密为明文；前端不应假定 `password` 字段是密文。
   */
  proxy_auth?: ProxyAuth | null;
}

/**
 * Task 31：代理认证信息。
 *
 * `password` 在前端为用户输入的明文；保存时由后端用 DPAPI 加密为密文落库，
 * 加载时由后端解密为明文返回前端。前端不应在日志、控制台或事件中输出此字段。
 */
export interface ProxyAuth {
  username: string;
  password: string;
}

/**
 * Task 31：代理测试结果。
 *
 * - `success`：代理是否可用。
 * - `exit_ip`：通过代理访问 `https://api.ipify.org` 返回的出口 IP。
 * - `latency_ms`：从发起请求到收到响应的耗时（毫秒）。
 * - `error`：失败时的脱敏中文说明；URL 中的 `userinfo` 段已被替换为 `***`。
 */
export interface ProxyTestResult {
  success: boolean;
  exit_ip?: string | null;
  latency_ms: number;
  error?: string | null;
}

export interface DownloadSegment { index: number; start_byte: number; end_byte: number; downloaded_bytes: number; status: string; }

// ===== 连接级实时状态（Task 18）=====
/**
 * 单条分片连接的实时状态。
 *
 * 后端通过 `task-connections` 事件每秒推送一次（与 `task-updated` 同步），
 * 数据来自 `SegmentRuntime` 的原子量采样，非模拟（AGENTS.md §3）。
 * 仅在任务处于 `downloading` 状态时推送；暂停/完成后会推送最后一次最终状态。
 */
export type ConnectionState =
  | "connecting"
  | "downloading"
  | "retrying"
  | "completed"
  | "failed"
  | "paused";

export interface SegmentStatus {
  /** 分片 ID（逻辑分片 index 转字符串） */
  segment_id: string;
  /** 起始偏移 */
  start_offset: number;
  /** 已下载字节 */
  downloaded_bytes: number;
  /** 该分片总字节 */
  total_bytes: number;
  /** 当前速度 bytes/sec */
  speed: number;
  /** 当前连接状态 */
  state: ConnectionState;
  /** 重试次数（连接级，独立于 task.retry_count） */
  retry_count: number;
  /** 错误信息（脱敏后），仅在 `failed` 状态下有意义 */
  error?: string | null;
}

export interface TaskConnectionsEvent {
  task_id: string;
  segments: SegmentStatus[];
  /** Unix 毫秒时间戳 */
  timestamp: number;
}

export interface NewTaskRequest {
  url: string;
  file_name?: string;
  destination?: string;
  headers: Record<string, string>;
  scheduled_at?: number;
  priority: number;
  expected_checksum?: string;
  source?: string;
  per_task_speed_limit: number;
  collision_policy: CollisionPolicy;
  completion_action: CompletionAction;
  media?: MediaSelection;
  connection_count?: number;
  start_paused?: boolean;
  /** 用户是否手动编辑过文件名（Task 20）。`true` 时跳过自动文件名清理规则。 */
  user_edited_file_name?: boolean;
}

export interface AppSettings {
  download_dir: string;
  concurrent_downloads: number;
  connections_per_download: number;
  speed_limit_kbps: number;
  start_minimized: boolean;
  minimize_to_tray: boolean;
  close_to_tray: boolean;
  notifications: boolean;
  auto_start: boolean;
  theme: "system" | "light" | "dark";
  accent_color: "system" | "blue" | "cyan" | "green" | "purple" | "orange";
  frosted_glass: boolean;
  language: string;
  intercept_browser_downloads: boolean;
  min_file_size_mb: number;
  clipboard_monitor: boolean;
  proxy_mode: "system" | "none" | "manual";
  proxy_url: string;
  proxy_username: string;
  proxy_password: string;
  user_agent: string;
  default_collision_policy: CollisionPolicy;
  default_completion_action: CompletionAction;
  max_retries: number;
  retry_base_seconds: number;
  verify_after_download: boolean;
  media_tool_auto_update: boolean;
  yt_dlp_path: string;
  ffmpeg_path: string;
  ffprobe_path: string;
  youtube_po_token?: string;
  low_memory_mode: boolean;
  window_width?: number;
  window_height?: number;
  auto_scale_ui?: boolean;
  /** 全局默认重试策略（Task 14）。任务未设置 `retry_policy_override` 时使用。 */
  default_retry_policy: RetryPolicy;
  /** Task 22：紧凑行高（true=32px / false=36px）。 */
  row_compact: boolean;
  /** Task 22：新建任务或切换选中任务时详情栏默认折叠。 */
  detail_default_collapsed: boolean;
  /** Task 22：颜色方案枚举。System 跟随 prefers-color-scheme；Light/Dark 强制覆盖。 */
  color_scheme: ColorScheme;
  /** Task 24：已完成任务超过 N 天后自动归入历史视图。默认 30。 */
  archive_days: number;
  /** Task 24：主列表已完成任务数量阈值，超过 M 条时最旧的归入历史视图。默认 100。 */
  archive_threshold: number;
  /** Task 30.1：下载完成后发送系统通知。默认 true。 */
  notify_on_complete: boolean;
  /** Task 30.1：下载失败后发送系统通知。默认 true。 */
  notify_on_failure: boolean;
  /** Task 30.1：下载完成时播放提示音。默认 true。 */
  notify_sound_enabled: boolean;
  /** Task 30.1：下载失败时播放提示音。默认 false。 */
  notify_failure_sound_enabled: boolean;
  /**
   * Task 31：PAC（Proxy Auto-Config）脚本文件路径。
   * `null`/`undefined` 表示不使用 PAC；非空时由系统代理读取。
   * 仅在 `proxy_mode = "system"` 时生效（system 模式下操作系统会自动使用 PAC）。
   */
  pac_script_path?: string | null;
  /** Task 32：计量网络下自动暂停 Downloading 任务。默认 true。 */
  metered_auto_pause: boolean;
  /** Task 32：用户在计量网络下手动恢复后置 true，避免重复自动暂停；网络变为非计量时由后端清零。 */
  user_resumed_after_metered: boolean;
  /** Task 21：自定义列表快捷键配置。未设置时由前端填充默认组合。 */
  shortcut_keys?: ShortcutKeys;
}

/** Task 21：列表快捷键配置映射。 */
export interface ShortcutKeys {
  new_task: string;
  select_all: string;
  copy_url: string;
  open_folder: string;
  toggle_pause: string;
  rename_task: string;
  delete_task: string;
  delete_file: string;
}

/** Task 22：颜色方案枚举，与 Rust `ColorScheme` 对应，序列化为 lowercase。 */
export type ColorScheme = "system" | "light" | "dark";

export interface PairingInfo { code: string; expires_at: number; paired_extension?: string; }
export type ToolPhase = "missing" | "downloading" | "verifying" | "extracting" | "ready" | "failed";
export type ToolComponent = "yt-dlp" | "ffmpeg";
export interface ToolStatus {
  state: ToolPhase;
  version: string;
  downloaded_bytes: number;
  total_bytes: number;
  installed_bytes: number;
  error?: string;
  yt_dlp_available: boolean;
  ffmpeg_available: boolean;
  active_component?: ToolComponent;
  yt_dlp_version: string;
  ffmpeg_version: string;
  yt_dlp_download_bytes: number;
  ffmpeg_download_bytes: number;
  yt_dlp_installed_bytes: number;
  ffmpeg_installed_bytes: number;
  yt_dlp_source: "missing" | "custom" | "bundled" | "system";
  ffmpeg_source: "missing" | "custom" | "bundled" | "system";
  yt_dlp_resolved_path?: string;
  ffmpeg_resolved_path?: string;
}
export interface DetectedMediaTools {
  yt_dlp_path?: string;
  ffmpeg_path?: string;
  ffprobe_path?: string;
}
export interface MediaFormat {
  id: string;
  label: string;
  extension?: string;
  width?: number;
  height?: number;
  file_size?: number;
  has_video: boolean;
  has_audio: boolean;
  requires_ffmpeg: boolean;
  /**
   * Task 42：图集场景下的图片直链 URL。
   *
   * 仅当 `media_type = "gallery"` 且该格式为图片项
   * （`has_video=false`、`has_audio=false`、`extension` 为 jpg/jpeg/png/webp/gif/bmp）
   * 时填充；其余场景始终为 `undefined`/`null`。
   *
   * 后端 `#[serde(default)]` 保证旧 JSON/旧扩展请求缺失此字段时安全反序列化为 `None`，
   * 满足 AGENTS.md §2"新增序列化字段必须提供安全默认值"。
   */
  image_url?: string | null;
}
/**
 * 媒体内容类型（Task 38 / Task 41）。
 *
 * 与后端 `MediaType` 枚举对应，序列化为 kebab-case 字符串。
 * 由 `MediaProbeResult.media_type` 字段携带，前端据此选择下载策略：
 * - `"video"`：普通视频（默认，向后兼容）
 * - `"audio"`：纯音频（Twitter Spaces、YouTube 音乐）
 * - `"gallery"`：图集（抖音 note、TikTok photo）
 * - `"mixed"`：混合内容
 */
export type MediaType = "video" | "audio" | "gallery" | "mixed" | "collection";
export interface MediaEpisode { index: number; title: string; url: string; duration?: number; }
export interface MediaProbeResult { title: string; thumbnail?: string; extractor?: string; duration?: number; formats: MediaFormat[]; subtitles: string[]; drm: boolean; media_type: MediaType; episodes?: MediaEpisode[]; }
export interface TaskEvent { task: DownloadTask; event: string; }
export type FilterKey = "all" | TaskStatus | "images" | "video" | "audio" | "documents" | "archives" | "apps";

// Task 26: 应用更新检查与提醒（只检查不自动下载，AGENTS.md §6）。
export interface UpdateInfo {
  version: string;
  release_date: string;
  download_url: string;
  sha256?: string;
  release_notes: string;
}
export interface UpdateCheckResult {
  latest?: UpdateInfo;
  has_update: boolean;
  current_version: string;
  error?: string;
}
export interface ExtensionCompatibilityResult {
  compatible: boolean;
  app_version: string;
  extension_version: string;
  message: string;
}

// ===== 错误诊断（Task 3）=====
export type ErrorCategory =
  | "auth-expired"
  | "range-invalid"
  | "disk-full"
  | "proxy-failed"
  | "etag-changed"
  | "checksum-failed"
  | "network-reset"
  | "tls-failed"
  | "server-error"
  | "timeout"
  | "disk-io"
  | "remote-changed"
  | "unknown";

export interface SuggestedAction {
  /** 稳定英文标识，前端依据它调用对应 Tauri 命令或 UI 流程 */
  action_id: "refetch_url" | "clear_shards" | "change_dir" | "disable_proxy" | "reverify" | "retry" | "redownload";
  /** 简体中文按钮文案 */
  label: string;
}

export interface ErrorDiagnosis {
  category: ErrorCategory;
  /** 简体中文标题 */
  title: string;
  /** 简体中文说明 */
  description: string;
  suggested_actions: SuggestedAction[];
  /** 脱敏后的原始错误（Cookie/Authorization/代理密码/URL token 已替换为 ***） */
  raw_error_redacted: string;
}

// ===== 预检结果（Task 1 / Task 9）=====
export interface PrecheckRequest {
  url: string;
  target_directory?: string;
  suggested_filename?: string;
  headers?: Record<string, string>;
  proxy_override?: string | null;
  proxy_auth?: ProxyAuth | null;
}

export interface RedirectHop {
  url: string;
  status: number;
}

export type PrecheckConflictType =
  | "duplicate-url"
  | "duplicate-final-url"
  | "duplicate-target-path";

export interface PrecheckConflict {
  conflict_type: PrecheckConflictType;
  existing_task_id: string;
  existing_task_label: string;
}

export type PrecheckDiskState = "sufficient" | "insufficient" | "unknown";

export interface PrecheckResult {
  original_url: string;
  final_url: string;
  redirect_chain: RedirectHop[];
  file_name: string;
  file_size?: number;
  etag?: string;
  last_modified?: string;
  accepts_ranges: boolean;
  content_type?: string;
  suggested_connections: number;
  supports_resume: boolean;
  target_directory: string;
  available_disk_bytes: number;
  required_disk_bytes: number;
  disk_ok: boolean;
  disk_state?: PrecheckDiskState;
  conflicts: PrecheckConflict[];
  warnings: string[];
}

// ===== 启动自检（Task 5 / Task 9.5）=====
export interface SelfcheckReport {
  interrupted_count: number;
  dropped_shards: number;
  recovered_tasks: string[];
}

// ===== 队列调度可观察性（Task 15）=====
/**
 * 任务等待原因。
 *
 * 后端使用 serde 内部标签枚举序列化，`kind` 字段为 kebab-case 变体名。
 * 前端通过判别联合类型安全地访问各变体的附加字段。
 * `scheduled_at` 为 Unix 毫秒时间戳字符串，由前端格式化为本地时间。
 */
export type WaitReason =
  | { kind: "not-waiting" }
  | { kind: "queued-behind"; ahead_count: number }
  | { kind: "waiting-media-tools" }
  | { kind: "waiting-user-confirmation" }
  | { kind: "waiting-scheduled-time"; scheduled_at: string }
  | { kind: "waiting-concurrency-limit"; active_count: number }
  | { kind: "paused" }
  | { kind: "paused-by-low-disk" }
  | { kind: "paused-by-metered" }
  | { kind: "interrupted" }
  | { kind: "remote-changed" }
  | { kind: "unknown" };

/** Task 32：`metered-network-detected` 事件 payload。后端在自动暂停 ≥1 个任务时 emit。 */
export interface MeteredNetworkDetectedPayload {
  /** 本次被自动暂停的任务数 */
  paused_count: number;
}

// ===== 自动分类规则（Task 11）=====
export type CategoryRuleType = "domain" | "mime" | "regex";

export interface CategoryRule {
  id: string;
  name: string;
  rule_type: CategoryRuleType;
  pattern: string;
  target_directory: string;
  enabled: boolean;
  priority: number;
}

export interface CategoryRuleTestResult {
  matched: boolean;
  target_directory: string;
}

// ===== 任务模板（Task 36）=====
/**
 * 任务模板（Task 36）。
 *
 * 用于在新建任务时按域名匹配自动套用一组下载参数（连接数、限速、请求头、
 * 保存目录、完成动作）。匹配语义：
 * - `domain_pattern` 支持精确域名（`github.com`）与通配符子域（`*.example.com`）。
 * - 多模板同时命中时按 `priority` 升序取优先级最高（数字越小越优先）。
 * - `enabled = false` 的模板不参与匹配。
 *
 * 字段语义与 `NewTaskRequest` 对应字段一致；`null`/`undefined` 表示不覆盖。
 */
export interface TaskTemplate {
  id: string;
  name: string;
  domain_pattern: string;
  /** `null`/`undefined` 表示不覆盖；非空值必须为 1/2/4/8/16/32 之一。 */
  connections?: number | null;
  /** `null`/`undefined` 表示不限速。单位 bytes/sec。 */
  speed_limit?: number | null;
  /** `null`/`undefined` 表示不覆盖；非空 map 会与请求头合并（模板头优先级低于用户已设的头）。 */
  headers?: Record<string, string> | null;
  /** `null`/`undefined` 表示不覆盖。 */
  destination?: string | null;
  /** `null`/`undefined` 表示不覆盖。 */
  completion_action?: CompletionAction | null;
  enabled: boolean;
  priority: number;
}

/**
 * 模板匹配测试结果（Task 36）。
 *
 * 由 `task_template_test` 命令返回，描述给定 URL 是否命中模板以及命中的模板 ID。
 * `matched_template_id` 为 `null` 表示无模板命中。
 */
export interface TaskTemplateTestResult {
  matched: boolean;
  matched_template_id?: string | null;
  matched_template_name?: string | null;
}

// ===== 文件名清理规则（Task 20）=====
/**
 * 文件名清理规则。
 *
 * 用于在保存下载文件前对文件名做正则替换，去除站点水印、画质标记重复等噪声。
 * - `pattern`：正则表达式（regex crate 语法，与后端一致）
 * - `replacement`：替换为的字符串（可为空字符串以直接删除匹配内容）
 * - `enabled`：是否启用；未启用的规则在应用时跳过
 * - `priority`：升序遍历，数字越小越先执行
 *
 * 仅在用户未手动编辑文件名时应用（见 NewTaskRequest.user_edited_file_name）。
 */
export interface FilenameCleanupRule {
  id: string;
  name: string;
  pattern: string;
  replacement: string;
  enabled: boolean;
  priority: number;
}

// ===== 重复任务检测（Task 10）=====
export type DuplicateType =
  | "same-url"
  | "same-final-url"
  | "same-target-path"
  | "same-checksum";

export interface DuplicateMatch {
  duplicate_type: DuplicateType;
  existing_task_id: string;
  /** 文件名优先，文件名为空时回退到 URL */
  existing_task_label: string;
  existing_task_status: string;
}

export interface DuplicateCheckResult {
  matches: DuplicateMatch[];
}

// ===== 下载配置预设（Task 12）=====
/**
 * 下载配置预设。
 *
 * 内置预设：`default` / `lightweight` / `large-file` / `background` / `night`，
 * 自定义预设 `is_builtin = false`。`connections` 必须是 1/2/4/8/16/32 之一。
 *
 * `scheduled_at` 在数据库中以 "HH:MM" 24 小时制字符串存储（夜间预设 = "22:00"）；
 * 应用到任务时由后端转换为下一次该时刻的 Unix 毫秒时间戳。
 * `null` 表示立即开始下载，不绑定计划时间。
 */
export interface DownloadPreset {
  id: string;
  name: string;
  connections: number;
  speed_limit?: number | null;
  completion_action?: CompletionAction | null;
  verify_checksum: boolean;
  scheduled_at?: string | null;
  is_builtin: boolean;
}

// ===== URL 历史记录（Task 19）=====
/**
 * URL 历史记录条目。
 *
 * 用于新建任务输入框的下拉历史。后端表容量 20 条（LRU），
 * 重复添加同一 URL 只更新 `last_used`（Unix 毫秒时间戳）。
 */
export interface UrlHistoryEntry {
  url: string;
  last_used: number;
}

// ===== 深链与文件关联（Task 29）=====
/**
 * `deep-link-received` 事件 payload。
 *
 * - `add`：用户点击 `maobu://add?url=...` 链接，前端打开新建任务对话框并预填 URL。
 * - `import`：用户双击 `.maobu-task` 文件，后端已导入 `count` 个任务，前端显示 toast。
 */
export interface DeepLinkReceivedPayload {
  action: "add" | "import";
  url?: string | null;
  count?: number | null;
}

// ===== Task 30：下载完成通知与声音 =====
/**
 * `task-notification` 事件 payload。
 *
 * 后端在任务进入 Completed / Failed 终态、且用户开启对应通知开关时 emit。
 * 前端依据 `kind` 决定：
 * - 播放哪种提示音（完成音上升 / 失败音下降），受 `notify_sound_enabled` / `notify_failure_sound_enabled` 控制；
 * - 失败时显示带"一键重试"按钮的 toast（点击跳转到对应任务）。
 */
export interface TaskNotificationPayload {
  task_id: string;
  kind: "completed" | "failed";
  title: string;
  body: string;
}

// ===== 完整备份与恢复（Task 27）=====
/**
 * 设置差异描述。`changed_fields` 列出不一致的字段名（英文协议字段名）；
 * `identical = true` 表示设置完全相同，恢复时不会改动。
 */
export interface SettingsDiff {
  changed_fields: string[];
  identical: boolean;
}

/**
 * 恢复前预览结果。
 *
 * 列出本次恢复将新增、覆盖、跳过的条数，由后端在内存中比对当前数据库状态得出。
 * 所有计数均为非负整数。前端在用户确认前展示此预览。
 */
export interface RestorePreview {
  settings_diff: SettingsDiff;
  new_category_rules: number;
  override_category_rules: number;
  new_filename_cleanup_rules: number;
  override_filename_cleanup_rules: number;
  new_presets: number;
  override_presets: number;
  new_url_history: number;
  new_tasks: number;
  duplicate_tasks: number;
  includes_auth: boolean;
  encrypted: boolean;
  created_at: string;
  app_version: string;
}

/**
 * 恢复执行结果统计。由 `backup_restore` 在恢复完成后返回。
 *
 * - `added_tasks` / `skipped_tasks`：任务按 ID 去重，已存在的跳过（不覆盖用户进度）。
 * - `rules_applied`：分类规则 + 文件名清理规则 + 下载预设的应用总数（含新增与覆盖）。
 * - `url_history_added`：URL 历史去重后写入的条数。
 * - `settings_replaced`：备份中包含 settings 字段时为 true。
 */
export interface RestoreStats {
  added_tasks: number;
  skipped_tasks: number;
  rules_applied: number;
  url_history_added: number;
  settings_replaced: boolean;
}

// ===== 标签与高级筛选（Task 25）=====

/**
 * 用户标签。`color` 为 `#RRGGBB` 格式的十六进制颜色字符串。
 * `name` 在表内唯一；`id` 由前端生成（如 `tag-<timestamp>-<rand>`）。
 */
export interface Tag {
  id: string;
  name: string;
  color: string;
}

/** 任务到标签的映射，键为 task_id，值为该任务关联的全部标签。 */
export type TaskTagsMap = Record<string, Tag[]>;

/**
 * 高级筛选条件。所有字段可选；空数组/undefined 表示不限制该维度。
 * `source` 取值与 DownloadTask.source 一致（如 "manual" / "extension" / "deep-link"）。
 */
export interface AdvancedFilter {
  statuses: TaskStatus[];
  domain: string;
  dateFrom: number | null;
  dateTo: number | null;
  sizeMin: number | null;
  sizeMax: number | null;
  tagIds: string[];
  sources: string[];
}

/** 默认空筛选（不限制任何维度）。 */
export const EMPTY_ADVANCED_FILTER: AdvancedFilter = {
  statuses: [],
  domain: "",
  dateFrom: null,
  dateTo: null,
  sizeMin: null,
  sizeMax: null,
  tagIds: [],
  sources: [],
};

/**
 * 快捷视图：保存的筛选配置。
 * `id` 由前端生成；`name` 用户可见；`filter` 为完整筛选条件。
 * 存储在 localStorage，不进入 SQLite（与设置无关的个人偏好）。
 */
export interface QuickView {
  id: string;
  name: string;
  filter: AdvancedFilter;
}

// ===== Task 34：便携版 / 应用信息 =====
/**
 * 应用信息（Task 34.3）。
 *
 * 由 `app_get_info` 命令返回，前端用于在设置页"关于"分组显示便携模式状态。
 * - `version`：编译期 `CARGO_PKG_VERSION`，例如 "0.6.5"
 * - `portable_mode`：便携模式是否生效。环境变量 `MAOBU_FETCH_DATA_DIR`
 *   覆盖时为 false（即使存在 `maobu.portable` 标记也不视为便携）。
 * - `data_dir`：当前生效的数据目录绝对路径，仅用于在关于页展示。
 */
export interface AppInfo {
  version: string;
  portable_mode: boolean;
  data_dir: string;
}

// ===== Task 37 / 44：媒体平台识别与错误翻译 =====
/**
 * 媒体平台枚举（Task 37.1）。
 *
 * 与后端 `MediaPlatform` 枚举对应，序列化为 kebab-case 字符串。
 * 由 `media_detect_platform` 命令返回，前端用于在新建任务对话框展示
 * "检测到：抖音" 等提示。
 *
 * - `"douyin"`：抖音（v.douyin.com / www.douyin.com / iesdouyin.com）
 * - `"tiktok"`：TikTok（www.tiktok.com / vm.tiktok.com / vt.tiktok.com）
 * - `"twitter"`：Twitter/X（twitter.com / x.com / t.co）
 * - `"youtube"`：YouTube（youtube.com / youtu.be / m.youtube.com）
 * - `"bilibili"`：哔哩哔哩（bilibili.com / b23.tv）
 * - `"weibo"`：微博（weibo.com / weibo.cn / t.cn）
 * - `"unknown"`：未识别的平台，仍可走通用 yt-dlp 流程
 */
export type MediaPlatform =
  | "douyin"
  | "tiktok"
  | "twitter"
  | "youtube"
  | "bilibili"
  | "weibo"
  | "unknown";

/**
 * 媒体平台错误类别（Task 37.6 / Task 44.2）。
 *
 * 与后端 `MediaPlatformError` 枚举对应，序列化为 kebab-case 字符串。
 * 用于把 yt-dlp stderr 中的平台特定错误映射为标准类别，
 * 每个类别对应一个中文文案（由后端 `platform_error_to_chinese` 翻译）。
 *
 * - `"login-expired"`：登录态失效（Cookie 过期或未提供）
 * - `"region-blocked"`：地区限制（如 TikTok 在某些地区不可用）
 * - `"link-expired"`：链接已失效或内容已被删除
 * - `"drm-protected"`：内容受 DRM 保护，必须拒绝（AGENTS.md §6）
 * - `"unsupported"`：平台暂不支持此类型内容
 * - `"unknown"`：未识别的错误
 */
export type MediaPlatformError =
  | "login-expired"
  | "region-blocked"
  | "link-expired"
  | "drm-protected"
  | "unsupported"
  | "unknown";

/**
 * 把 MediaPlatform 字符串映射为前端展示用的中文名称。
 *
 * 用于新建任务对话框展示"检测到：抖音"等提示。
 * `"unknown"` 返回空字符串，避免对未识别平台显示误导性提示。
 */
export function mediaPlatformDisplayName(platform: MediaPlatform): string {
  switch (platform) {
    case "douyin": return "抖音";
    case "tiktok": return "TikTok";
    case "twitter": return "Twitter/X";
    case "youtube": return "YouTube";
    case "bilibili": return "哔哩哔哩";
    case "weibo": return "微博";
    case "unknown": return "";
  }
}

/**
 * Task 46：按域名存储的媒体凭证。
 *
 * 用于在分析/下载媒体时复用用户已保存的 Cookie/Referer/User-Agent，
 * 避免每次重新输入。`cookie` 字段在数据库中以 DPAPI 密文形式存储，
 * 但在协议中始终为明文——加解密只在后端 store 层发生。
 *
 * - `domain`：注册域名（去掉前导 `www.`），作为主键。如 `example.com`。
 * - `cookie`：明文 Cookie 字符串（`name=value; name2=value2`）。
 * - `referer` / `user_agent`：可选辅助头。
 * - `updated_at`：ISO 8601 UTC 字符串，仅用于展示。
 */
export interface MediaCredential {
  domain: string;
  cookie?: string;
  referer?: string | null;
  user_agent?: string | null;
  updated_at?: string;
}

/**
 * 媒体凭证在线检测结果。
 */
export interface MediaCredentialCheckResult {
  domain: string;
  valid: boolean;
  message: string;
  tested_at: string;
}

/**
 * Task 43：平台命名模板。
 *
 * 用于在媒体下载完成后按平台套用文件名模板。每条模板绑定一个 platform key
 * （小写英文，与 MediaPlatform 字符串值对应：douyin/tiktok/twitter/youtube/
 * bilibili/weibo/unknown）。
 *
 * - `id`：稳定英文标识。内置模板使用稳定 ID（如 `douyin-default`）；
 *   自定义模板由前端生成（如 `template-<uuid>`）。
 * - `platform`：平台 key（小写英文）。
 * - `template`：模板字符串，支持变量 `{author}`/`{title}`/`{date}`/`{platform}`
 *   /`{id}`/`{channel}`/`{bvid}`。未知变量原样保留；缺失的已知变量替换为空。
 * - `enabled`：是否启用；未启用的模板在匹配时被跳过。
 * - `is_builtin`：内置模板标记。内置模板可编辑、可禁用，但前端应禁止删除
 *   （AGENTS.md §3：不得改变用户设置除非用户明确触发）。
 */
export interface PlatformNamingTemplate {
  id: string;
  platform: string;
  template: string;
  enabled: boolean;
  is_builtin?: boolean;
}

/**
 * Task 44：平台支持级别。
 *
 * 与后端 `SupportLevel` 枚举对应，序列化为 kebab-case 字符串。
 * 用于在新建任务对话框和设置页"平台兼容性"子区域展示徽章：
 *
 * - `"verified"`：经过完整测试，预期可用（绿色徽章 + "已验证"文字）
 * - `"experimental"`：基本可用但成功率受平台变更影响（橙色徽章 + "实验性"文字）
 * - `"unsupported"`：明确不支持（红色徽章 + "不支持"文字，禁用下载按钮）
 *
 * 徽章同时使用颜色和文字标识，不依赖单一颜色（AGENTS.md §4 无障碍）。
 */
export type SupportLevel = "verified" | "experimental" | "unsupported";

/**
 * Task 44：单条平台兼容性记录。
 *
 * - `platform`：`MediaPlatform` 序列化值（`"douyin"` / `"tiktok"` / `"twitter"` /
 *   `"youtube"` / `"bilibili"` / `"weibo"` / `"unknown"`）。
 * - `level`：支持级别，决定徽章颜色和下载按钮可用性。
 * - `notes`：中文说明（如"YouTube 普通视频可直接下载"）。
 * - `known_issues`：已知问题列表，每项为一条中文短描述。
 * - `last_tested_at`：最近一次回归测试时间（ISO 8601 UTC 字符串，可空）。
 */
export interface PlatformCompatibility {
  platform: MediaPlatform | string;
  level: SupportLevel;
  notes?: string;
  known_issues?: string[];
  last_tested_at?: string;
}

export interface CacheInspectResult {
  total_bytes: number;
  file_count: number;
}

export interface CacheClearResult {
  freed_bytes: number;
  deleted_files_count: number;
}

/**
 * Task 44：根据 SupportLevel 返回前端展示用的中文徽章文案。
 *
 * 用于新建任务对话框和设置页"平台兼容性"子区域。
 * 不依赖颜色单独区分（AGENTS.md §4），同时返回文字标识。
 */
export function supportLevelLabel(level: SupportLevel): string {
  switch (level) {
    case "verified": return "已验证";
    case "experimental": return "实验性";
    case "unsupported": return "不支持";
  }
}

/**
 * Task 44：根据 SupportLevel 返回 CSS 颜色变量名（用于徽章样式）。
 *
 * 颜色仅作为辅助标识，必须同时展示 [`supportLevelLabel`] 返回的文字。
 */
export function supportLevelColor(level: SupportLevel): string {
  switch (level) {
    case "verified": return "var(--success-color, #10b981)";
    case "experimental": return "var(--warning-color, #f59e0b)";
    case "unsupported": return "var(--danger-color, #ef4444)";
  }
}
