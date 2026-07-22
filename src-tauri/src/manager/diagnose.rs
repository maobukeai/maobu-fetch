//! 错误诊断系统（Task 3）。
//!
//! 把下载过程中的底层错误分类为标准错误类别，每类对应一组建议操作。
//! 所有原始错误在返回给前端前必须经过 `redact_sensitive` 脱敏，
//! 不得泄露 Cookie、Authorization、代理密码或 URL 中的 token 段。
//!
//! 分类优先级：
//! 1. HTTP 状态码（401/403 → AuthExpired, 416 → RangeInvalid, 5xx → ServerError）
//! 2. 错误字符串关键词匹配（reqwest/hyper/IO 传输层错误）
//! 3. 默认 Unknown
//!
//! 安全约束（AGENTS.md §3 / §7）：
//! - 认证信息、Cookie、Authorization、代理密码和持久令牌不得写入日志、错误历史或前端调试输出
//! - 运行时错误必须返回可操作的中文信息；底层错误可保留原因，但不得泄露密钥或完整认证头

use crate::media_platforms::{
    classify_bilibili_error, classify_douyin_error, classify_tiktok_error, classify_twitter_error,
    classify_weibo_error, classify_youtube_error, MediaPlatform,
};
use crate::models::{ErrorCategory, ErrorDiagnosis, SuggestedAction};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

// ===== Task 37 / 40 / 44：媒体平台错误分类与中文翻译 =====
//
// 设计要点（AGENTS.md §3 / §7 / §8）：
// - `MediaPlatformError` 在本模块定义并对外暴露（`media_platforms.rs` 等模块通过
//   `crate::manager::diagnose::MediaPlatformError` 访问，避免重复定义）。
// - `classify_platform_error` 仅基于 stderr 文本模式匹配，不发起新的网络请求。
// - `platform_error_to_chinese` 返回的中文文案不得包含 Cookie、Authorization
//   等敏感信息（输入 stderr 应已被 `redact_sensitive` 脱敏）。

/// 媒体平台错误类别（Task 37 / Task 40 / Task 44）。
///
/// 把 yt-dlp 在解析各平台媒体时的失败模式映射为标准类别，
/// 每个类别对应一个中文文案（由 `platform_error_to_chinese` 翻译）。
///
/// 序列化使用 kebab-case，与前端 TypeScript 联合类型对应。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum MediaPlatformError {
    /// 登录态失效（Cookie 过期或未提供）。需用户重新获取 Cookie。
    LoginExpired,
    /// 地区限制（如 TikTok 在某些地区不可用）。
    RegionBlocked,
    /// 链接已失效或内容已被删除（404 / Not Found / removed）。
    LinkExpired,
    /// 内容受 DRM 保护，不得尝试绕过（AGENTS.md §6）。
    DrmProtected,
    /// 平台暂不支持此类型内容（如未实现的图集子类型）。
    Unsupported,
    /// 未识别的错误。
    #[default]
    Unknown,
}

/// 根据 yt-dlp stderr 文本和平台特征分类错误（Task 37 / 39 / 40 / 44）。
///
/// 匹配优先级（大小写不敏感）：
/// 1. **DRM**：含 `drm`、`_has_drm` → `DrmProtected`（AGENTS.md §6 必须拒绝）
/// 2. **平台特定识别**：对 Twitter/TikTok/抖音 调用
///    [`crate::media_platforms`] 中的专用识别函数（如 Twitter 的
///    `sensitive` / `age-restricted` / `tweet not found`，TikTok 的
///    `429 + region` 组合，抖音的 `video not found`）。返回非 `Unknown`
///    时直接采用。
/// 3. **地区限制**：含 `geo restricted`、`not available in your country`、
///    `unavailable in your region` → `RegionBlocked`
/// 4. **登录失效**：含 `403`、`forbidden`、`login required`、`cookie`、`unauthorized`、
///    `401`、`log in`、`authentication required`、`age-restricted`、
///    `sign in to confirm your age`（YouTube 年龄限制）、`premium only`、
///    `vip`（B 站会员限制，Task 40）→ `LoginExpired`
/// 5. **链接失效**：含 `404`、`not found`、`removed`、`deleted`、`video unavailable`、
///    `no longer available` → `LinkExpired`
/// 6. **不支持**：含 `unsupported url`、`no video formats found`、`unsupported` → `Unsupported`
/// 7. **默认**：`Unknown`
///
/// # Task 39 / 40 平台特定模式
/// - **Twitter/X**：`tweet not found`、`status not found`、`sensitive`、
///   `age-restricted`、`login required`、`cookie(s) required`、`403`、`forbidden`
///   由 [`classify_twitter_error`] 识别（Task 39.3）
/// - **YouTube 年龄限制**：`age-restricted`、`Sign in to confirm your age` → `LoginExpired`
/// - **YouTube 机器人验证**：`Sign in to confirm you're not a bot`、
///   `cookies-from-browser`、`cookies for the authentication` → `LoginExpired`
///   （自 2024 年 YouTube 加强 PO Token 校验后常见，需用户在「设置 → 媒体凭证」
///   提供 Cookie）
/// - **B 站会员限制**：`premium only`、`VIP` → `LoginExpired`
/// - **微博链接失效**：`404`、`deleted` → `LinkExpired`（已覆盖）
///
/// 平台特定识别优先于通用关键词匹配，但返回 `Unknown` 时回退到通用匹配，
/// 确保已覆盖的通用场景（如 403/404/cookie）不受影响。`DrmProtected` 优先级最高，
/// 确保任何 DRM 信号都能被捕获（AGENTS.md §6 强约束）。
///
/// # 安全
/// - 不会泄露 Cookie/Authorization/代理密码；关键词匹配不包含敏感字段。
/// - 输入 stderr 应已由 [`redact_sensitive`] 脱敏。
pub fn classify_platform_error(platform: MediaPlatform, stderr: &str) -> MediaPlatformError {
    let lower = stderr.to_ascii_lowercase();

    // DRM 检测优先级最高（AGENTS.md §6 必须明确拒绝）
    if lower.contains("drm") || lower.contains("_has_drm") || lower.contains("protected by drm") {
        return MediaPlatformError::DrmProtected;
    }

    // 平台特定识别：优先于通用关键词匹配，返回非 Unknown 时直接采用。
    // 这样能覆盖 Twitter 的 sensitive / age-restricted、TikTok 的 429+region、
    // YouTube 的 region / age-restricted、B 站的 premium / vip、微博的 404 / deleted
    // 等平台专属模式；返回 Unknown 时回退到通用关键词匹配。
    let platform_result = match platform {
        MediaPlatform::Twitter => classify_twitter_error(&lower),
        MediaPlatform::TikTok => classify_tiktok_error(&lower),
        MediaPlatform::Douyin => classify_douyin_error(&lower),
        MediaPlatform::YouTube => classify_youtube_error(&lower),
        MediaPlatform::Bilibili => classify_bilibili_error(&lower),
        MediaPlatform::Weibo => classify_weibo_error(&lower),
        MediaPlatform::Unknown => MediaPlatformError::Unknown,
    };
    if platform_result != MediaPlatformError::Unknown {
        return platform_result;
    }

    // 通用关键词匹配（兜底）
    // 地区限制
    if lower.contains("geo restricted")
        || lower.contains("geo-restricted")
        || lower.contains("not available in your country")
        || lower.contains("region restricted")
        || lower.contains("unavailable in your region")
    {
        return MediaPlatformError::RegionBlocked;
    }
    // 登录失效（含 Task 40：YouTube 年龄限制、B 站会员限制）
    if lower.contains("403")
        || lower.contains("forbidden")
        || lower.contains("login required")
        || lower.contains("cookie")
        || lower.contains("unauthorized")
        || lower.contains("401")
        || lower.contains("log in")
        || lower.contains("authentication required")
        // Task 40.1：YouTube 年龄限制（yt-dlp 输出原文）
        || lower.contains("age-restricted")
        || lower.contains("age restricted")
        || lower.contains("sign in to confirm your age")
        // Task 40.2：B 站会员/付费内容限制
        || lower.contains("premium only")
        || lower.contains("vip")
    {
        return MediaPlatformError::LoginExpired;
    }
    // 链接失效（Task 40.3：微博 404/deleted 已覆盖）
    if lower.contains("404")
        || lower.contains("not found")
        || lower.contains("removed")
        || lower.contains("deleted")
        || lower.contains("video unavailable")
        || lower.contains("no longer available")
    {
        return MediaPlatformError::LinkExpired;
    }
    // 不支持的类型
    if lower.contains("unsupported url")
        || lower.contains("no video formats found")
        || lower.contains("unsupported")
    {
        return MediaPlatformError::Unsupported;
    }

    MediaPlatformError::Unknown
}

/// 把平台错误类别翻译为简体中文文案（Task 37.6 / Task 44.2）。
///
/// 返回的文案遵循 AGENTS.md §7"运行时错误必须返回可操作的中文信息"，
/// 不包含 Cookie、Authorization 等敏感字段（输入 stderr 应已脱敏）。
///
/// 平台特定文案：
/// - `(LoginExpired, Douyin)` → "抖音登录已失效，请重新获取 Cookie"
/// - `(LoginExpired, TikTok)` → "TikTok 登录已失效，请重新获取 Cookie"
/// - `(LoginExpired, Twitter)` → "Twitter/X 登录已失效，请重新获取 Cookie"
/// - `(RegionBlocked, TikTok)` → "该内容在你的地区不可用"
/// - `(LinkExpired, _)` → "该链接已失效或已被删除"
/// - `(DrmProtected, _)` → "该内容受 DRM 保护，无法下载"
/// - `(Unsupported, _)` → "该平台暂不支持下载此类型内容"
/// - `(Unknown, _)` → "下载失败，请稍后重试"
pub fn platform_error_to_chinese(error: MediaPlatformError, platform: MediaPlatform) -> String {
    match (error, platform) {
        (MediaPlatformError::LoginExpired, MediaPlatform::Douyin) => {
            "抖音登录已失效，请重新获取 Cookie".into()
        }
        (MediaPlatformError::LoginExpired, MediaPlatform::TikTok) => {
            "TikTok 登录已失效，请重新获取 Cookie".into()
        }
        (MediaPlatformError::LoginExpired, MediaPlatform::Twitter) => {
            "Twitter/X 登录已失效，请重新获取 Cookie".into()
        }
        (MediaPlatformError::LoginExpired, MediaPlatform::YouTube) => {
            "YouTube 反爬虫验证失败（PO Token）。这是 YouTube 强制的反机器人机制，不是软件 bug。\n\
             可尝试：1) 在「设置 → 媒体工具/高级」配置 YouTube PO Token 凭证；\n\
             2) 用无痕窗口登录 YouTube 后访问 https://www.youtube.com/robots.txt，导出 Cookie 到「设置 → 媒体凭证」（此 Cookie 不会被轮换）；\n\
             3) 稍后重试（PO Token 有时效，12 小时后可能自动恢复）；\n\
             4) 升级 yt-dlp 到最新版（「设置 → 媒体工具 → 检查更新」）。".into()
        }
        (MediaPlatformError::LoginExpired, MediaPlatform::Bilibili) => {
            "哔哩哔哩登录已失效，请重新获取 Cookie".into()
        }
        (MediaPlatformError::LoginExpired, MediaPlatform::Weibo) => {
            "微博登录已失效，请重新获取 Cookie".into()
        }
        (MediaPlatformError::LoginExpired, MediaPlatform::Unknown) => {
            "登录已失效，请重新获取认证信息".into()
        }
        (MediaPlatformError::RegionBlocked, _) => "该内容在你的地区不可用".into(),
        (MediaPlatformError::LinkExpired, _) => "该链接已失效或已被删除".into(),
        (MediaPlatformError::DrmProtected, _) => "该内容受 DRM 保护，无法下载".into(),
        (MediaPlatformError::Unsupported, _) => "该平台暂不支持下载此类型内容".into(),
        (MediaPlatformError::Unknown, _) => "下载失败，请稍后重试".into(),
    }
}

/// 错误诊断上下文，提供分类所需的任务元信息。
///
/// 由 `task_diagnose` 命令从 `DownloadTask` + `AppSettings` 构造，
/// 不直接序列化给前端。
#[derive(Clone, Debug)]
pub struct ErrorContext {
    /// 任务 URL（仅用于上下文判断，不直接输出到诊断结果，避免泄露敏感参数）。
    #[allow(dead_code)]
    pub url: String,
    /// 任务是否记录了 ETag（影响 ETagChanged 描述）。
    pub has_etag: bool,
    /// 当前是否启用了代理（影响 ProxyFailed/NetworkReset/Timeout 描述）。
    pub is_proxy_used: bool,
    /// 任务是否配置了期望校验值（影响 ChecksumFailed 描述）。
    pub has_checksum: bool,
}

impl ErrorContext {
    /// 从任务和设置标志构造诊断上下文。
    pub fn from_task_fields(
        url: String,
        etag: Option<&str>,
        is_proxy_used: bool,
        expected_checksum: Option<&str>,
    ) -> Self {
        Self {
            url,
            has_etag: etag.is_some(),
            is_proxy_used,
            has_checksum: expected_checksum.is_some(),
        }
    }
}

/// 把底层错误分类为标准错误诊断结果。
///
/// # 参数
/// - `error`: 原始错误字符串（将自动脱敏后存入 `raw_error_redacted`）
/// - `status_code`: HTTP 响应状态码（如有）
/// - `context`: 任务上下文（用于细化中文描述）
///
/// # 返回
/// 包含分类、标题、说明、建议操作和脱敏原始错误的 `ErrorDiagnosis`。
pub fn classify_error(
    error: &str,
    status_code: Option<u16>,
    context: &ErrorContext,
) -> ErrorDiagnosis {
    let category = classify_by_status_code(status_code)
        .or_else(|| classify_by_keyword(error))
        .unwrap_or(ErrorCategory::Unknown);

    let (title, description) = title_and_description(category, context);
    let suggested_actions = suggested_actions_for(category);
    let raw_error_redacted = redact_sensitive(error);

    ErrorDiagnosis {
        category,
        title,
        description,
        suggested_actions,
        raw_error_redacted,
    }
}

/// 根据 HTTP 状态码分类。
///
/// 仅匹配明确的 HTTP 错误状态码；2xx/3xx/4xx 其他状态码不在此处理，
/// 交由关键词匹配或默认 Unknown。
fn classify_by_status_code(status_code: Option<u16>) -> Option<ErrorCategory> {
    let code = status_code?;
    match code {
        401 | 403 => Some(ErrorCategory::AuthExpired),
        416 => Some(ErrorCategory::RangeInvalid),
        500..=599 => Some(ErrorCategory::ServerError),
        _ => None,
    }
}

/// 根据错误字符串关键词分类（大小写不敏感）。
///
/// 匹配 reqwest/hyper/IO 错误消息中的特征关键词。
/// 按网络→超时→TLS→磁盘→代理→校验→ETag→IO 的顺序检查，
/// 先匹配的优先返回。
fn classify_by_keyword(error: &str) -> Option<ErrorCategory> {
    let lower = error.to_ascii_lowercase();
    if lower.contains("connection reset") || lower.contains("broken pipe") {
        return Some(ErrorCategory::NetworkReset);
    }
    if lower.contains("timed out") || lower.contains("timeout") {
        return Some(ErrorCategory::Timeout);
    }
    if lower.contains("tls") || lower.contains("certificate") {
        return Some(ErrorCategory::TlsFailed);
    }
    if lower.contains("no space left") || lower.contains("disk full") {
        return Some(ErrorCategory::DiskFull);
    }
    if lower.contains("proxy") || lower.contains("connect to proxy failed") {
        return Some(ErrorCategory::ProxyFailed);
    }
    if lower.contains("checksum") || lower.contains("sha256") {
        return Some(ErrorCategory::ChecksumFailed);
    }
    if lower.contains("etag") {
        return Some(ErrorCategory::ETagChanged);
    }
    if lower.contains("remote_changed")
        || lower.contains("remote-changed")
        || lower.contains("remotechanged")
        || lower.contains("远端资源已变化")
    {
        return Some(ErrorCategory::RemoteChanged);
    }
    if lower.contains("i/o") || lower.contains("permission denied") {
        return Some(ErrorCategory::DiskIo);
    }
    None
}

/// 返回每个错误类别的标题和中文说明。
///
/// 描述会根据上下文（是否使用代理、是否有 ETag/校验值）细化，
/// 帮助用户理解错误原因和下一步操作。
fn title_and_description(category: ErrorCategory, context: &ErrorContext) -> (String, String) {
    match category {
        ErrorCategory::AuthExpired => (
            "链接或登录信息已过期".into(),
            "服务器返回 401/403，可能是下载链接已过期或需要登录认证。请重新从浏览器获取链接后再试。".into(),
        ),
        ErrorCategory::RangeInvalid => (
            "分片范围无效".into(),
            "服务器返回 416 Range Not Satisfiable，远端文件可能已变化或分片已失效。建议清除旧分片后重新下载。".into(),
        ),
        ErrorCategory::DiskFull => (
            "磁盘空间不足".into(),
            "目标磁盘没有足够空间完成下载。请更换保存目录或清理磁盘后重试。".into(),
        ),
        ErrorCategory::ProxyFailed => (
            "代理连接失败".into(),
            if context.is_proxy_used {
                "无法通过代理连接到服务器。请检查代理设置（地址、端口、认证）或暂时禁用代理后重试。".into()
            } else {
                "代理连接失败。请检查系统代理设置或暂时禁用代理后重试。".into()
            },
        ),
        ErrorCategory::ETagChanged => (
            "远端资源已变化".into(),
            if context.has_etag {
                "ETag 校验不一致，远端文件已被更新。为避免拼接损坏的数据，建议保留旧文件并重新下载。".into()
            } else {
                "检测到 ETag 变化，远端文件已被更新。建议保留旧文件并重新下载。".into()
            },
        ),
        ErrorCategory::ChecksumFailed => (
            "文件校验失败".into(),
            if context.has_checksum {
                "下载文件的 SHA-256 校验值与预期不一致，文件可能已损坏。建议重新下载损坏分片或重新校验整个文件。".into()
            } else {
                "文件校验失败。建议重新下载损坏分片或重新校验整个文件。".into()
            },
        ),
        ErrorCategory::NetworkReset => (
            "网络连接被重置".into(),
            if context.is_proxy_used {
                "下载过程中连接被重置或断开。当前正在使用代理，请检查代理稳定性后重试。".into()
            } else {
                "下载过程中连接被重置或断开。请检查网络连接后重试。".into()
            },
        ),
        ErrorCategory::TlsFailed => (
            "TLS 证书校验失败".into(),
            "无法建立安全的 TLS 连接，可能是证书问题或代理拦截。请检查代理设置或确认服务器证书有效。".into(),
        ),
        ErrorCategory::ServerError => (
            "服务器错误".into(),
            "服务器返回 5xx 错误，可能是服务器临时故障或维护中。建议稍后重试。".into(),
        ),
        ErrorCategory::Timeout => (
            "请求超时".into(),
            if context.is_proxy_used {
                "下载连接超时。当前正在使用代理，请检查代理响应速度或稍后重试。".into()
            } else {
                "下载连接超时，可能是网络不稳定或服务器响应慢。请稍后重试。".into()
            },
        ),
        ErrorCategory::DiskIo => (
            "磁盘读写错误".into(),
            "无法读写目标磁盘，可能是权限不足或磁盘故障。请检查目录权限或更换保存目录。".into(),
        ),
        ErrorCategory::RemoteChanged => (
            "远端资源已变化".into(),
            "Last-Modified 或 ETag 不一致，远端文件已被更新。为避免拼接旧分片，建议保留旧文件并重新下载。".into(),
        ),
        ErrorCategory::Unknown => (
            "未知错误".into(),
            "发生未识别的错误。建议重试；如问题持续，请检查网络连接、磁盘空间和代理设置。".into(),
        ),
    }
}

/// 返回每个错误类别对应的建议操作列表。
///
/// `action_id` 是稳定英文标识，前端依据它调用对应 Tauri 命令或 UI 流程：
/// - `refetch_url`: 提示用户重新从浏览器粘贴 URL
/// - `clear_shards`: 清除旧分片文件（调用 task_action "redownload"）
/// - `change_dir`: 提示用户更换保存目录
/// - `disable_proxy`: 跳转到代理设置页
/// - `reverify`: 调用 task_verify 重新校验
/// - `retry`: 调用 task_action "retry"
/// - `redownload`: 保留旧文件并重新下载（调用 task_action "redownload"）
fn suggested_actions_for(category: ErrorCategory) -> Vec<SuggestedAction> {
    match category {
        ErrorCategory::AuthExpired => vec![
            SuggestedAction {
                action_id: "refetch_url".into(),
                label: "重新从浏览器获取".into(),
            },
            SuggestedAction {
                action_id: "retry".into(),
                label: "重试".into(),
            },
        ],
        ErrorCategory::RangeInvalid => vec![
            SuggestedAction {
                action_id: "clear_shards".into(),
                label: "清除旧分片并重新下载".into(),
            },
            SuggestedAction {
                action_id: "retry".into(),
                label: "重试".into(),
            },
        ],
        ErrorCategory::DiskFull => vec![SuggestedAction {
            action_id: "change_dir".into(),
            label: "更换保存目录".into(),
        }],
        ErrorCategory::ProxyFailed => vec![
            SuggestedAction {
                action_id: "disable_proxy".into(),
                label: "检查代理设置".into(),
            },
            SuggestedAction {
                action_id: "retry".into(),
                label: "重试".into(),
            },
        ],
        ErrorCategory::ETagChanged | ErrorCategory::RemoteChanged => vec![SuggestedAction {
            action_id: "redownload".into(),
            label: "保留旧文件并重新开始".into(),
        }],
        ErrorCategory::ChecksumFailed => vec![
            SuggestedAction {
                action_id: "clear_shards".into(),
                label: "重新下载损坏分片".into(),
            },
            SuggestedAction {
                action_id: "reverify".into(),
                label: "重新校验整个文件".into(),
            },
        ],
        ErrorCategory::NetworkReset | ErrorCategory::Timeout => vec![SuggestedAction {
            action_id: "retry".into(),
            label: "重试".into(),
        }],
        ErrorCategory::TlsFailed => vec![
            SuggestedAction {
                action_id: "disable_proxy".into(),
                label: "检查代理设置".into(),
            },
            SuggestedAction {
                action_id: "retry".into(),
                label: "重试".into(),
            },
        ],
        ErrorCategory::ServerError => vec![SuggestedAction {
            action_id: "retry".into(),
            label: "稍后重试".into(),
        }],
        ErrorCategory::DiskIo => vec![SuggestedAction {
            action_id: "change_dir".into(),
            label: "更换保存目录".into(),
        }],
        ErrorCategory::Unknown => vec![SuggestedAction {
            action_id: "retry".into(),
            label: "重试".into(),
        }],
    }
}

/// 对原始错误文本进行脱敏，把敏感字段值替换为 `***`。
///
/// 覆盖以下模式（大小写不敏感）：
/// - HTTP 头：`Cookie: <value>`、`Authorization: <value>`、`proxy-password: <value>`、`Set-Cookie: <value>`
/// - 键值对：`password=<value>`、`pass=<value>`、`pwd=<value>`、`token=<value>`、`access_token=<value>`、`proxy_password=<value>`
/// - URL 查询参数：`?token=xxx`、`?sign=xxx`、`?auth=xxx`、`?signature=xxx`
///
/// 替换后保留字段名，仅隐藏值，便于用户识别错误类型而不泄露密钥。
pub fn redact_sensitive(text: &str) -> String {
    let header_re = header_regex();
    let kv_re = kv_param_regex();
    let result = header_re.replace_all(text, "${1}: ***");
    let result = kv_re.replace_all(&result, "${1}=***");
    result.into_owned()
}

/// 编译并缓存 HTTP 头脱敏正则。
///
/// 匹配 `Cookie:`、`Set-Cookie:`、`Authorization:`、`proxy-password:` 后的值（至行尾）。
/// 正则模式为编译期常量，编译失败属于编程错误（不可恢复），因此使用 `expect`。
fn header_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b((?:set-)?cookie|authorization|proxy-password)\s*:\s*[^\r\n]*")
            .expect("header redaction regex is statically validated")
    })
}

/// 编译并缓存键值对脱敏正则。
///
/// 匹配 `password=`、`pass=`、`pwd=`、`token=`、`access_token=`、`proxy_password=`、
/// `sign=`、`auth=`、`signature=` 后的值（直到分隔符或空白）。
/// 正则模式为编译期常量，编译失败属于编程错误（不可恢复），因此使用 `expect`。
fn kv_param_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(password|access_token|proxy_password|pwd|pass|token|sign|auth|signature)=([^&;\s,#]*)")
            .expect("key-value redaction regex is statically validated")
    })
}

// ===== 单元测试 =====
#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ErrorContext {
        ErrorContext {
            url: "https://example.com/file.zip".into(),
            has_etag: true,
            is_proxy_used: false,
            has_checksum: true,
        }
    }

    fn ctx_no_proxy() -> ErrorContext {
        ErrorContext {
            url: "https://example.com/file.zip".into(),
            has_etag: false,
            is_proxy_used: false,
            has_checksum: false,
        }
    }

    fn ctx_proxy() -> ErrorContext {
        ErrorContext {
            url: "https://example.com/file.zip".into(),
            has_etag: false,
            is_proxy_used: true,
            has_checksum: false,
        }
    }

    // ---- 分类：HTTP 状态码 ----

    #[test]
    fn diagnose_status_401_maps_to_auth_expired() {
        let d = classify_error("Unauthorized", Some(401), &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::AuthExpired);
    }

    #[test]
    fn diagnose_status_403_maps_to_auth_expired() {
        let d = classify_error("Forbidden", Some(403), &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::AuthExpired);
    }

    #[test]
    fn diagnose_status_416_maps_to_range_invalid() {
        let d = classify_error("Range Not Satisfiable", Some(416), &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::RangeInvalid);
    }

    #[test]
    fn diagnose_status_500_maps_to_server_error() {
        let d = classify_error("Internal Server Error", Some(500), &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::ServerError);
    }

    #[test]
    fn diagnose_status_503_maps_to_server_error() {
        let d = classify_error("Service Unavailable", Some(503), &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::ServerError);
    }

    #[test]
    fn diagnose_status_200_does_not_classify_by_status() {
        // 200 不是错误状态码，应回退到关键词或 Unknown
        let d = classify_error("some generic error", Some(200), &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::Unknown);
    }

    // ---- 分类：关键词匹配 ----

    #[test]
    fn diagnose_connection_reset_maps_to_network_reset() {
        let d = classify_error(
            "error sending request: connection reset by peer",
            None,
            &ctx_no_proxy(),
        );
        assert_eq!(d.category, ErrorCategory::NetworkReset);
    }

    #[test]
    fn diagnose_broken_pipe_maps_to_network_reset() {
        let d = classify_error("write error: broken pipe", None, &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::NetworkReset);
    }

    #[test]
    fn diagnose_timed_out_maps_to_timeout() {
        let d = classify_error("request timed out", None, &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::Timeout);
    }

    #[test]
    fn diagnose_timeout_keyword_maps_to_timeout() {
        let d = classify_error("operation timeout occurred", None, &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::Timeout);
    }

    #[test]
    fn diagnose_tls_maps_to_tls_failed() {
        let d = classify_error("tls handshake failed", None, &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::TlsFailed);
    }

    #[test]
    fn diagnose_certificate_maps_to_tls_failed() {
        let d = classify_error("invalid certificate", None, &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::TlsFailed);
    }

    #[test]
    fn diagnose_no_space_left_maps_to_disk_full() {
        let d = classify_error("No space left on device", None, &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::DiskFull);
    }

    #[test]
    fn diagnose_disk_full_maps_to_disk_full() {
        let d = classify_error("disk full error", None, &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::DiskFull);
    }

    #[test]
    fn diagnose_proxy_maps_to_proxy_failed() {
        let d = classify_error("connect to proxy failed", None, &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::ProxyFailed);
    }

    #[test]
    fn diagnose_checksum_maps_to_checksum_failed() {
        let d = classify_error("checksum mismatch", None, &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::ChecksumFailed);
    }

    #[test]
    fn diagnose_sha256_maps_to_checksum_failed() {
        let d = classify_error("sha256 verification failed", None, &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::ChecksumFailed);
    }

    #[test]
    fn diagnose_etag_maps_to_etag_changed() {
        let d = classify_error("etag mismatch", None, &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::ETagChanged);
    }

    #[test]
    fn diagnose_remote_changed_maps_to_remote_changed() {
        let d = classify_error(
            "REMOTE_CHANGED:远端资源已变化，是否重新下载？",
            None,
            &ctx_no_proxy(),
        );
        assert_eq!(d.category, ErrorCategory::RemoteChanged);
    }

    #[test]
    fn diagnose_io_error_maps_to_disk_io() {
        let d = classify_error("i/o error: permission required", None, &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::DiskIo);
    }

    #[test]
    fn diagnose_permission_denied_maps_to_disk_io() {
        let d = classify_error("permission denied", None, &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::DiskIo);
    }

    #[test]
    fn diagnose_unknown_error_maps_to_unknown() {
        let d = classify_error("something went wrong", None, &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::Unknown);
    }

    // ---- 分类：优先级 ----

    #[test]
    fn diagnose_status_code_takes_precedence_over_keyword() {
        // 403 + "proxy" 关键词 → AuthExpired（状态码优先）
        let d = classify_error("proxy connection failed", Some(403), &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::AuthExpired);
    }

    #[test]
    fn diagnose_416_takes_precedence_over_etag_keyword() {
        // 416 + "etag" 关键词 → RangeInvalid（状态码优先）
        let d = classify_error("etag changed", Some(416), &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::RangeInvalid);
    }

    // ---- 建议操作 ----

    #[test]
    fn suggested_actions_for_auth_expired() {
        let actions = suggested_actions_for(ErrorCategory::AuthExpired);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action_id, "refetch_url");
        assert_eq!(actions[0].label, "重新从浏览器获取");
        assert_eq!(actions[1].action_id, "retry");
        assert_eq!(actions[1].label, "重试");
    }

    #[test]
    fn suggested_actions_for_range_invalid() {
        let actions = suggested_actions_for(ErrorCategory::RangeInvalid);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action_id, "clear_shards");
        assert_eq!(actions[1].action_id, "retry");
    }

    #[test]
    fn suggested_actions_for_disk_full() {
        let actions = suggested_actions_for(ErrorCategory::DiskFull);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_id, "change_dir");
    }

    #[test]
    fn suggested_actions_for_proxy_failed() {
        let actions = suggested_actions_for(ErrorCategory::ProxyFailed);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action_id, "disable_proxy");
        assert_eq!(actions[1].action_id, "retry");
    }

    #[test]
    fn suggested_actions_for_etag_changed() {
        let actions = suggested_actions_for(ErrorCategory::ETagChanged);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_id, "redownload");
    }

    #[test]
    fn suggested_actions_for_remote_changed() {
        let actions = suggested_actions_for(ErrorCategory::RemoteChanged);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_id, "redownload");
    }

    #[test]
    fn suggested_actions_for_checksum_failed() {
        let actions = suggested_actions_for(ErrorCategory::ChecksumFailed);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action_id, "clear_shards");
        assert_eq!(actions[1].action_id, "reverify");
    }

    #[test]
    fn suggested_actions_for_network_reset() {
        let actions = suggested_actions_for(ErrorCategory::NetworkReset);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_id, "retry");
    }

    #[test]
    fn suggested_actions_for_timeout() {
        let actions = suggested_actions_for(ErrorCategory::Timeout);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_id, "retry");
    }

    #[test]
    fn suggested_actions_for_tls_failed() {
        let actions = suggested_actions_for(ErrorCategory::TlsFailed);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action_id, "disable_proxy");
        assert_eq!(actions[1].action_id, "retry");
    }

    #[test]
    fn suggested_actions_for_server_error() {
        let actions = suggested_actions_for(ErrorCategory::ServerError);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_id, "retry");
        assert_eq!(actions[0].label, "稍后重试");
    }

    #[test]
    fn suggested_actions_for_disk_io() {
        let actions = suggested_actions_for(ErrorCategory::DiskIo);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_id, "change_dir");
    }

    #[test]
    fn suggested_actions_for_unknown() {
        let actions = suggested_actions_for(ErrorCategory::Unknown);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_id, "retry");
    }

    // ---- 标题与描述 ----

    #[test]
    fn title_and_description_are_chinese() {
        let (title, desc) = title_and_description(ErrorCategory::AuthExpired, &ctx());
        assert!(title.contains("过期"));
        assert!(desc.contains("401/403"));
    }

    #[test]
    fn description_mentions_proxy_when_used() {
        let (_, desc) = title_and_description(ErrorCategory::NetworkReset, &ctx_proxy());
        assert!(desc.contains("代理"));
    }

    #[test]
    fn description_does_not_mention_proxy_when_not_used() {
        let (_, desc) = title_and_description(ErrorCategory::NetworkReset, &ctx_no_proxy());
        assert!(!desc.contains("代理"));
    }

    // ---- 脱敏：HTTP 头 ----

    #[test]
    fn redact_cookie_header() {
        let input = "Cookie: session=abc123; user=alice";
        let redacted = redact_sensitive(input);
        assert!(redacted.contains("Cookie: ***"));
        assert!(!redacted.contains("abc123"));
        assert!(!redacted.contains("alice"));
    }

    #[test]
    fn redact_cookie_header_lowercase() {
        let input = "cookie: session=secret";
        let redacted = redact_sensitive(input);
        assert!(redacted.contains("cookie: ***"));
        assert!(!redacted.contains("secret"));
    }

    #[test]
    fn redact_authorization_header() {
        let input = "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9";
        let redacted = redact_sensitive(input);
        assert!(redacted.contains("Authorization: ***"));
        assert!(!redacted.contains("eyJhbGciOiJIUzI1NiJ9"));
    }

    #[test]
    fn redact_authorization_header_lowercase() {
        let input = "authorization: Basic dXNlcjpwYXNz";
        let redacted = redact_sensitive(input);
        assert!(redacted.contains("authorization: ***"));
        assert!(!redacted.contains("dXNlcjpwYXNz"));
    }

    #[test]
    fn redact_proxy_password_header() {
        let input = "proxy-password: s3cret";
        let redacted = redact_sensitive(input);
        assert!(redacted.contains("proxy-password: ***"));
        assert!(!redacted.contains("s3cret"));
    }

    #[test]
    fn redact_set_cookie_header() {
        let input = "Set-Cookie: session=abc; Path=/";
        let redacted = redact_sensitive(input);
        assert!(redacted.contains("Set-Cookie: ***"));
        assert!(!redacted.contains("session=abc"));
    }

    // ---- 脱敏：键值对 ----

    #[test]
    fn redact_password_kv() {
        let input = "password=mypass123";
        let redacted = redact_sensitive(input);
        assert_eq!(redacted, "password=***");
    }

    #[test]
    fn redact_pass_kv() {
        let input = "pass=secret";
        let redacted = redact_sensitive(input);
        assert_eq!(redacted, "pass=***");
    }

    #[test]
    fn redact_pwd_kv() {
        let input = "pwd=abc";
        let redacted = redact_sensitive(input);
        assert_eq!(redacted, "pwd=***");
    }

    #[test]
    fn redact_token_kv() {
        let input = "token=eyJtoken";
        let redacted = redact_sensitive(input);
        assert_eq!(redacted, "token=***");
    }

    #[test]
    fn redact_access_token_kv() {
        let input = "access_token=eyJaccess";
        let redacted = redact_sensitive(input);
        assert_eq!(redacted, "access_token=***");
    }

    #[test]
    fn redact_proxy_password_kv() {
        let input = "proxy_password=proxy_pass";
        let redacted = redact_sensitive(input);
        assert_eq!(redacted, "proxy_password=***");
    }

    #[test]
    fn redact_password_in_form_encoded_data() {
        let input = "user=alice&password=mypass&token=secret";
        let redacted = redact_sensitive(input);
        assert!(redacted.contains("user=alice"));
        assert!(redacted.contains("password=***"));
        assert!(redacted.contains("token=***"));
        assert!(!redacted.contains("mypass"));
        assert!(!redacted.contains("secret"));
    }

    // ---- 脱敏：URL 查询参数 ----

    #[test]
    fn redact_url_token_param() {
        let input = "https://example.com/file?token=secret123";
        let redacted = redact_sensitive(input);
        assert!(redacted.contains("token=***"));
        assert!(!redacted.contains("secret123"));
    }

    #[test]
    fn redact_url_sign_param() {
        let input = "https://example.com/file?sign=abc456";
        let redacted = redact_sensitive(input);
        assert!(redacted.contains("sign=***"));
        assert!(!redacted.contains("abc456"));
    }

    #[test]
    fn redact_url_auth_param() {
        let input = "https://example.com/file?auth=authval";
        let redacted = redact_sensitive(input);
        assert!(redacted.contains("auth=***"));
        assert!(!redacted.contains("authval"));
    }

    #[test]
    fn redact_url_signature_param() {
        let input = "https://example.com/file?signature=sigval";
        let redacted = redact_sensitive(input);
        assert!(redacted.contains("signature=***"));
        assert!(!redacted.contains("sigval"));
    }

    #[test]
    fn redact_url_multiple_params() {
        let input = "https://example.com/file?token=secret&other=val&sign=abc";
        let redacted = redact_sensitive(input);
        assert!(redacted.contains("token=***"));
        assert!(redacted.contains("other=val"));
        assert!(redacted.contains("sign=***"));
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("abc"));
    }

    // ---- 脱敏：综合与不泄露 ----

    #[test]
    fn redact_preserves_non_sensitive_content() {
        let input = "connection reset by peer for https://example.com/file.zip";
        let redacted = redact_sensitive(input);
        assert!(redacted.contains("connection reset"));
        assert!(redacted.contains("example.com"));
    }

    #[test]
    fn redact_empty_string_returns_empty() {
        assert_eq!(redact_sensitive(""), "");
    }

    #[test]
    fn redact_no_sensitive_data_returns_unchanged() {
        let input = "connection reset by peer";
        assert_eq!(redact_sensitive(input), input);
    }

    #[test]
    fn redact_does_not_leak_any_sensitive_value() {
        let sensitive_values = [
            "secret123",
            "mypass",
            "eyJtoken",
            "abc456",
            "authval",
            "sigval",
        ];
        let input = "Cookie: secret123\nAuthorization: mypass\ntoken=eyJtoken\n?sign=abc456&auth=authval&signature=sigval";
        let redacted = redact_sensitive(input);
        for value in &sensitive_values {
            assert!(
                !redacted.contains(value),
                "脱敏后仍包含敏感值 '{value}': {redacted}"
            );
        }
    }

    #[test]
    fn redact_mixed_case_headers() {
        let input = "COOKIE: session=abc\nAuthorization: Bearer xyz";
        let redacted = redact_sensitive(input);
        assert!(redacted.contains("COOKIE: ***"));
        assert!(redacted.contains("Authorization: ***"));
        assert!(!redacted.contains("abc"));
        assert!(!redacted.contains("xyz"));
    }

    // ---- 端到端：classify_error 返回脱敏的 raw_error ----

    #[test]
    fn classify_error_redacts_raw_error() {
        let input = "Cookie: session=secret\nconnection reset by peer";
        let d = classify_error(input, None, &ctx_no_proxy());
        assert_eq!(d.category, ErrorCategory::NetworkReset);
        assert!(d.raw_error_redacted.contains("Cookie: ***"));
        assert!(d.raw_error_redacted.contains("connection reset"));
        assert!(!d.raw_error_redacted.contains("secret"));
    }

    #[test]
    fn classify_error_returns_non_empty_title_and_description() {
        let d = classify_error("some error", None, &ctx_no_proxy());
        assert!(!d.title.is_empty());
        assert!(!d.description.is_empty());
    }

    #[test]
    fn classify_error_returns_at_least_one_suggested_action() {
        let d = classify_error("some error", None, &ctx_no_proxy());
        assert!(!d.suggested_actions.is_empty());
    }

    #[test]
    fn classify_error_context_proxy_reflects_in_description() {
        let d = classify_error("connection reset", None, &ctx_proxy());
        assert!(d.description.contains("代理"));
    }

    // ===== Task 37 / 44：MediaPlatformError 分类与中文翻译测试 =====

    #[test]
    fn classify_douyin_login_expired_with_403() {
        let stderr = "HTTP Error 403: Forbidden";
        let error = classify_platform_error(MediaPlatform::Douyin, stderr);
        assert_eq!(error, MediaPlatformError::LoginExpired);
    }

    #[test]
    fn classify_douyin_login_expired_with_forbidden() {
        let stderr = "ERROR: [ Douyin ] Forbidden: login required";
        let error = classify_platform_error(MediaPlatform::Douyin, stderr);
        assert_eq!(error, MediaPlatformError::LoginExpired);
    }

    #[test]
    fn classify_douyin_login_expired_with_cookie_keyword() {
        let stderr = "ERROR: Cookie expired, please refresh";
        let error = classify_platform_error(MediaPlatform::Douyin, stderr);
        assert_eq!(error, MediaPlatformError::LoginExpired);
    }

    #[test]
    fn classify_douyin_login_expired_with_401() {
        let stderr = "HTTP Error 401: Unauthorized";
        let error = classify_platform_error(MediaPlatform::Douyin, stderr);
        assert_eq!(error, MediaPlatformError::LoginExpired);
    }

    #[test]
    fn classify_douyin_link_expired_with_404() {
        let stderr = "HTTP Error 404: Not Found";
        let error = classify_platform_error(MediaPlatform::Douyin, stderr);
        assert_eq!(error, MediaPlatformError::LinkExpired);
    }

    #[test]
    fn classify_douyin_link_expired_with_not_found_keyword() {
        let stderr = "ERROR: video not found";
        let error = classify_platform_error(MediaPlatform::Douyin, stderr);
        assert_eq!(error, MediaPlatformError::LinkExpired);
    }

    #[test]
    fn classify_douyin_link_expired_with_removed_keyword() {
        let stderr = "ERROR: video has been removed by the author";
        let error = classify_platform_error(MediaPlatform::Douyin, stderr);
        assert_eq!(error, MediaPlatformError::LinkExpired);
    }

    #[test]
    fn classify_douyin_link_expired_with_deleted_keyword() {
        let stderr = "ERROR: this post has been deleted";
        let error = classify_platform_error(MediaPlatform::Douyin, stderr);
        assert_eq!(error, MediaPlatformError::LinkExpired);
    }

    #[test]
    fn classify_drm_protected_takes_priority() {
        // DRM 检测应优先于登录失效（AGENTS.md §6 必须明确拒绝）
        let stderr = "HTTP 403 Forbidden: content is DRM protected";
        let error = classify_platform_error(MediaPlatform::Douyin, stderr);
        assert_eq!(error, MediaPlatformError::DrmProtected);
    }

    #[test]
    fn classify_drm_protected_with_has_drm_flag() {
        let stderr = "WARNING: _has_drm is set to true";
        let error = classify_platform_error(MediaPlatform::YouTube, stderr);
        assert_eq!(error, MediaPlatformError::DrmProtected);
    }

    #[test]
    fn classify_region_blocked_tiktok() {
        let stderr = "ERROR: This content is geo restricted in your region";
        let error = classify_platform_error(MediaPlatform::TikTok, stderr);
        assert_eq!(error, MediaPlatformError::RegionBlocked);
    }

    #[test]
    fn classify_region_blocked_with_country_message() {
        let stderr = "ERROR: not available in your country";
        let error = classify_platform_error(MediaPlatform::TikTok, stderr);
        assert_eq!(error, MediaPlatformError::RegionBlocked);
    }

    #[test]
    fn classify_unsupported_url() {
        let stderr = "ERROR: Unsupported URL: https://example.com/unknown";
        let error = classify_platform_error(MediaPlatform::Douyin, stderr);
        assert_eq!(error, MediaPlatformError::Unsupported);
    }

    #[test]
    fn classify_unsupported_no_video_formats() {
        let stderr = "ERROR: no video formats found";
        let error = classify_platform_error(MediaPlatform::Douyin, stderr);
        assert_eq!(error, MediaPlatformError::Unsupported);
    }

    #[test]
    fn classify_unknown_error_for_unrecognized_pattern() {
        let stderr = "ERROR: some unknown internal error";
        let error = classify_platform_error(MediaPlatform::Douyin, stderr);
        assert_eq!(error, MediaPlatformError::Unknown);
    }

    #[test]
    fn classify_empty_stderr_returns_unknown() {
        let error = classify_platform_error(MediaPlatform::Douyin, "");
        assert_eq!(error, MediaPlatformError::Unknown);
    }

    #[test]
    fn classify_is_case_insensitive() {
        // 大小写不敏感匹配
        let error = classify_platform_error(MediaPlatform::Douyin, "FORBIDDEN BY 403");
        assert_eq!(error, MediaPlatformError::LoginExpired);
    }

    // ===== Task 39：Twitter/X 平台 classify_platform_error 集成测试 =====
    //
    // 这些测试验证 `classify_platform_error` 调度器对 Twitter 平台的正确分发：
    // - 平台特定关键词（如 `sensitive`、`tweet not found`）由 `classify_twitter_error` 识别
    // - 通用关键词（如 `403`、`cookie`）由调度器兜底匹配
    // - DRM 优先级最高，任何平台都先检查

    #[test]
    fn classify_twitter_login_required() {
        let stderr = "ERROR: [twitter] 123: Login required to access this resource";
        let error = classify_platform_error(MediaPlatform::Twitter, stderr);
        assert_eq!(error, MediaPlatformError::LoginExpired);
    }

    #[test]
    fn classify_twitter_cookie_required() {
        let stderr = "ERROR: [twitter] Cookie required. Please provide cookies.";
        let error = classify_platform_error(MediaPlatform::Twitter, stderr);
        assert_eq!(error, MediaPlatformError::LoginExpired);
    }

    #[test]
    fn classify_twitter_tweet_not_found() {
        let stderr = "ERROR: [twitter] Tweet not found";
        let error = classify_platform_error(MediaPlatform::Twitter, stderr);
        assert_eq!(error, MediaPlatformError::LinkExpired);
    }

    #[test]
    fn classify_twitter_status_not_found() {
        let stderr = "ERROR: [twitter] Status not found";
        let error = classify_platform_error(MediaPlatform::Twitter, stderr);
        assert_eq!(error, MediaPlatformError::LinkExpired);
    }

    #[test]
    fn classify_twitter_404() {
        let stderr = "HTTP Error 404: Not Found";
        let error = classify_platform_error(MediaPlatform::Twitter, stderr);
        assert_eq!(error, MediaPlatformError::LinkExpired);
    }

    #[test]
    fn classify_twitter_sensitive_content() {
        // `sensitive` 关键词仅由 classify_twitter_error 识别（通用匹配不包含此关键词）
        let stderr = "ERROR: [twitter] Sensitive content";
        let error = classify_platform_error(MediaPlatform::Twitter, stderr);
        assert_eq!(error, MediaPlatformError::LoginExpired);
    }

    #[test]
    fn classify_twitter_age_restricted() {
        let stderr = "ERROR: [twitter] Age-restricted content";
        let error = classify_platform_error(MediaPlatform::Twitter, stderr);
        assert_eq!(error, MediaPlatformError::LoginExpired);
    }

    #[test]
    fn classify_twitter_403_forbidden() {
        let stderr = "HTTP Error 403: Forbidden";
        let error = classify_platform_error(MediaPlatform::Twitter, stderr);
        assert_eq!(error, MediaPlatformError::LoginExpired);
    }

    #[test]
    fn classify_twitter_drm_takes_priority_over_login() {
        // DRM 检测优先于平台特定识别（AGENTS.md §6 必须明确拒绝）
        let stderr = "ERROR: [twitter] DRM protected, login required";
        let error = classify_platform_error(MediaPlatform::Twitter, stderr);
        assert_eq!(error, MediaPlatformError::DrmProtected);
    }

    #[test]
    fn classify_twitter_unknown_error() {
        let stderr = "ERROR: [twitter] something unexpected happened";
        let error = classify_platform_error(MediaPlatform::Twitter, stderr);
        assert_eq!(error, MediaPlatformError::Unknown);
    }

    #[test]
    fn classify_twitter_case_insensitive() {
        assert_eq!(
            classify_platform_error(MediaPlatform::Twitter, "LOGIN REQUIRED"),
            MediaPlatformError::LoginExpired
        );
        assert_eq!(
            classify_platform_error(MediaPlatform::Twitter, "Tweet NOT FOUND"),
            MediaPlatformError::LinkExpired
        );
        assert_eq!(
            classify_platform_error(MediaPlatform::Twitter, "SENSITIVE CONTENT"),
            MediaPlatformError::LoginExpired
        );
    }

    #[test]
    fn classify_twitter_link_expired_priority_over_login() {
        // 同时含 "tweet not found" 和 "login required"：链接失效优先
        let stderr = "Tweet not found, login required to view";
        let error = classify_platform_error(MediaPlatform::Twitter, stderr);
        assert_eq!(error, MediaPlatformError::LinkExpired);
    }

    // ---- platform_error_to_chinese：平台特定文案 ----

    #[test]
    fn platform_error_to_chinese_douyin_login() {
        let msg =
            platform_error_to_chinese(MediaPlatformError::LoginExpired, MediaPlatform::Douyin);
        assert_eq!(msg, "抖音登录已失效，请重新获取 Cookie");
    }

    #[test]
    fn platform_error_to_chinese_tiktok_login() {
        let msg =
            platform_error_to_chinese(MediaPlatformError::LoginExpired, MediaPlatform::TikTok);
        assert_eq!(msg, "TikTok 登录已失效，请重新获取 Cookie");
    }

    #[test]
    fn platform_error_to_chinese_twitter_login() {
        let msg =
            platform_error_to_chinese(MediaPlatformError::LoginExpired, MediaPlatform::Twitter);
        assert_eq!(msg, "Twitter/X 登录已失效，请重新获取 Cookie");
    }

    #[test]
    fn platform_error_to_chinese_youtube_login() {
        let msg =
            platform_error_to_chinese(MediaPlatformError::LoginExpired, MediaPlatform::YouTube);
        // 文案应明确告诉用户去「设置 → 媒体凭证」提供 YouTube Cookie，
        // 而不是模糊的"请重新登录 Google 账号"（机器人验证场景用户可能从未登录过）
        assert!(msg.contains("媒体凭证"));
        assert!(msg.contains("Cookie"));
        assert!(msg.contains("YouTube"));
    }

    #[test]
    fn platform_error_to_chinese_bilibili_login() {
        let msg =
            platform_error_to_chinese(MediaPlatformError::LoginExpired, MediaPlatform::Bilibili);
        assert_eq!(msg, "哔哩哔哩登录已失效，请重新获取 Cookie");
    }

    #[test]
    fn platform_error_to_chinese_weibo_login() {
        let msg = platform_error_to_chinese(MediaPlatformError::LoginExpired, MediaPlatform::Weibo);
        assert_eq!(msg, "微博登录已失效，请重新获取 Cookie");
    }

    #[test]
    fn platform_error_to_chinese_unknown_platform_login() {
        let msg =
            platform_error_to_chinese(MediaPlatformError::LoginExpired, MediaPlatform::Unknown);
        assert_eq!(msg, "登录已失效，请重新获取认证信息");
    }

    #[test]
    fn platform_error_to_chinese_tiktok_region_blocked() {
        let msg =
            platform_error_to_chinese(MediaPlatformError::RegionBlocked, MediaPlatform::TikTok);
        assert_eq!(msg, "该内容在你的地区不可用");
    }

    #[test]
    fn platform_error_to_chinese_region_blocked_same_for_all_platforms() {
        // RegionBlocked 对所有平台返回相同文案
        for platform in [
            MediaPlatform::Douyin,
            MediaPlatform::TikTok,
            MediaPlatform::Twitter,
            MediaPlatform::YouTube,
            MediaPlatform::Bilibili,
            MediaPlatform::Weibo,
            MediaPlatform::Unknown,
        ] {
            let msg = platform_error_to_chinese(MediaPlatformError::RegionBlocked, platform);
            assert_eq!(msg, "该内容在你的地区不可用");
        }
    }

    #[test]
    fn platform_error_to_chinese_link_expired() {
        for platform in [
            MediaPlatform::Douyin,
            MediaPlatform::TikTok,
            MediaPlatform::Twitter,
            MediaPlatform::YouTube,
            MediaPlatform::Bilibili,
            MediaPlatform::Weibo,
            MediaPlatform::Unknown,
        ] {
            let msg = platform_error_to_chinese(MediaPlatformError::LinkExpired, platform);
            assert_eq!(msg, "该链接已失效或已被删除");
        }
    }

    #[test]
    fn platform_error_to_chinese_drm_protected() {
        for platform in [
            MediaPlatform::Douyin,
            MediaPlatform::TikTok,
            MediaPlatform::YouTube,
            MediaPlatform::Unknown,
        ] {
            let msg = platform_error_to_chinese(MediaPlatformError::DrmProtected, platform);
            assert_eq!(msg, "该内容受 DRM 保护，无法下载");
        }
    }

    #[test]
    fn platform_error_to_chinese_unsupported() {
        for platform in [
            MediaPlatform::Douyin,
            MediaPlatform::TikTok,
            MediaPlatform::Unknown,
        ] {
            let msg = platform_error_to_chinese(MediaPlatformError::Unsupported, platform);
            assert_eq!(msg, "该平台暂不支持下载此类型内容");
        }
    }

    #[test]
    fn platform_error_to_chinese_unknown() {
        for platform in [
            MediaPlatform::Douyin,
            MediaPlatform::TikTok,
            MediaPlatform::Unknown,
        ] {
            let msg = platform_error_to_chinese(MediaPlatformError::Unknown, platform);
            assert_eq!(msg, "下载失败，请稍后重试");
        }
    }

    #[test]
    fn platform_error_to_chinese_all_messages_are_chinese() {
        // 确保所有组合返回非空中文文案
        for error in [
            MediaPlatformError::LoginExpired,
            MediaPlatformError::RegionBlocked,
            MediaPlatformError::LinkExpired,
            MediaPlatformError::DrmProtected,
            MediaPlatformError::Unsupported,
            MediaPlatformError::Unknown,
        ] {
            for platform in [
                MediaPlatform::Douyin,
                MediaPlatform::TikTok,
                MediaPlatform::Twitter,
                MediaPlatform::YouTube,
                MediaPlatform::Bilibili,
                MediaPlatform::Weibo,
                MediaPlatform::Unknown,
            ] {
                let msg = platform_error_to_chinese(error, platform);
                assert!(
                    !msg.is_empty(),
                    "错误 {error:?} + 平台 {platform:?} 返回空文案"
                );
                // 至少含一个中文字符（CJK Unified Ideographs 范围）
                assert!(
                    msg.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
                    "错误 {error:?} + 平台 {platform:?} 返回非中文文案: {msg}"
                );
            }
        }
    }

    #[test]
    fn platform_error_to_chinese_does_not_leak_cookie_value() {
        // 即使输入 stderr 含 Cookie 原文，输出文案应为固定模板，不含 Cookie 值
        let stderr = "Cookie: session=secret_value_abc123";
        let error = classify_platform_error(MediaPlatform::Douyin, stderr);
        let msg = platform_error_to_chinese(error, MediaPlatform::Douyin);
        assert!(!msg.contains("secret_value_abc123"));
        assert!(!msg.contains("session="));
    }

    #[test]
    fn media_platform_error_default_is_unknown() {
        // Default 应为 Unknown，保证旧 JSON 缺失字段时安全回退
        let error = MediaPlatformError::default();
        assert_eq!(error, MediaPlatformError::Unknown);
    }

    // ===== Task 40：YouTube / B 站 / 微博 平台特定错误识别 =====

    #[test]
    fn classify_youtube_age_restricted() {
        // YouTube 年龄限制：yt-dlp stderr 通常含 "age-restricted" 或
        // "Sign in to confirm your age"
        let stderr1 = "ERROR: [youtube] abc: Video is age-restricted";
        assert_eq!(
            classify_platform_error(MediaPlatform::YouTube, stderr1),
            MediaPlatformError::LoginExpired
        );
        let stderr2 = "ERROR: Sign in to confirm your age";
        assert_eq!(
            classify_platform_error(MediaPlatform::YouTube, stderr2),
            MediaPlatformError::LoginExpired
        );
        // 大小写不敏感
        let stderr3 = "AGE-RESTRICTED content, please sign in";
        assert_eq!(
            classify_platform_error(MediaPlatform::YouTube, stderr3),
            MediaPlatformError::LoginExpired
        );
    }

    #[test]
    fn classify_youtube_bot_check() {
        // YouTube 机器人验证（PO Token 校验失败）：yt-dlp 输出形如
        // "Sign in to confirm you're not a bot. Use --cookies-from-browser or
        //  --cookies for the authentication."
        // 此错误自 2024 年 YouTube 加强反爬虫后常见，统一识别为 LoginExpired
        // 并通过中文文案引导用户提供 Cookie
        let stderr1 = "ERROR: [youtube] abc: Sign in to confirm you're not a bot. \
            Use --cookies-from-browser or --cookies for the authentication.";
        assert_eq!(
            classify_platform_error(MediaPlatform::YouTube, stderr1),
            MediaPlatformError::LoginExpired
        );
        // 大小写不敏感
        let stderr2 = "SIGN IN TO CONFIRM YOU'RE NOT A BOT";
        assert_eq!(
            classify_platform_error(MediaPlatform::YouTube, stderr2),
            MediaPlatformError::LoginExpired
        );
        // 仅含 "cookies-from-browser" 提示也应识别（兜底匹配）
        let stderr3 = "Use --cookies-from-browser to pass cookies";
        assert_eq!(
            classify_platform_error(MediaPlatform::YouTube, stderr3),
            MediaPlatformError::LoginExpired
        );
        // 中文文案应引导用户到「设置 → 媒体凭证」，且不包含敏感字段
        let msg = platform_error_to_chinese(
            MediaPlatformError::LoginExpired,
            MediaPlatform::YouTube,
        );
        assert!(msg.contains("媒体凭证"));
        assert!(msg.contains("Cookie"));
        // 与 Twitter 的 Cookie 文案区分（不应混淆平台）
        assert!(!msg.contains("Twitter"));
    }

    #[test]
    fn classify_youtube_region_blocked() {
        // YouTube 地区限制：含 "not available in your country"
        let stderr = "ERROR: This video is not available in your country";
        assert_eq!(
            classify_platform_error(MediaPlatform::YouTube, stderr),
            MediaPlatformError::RegionBlocked
        );
        // "geo restricted" 也应识别为地区限制
        let stderr2 = "ERROR: The uploader has not made this video available in your region";
        assert_eq!(
            classify_platform_error(MediaPlatform::YouTube, stderr2),
            MediaPlatformError::RegionBlocked
        );
    }

    #[test]
    fn classify_bilibili_premium_required() {
        // B 站会员限制：含 "premium only" 或 "VIP"
        let stderr1 = "ERROR: This is a premium-only video";
        assert_eq!(
            classify_platform_error(MediaPlatform::Bilibili, stderr1),
            MediaPlatformError::LoginExpired
        );
        let stderr2 = "ERROR: VIP member only, please login";
        assert_eq!(
            classify_platform_error(MediaPlatform::Bilibili, stderr2),
            MediaPlatformError::LoginExpired
        );
        // 大小写不敏感
        let stderr3 = "PREMIUM ONLY";
        assert_eq!(
            classify_platform_error(MediaPlatform::Bilibili, stderr3),
            MediaPlatformError::LoginExpired
        );
    }

    #[test]
    fn classify_weibo_link_expired() {
        // 微博链接失效：含 "404" 或 "deleted"
        let stderr1 = "HTTP Error 404: Not Found";
        assert_eq!(
            classify_platform_error(MediaPlatform::Weibo, stderr1),
            MediaPlatformError::LinkExpired
        );
        let stderr2 = "ERROR: this post has been deleted";
        assert_eq!(
            classify_platform_error(MediaPlatform::Weibo, stderr2),
            MediaPlatformError::LinkExpired
        );
        // "removed" 也应识别为链接失效
        let stderr3 = "ERROR: video has been removed by the author";
        assert_eq!(
            classify_platform_error(MediaPlatform::Weibo, stderr3),
            MediaPlatformError::LinkExpired
        );
    }
}
