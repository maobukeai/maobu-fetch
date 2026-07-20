use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Queued,
    Downloading,
    Paused,
    Completed,
    Failed,
    Cancelled,
    Scheduled,
    Verifying,
    Interrupted,
    #[serde(rename = "waiting-network")]
    WaitingNetwork,
    #[serde(rename = "remote-changed")]
    RemoteChanged,
    #[serde(rename = "paused-by-low-disk")]
    PausedByLowDisk,
    /// Task 32：计量网络下自动暂停的任务。
    ///
    /// 仅由网络感知定时检查触发；用户在详情面板点击"继续"后状态切换为 Queued，
    /// 并将 `AppSettings::user_resumed_after_metered` 置为 true，
    /// 直至下一次网络变为非计量时清零，避免重复自动暂停。
    #[serde(rename = "paused-by-metered")]
    PausedByMetered,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Downloading => "downloading",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Scheduled => "scheduled",
            Self::Verifying => "verifying",
            Self::Interrupted => "interrupted",
            Self::WaitingNetwork => "waiting-network",
            Self::RemoteChanged => "remote-changed",
            Self::PausedByLowDisk => "paused-by-low-disk",
            Self::PausedByMetered => "paused-by-metered",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "downloading" => Self::Downloading,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "scheduled" => Self::Scheduled,
            "verifying" => Self::Verifying,
            "interrupted" => Self::Interrupted,
            "waiting-network" => Self::WaitingNetwork,
            "remote-changed" => Self::RemoteChanged,
            "paused-by-low-disk" => Self::PausedByLowDisk,
            "paused-by-metered" => Self::PausedByMetered,
            _ => Self::Queued,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SelfcheckReport {
    pub interrupted_count: u32,
    pub dropped_shards: u32,
    pub recovered_tasks: Vec<String>,
}

/// 任务优先级上下限（Task 16）。
///
/// 语义：**数字越小越优先**。默认 0 表示普通优先级；
/// `-1000` 表示置顶，`+1000` 表示置底。
/// 调度排序按 `priority ASC, queue_position ASC`。
pub const MIN_PRIORITY: i32 = -1000;
pub const MAX_PRIORITY: i32 = 1000;
/// 上移/下移单步相对值（Task 16 右键菜单）。
///
/// 后端不直接使用此常量（前端计算新 priority 后通过 `task_update_options` 传入），
/// 保留为公共常量供测试和未来扩展引用。
#[allow(dead_code)]
pub const PRIORITY_STEP: i32 = 10;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DownloadTask {
    pub id: String,
    pub url: String,
    pub file_name: String,
    pub destination: String,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub speed: u64,
    pub eta_seconds: Option<u64>,
    pub status: TaskStatus,
    pub error: Option<String>,
    pub created_at: u64,
    pub completed_at: Option<u64>,
    pub scheduled_at: Option<u64>,
    pub category: String,
    pub queue_position: i64,
    pub priority: i32,
    pub retry_count: u32,
    pub max_retries: u32,
    pub checksum_sha256: Option<String>,
    pub expected_checksum: Option<String>,
    pub source: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    #[serde(default)]
    pub final_url: Option<String>,
    #[serde(default)]
    pub response_status: Option<u16>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub accepts_ranges: Option<bool>,
    pub headers: HashMap<String, String>,
    pub media: Option<MediaSelection>,
    pub per_task_speed_limit: u64,
    pub collision_policy: CollisionPolicy,
    #[serde(default)]
    pub completion_action: CompletionAction,
    pub connection_count: u8,
    #[serde(default)]
    pub active_connections: u8,
    #[serde(default)]
    pub segments: Vec<DownloadSegment>,
    /// 任务级重试策略覆盖（Task 14）。`None` 表示使用全局默认。
    #[serde(default)]
    pub retry_policy_override: Option<RetryPolicy>,
    /// Task 31：任务级代理 URL 覆盖。
    ///
    /// - `None`：使用全局 `AppSettings.proxy_mode`/`proxy_url`。
    /// - `Some("")`：显式禁用代理（覆盖全局 manual 代理）。
    /// - `Some(url)`：使用指定 URL，覆盖全局设置。
    ///
    /// 旧 JSON/数据库缺失此字段时通过 serde 默认 `None` 安全回退到全局。
    #[serde(default)]
    pub proxy_override: Option<String>,
    /// Task 31：任务级代理认证。仅当 `proxy_override` 为 `Some(url)` 时生效。
    /// `None` 表示不使用认证；用户名为空时也表示无认证。
    /// 密码以 DPAPI 加密后的密文形式存储（见 `secure_storage`），不出现在日志或前端调试输出。
    #[serde(default)]
    pub proxy_auth: Option<ProxyAuth>,
}

/// Task 31：代理认证信息。
///
/// `password` 字段在持久化时由调用方使用 DPAPI 加密为密文，反序列化时由
/// `manager`/`store` 在读取后解密为明文供 reqwest 使用。结构本身仅作为
/// 数据载体，不假定 `password` 是明文还是密文——加解密由调用站点控制。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ProxyAuth {
    pub username: String,
    /// 认证密码。在 DB 中以 DPAPI 密文存储；内存中可能为明文或密文，
    /// 取决于加载位置（store 加载后立即解密为明文供 manager 使用）。
    pub password: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadSegment {
    pub index: u8,
    pub start_byte: u64,
    pub end_byte: u64,
    pub downloaded_bytes: u64,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct MediaSelection {
    pub extractor: Option<String>,
    pub format_id: Option<String>,
    pub format_label: Option<String>,
    pub subtitles: Vec<String>,
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub requires_ffmpeg: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CollisionPolicy {
    Overwrite,
    Skip,
    #[default]
    Rename,
}

/// 重复任务检测分类（Task 10）。
///
/// 用于 `duplicate_check` 命令返回结果，标识新任务与已有任务的冲突类型。
/// 序列化使用 kebab-case 以保持与 `PrecheckConflictType` 一致。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Copy)]
#[serde(rename_all = "kebab-case")]
pub enum DuplicateType {
    /// 完全相同 URL（剥离跟踪参数后比对）。
    SameUrl,
    /// 重定向后相同 URL（剥离跟踪参数后比对）。
    SameFinalUrl,
    /// 相同目标文件路径（规范化后比对）。
    SameTargetPath,
    /// 已完成文件大小 + SHA-256 相同。
    SameChecksum,
}

impl DuplicateType {
    /// 返回前端展示用的中文名称。
    ///
    /// 当前由前端 `duplicateTypeLabel` 映射负责显示，此方法保留作为
    /// 后端诊断与未来 Tauri 命令的备用入口（含对应单元测试）。
    #[allow(dead_code)]
    pub fn label(&self) -> &'static str {
        match self {
            Self::SameUrl => "URL 冲突",
            Self::SameFinalUrl => "最终地址冲突",
            Self::SameTargetPath => "目标文件冲突",
            Self::SameChecksum => "已下载过相同文件",
        }
    }
}

/// 单条重复匹配。
///
/// `existing_task_label` 优先使用文件名，文件名为空时回退到 URL。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DuplicateMatch {
    pub duplicate_type: DuplicateType,
    pub existing_task_id: String,
    pub existing_task_label: String,
    pub existing_task_status: String,
}

/// 重复检测汇总结果。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DuplicateCheckResult {
    /// `#[serde(default)]` 保证旧 JSON（无 `matches` 字段）可安全反序列化为空列表。
    #[serde(default)]
    pub matches: Vec<DuplicateMatch>,
}

/// 下载完成后的动作（Task 17 扩展）。
///
/// 旧变体 `None` / `OpenFolder` / `RunFile` / `Shutdown` / `Hibernate` 保持不变，
/// 新增 `Quit` / `RunCommand` / `CopyTo` / `MoveTo`。所有变体序列化为 kebab-case；
/// 带数据的变体使用 serde 默认的外部标签格式，例如：
/// `{"run-command": {"command": "...", "args": [...], "working_dir": null}}`。
///
/// `#[serde(default)]` + `Default` 保证旧 JSON（不含新变体）安全反序列化为 `None`。
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CompletionAction {
    #[default]
    None,
    OpenFolder,
    RunFile,
    /// 下载完成后关机，复用 PowerAction::Shutdown 的 shutdown.exe 调用。
    Shutdown,
    /// 下载完成后休眠，复用 PowerAction::Hibernate 的 shutdown.exe /h 调用。
    Hibernate,
    /// 下载完成后退出应用（Task 17）。调用 `app.exit(0)`，不强制关闭其它活动任务，
    /// 由调用方在退出前确保状态已持久化。
    Quit,
    /// 下载完成后运行用户自定义命令（Task 17）。
    ///
    /// `command` 为可执行文件路径；`args` 中每个元素都会经过 `expand_template` 替换；
    /// `working_dir` 为启动目录，`None` 表示继承当前工作目录。
    /// 命令失败不影响任务状态（仅记录到任务 error 字段）。
    RunCommand {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        working_dir: Option<String>,
    },
    /// 下载完成后复制文件到指定目录（Task 17）。
    ///
    /// `target_directory` 不能含 `..` 路径穿越；`rename_pattern` 经过模板替换后
    /// 仅作为文件名（不能含路径分隔符），`None` 表示保留原文件名。
    /// 重名时按任务 `collision_policy` 处理。源文件不会被删除。
    CopyTo {
        target_directory: String,
        #[serde(default)]
        rename_pattern: Option<String>,
    },
    /// 下载完成后移动文件到指定目录（Task 17）。
    ///
    /// 同 `CopyTo`，但使用 `rename` 完成移动；跨盘移动时退化为 `copy + remove_file`。
    /// 成功移动后原路径文件不再存在（这是 MoveTo 的预期行为，不违反 §7 禁止递归删除）。
    MoveTo {
        target_directory: String,
        #[serde(default)]
        rename_pattern: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PowerAction {
    #[default]
    None,
    Shutdown,
    Hibernate,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PowerActionPhase {
    #[default]
    Idle,
    Armed,
    Countdown,
    Blocked,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PowerActionState {
    pub action: PowerAction,
    pub phase: PowerActionPhase,
    pub remaining_seconds: u64,
    pub target_count: usize,
    pub message: Option<String>,
}

/// 下载配置预设（Task 12）。
///
/// 内置预设：`default` / `lightweight` / `large-file` / `background` / `night`，
/// 自定义预设 `is_builtin = false`。`connections` 必须是 1/2/4/8/16/32 之一，
/// 由 `validate_preset_connections` 在新增/更新时校验。
///
/// `scheduled_at` 在数据库中以 "HH:MM" 24 小时制字符串存储（夜间预设 = "22:00"）；
/// 前端在应用到任务时由 `preset_apply_to_task` 将其转换为下一次该时刻的 Unix 毫秒时间戳。
/// `None` 表示立即开始下载，不绑定计划时间。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadPreset {
    pub id: String,
    pub name: String,
    pub connections: u8,
    pub speed_limit: Option<u64>,
    pub completion_action: Option<CompletionAction>,
    pub verify_checksum: bool,
    pub scheduled_at: Option<String>,
    pub is_builtin: bool,
}

/// 分类规则匹配类型（Task 11）。
///
/// - `Domain`：按 URL 域名后缀匹配（如 `github.com` 匹配 `api.github.com`）
/// - `Mime`：按 Content-Type 主类型匹配（如 `video` 匹配 `video/mp4`）
/// - `Regex`：按文件名正则匹配
///
/// 序列化使用 kebab-case 以与现有协议字段保持一致。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CategoryRuleType {
    Domain,
    Mime,
    Regex,
}

impl CategoryRuleType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Domain => "domain",
            Self::Mime => "mime",
            Self::Regex => "regex",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "mime" => Self::Mime,
            "regex" => Self::Regex,
            _ => Self::Domain,
        }
    }
}

/// URL 历史记录条目（Task 19）。
///
/// 用于新建任务输入框的下拉历史。`url` 在表内唯一，
/// 重复添加时仅更新 `last_used`（Unix 毫秒时间戳）。
/// 表容量限制为 20 条，超过则按 `last_used` 升序删除最旧的。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UrlHistoryEntry {
    pub url: String,
    pub last_used: i64,
}

/// 任务模板（Task 36）。
///
/// 用于在新建任务时按域名匹配自动套用一组下载参数（连接数、限速、请求头、
/// 保存目录、完成动作）。匹配语义：
/// - `domain_pattern` 支持精确域名（`github.com`）与通配符子域（`*.example.com`）。
/// - 多模板同时命中时按 `priority` 升序取优先级最高（数字越小越优先）。
/// - `enabled = false` 的模板不参与匹配。
///
/// 字段语义与 `NewTaskRequest` 对应字段一致：
/// - `connections`：`None` 表示不覆盖（使用全局或请求自带值）；`Some(u8)` 必须是
///   1/2/4/8/16/32 之一，由 manager 层在保存/套用时校验（AGENTS.md §3）。
/// - `speed_limit`：`None` 表示不限速；非零值覆盖 `NewTaskRequest::per_task_speed_limit`。
/// - `headers`：`None` 表示不覆盖；`Some(map)` 会与请求头合并（模板头优先）。
/// - `destination`：`None` 表示不覆盖；非空字符串覆盖请求 `destination`。
/// - `completion_action`：`None` 表示不覆盖；非 `None` 值覆盖请求完成动作。
///
/// `#[serde(default)]` 保证旧 JSON（不含新字段）安全反序列化为 `None`/默认值，
/// 与 AGENTS.md §2"新增序列化字段必须提供安全默认值"一致。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskTemplate {
    pub id: String,
    pub name: String,
    pub domain_pattern: String,
    #[serde(default)]
    pub connections: Option<u8>,
    #[serde(default)]
    pub speed_limit: Option<u64>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub destination: Option<String>,
    #[serde(default)]
    pub completion_action: Option<CompletionAction>,
    pub enabled: bool,
    pub priority: i32,
}

/// 模板匹配测试结果（Task 36）。
///
/// 由 `task_template_test` 命令返回，描述给定 URL 是否命中模板以及命中的模板 ID。
/// `matched_template_id` 为 `None` 表示无模板命中。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TaskTemplateTestResult {
    pub matched: bool,
    pub matched_template_id: Option<String>,
    pub matched_template_name: Option<String>,
}

/// 分类规则（Task 11）。
///
/// 用户可配置多条规则，按 `priority` 升序遍历；
/// 第一个命中的规则的目标目录被用作新任务的保存目录。
/// `target_directory` 在保存时已规范化（去除尾部 `/` 或 `\`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CategoryRule {
    pub id: String,
    pub name: String,
    pub rule_type: CategoryRuleType,
    pub pattern: String,
    pub target_directory: String,
    pub enabled: bool,
    pub priority: i32,
}

/// 规则测试结果（Task 11）。
///
/// `matched` 表示规则是否命中；
/// 命中时 `target_directory` 为目标目录（已规范化）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CategoryRuleTestResult {
    pub matched: bool,
    pub target_directory: String,
}

/// 文件名清理规则（Task 20）。
///
/// 用于在保存下载文件前对文件名做正则替换，去除站点水印、画质标记重复等噪声。
/// - `pattern`：正则表达式（regex crate 语法）
/// - `replacement`：替换为的字符串（可为空字符串以直接删除匹配内容）
/// - `enabled`：是否启用；未启用的规则在应用时跳过
/// - `priority`：升序遍历，数字越小越先执行
///
/// 仅在用户未手动编辑文件名时应用（见 `NewTaskRequest` 与 `manager::add`）。
/// 应用顺序：在分类规则匹配之后、最终目标路径预留之前。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FilenameCleanupRule {
    pub id: String,
    pub name: String,
    pub pattern: String,
    pub replacement: String,
    pub enabled: bool,
    pub priority: i32,
}

/// 用户标签（Task 25）。
///
/// 用于任务的多对多分类标签。`color` 为 `#RRGGBB` 格式的十六进制颜色字符串
/// （如 `#3B82F6`），前端 chip 显示时直接使用此颜色作为背景，并通过文字阴影
/// 保证可读性，符合 AGENTS.md §4"交互不能只依赖颜色"（chip 同时包含名称文字）。
///
/// - `id`：稳定英文标识，由调用方生成（如 `tag-<uuid>`）。
/// - `name`：用户可见的简体中文标签名，表内唯一。
/// - `color`：hex 颜色字符串，存储时校验为 `#` + 6 位十六进制。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewTaskRequest {
    pub url: String,
    pub file_name: Option<String>,
    pub destination: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub scheduled_at: Option<u64>,
    #[serde(default)]
    pub priority: i32,
    pub expected_checksum: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub per_task_speed_limit: u64,
    #[serde(default)]
    pub collision_policy: CollisionPolicy,
    #[serde(default)]
    pub completion_action: CompletionAction,
    pub media: Option<MediaSelection>,
    pub connection_count: Option<u8>,
    #[serde(default)]
    pub start_paused: bool,
    /// 用户是否手动编辑过文件名（Task 20）。
    ///
    /// `true` 时跳过自动文件名清理规则，保留用户输入。
    /// 旧版本 JSON/扩展请求未包含此字段时默认 `false`，
    /// 即向后兼容地启用自动清理。
    #[serde(default)]
    pub user_edited_file_name: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BatchTaskRequest {
    pub urls: Vec<String>,
    pub destination: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub scheduled_at: Option<u64>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub per_task_speed_limit: u64,
    #[serde(default)]
    pub collision_policy: CollisionPolicy,
    #[serde(default)]
    pub completion_action: CompletionAction,
    pub connection_count: Option<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskExportFile {
    pub schema_version: u32,
    pub exported_at: u64,
    pub tasks: Vec<TaskExportItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskExportItem {
    pub url: String,
    pub file_name: String,
    pub priority: i32,
    pub scheduled_at: Option<u64>,
    pub expected_checksum: Option<String>,
    pub per_task_speed_limit: u64,
    pub collision_policy: CollisionPolicy,
    pub completion_action: CompletionAction,
    pub media: Option<MediaSelection>,
    pub connection_count: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AppSettings {
    pub download_dir: String,
    pub concurrent_downloads: u8,
    pub connections_per_download: u8,
    pub speed_limit_kbps: u64,
    pub start_minimized: bool,
    pub minimize_to_tray: bool,
    pub close_to_tray: bool,
    pub notifications: bool,
    pub auto_start: bool,
    pub theme: String,
    #[serde(default = "default_accent_color")]
    pub accent_color: String,
    #[serde(default)]
    pub frosted_glass: bool,
    pub language: String,
    pub intercept_browser_downloads: bool,
    pub min_file_size_mb: u64,
    pub clipboard_monitor: bool,
    pub proxy_mode: String,
    pub proxy_url: String,
    pub proxy_username: String,
    pub proxy_password: String,
    pub user_agent: String,
    pub default_collision_policy: CollisionPolicy,
    #[serde(default)]
    pub default_completion_action: CompletionAction,
    pub max_retries: u32,
    pub retry_base_seconds: u64,
    pub verify_after_download: bool,
    pub media_tool_auto_update: bool,
    #[serde(default)]
    pub yt_dlp_path: String,
    #[serde(default)]
    pub ffmpeg_path: String,
    #[serde(default)]
    pub ffprobe_path: String,
    #[serde(default)]
    pub low_memory_mode: bool,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub auto_scale_ui: Option<bool>,
    /// 全局默认重试策略（Task 14）。任务未设置 `retry_policy_override` 时使用。
    #[serde(default)]
    pub default_retry_policy: RetryPolicy,
    /// Task 22.1：紧凑行高（true=32px / false=36px），默认 false 保持现有视觉。
    #[serde(default)]
    pub row_compact: bool,
    /// Task 22.1：新建任务或切换选中任务时详情栏默认折叠，默认 true（紧凑优先）。
    #[serde(default = "default_detail_default_collapsed")]
    pub detail_default_collapsed: bool,
    /// Task 22.1：颜色方案枚举，独立于旧 `theme` 字符串字段。
    /// `System` 跟随 prefers-color-scheme；`Light`/`Dark` 强制覆盖系统。
    #[serde(default)]
    pub color_scheme: ColorScheme,
    /// Task 24.1：历史归档阈值——已完成任务超过 N 天后自动归入"历史"视图。
    ///
    /// 默认 30 天。前端依据 `completed_at` 与当前时间差判断是否归档。
    /// 旧 JSON 缺失此字段时通过 serde 默认值安全回填到 30。
    #[serde(default = "default_archive_days")]
    pub archive_days: u32,
    /// Task 24.1：主列表已完成任务数量阈值——超过 M 条已完成任务时，
    /// 最旧的已完成任务自动归入"历史"视图，避免主列表被旧任务占满。
    ///
    /// 默认 100 条。旧 JSON 缺失此字段时通过 serde 默认值安全回填到 100。
    #[serde(default = "default_archive_threshold")]
    pub archive_threshold: u32,
    /// Task 30.1：下载完成后发送系统通知。默认 true。
    /// 旧 JSON 缺失此字段时通过 serde 默认值安全回填到 true。
    #[serde(default = "default_notify_on_complete")]
    pub notify_on_complete: bool,
    /// Task 30.1：下载失败后发送系统通知。默认 true。
    /// 旧 JSON 缺失此字段时通过 serde 默认值安全回填到 true。
    #[serde(default = "default_notify_on_failure")]
    pub notify_on_failure: bool,
    /// Task 30.1：下载完成时播放提示音。默认 true。
    /// 旧 JSON 缺失此字段时通过 serde 默认值安全回填到 true。
    #[serde(default = "default_notify_sound_enabled")]
    pub notify_sound_enabled: bool,
    /// Task 30.1：下载失败时播放提示音。默认 false（避免打扰）。
    /// 旧 JSON 缺失此字段时通过 serde 默认值安全回填到 false。
    #[serde(default = "default_notify_failure_sound_enabled")]
    pub notify_failure_sound_enabled: bool,
    /// Task 31：PAC（Proxy Auto-Config）脚本路径。
    ///
    /// - `None`：不使用 PAC。
    /// - `Some(path)`：本地 PAC 文件绝对路径，仅当 `proxy_mode = "pac"` 时生效。
    ///
    /// 旧 JSON 缺失此字段时通过 serde 默认 `None` 安全回退。
    /// 路径必须为本地文件，不支持 `http://` 远程 PAC URL（避免引入额外网络请求）。
    #[serde(default)]
    pub pac_script_path: Option<String>,
    /// Task 32.1：是否启用计量网络自动暂停。默认 true。
    ///
    /// 旧 JSON 缺失此字段时通过 serde 默认值安全回填到 true，
    /// 保持与设计一致的"开箱默认启用"。
    #[serde(default = "default_metered_auto_pause")]
    pub metered_auto_pause: bool,
    /// Task 32.2：用户在计量网络下手动恢复后置为 true，定时检查据此跳过自动暂停。
    ///
    /// 当网络从计量变为非计量时由后端自动重置为 false，确保下次再进入计量网络时仍可生效。
    /// 旧 JSON 缺失此字段时通过 serde 默认值安全回填到 false。
    #[serde(default)]
    pub user_resumed_after_metered: bool,
}

/// Task 31：代理测试结果。
///
/// 由 `proxy_test` 命令返回，描述当前代理配置的连通性、出口 IP 与延迟。
/// `error` 字段为脱敏后的中文错误，不包含代理 URL 中的用户名/密码。
///
/// 旧 JSON 或扩展请求未包含 `success` / `latency_ms` 字段时，
/// `#[serde(default)]` 安全回退为 `false` / `0`（AGENTS.md §2
/// 新增序列化字段必须提供安全默认值）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ProxyTestResult {
    /// 测试是否成功（HTTP 200 且能解析 IP）。
    #[serde(default)]
    pub success: bool,
    /// 出口 IP（成功时为公网 IP 字符串，失败时为 `None`）。
    pub exit_ip: Option<String>,
    /// 完整请求耗时（毫秒）。
    #[serde(default)]
    pub latency_ms: u64,
    /// 失败时的脱敏错误信息。成功时为 `None`。
    pub error: Option<String>,
}

fn default_archive_days() -> u32 {
    30
}

fn default_archive_threshold() -> u32 {
    100
}

/// Task 30.1：通知与提示音默认值。完成通知/失败通知/完成提示音默认启用；
/// 失败提示音默认关闭以避免频繁打扰。
fn default_notify_on_complete() -> bool {
    true
}

fn default_notify_on_failure() -> bool {
    true
}

fn default_notify_sound_enabled() -> bool {
    true
}

fn default_notify_failure_sound_enabled() -> bool {
    false
}

/// Task 32.1：计量网络自动暂停默认启用。
fn default_metered_auto_pause() -> bool {
    true
}

/// Task 22.1：颜色方案枚举（System/Light/Dark）。
///
/// 序列化使用 lowercase 与旧 `theme` 字符串字段保持一致（"system"/"light"/"dark"），
/// 便于前端无差别处理；`Default = System` 满足向后兼容（旧 JSON 缺失此字段时回落到跟随系统）。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ColorScheme {
    System,
    Light,
    Dark,
}

impl Default for ColorScheme {
    fn default() -> Self {
        Self::System
    }
}

fn default_detail_default_collapsed() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        let download_dir = std::env::var_os("USERPROFILE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("Downloads")
            .to_string_lossy()
            .to_string();
        Self {
            download_dir,
            concurrent_downloads: 3,
            connections_per_download: 8,
            speed_limit_kbps: 0,
            start_minimized: false,
            minimize_to_tray: true,
            close_to_tray: false,
            notifications: true,
            auto_start: false,
            theme: "system".into(),
            accent_color: default_accent_color(),
            frosted_glass: false,
            language: "zh-CN".into(),
            intercept_browser_downloads: true,
            min_file_size_mb: 1,
            clipboard_monitor: false,
            proxy_mode: "system".into(),
            proxy_url: String::new(),
            proxy_username: String::new(),
            proxy_password: String::new(),
            user_agent: "MaobuFetch/0.5".into(),
            default_collision_policy: CollisionPolicy::Rename,
            default_completion_action: CompletionAction::None,
            max_retries: 3,
            retry_base_seconds: 2,
            verify_after_download: false,
            media_tool_auto_update: true,
            yt_dlp_path: String::new(),
            ffmpeg_path: String::new(),
            ffprobe_path: String::new(),
            low_memory_mode: false,
            window_width: Some(1024),
            window_height: Some(720),
            auto_scale_ui: Some(false),
            default_retry_policy: RetryPolicy::default(),
            row_compact: false,
            detail_default_collapsed: true,
            color_scheme: ColorScheme::default(),
            archive_days: default_archive_days(),
            archive_threshold: default_archive_threshold(),
            notify_on_complete: default_notify_on_complete(),
            notify_on_failure: default_notify_on_failure(),
            notify_sound_enabled: default_notify_sound_enabled(),
            notify_failure_sound_enabled: default_notify_failure_sound_enabled(),
            pac_script_path: None,
            metered_auto_pause: default_metered_auto_pause(),
            user_resumed_after_metered: false,
        }
    }
}

fn default_accent_color() -> String {
    "blue".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskProgressEvent {
    pub task: DownloadTask,
    pub event: String,
}

/// 磁盘空间不足暂停事件载荷。
///
/// 通过 `task-paused-by-low-disk` 和 `merge-blocked-by-low-disk` 事件发送给前端，
/// 用于在详情面板展示当前可用空间与所需空间，提示用户清理或更换目录。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LowDiskPayload {
    pub task_id: String,
    pub available_bytes: u64,
    pub required_bytes: u64,
}

/// 单条分片连接的实时状态（Task 18）。
///
/// 用于 `task-connections` 事件载荷，由下载循环每秒一次从真实运行时状态汇总而来，
/// 不得使用模拟数据（AGENTS.md §3）。所有字段均从 `SegmentRuntime` 的原子量派生。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionState {
    /// 连接中：分片尚未开始接收数据，等待 permit 或正在发起 Range 请求。
    Connecting,
    /// 下载中：至少一个窗口正在接收 chunk。
    Downloading,
    /// 重试中：连接出错后退避等待或重新发起请求。
    Retrying,
    /// 已完成：分片所有字节已落盘且校验通过。
    Completed,
    /// 失败：分片重试次数已达上限。
    Failed,
    /// 已暂停：任务被取消或暂停，连接已停止。
    Paused,
}

/// 单个分片的实时状态快照（Task 18）。
///
/// `error` 字段在发送给前端前必须经过 `redact_sensitive` 脱敏，
/// 不得包含 Cookie、Authorization、代理密码或 URL 中的 token 段（AGENTS.md §3、§7）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SegmentStatus {
    /// 分片 ID（使用逻辑分片 index 转字符串，便于前端 keying）。
    pub segment_id: String,
    /// 起始偏移（来自 `DownloadSegment::start_byte`）。
    pub start_offset: u64,
    /// 已下载字节（来自 `SegmentRuntime::downloaded_bytes` 原子量）。
    pub downloaded_bytes: u64,
    /// 该分片总字节（`end_byte - start_byte + 1`）。
    pub total_bytes: u64,
    /// 当前速度 bytes/sec（来自每秒采样的 EWMA，非模拟）。
    pub speed: u64,
    /// 当前连接状态。
    pub state: ConnectionState,
    /// 重试次数（连接级，独立于 `DownloadTask::retry_count`）。
    pub retry_count: u32,
    /// 错误信息（脱敏后），仅在 `Failed` 状态下有意义。
    pub error: Option<String>,
}

/// `task-connections` 事件载荷（Task 18.3）。
///
/// 频率：每秒一次，与 `task-updated` 同步发出，不更高（AGENTS.md §8）。
/// 仅在任务处于 `Downloading` 状态时推送；暂停 / 完成后不再推送。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TaskConnectionsEvent {
    pub task_id: String,
    pub segments: Vec<SegmentStatus>,
    /// Unix 毫秒时间戳（前端用于判断事件新旧）。
    pub timestamp: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairingInfo {
    pub code: String,
    pub expires_at: u64,
    pub paired_extension: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolPhase {
    Missing,
    Downloading,
    Verifying,
    Extracting,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ToolComponent {
    YtDlp,
    Ffmpeg,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolStatus {
    pub state: ToolPhase,
    pub version: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub installed_bytes: u64,
    pub error: Option<String>,
    pub yt_dlp_available: bool,
    pub ffmpeg_available: bool,
    pub active_component: Option<ToolComponent>,
    pub yt_dlp_version: String,
    pub ffmpeg_version: String,
    pub yt_dlp_download_bytes: u64,
    pub ffmpeg_download_bytes: u64,
    pub yt_dlp_installed_bytes: u64,
    pub ffmpeg_installed_bytes: u64,
    pub yt_dlp_source: String,
    pub ffmpeg_source: String,
    pub yt_dlp_resolved_path: Option<String>,
    pub ffmpeg_resolved_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetectedMediaTools {
    pub yt_dlp_path: Option<String>,
    pub ffmpeg_path: Option<String>,
    pub ffprobe_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaProbeResult {
    pub title: String,
    pub thumbnail: Option<String>,
    pub extractor: Option<String>,
    pub duration: Option<f64>,
    pub formats: Vec<MediaFormat>,
    pub subtitles: Vec<String>,
    pub drm: bool,
    /// Task 38 / Task 41：媒体内容类型，用于区分视频/音频/图集/混合。
    ///
    /// `#[serde(default)]` 保证旧数据库与旧 JSON 仍可读取（默认 `Video`），
    /// 满足 AGENTS.md §2"新增序列化字段必须提供安全默认值"。
    #[serde(default)]
    pub media_type: MediaType,
}

/// 媒体内容类型枚举（Task 38 / Task 41）。
///
/// 用于 `MediaProbeResult.media_type` 字段，前端据此选择不同的下载策略
/// （图集需要多图下载、音频仅下载音频流等）。
///
/// 序列化使用 kebab-case，与前端 TypeScript 联合类型对应。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum MediaType {
    /// 普通视频（默认值，向后兼容）。
    #[default]
    Video,
    /// 纯音频（如 Twitter Spaces、YouTube 音乐）。
    Audio,
    /// 图集（抖音 note、TikTok photo）。
    Gallery,
    /// 混合内容（视频+图集等）。
    Mixed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaFormat {
    pub id: String,
    pub label: String,
    pub extension: Option<String>,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub file_size: Option<u64>,
    pub has_video: bool,
    pub has_audio: bool,
    #[serde(default)]
    pub requires_ffmpeg: bool,
    /// Task 42：图集场景下的图片直链 URL。
    ///
    /// 仅当 `media_type = Gallery` 且该格式为图片项（vcodec=none、acodec=none、
    /// ext 为 jpg/jpeg/png/webp/gif/bmp）时填充；其余场景始终为 `None`。
    ///
    /// `#[serde(default)]` 保证旧 JSON/旧扩展请求缺失此字段时安全反序列化为 `None`，
    /// 满足 AGENTS.md §2"新增序列化字段必须提供安全默认值"。
    #[serde(default)]
    pub image_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PrecheckRequest {
    pub url: String,
    #[serde(default)]
    pub target_directory: Option<String>,
    #[serde(default)]
    pub suggested_filename: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub proxy_override: Option<String>,
    #[serde(default)]
    pub proxy_auth: Option<ProxyAuth>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RedirectHop {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub status: u16,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PrecheckConflictType {
    #[default]
    DuplicateUrl,
    DuplicateFinalUrl,
    DuplicateTargetPath,
}

impl PrecheckConflictType {
    pub fn label(&self) -> &'static str {
        match self {
            Self::DuplicateUrl => "URL 冲突",
            Self::DuplicateFinalUrl => "最终地址冲突",
            Self::DuplicateTargetPath => "目标文件冲突",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PrecheckConflict {
    #[serde(default)]
    pub conflict_type: PrecheckConflictType,
    #[serde(default)]
    pub existing_task_id: String,
    #[serde(default)]
    pub existing_task_label: String,
}

/// 磁盘可用空间计算状态（Task 1 重构）。
///
/// - `Sufficient`：已知大小且可用空间满足要求
/// - `Insufficient`：可用空间小于计算所需的最小门槛
/// - `Unknown`：未知文件大小或磁盘擦写信息获取失败
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PrecheckDiskState {
    #[default]
    Sufficient,
    Insufficient,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PrecheckResult {
    #[serde(default)]
    pub original_url: String,
    #[serde(default)]
    pub final_url: String,
    #[serde(default)]
    pub redirect_chain: Vec<RedirectHop>,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub file_size: Option<u64>,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    #[serde(default)]
    pub accepts_ranges: bool,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub suggested_connections: u8,
    #[serde(default)]
    pub supports_resume: bool,
    #[serde(default)]
    pub target_directory: String,
    #[serde(default)]
    pub available_disk_bytes: u64,
    #[serde(default)]
    pub required_disk_bytes: u64,
    #[serde(default)]
    pub disk_ok: bool,
    #[serde(default)]
    pub disk_state: PrecheckDiskState,
    #[serde(default)]
    pub conflicts: Vec<PrecheckConflict>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// 错误诊断分类（Task 3）。
///
/// 每个分类对应一组标准建议操作，前端依据 `action_id` 调用对应 Tauri 命令。
/// 序列化使用 kebab-case 以保持与现有协议字段一致。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorCategory {
    /// 401/403：链接或登录信息已过期。
    AuthExpired,
    /// 416：远端文件变化或分片失效。
    RangeInvalid,
    /// 磁盘空间不足。
    DiskFull,
    /// 代理连接失败。
    ProxyFailed,
    /// ETag 变化，远端资源已更新。
    ETagChanged,
    /// SHA-256 校验失败。
    ChecksumFailed,
    /// 连接被重置或断开。
    NetworkReset,
    /// TLS 握手或证书校验失败。
    TlsFailed,
    /// 5xx 服务器错误。
    ServerError,
    /// 请求超时。
    Timeout,
    /// 磁盘 IO 错误（权限、读写失败等）。
    DiskIo,
    /// 远端资源变化（Last-Modified 或 ETag 不一致）。
    RemoteChanged,
    /// 未识别的错误类型。
    Unknown,
}

impl Default for ErrorCategory {
    fn default() -> Self {
        Self::Unknown
    }
}

impl ErrorCategory {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AuthExpired => "auth-expired",
            Self::RangeInvalid => "range-invalid",
            Self::DiskFull => "disk-full",
            Self::ProxyFailed => "proxy-failed",
            Self::ETagChanged => "etag-changed",
            Self::ChecksumFailed => "checksum-failed",
            Self::NetworkReset => "network-reset",
            Self::TlsFailed => "tls-failed",
            Self::ServerError => "server-error",
            Self::Timeout => "timeout",
            Self::DiskIo => "disk-io",
            Self::RemoteChanged => "remote-changed",
            Self::Unknown => "unknown",
        }
    }
}

/// 建议操作项。
///
/// `action_id` 是稳定英文标识，对应前端将调用的 Tauri 命令或动作；
/// `label` 为简体中文按钮文案。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SuggestedAction {
    pub action_id: String,
    pub label: String,
}

/// 错误诊断结果。
///
/// `raw_error_redacted` 必须经过 `redact_sensitive` 脱敏，
/// 不得包含 Cookie、Authorization、代理密码或 URL 中的 token 段。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDiagnosis {
    pub category: ErrorCategory,
    pub title: String,
    pub description: String,
    pub suggested_actions: Vec<SuggestedAction>,
    pub raw_error_redacted: String,
}

/// 队列调度可观察性（Task 15）：任务等待原因。
///
/// 用于在任务详情面板展示"为什么这个任务还没开始"。
/// 内部标签使用 `kind` 字段，序列化为 kebab-case 以保持协议一致。
/// 所有带数据的变体字段均标记 `#[serde(default)]`，保证旧前端或旧 JSON
/// 反序列化时不会因缺失字段而失败。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum WaitReason {
    /// 任务正在下载或已完成，不在等待。
    NotWaiting,
    /// 等待前面 N 个任务完成。
    QueuedBehind {
        #[serde(default)]
        ahead_count: u32,
    },
    /// 等待媒体工具安装。
    WaitingMediaTools,
    /// 等待用户确认（如冲突）。
    WaitingUserConfirmation,
    /// 等待计划时间到达。
    WaitingScheduledTime {
        #[serde(default)]
        scheduled_at: String,
    },
    /// 等待并发槽位（全局并发限制已满）。
    WaitingConcurrencyLimit {
        #[serde(default)]
        active_count: u32,
    },
    /// 用户手动暂停。
    Paused,
    /// 磁盘空间不足暂停。
    PausedByLowDisk,
    /// Task 32：计量网络下自动暂停。
    PausedByMetered,
    /// 中断。
    Interrupted,
    /// 远端变化。
    RemoteChanged,
    /// 未知状态。
    Unknown,
}

impl Default for WaitReason {
    fn default() -> Self {
        Self::NotWaiting
    }
}

/// 任务级超时与重试策略的退避算法（Task 14）。
///
/// - `Fixed`：每次重试等待 `initial_backoff_ms` 毫秒
/// - `Exponential`：第 N 次重试等待 `min(initial * 2^(N-1), max_backoff_ms)` 毫秒
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BackoffStrategy {
    Fixed,
    Exponential,
}

impl Default for BackoffStrategy {
    fn default() -> Self {
        Self::Exponential
    }
}

/// 任务级超时与重试策略（Task 14）。
///
/// 该策略以"连接"为单位生效：每条 HTTP Range 连接独立计数 attempt，
/// 达到 `max_retries` 后该连接失败，任务进入 Failed 状态。
/// `task_timeout_secs` 优先于连接重试：即使 attempt 未达上限，
/// 超过总任务时长也强制失败。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetryPolicy {
    /// 单连接超时（秒），用于 reqwest ClientBuilder 的 connect_timeout + timeout。
    pub connection_timeout_secs: u64,
    /// 总任务超时（秒），`None` 表示不限。
    pub task_timeout_secs: Option<u64>,
    /// 最大重试次数（每条连接独立计数）。
    pub max_retries: u32,
    /// 退避策略：固定或指数。
    pub backoff: BackoffStrategy,
    /// 初始退避间隔（毫秒）。
    pub initial_backoff_ms: u64,
    /// 最大退避间隔（毫秒），指数退避下的上限。
    pub max_backoff_ms: u64,
}

// ===== Task 27: 完整备份与恢复 =====

/// 备份文件 schema 版本。仅在破坏性结构变更时递增；
/// 旧版本备份必须仍可被读取（向后兼容）。
pub const BACKUP_BUNDLE_VERSION: u32 = 1;

/// PBKDF2 派生密钥的迭代次数。100k 是 OWASP 2023 推荐下限，
/// 在普通办公电脑上派生耗时约 100ms，可接受。
pub const BACKUP_KDF_ITERATIONS: u32 = 100_000;

/// AES-256-GCM 密钥长度（字节）。
pub const BACKUP_KEY_SIZE: usize = 32;

/// AES-256-GCM nonce 长度（字节），由规范固定为 12。
pub const BACKUP_NONCE_SIZE: usize = 12;

/// PBKDF2 盐长度（字节），16 字节已足够防彩虹表。
pub const BACKUP_SALT_SIZE: usize = 16;

/// 完整备份 bundle（Task 27）。
///
/// 包含设置、分类规则、文件名清理规则、下载预设、URL 历史、任务列表。
/// `includes_auth = false` 时已脱敏（Cookie/Authorization/代理密码被清空）；
/// `includes_auth = true` 时保留认证信息，导出文件必须加密。
///
/// 序列化字段全部使用 `#[serde(default)]` 保证旧版本备份文件可安全读取，
/// 不会因新增字段而失败。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackupBundle {
    /// 备份格式版本号，等于 [`BACKUP_BUNDLE_VERSION`]。
    pub version: u32,
    /// 备份创建时间（ISO 8601 UTC 字符串，便于人类阅读）。
    pub created_at: String,
    /// 创建备份时的应用版本号（如 "0.5.7"）。
    #[serde(default)]
    pub app_version: String,
    /// 应用设置。`None` 表示备份中不包含设置（罕见，向前兼容）。
    #[serde(default)]
    pub settings: Option<AppSettings>,
    /// 分类规则列表（按 priority 升序）。
    #[serde(default)]
    pub category_rules: Vec<CategoryRule>,
    /// 文件名清理规则列表（按 priority 升序）。
    #[serde(default)]
    pub filename_cleanup_rules: Vec<FilenameCleanupRule>,
    /// 下载预设列表（内置 + 自定义）。
    #[serde(default)]
    pub download_presets: Vec<DownloadPreset>,
    /// URL 历史记录列表（最近 20 条）。
    #[serde(default)]
    pub url_history: Vec<UrlHistoryEntry>,
    /// 任务列表（保留状态与历史，恢复时按 ID 去重）。
    #[serde(default)]
    pub tasks: Vec<DownloadTask>,
    /// 是否包含认证信息。`true` 时备份文件必须加密。
    #[serde(default)]
    pub includes_auth: bool,
}

/// 备份文件清单（明文，未加密部分），用于在不解密的情况下识别备份元信息。
///
/// 序列化为 JSON 文件顶层的 `manifest` 字段。`encrypted = true` 时
/// 真正的 bundle 数据被加密存于 `ciphertext` 字段；`encrypted = false`
/// 时 bundle 数据直接存于 `bundle` 字段。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackupManifest {
    /// 备份格式版本号。
    pub version: u32,
    /// 备份创建时间（ISO 8601 UTC 字符串）。
    pub created_at: String,
    /// 创建备份时的应用版本号。
    #[serde(default)]
    pub app_version: String,
    /// 是否加密。`true` 时 `ciphertext` 字段存在。
    pub encrypted: bool,
    /// 是否包含认证信息。
    pub includes_auth: bool,
    /// 加密元数据。`encrypted = false` 时为 `None`。
    #[serde(default)]
    pub kdf: Option<BackupKdfInfo>,
    /// 对称加密元数据。`encrypted = false` 时为 `None`。
    #[serde(default)]
    pub cipher: Option<BackupCipherInfo>,
}

/// PBKDF2 密钥派生参数（明文存储，用于恢复时重新派生密钥）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupKdfInfo {
    /// 算法标识，目前固定为 `"pbkdf2-sha256"`。
    pub algorithm: String,
    /// 迭代次数。
    pub iterations: u32,
    /// 盐（base64 编码）。
    pub salt: String,
    /// 派生密钥长度（字节）。
    pub key_size: u32,
}

/// 对称加密参数（明文存储 nonce 等）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupCipherInfo {
    /// 算法标识，目前固定为 `"aes-256-gcm"`。
    pub algorithm: String,
    /// Nonce / IV（base64 编码）。
    pub nonce: String,
}

/// 恢复前预览结果（Task 27.3）。
///
/// 列出本次恢复将新增、覆盖、跳过的条数，便于用户在确认前评估影响。
/// 所有计数均为非负整数，由后端在内存中比对当前数据库状态得出。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RestorePreview {
    /// 设置差异描述。
    #[serde(default)]
    pub settings_diff: SettingsDiff,
    /// 新增的分类规则数量（按 ID 比对，当前数据库中不存在）。
    #[serde(default)]
    pub new_category_rules: u32,
    /// 覆盖的分类规则数量（按 ID 比对，已存在 → 将被更新）。
    #[serde(default)]
    pub override_category_rules: u32,
    /// 新增的文件名清理规则数量。
    #[serde(default)]
    pub new_filename_cleanup_rules: u32,
    /// 覆盖的文件名清理规则数量。
    #[serde(default)]
    pub override_filename_cleanup_rules: u32,
    /// 新增的下载预设数量（按 ID 比对）。
    #[serde(default)]
    pub new_presets: u32,
    /// 覆盖的下载预设数量。
    #[serde(default)]
    pub override_presets: u32,
    /// 新增的 URL 历史条数（按 URL 比对）。
    #[serde(default)]
    pub new_url_history: u32,
    /// 新增的任务数量（按 ID 比对，当前数据库中不存在）。
    #[serde(default)]
    pub new_tasks: u32,
    /// 重复的任务数量（按 ID 比对，已存在 → 跳过，不覆盖用户进度）。
    #[serde(default)]
    pub duplicate_tasks: u32,
    /// 备份是否包含认证信息（前端展示用）。
    #[serde(default)]
    pub includes_auth: bool,
    /// 备份是否加密（前端展示用）。
    #[serde(default)]
    pub encrypted: bool,
    /// 备份创建时间（前端展示用）。
    #[serde(default)]
    pub created_at: String,
    /// 备份应用版本号（前端展示用）。
    #[serde(default)]
    pub app_version: String,
}

/// 设置差异描述（Task 27.3）。
///
/// `changed_fields` 列出备份与当前设置不一致的字段名（英文，与协议一致）。
/// `identical = true` 表示设置完全相同，恢复时不会改动。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettingsDiff {
    /// 不一致的字段名列表（英文协议字段名）。
    #[serde(default)]
    pub changed_fields: Vec<String>,
    /// 设置是否完全相同。
    #[serde(default)]
    pub identical: bool,
}

/// 恢复执行结果统计（Task 27.3 / 27.6）。
///
/// 由 `backup_restore` 在恢复完成后返回，前端据此向用户展示恢复结果摘要。
/// 所有计数字段为非负整数，反映实际写入数据库的条目数（已存在被覆盖的也计入"应用"）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestoreStats {
    /// 实际新增的任务数量（已存在的被跳过，不计入此处）。
    #[serde(default)]
    pub added_tasks: u32,
    /// 因 ID 已存在而跳过的任务数量（不覆盖用户进度）。
    #[serde(default)]
    pub skipped_tasks: u32,
    /// 已应用的规则总数（分类规则 + 文件名清理规则 + 下载预设，包含新增与覆盖）。
    #[serde(default)]
    pub rules_applied: u32,
    /// 已添加（或更新 `last_used`）的 URL 历史条数。
    #[serde(default)]
    pub url_history_added: u32,
    /// 是否替换了应用设置（备份中包含 settings 字段时为 true）。
    #[serde(default)]
    pub settings_replaced: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            connection_timeout_secs: 60,
            task_timeout_secs: None,
            max_retries: 5,
            backoff: BackoffStrategy::Exponential,
            initial_backoff_ms: 1000,
            max_backoff_ms: 60_000,
        }
    }
}

/// 应用更新信息（Task 26）。
///
/// 由 `updater::check_app_update` 通过 GitHub Releases API 获取后填充。
/// 仅用于"检查并提醒"，不触发自动下载（AGENTS.md §6）。
///
/// - `version`：最新 release 的 tag，已剥离前导 `v`（如 `0.5.7`）。
/// - `release_date`：GitHub `published_at` 原值（ISO 8601 字符串）。
/// - `download_url`：release 的 `html_url`，作为"前往下载页"按钮目标，
///   不指向单个二进制资源以避免误触发自动下载。
/// - `sha256`：当前 release 资产的 SHA-256；GitHub Releases 不一定提供，
///   未提供时为 `None`，前端不展示校验值。
/// - `release_notes`：release `body` 字段原文（Markdown）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct UpdateInfo {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub release_date: String,
    #[serde(default)]
    pub download_url: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub release_notes: String,
}

/// 应用更新检查结果（Task 26）。
///
/// - `latest`：远端最新 release 信息；网络失败或解析失败时为 `None`。
/// - `has_update`：`true` 表示远端版本严格大于当前版本。
/// - `current_version`：参与比较的本地版本（透传给前端展示）。
/// - `error`：失败时为可读中文错误（脱敏后），成功时为 `None`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct UpdateCheckResult {
    #[serde(default)]
    pub latest: Option<UpdateInfo>,
    #[serde(default)]
    pub has_update: bool,
    #[serde(default)]
    pub current_version: String,
    #[serde(default)]
    pub error: Option<String>,
}

/// 扩展兼容性检查结果（Task 26.3）。
///
/// `compatible = false` 时 `message` 给出中文说明，前端展示为提示卡片。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ExtensionCompatibilityResult {
    #[serde(default)]
    pub compatible: bool,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub extension_version: String,
    #[serde(default)]
    pub message: String,
}

/// Task 46：按域名存储的媒体凭证。
///
/// 用于在分析/下载媒体时复用用户已保存的 Cookie/Referer/User-Agent，
/// 避免每次重新输入。`cookie` 字段在数据库中以 DPAPI 密文形式存储
/// （`cookie_encrypted` 列），但在结构体与前端协议中始终为明文——
/// 加解密只在 `store` 层发生。
///
/// - `domain`：注册域名（去掉前导 `www.`），作为主键。如 `example.com`。
/// - `cookie`：明文 Cookie 字符串（`name=value; name2=value2`）。
///   保存时由 `store` 调用 `secure_storage::encrypt_password` 加密为密文落库。
/// - `referer` / `user_agent`：可选辅助头，不加密（非机密信息）。
/// - `updated_at`：ISO 8601 UTC 字符串，仅用于前端展示。
///
/// `#[serde(default)]` 保证旧 JSON/前端请求缺失字段时安全反序列化。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MediaCredential {
    pub domain: String,
    #[serde(default)]
    pub cookie: String,
    #[serde(default)]
    pub referer: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub updated_at: String,
}

/// 平台命名模板（Task 43）。
///
/// 用于在媒体下载完成后按平台套用文件名模板。每条模板绑定一个 `platform`
/// 字段（值为 `MediaPlatform::as_str()` 返回的小写字符串，如 `douyin`/`tiktok`/
/// `twitter`/`youtube`/`bilibili`/`weibo`/`unknown`），仅在该平台的任务上生效。
///
/// 字段语义：
/// - `id`：稳定英文标识，自定义模板由调用方生成（如 `template-<uuid>`），
///   内置模板使用稳定 ID（如 `douyin-default`）。
/// - `platform`：平台 key（小写英文），与 `MediaPlatform::as_str()` 对应。
/// - `template`：模板字符串，支持变量 `{author}`/`{title}`/`{date}`/`{platform}`
///   /`{id}`/`{channel}`/`{bvid}`，由 `manager::naming_template::apply_naming_template`
///   替换并清理非法字符与截断到 100 字符（不含扩展名）。
/// - `enabled`：是否启用；`false` 的模板在匹配时被跳过。
/// - `is_builtin`：内置模板标记；内置模板可编辑、可禁用，但前端应禁止删除
///   （AGENTS.md §3：不得改变用户设置除非用户明确触发；内置模板作为安全默认）。
///
/// `#[serde(default)]` 保证旧 JSON（不含 `is_builtin` 等新字段）安全反序列化为默认值，
/// 与 AGENTS.md §2"新增序列化字段必须提供安全默认值"一致。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlatformNamingTemplate {
    pub id: String,
    pub platform: String,
    pub template: String,
    pub enabled: bool,
    #[serde(default)]
    pub is_builtin: bool,
}

/// Task 44：平台支持级别。
///
/// 用于在新建任务对话框和设置页"平台兼容性"子区域中向用户展示
/// 当前对各媒体平台的支持程度。`#[serde(rename_all = "kebab-case")]`
/// 保证序列化为 `"verified"` / `"experimental"` / `"unsupported"`，
/// 与前端 TypeScript 联合类型对应。
///
/// - `Verified`：经过完整测试，预期可用（如 YouTube / 哔哩哔哩）。
/// - `Experimental`：基本可用但成功率受平台变更影响（如抖音 / TikTok /
///   Twitter / 微博），可能需要用户手动提供 Cookie。
/// - `Unsupported`：明确不支持，新建任务时应禁用"开始下载"按钮
///   并提示用户。`#[default]` 为 `Experimental`，保证旧数据库
///   /旧 JSON 缺失字段时安全反序列化。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SupportLevel {
    Verified,
    Experimental,
    #[default]
    Unsupported,
}

/// Task 44：单条平台兼容性记录。
///
/// `platform` 为 `MediaPlatform` 序列化值（`"douyin"` / `"tiktok"` /
/// `"twitter"` / `"youtube"` / `"bilibili"` / `"weibo"` / `"unknown"`），
/// 以字符串形式存储以避免 `models` ↔ `media_platforms` 循环依赖。
///
/// - `notes`：前端展示用的中文说明（如"YouTube 普通视频可直接下载"）。
/// - `known_issues`：已知问题列表，每项为一条中文短描述。
/// - `last_tested_at`：最近一次回归测试时间（ISO 8601 UTC 字符串）。
///
/// `#[serde(default)]` 保证旧 JSON/前端请求缺失字段时安全反序列化。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PlatformCompatibility {
    pub platform: String,
    pub level: SupportLevel,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub known_issues: Vec<String>,
    #[serde(default)]
    pub last_tested_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_json_defaults_low_memory_mode_to_off() {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("low_memory_mode");
        object.remove("yt_dlp_path");
        object.remove("ffmpeg_path");
        object.remove("ffprobe_path");
        object.remove("default_completion_action");
        object.remove("accent_color");

        let restored: AppSettings = serde_json::from_value(value).unwrap();
        assert!(!restored.low_memory_mode);
        assert!(restored.yt_dlp_path.is_empty());
        assert!(restored.ffmpeg_path.is_empty());
        assert!(restored.ffprobe_path.is_empty());
        assert_eq!(restored.default_completion_action, CompletionAction::None);
        assert_eq!(restored.accent_color, "blue");
    }

    #[test]
    fn old_settings_json_defaults_frosted_glass_to_off() {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value.as_object_mut().unwrap().remove("frosted_glass");

        let restored: AppSettings = serde_json::from_value(value).unwrap();
        assert!(!restored.frosted_glass);
    }

    #[test]
    fn old_browser_media_request_defaults_ffmpeg_requirement_to_off() {
        let selection: MediaSelection = serde_json::from_value(serde_json::json!({
            "format_id": "18",
            "subtitles": []
        }))
        .unwrap();
        assert!(!selection.requires_ffmpeg);
    }

    #[test]
    fn old_extension_request_defaults_completion_action_to_none() {
        let request: NewTaskRequest = serde_json::from_value(serde_json::json!({
            "url": "https://example.com/file.zip"
        }))
        .unwrap();
        assert_eq!(request.completion_action, CompletionAction::None);
    }

    #[test]
    fn waiting_network_status_uses_stable_protocol_value() {
        assert_eq!(
            serde_json::to_string(&TaskStatus::WaitingNetwork).unwrap(),
            "\"waiting-network\""
        );
        assert_eq!(
            TaskStatus::from_db("waiting-network"),
            TaskStatus::WaitingNetwork
        );
    }

    #[test]
    fn interrupted_status_round_trips_through_db_and_json() {
        assert_eq!(
            serde_json::to_string(&TaskStatus::Interrupted).unwrap(),
            "\"interrupted\""
        );
        assert_eq!(TaskStatus::from_db("interrupted"), TaskStatus::Interrupted);
        assert_eq!(TaskStatus::Interrupted.as_str(), "interrupted");
    }

    #[test]
    fn remote_changed_status_round_trips_through_db_and_json() {
        assert_eq!(
            serde_json::to_string(&TaskStatus::RemoteChanged).unwrap(),
            "\"remote-changed\""
        );
        assert_eq!(
            TaskStatus::from_db("remote-changed"),
            TaskStatus::RemoteChanged
        );
        assert_eq!(TaskStatus::RemoteChanged.as_str(), "remote-changed");
    }

    #[test]
    fn paused_by_low_disk_status_round_trips_through_db_and_json() {
        assert_eq!(
            serde_json::to_string(&TaskStatus::PausedByLowDisk).unwrap(),
            "\"paused-by-low-disk\""
        );
        assert_eq!(
            TaskStatus::from_db("paused-by-low-disk"),
            TaskStatus::PausedByLowDisk
        );
        assert_eq!(TaskStatus::PausedByLowDisk.as_str(), "paused-by-low-disk");
    }

    #[test]
    fn paused_by_low_disk_unknown_db_value_falls_back_to_queued() {
        // 旧数据库不会包含此状态，from_db 已显式映射；任意未知值仍回落到 Queued。
        assert_eq!(
            TaskStatus::from_db("paused-by-low-disk-extra"),
            TaskStatus::Queued
        );
    }

    #[test]
    fn paused_by_metered_status_round_trips_through_db_and_json() {
        // Task 32：新增 PausedByMetered 状态序列化为 "paused-by-metered"，
        // 旧数据库不会包含此值；from_db 显式映射保证向后兼容。
        assert_eq!(
            serde_json::to_string(&TaskStatus::PausedByMetered).unwrap(),
            "\"paused-by-metered\""
        );
        assert_eq!(
            TaskStatus::from_db("paused-by-metered"),
            TaskStatus::PausedByMetered
        );
        assert_eq!(TaskStatus::PausedByMetered.as_str(), "paused-by-metered");
    }

    #[test]
    fn old_settings_json_defaults_metered_auto_pause_to_on() {
        // 旧版本 settings JSON 不会包含 metered_auto_pause 字段，
        // serde(default = "default_metered_auto_pause") 应安全填充为 true。
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value.as_object_mut().unwrap().remove("metered_auto_pause");

        let restored: AppSettings = serde_json::from_value(value).unwrap();
        assert!(restored.metered_auto_pause);
    }

    #[test]
    fn old_settings_json_defaults_user_resumed_after_metered_to_false() {
        // 旧版本 settings JSON 不会包含 user_resumed_after_metered 字段，
        // serde(default) 应安全填充为 false。
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("user_resumed_after_metered");

        let restored: AppSettings = serde_json::from_value(value).unwrap();
        assert!(!restored.user_resumed_after_metered);
    }

    #[test]
    fn app_settings_default_has_task32_metered_defaults() {
        let settings = AppSettings::default();
        assert!(settings.metered_auto_pause);
        assert!(!settings.user_resumed_after_metered);
    }

    #[test]
    fn metered_settings_round_trip_through_json() {
        let mut settings = AppSettings::default();
        settings.metered_auto_pause = false;
        settings.user_resumed_after_metered = true;
        let json = serde_json::to_string(&settings).unwrap();
        let restored: AppSettings = serde_json::from_str(&json).unwrap();
        assert!(!restored.metered_auto_pause);
        assert!(restored.user_resumed_after_metered);
    }

    #[test]
    fn unknown_db_status_falls_back_to_queued() {
        assert_eq!(TaskStatus::from_db("nonexistent"), TaskStatus::Queued);
    }

    #[test]
    fn power_action_uses_stable_kebab_case_values() {
        assert_eq!(
            serde_json::to_string(&PowerAction::Hibernate).unwrap(),
            "\"hibernate\""
        );
        assert_eq!(
            serde_json::to_string(&PowerActionPhase::Countdown).unwrap(),
            "\"countdown\""
        );
    }

    #[test]
    fn precheck_result_defaults_to_safe_empty_state() {
        let restored: PrecheckResult = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(restored.original_url.is_empty());
        assert!(restored.final_url.is_empty());
        assert!(restored.redirect_chain.is_empty());
        assert!(restored.file_name.is_empty());
        assert!(restored.file_size.is_none());
        assert!(restored.etag.is_none());
        assert!(restored.last_modified.is_none());
        assert!(!restored.accepts_ranges);
        assert!(!restored.supports_resume);
        assert_eq!(restored.suggested_connections, 0);
        assert!(restored.conflicts.is_empty());
        assert!(restored.warnings.is_empty());
        assert!(!restored.disk_ok);
    }

    #[test]
    fn precheck_request_defaults_optional_fields_to_none() {
        let request: PrecheckRequest =
            serde_json::from_value(serde_json::json!({"url": "https://example.com"})).unwrap();
        assert_eq!(request.url, "https://example.com");
        assert!(request.target_directory.is_none());
        assert!(request.suggested_filename.is_none());
    }

    #[test]
    fn precheck_conflict_type_uses_kebab_case_serialization() {
        assert_eq!(
            serde_json::to_string(&PrecheckConflictType::DuplicateUrl).unwrap(),
            "\"duplicate-url\""
        );
        assert_eq!(
            serde_json::to_string(&PrecheckConflictType::DuplicateFinalUrl).unwrap(),
            "\"duplicate-final-url\""
        );
        assert_eq!(
            serde_json::to_string(&PrecheckConflictType::DuplicateTargetPath).unwrap(),
            "\"duplicate-target-path\""
        );
    }

    #[test]
    fn precheck_conflict_type_default_is_duplicate_url() {
        let restored: PrecheckConflict = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(restored.conflict_type, PrecheckConflictType::DuplicateUrl);
    }

    #[test]
    fn completion_action_shutdown_and_hibernate_round_trip_as_kebab_case() {
        assert_eq!(
            serde_json::to_string(&CompletionAction::Shutdown).unwrap(),
            "\"shutdown\""
        );
        assert_eq!(
            serde_json::to_string(&CompletionAction::Hibernate).unwrap(),
            "\"hibernate\""
        );
        let restored: CompletionAction = serde_json::from_str("\"shutdown\"").unwrap();
        assert_eq!(restored, CompletionAction::Shutdown);
    }

    #[test]
    fn completion_action_unknown_value_falls_back_to_none() {
        // 旧版本 JSON 不会包含 shutdown / hibernate，未知值应安全回落到 None。
        let restored: CompletionAction = serde_json::from_str("\"power-off\"").unwrap_or_default();
        assert_eq!(restored, CompletionAction::None);
    }

    #[test]
    fn download_preset_round_trips_through_json() {
        let preset = DownloadPreset {
            id: "night".into(),
            name: "夜间下载".into(),
            connections: 8,
            speed_limit: None,
            completion_action: Some(CompletionAction::Shutdown),
            verify_checksum: false,
            scheduled_at: Some("22:00".into()),
            is_builtin: true,
        };
        let json = serde_json::to_string(&preset).unwrap();
        let restored: DownloadPreset = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, preset);
        assert_eq!(restored.completion_action, Some(CompletionAction::Shutdown));
        assert_eq!(restored.scheduled_at.as_deref(), Some("22:00"));
    }

    #[test]
    fn download_preset_defaults_optional_fields_when_missing() {
        // 模拟旧版本 JSON 缺少字段时仍可安全反序列化。
        let restored: DownloadPreset = serde_json::from_value(serde_json::json!({
            "id": "default",
            "name": "普通下载",
            "connections": 8,
            "verify_checksum": false,
            "is_builtin": true
        }))
        .unwrap();
        assert!(restored.speed_limit.is_none());
        assert!(restored.completion_action.is_none());
        assert!(restored.scheduled_at.is_none());
    }

    #[test]
    fn duplicate_type_uses_kebab_case_serialization() {
        assert_eq!(
            serde_json::to_string(&DuplicateType::SameUrl).unwrap(),
            "\"same-url\""
        );
        assert_eq!(
            serde_json::to_string(&DuplicateType::SameFinalUrl).unwrap(),
            "\"same-final-url\""
        );
        assert_eq!(
            serde_json::to_string(&DuplicateType::SameTargetPath).unwrap(),
            "\"same-target-path\""
        );
        assert_eq!(
            serde_json::to_string(&DuplicateType::SameChecksum).unwrap(),
            "\"same-checksum\""
        );
    }

    #[test]
    fn duplicate_type_label_returns_chinese_text() {
        assert_eq!(DuplicateType::SameUrl.label(), "URL 冲突");
        assert_eq!(DuplicateType::SameFinalUrl.label(), "最终地址冲突");
        assert_eq!(DuplicateType::SameTargetPath.label(), "目标文件冲突");
        assert_eq!(DuplicateType::SameChecksum.label(), "已下载过相同文件");
    }

    #[test]
    fn duplicate_check_result_defaults_to_empty_matches() {
        let restored: DuplicateCheckResult = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(restored.matches.is_empty());
    }

    #[test]
    fn duplicate_match_round_trips_through_json() {
        let m = DuplicateMatch {
            duplicate_type: DuplicateType::SameChecksum,
            existing_task_id: "task-1".into(),
            existing_task_label: "report.pdf".into(),
            existing_task_status: "completed".into(),
        };
        let json = serde_json::to_string(&m).unwrap();
        let restored: DuplicateMatch = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, m);
        assert_eq!(restored.duplicate_type, DuplicateType::SameChecksum);
    }

    #[test]
    fn backoff_strategy_uses_kebab_case_serialization() {
        assert_eq!(
            serde_json::to_string(&BackoffStrategy::Fixed).unwrap(),
            "\"fixed\""
        );
        assert_eq!(
            serde_json::to_string(&BackoffStrategy::Exponential).unwrap(),
            "\"exponential\""
        );
    }

    #[test]
    fn backoff_strategy_default_is_exponential() {
        assert_eq!(BackoffStrategy::default(), BackoffStrategy::Exponential);
    }

    #[test]
    fn retry_policy_default_matches_spec_constants() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.connection_timeout_secs, 60);
        assert_eq!(policy.task_timeout_secs, None);
        assert_eq!(policy.max_retries, 5);
        assert_eq!(policy.backoff, BackoffStrategy::Exponential);
        assert_eq!(policy.initial_backoff_ms, 1000);
        assert_eq!(policy.max_backoff_ms, 60_000);
    }

    #[test]
    fn retry_policy_round_trips_through_json() {
        let policy = RetryPolicy {
            connection_timeout_secs: 30,
            task_timeout_secs: Some(600),
            max_retries: 3,
            backoff: BackoffStrategy::Fixed,
            initial_backoff_ms: 500,
            max_backoff_ms: 10_000,
        };
        let json = serde_json::to_string(&policy).unwrap();
        let restored: RetryPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, policy);
    }

    #[test]
    fn old_settings_json_defaults_retry_policy_to_spec_defaults() {
        // 旧版本 settings JSON 不会包含 default_retry_policy 字段，
        // serde(default) 应安全填充为 RetryPolicy::default()。
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("default_retry_policy");
        let restored: AppSettings = serde_json::from_value(value).unwrap();
        assert_eq!(restored.default_retry_policy, RetryPolicy::default());
    }

    #[test]
    fn old_task_json_defaults_retry_policy_override_to_none() {
        // 旧版本 task JSON 不会包含 retry_policy_override 字段，
        // serde(default) 应安全填充为 None。
        let mut value = serde_json::to_value(make_minimal_task_value()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("retry_policy_override");
        let restored: DownloadTask = serde_json::from_value(value).unwrap();
        assert!(restored.retry_policy_override.is_none());
    }

    // ===== Task 22: 主题与紧凑度细粒度控制 =====

    #[test]
    fn color_scheme_serializes_as_lowercase_kebab() {
        assert_eq!(
            serde_json::to_string(&ColorScheme::System).unwrap(),
            "\"system\""
        );
        assert_eq!(
            serde_json::to_string(&ColorScheme::Light).unwrap(),
            "\"light\""
        );
        assert_eq!(
            serde_json::to_string(&ColorScheme::Dark).unwrap(),
            "\"dark\""
        );
    }

    #[test]
    fn color_scheme_default_is_system() {
        assert_eq!(ColorScheme::default(), ColorScheme::System);
    }

    #[test]
    fn color_scheme_round_trips_through_json() {
        let scheme = ColorScheme::Dark;
        let json = serde_json::to_string(&scheme).unwrap();
        let restored: ColorScheme = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, scheme);
    }

    #[test]
    fn old_settings_json_defaults_row_compact_to_off() {
        // 旧版本 settings JSON 不会包含 row_compact 字段，
        // serde(default) 应安全填充为 false（标准行高 36px）。
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value.as_object_mut().unwrap().remove("row_compact");

        let restored: AppSettings = serde_json::from_value(value).unwrap();
        assert!(!restored.row_compact);
    }

    #[test]
    fn old_settings_json_defaults_detail_default_collapsed_to_on() {
        // 旧版本 settings JSON 不会包含 detail_default_collapsed 字段，
        // serde(default = "default_detail_default_collapsed") 应安全填充为 true。
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("detail_default_collapsed");

        let restored: AppSettings = serde_json::from_value(value).unwrap();
        assert!(restored.detail_default_collapsed);
    }

    #[test]
    fn old_settings_json_defaults_color_scheme_to_system() {
        // 旧版本 settings JSON 不会包含 color_scheme 字段，
        // serde(default) 应安全填充为 ColorScheme::System。
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value.as_object_mut().unwrap().remove("color_scheme");

        let restored: AppSettings = serde_json::from_value(value).unwrap();
        assert_eq!(restored.color_scheme, ColorScheme::System);
    }

    #[test]
    fn app_settings_default_has_task22_defaults() {
        let settings = AppSettings::default();
        assert!(!settings.row_compact);
        assert!(settings.detail_default_collapsed);
        assert_eq!(settings.color_scheme, ColorScheme::System);
    }

    // ===== Task 24: 历史归档与折叠 =====

    #[test]
    fn app_settings_default_has_task24_archive_defaults() {
        let settings = AppSettings::default();
        assert_eq!(settings.archive_days, 30);
        assert_eq!(settings.archive_threshold, 100);
    }

    #[test]
    fn old_settings_json_defaults_archive_days_to_30() {
        // 旧版本 settings JSON 不会包含 archive_days 字段，
        // serde(default = "default_archive_days") 应安全填充为 30。
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value.as_object_mut().unwrap().remove("archive_days");

        let restored: AppSettings = serde_json::from_value(value).unwrap();
        assert_eq!(restored.archive_days, 30);
    }

    #[test]
    fn old_settings_json_defaults_archive_threshold_to_100() {
        // 旧版本 settings JSON 不会包含 archive_threshold 字段，
        // serde(default = "default_archive_threshold") 应安全填充为 100。
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value.as_object_mut().unwrap().remove("archive_threshold");

        let restored: AppSettings = serde_json::from_value(value).unwrap();
        assert_eq!(restored.archive_threshold, 100);
    }

    #[test]
    fn archive_settings_round_trip_through_json() {
        let mut settings = AppSettings::default();
        settings.archive_days = 7;
        settings.archive_threshold = 50;
        let json = serde_json::to_string(&settings).unwrap();
        let restored: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.archive_days, 7);
        assert_eq!(restored.archive_threshold, 50);
    }

    // ===== Task 30: 下载完成通知与声音 =====

    #[test]
    fn app_settings_default_has_task30_notification_defaults() {
        let settings = AppSettings::default();
        assert!(settings.notify_on_complete);
        assert!(settings.notify_on_failure);
        assert!(settings.notify_sound_enabled);
        assert!(!settings.notify_failure_sound_enabled);
    }

    #[test]
    fn old_settings_json_defaults_notify_on_complete_to_true() {
        // 旧版本 settings JSON 不会包含 notify_on_complete 字段，
        // serde(default = "default_notify_on_complete") 应安全填充为 true。
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value.as_object_mut().unwrap().remove("notify_on_complete");

        let restored: AppSettings = serde_json::from_value(value).unwrap();
        assert!(restored.notify_on_complete);
    }

    #[test]
    fn old_settings_json_defaults_notify_on_failure_to_true() {
        // 旧版本 settings JSON 不会包含 notify_on_failure 字段，
        // serde(default = "default_notify_on_failure") 应安全填充为 true。
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value.as_object_mut().unwrap().remove("notify_on_failure");

        let restored: AppSettings = serde_json::from_value(value).unwrap();
        assert!(restored.notify_on_failure);
    }

    #[test]
    fn old_settings_json_defaults_notify_sound_enabled_to_true() {
        // 旧版本 settings JSON 不会包含 notify_sound_enabled 字段，
        // serde(default = "default_notify_sound_enabled") 应安全填充为 true。
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("notify_sound_enabled");

        let restored: AppSettings = serde_json::from_value(value).unwrap();
        assert!(restored.notify_sound_enabled);
    }

    #[test]
    fn old_settings_json_defaults_notify_failure_sound_enabled_to_false() {
        // 旧版本 settings JSON 不会包含 notify_failure_sound_enabled 字段，
        // serde(default = "default_notify_failure_sound_enabled") 应安全填充为 false。
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("notify_failure_sound_enabled");

        let restored: AppSettings = serde_json::from_value(value).unwrap();
        assert!(!restored.notify_failure_sound_enabled);
    }

    #[test]
    fn notification_settings_round_trip_through_json() {
        let mut settings = AppSettings::default();
        settings.notify_on_complete = false;
        settings.notify_on_failure = false;
        settings.notify_sound_enabled = false;
        settings.notify_failure_sound_enabled = true;
        let json = serde_json::to_string(&settings).unwrap();
        let restored: AppSettings = serde_json::from_str(&json).unwrap();
        assert!(!restored.notify_on_complete);
        assert!(!restored.notify_on_failure);
        assert!(!restored.notify_sound_enabled);
        assert!(restored.notify_failure_sound_enabled);
    }

    // ===== Task 25: 标签 =====

    #[test]
    fn tag_round_trips_through_json() {
        let tag = Tag {
            id: "tag-1".into(),
            name: "工作".into(),
            color: "#3B82F6".into(),
        };
        let json = serde_json::to_string(&tag).unwrap();
        let restored: Tag = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, tag);
        assert_eq!(restored.color, "#3B82F6");
    }

    #[test]
    fn tag_serialized_fields_use_stable_english_keys() {
        let tag = Tag {
            id: "tag-2".into(),
            name: "影视".into(),
            color: "#10B981".into(),
        };
        let value = serde_json::to_value(&tag).unwrap();
        let obj = value.as_object().unwrap();
        // 协议字段名锁定为英文，前端依赖这三个键读取。
        assert!(obj.contains_key("id"));
        assert!(obj.contains_key("name"));
        assert!(obj.contains_key("color"));
        assert_eq!(obj.len(), 3);
    }

    // ===== Task 18: 连接级实时状态 =====

    #[test]
    fn connection_state_uses_kebab_case_serialization() {
        assert_eq!(
            serde_json::to_string(&ConnectionState::Connecting).unwrap(),
            "\"connecting\""
        );
        assert_eq!(
            serde_json::to_string(&ConnectionState::Downloading).unwrap(),
            "\"downloading\""
        );
        assert_eq!(
            serde_json::to_string(&ConnectionState::Retrying).unwrap(),
            "\"retrying\""
        );
        assert_eq!(
            serde_json::to_string(&ConnectionState::Completed).unwrap(),
            "\"completed\""
        );
        assert_eq!(
            serde_json::to_string(&ConnectionState::Failed).unwrap(),
            "\"failed\""
        );
        assert_eq!(
            serde_json::to_string(&ConnectionState::Paused).unwrap(),
            "\"paused\""
        );
    }

    #[test]
    fn segment_status_round_trips_through_json() {
        let status = SegmentStatus {
            segment_id: "0".into(),
            start_offset: 0,
            downloaded_bytes: 1024,
            total_bytes: 4096,
            speed: 256 * 1024,
            state: ConnectionState::Downloading,
            retry_count: 0,
            error: None,
        };
        let json = serde_json::to_string(&status).unwrap();
        let restored: SegmentStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, status);
        assert_eq!(restored.state, ConnectionState::Downloading);
    }

    #[test]
    fn segment_status_with_error_round_trips_through_json() {
        let status = SegmentStatus {
            segment_id: "3".into(),
            start_offset: 12288,
            downloaded_bytes: 0,
            total_bytes: 4096,
            speed: 0,
            state: ConnectionState::Failed,
            retry_count: 5,
            error: Some("连接被重置".into()),
        };
        let json = serde_json::to_string(&status).unwrap();
        let restored: SegmentStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, status);
        assert_eq!(restored.error.as_deref(), Some("连接被重置"));
        assert_eq!(restored.retry_count, 5);
    }

    #[test]
    fn task_connections_event_round_trips_through_json() {
        let event = TaskConnectionsEvent {
            task_id: "task-1".into(),
            segments: vec![
                SegmentStatus {
                    segment_id: "0".into(),
                    start_offset: 0,
                    downloaded_bytes: 1024,
                    total_bytes: 1024,
                    speed: 0,
                    state: ConnectionState::Completed,
                    retry_count: 0,
                    error: None,
                },
                SegmentStatus {
                    segment_id: "1".into(),
                    start_offset: 1024,
                    downloaded_bytes: 512,
                    total_bytes: 1024,
                    speed: 100 * 1024,
                    state: ConnectionState::Downloading,
                    retry_count: 0,
                    error: None,
                },
            ],
            timestamp: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&event).unwrap();
        let restored: TaskConnectionsEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, event);
        assert_eq!(restored.segments.len(), 2);
        assert_eq!(restored.segments[0].state, ConnectionState::Completed);
        assert_eq!(restored.segments[1].state, ConnectionState::Downloading);
    }

    fn make_minimal_task_value() -> serde_json::Value {
        serde_json::json!({
            "id": "task-id",
            "url": "https://example.com/file",
            "file_name": "file",
            "destination": ".",
            "total_bytes": 0,
            "downloaded_bytes": 0,
            "speed": 0,
            "status": "queued",
            "created_at": 0,
            "category": "other",
            "queue_position": 0,
            "priority": 0,
            "retry_count": 0,
            "max_retries": 3,
            "source": "desktop",
            "headers": {},
            "per_task_speed_limit": 0,
            "collision_policy": "rename",
            "connection_count": 1
        })
    }

    // ===== Task 31: 代理配置精细化 =====

    #[test]
    fn old_task_json_defaults_proxy_override_and_auth_to_none() {
        // 旧版本 task JSON 不会包含 proxy_override / proxy_auth 字段，
        // serde(default) 应安全填充为 None。
        let value = make_minimal_task_value();
        let restored: DownloadTask = serde_json::from_value(value).unwrap();
        assert!(restored.proxy_override.is_none());
        assert!(restored.proxy_auth.is_none());
    }

    #[test]
    fn proxy_override_and_auth_round_trip_through_json() {
        let task = DownloadTask {
            id: "task-proxy".into(),
            url: "https://example.com/file".into(),
            file_name: "file".into(),
            destination: ".".into(),
            total_bytes: 0,
            downloaded_bytes: 0,
            speed: 0,
            eta_seconds: None,
            status: TaskStatus::Queued,
            error: None,
            created_at: 0,
            completed_at: None,
            scheduled_at: None,
            category: "other".into(),
            queue_position: 0,
            priority: 0,
            retry_count: 0,
            max_retries: 3,
            checksum_sha256: None,
            expected_checksum: None,
            source: "desktop".into(),
            etag: None,
            last_modified: None,
            headers: HashMap::new(),
            media: None,
            per_task_speed_limit: 0,
            collision_policy: CollisionPolicy::Rename,
            completion_action: CompletionAction::None,
            connection_count: 1,
            active_connections: 0,
            segments: Vec::new(),
            retry_policy_override: None,
            proxy_override: Some("http://127.0.0.1:7890".into()),
            proxy_auth: Some(ProxyAuth {
                username: "alice".into(),
                password: "secret".into(),
            }),
            final_url: None,
            response_status: None,
            content_type: None,
            accepts_ranges: None,
        };
        let json = serde_json::to_string(&task).unwrap();
        let restored: DownloadTask = serde_json::from_str(&json).unwrap();
        assert_eq!(
            restored.proxy_override.as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(restored.proxy_auth, task.proxy_auth);
    }

    #[test]
    fn proxy_auth_default_is_empty_strings() {
        let auth = ProxyAuth::default();
        assert!(auth.username.is_empty());
        assert!(auth.password.is_empty());
    }

    #[test]
    fn old_settings_json_defaults_pac_script_path_to_none() {
        // 旧版本 settings JSON 不会包含 pac_script_path 字段，
        // serde(default) 应安全填充为 None。
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        value.as_object_mut().unwrap().remove("pac_script_path");
        let restored: AppSettings = serde_json::from_value(value).unwrap();
        assert!(restored.pac_script_path.is_none());
    }

    #[test]
    fn pac_script_path_round_trips_through_json() {
        let mut settings = AppSettings::default();
        settings.pac_script_path = Some("C:\\Users\\me\\proxy.pac".into());
        let json = serde_json::to_string(&settings).unwrap();
        let restored: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(
            restored.pac_script_path.as_deref(),
            Some("C:\\Users\\me\\proxy.pac")
        );
    }

    #[test]
    fn proxy_test_result_defaults_to_safe_empty_state() {
        let restored: ProxyTestResult = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(!restored.success);
        assert!(restored.exit_ip.is_none());
        assert_eq!(restored.latency_ms, 0);
        assert!(restored.error.is_none());
    }

    #[test]
    fn proxy_test_result_round_trips_through_json() {
        let result = ProxyTestResult {
            success: true,
            exit_ip: Some("203.0.113.10".into()),
            latency_ms: 250,
            error: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        let restored: ProxyTestResult = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, result);
        assert!(restored.success);
        assert_eq!(restored.exit_ip.as_deref(), Some("203.0.113.10"));
    }

    // ===== Task 26: 更新检查与提醒 =====

    #[test]
    fn update_info_round_trips_through_json() {
        let info = UpdateInfo {
            version: "0.6.0".into(),
            release_date: "2026-07-20T10:00:00Z".into(),
            download_url: "https://github.com/maobukeai/maobu-fetch/releases/tag/v0.6.0".into(),
            sha256: Some("3a48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a27".into()),
            release_notes: "## 新增\n- 更新检查与提醒功能".into(),
        };
        let json = serde_json::to_string(&info).unwrap();
        let restored: UpdateInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, info);
        assert_eq!(
            restored.sha256.as_deref(),
            Some("3a48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a27")
        );
    }

    #[test]
    fn update_info_defaults_to_safe_empty_state() {
        // 旧 JSON/扩展请求未包含字段时，serde(default) 安全回落为空。
        let restored: UpdateInfo = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(restored.version.is_empty());
        assert!(restored.release_date.is_empty());
        assert!(restored.download_url.is_empty());
        assert!(restored.sha256.is_none());
        assert!(restored.release_notes.is_empty());
    }

    #[test]
    fn update_check_result_defaults_to_no_update_no_error() {
        let restored: UpdateCheckResult = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(restored.latest.is_none());
        assert!(!restored.has_update);
        assert!(restored.current_version.is_empty());
        assert!(restored.error.is_none());
    }

    #[test]
    fn extension_compatibility_result_defaults_to_incompatible() {
        let restored: ExtensionCompatibilityResult =
            serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(!restored.compatible);
        assert!(restored.app_version.is_empty());
        assert!(restored.extension_version.is_empty());
        assert!(restored.message.is_empty());
    }
}
