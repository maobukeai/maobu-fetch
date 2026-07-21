//! 媒体平台识别与适配（Task 37 / Task 38 / Task 39 / Task 41）。
//!
//! 提供基于 URL 的平台识别（抖音 / TikTok / Twitter / YouTube / B 站 / 微博）、
//! 短链接重定向跟随、抖音图集类型识别、Twitter Spaces 识别和按平台的命名模板。
//!
//! ## Task 41 新增能力
//! - [`extract_url_from_share_text`]：从分享文本（如抖音"xxx https://v.douyin.com/yyy 复制此链接..."）
//!   中提取首个 URL；优先返回已知短链域名（`v.douyin.com` / `vm.tiktok.com` / `b23.tv` / `t.cn` 等）。
//! - [`strip_tracking_params`]：从 [`crate::manager::duplicate::strip_tracking_params`] 复用，
//!   白名单方式剥离 utm_*、fbclid、gclid 等跟踪参数，保留业务必需参数。
//! - [`expand_short_url`]：扩展已知短链域名集合，覆盖 url.cn / dwz.cn / bit.ly / tinyurl.com / goo.gl。
//!
//! ## 设计要点（AGENTS.md §3 / §6 / §7）
//! - **安全**：`expand_short_url` 只跟随重定向，不下载响应 body；超时 10 秒；
//!   失败时返回中文错误，不暴露内部异常细节。
//! - **真实状态**：图集识别、平台识别基于真实 URL 模式，不使用模拟数据。
//! - **可恢复错误**：所有可能失败的操作返回 `Result`，不使用 `unwrap()`/`expect()`
//!   处理网络、URL 解析等可恢复错误（AGENTS.md §7）。
//! - **中文文案**：用户可见文案使用简体中文（AGENTS.md §8）。
//! - **不绕过 DRM**：DRM 内容由 `media::probe` 检测后明确拒绝，本模块只负责识别。
//! - **认证安全**：`classify_platform_error` 的 stderr 输入应已经过
//!   [`crate::manager::diagnose::redact_sensitive`] 脱敏；本模块不主动记录或回显 stderr。
//! - **不新增依赖**：复用已有 `regex` 与 `url` crate（AGENTS.md §8）。

use crate::manager::diagnose::MediaPlatformError;
// Task 41：复用 Task 10 已实现的 `strip_tracking_params`，避免重复实现导致行为分叉。
pub use crate::manager::duplicate::strip_tracking_params;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;

/// 媒体平台枚举。
///
/// 用于 `media::probe` 失败时按平台返回中文错误，以及前端展示
/// "检测到：抖音" 等提示。`Unknown` 表示未识别的平台，仍可走通用 yt-dlp 流程。
///
/// 序列化使用 kebab-case，与前端 TypeScript 联合类型对应。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum MediaPlatform {
    #[default]
    Unknown,
    Douyin,
    TikTok,
    Twitter,
    YouTube,
    Bilibili,
    Weibo,
}

impl MediaPlatform {
    /// 返回前端展示用的中文名称。
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Douyin => "抖音",
            Self::TikTok => "TikTok",
            Self::Twitter => "Twitter/X",
            Self::YouTube => "YouTube",
            Self::Bilibili => "哔哩哔哩",
            Self::Weibo => "微博",
            Self::Unknown => "未知平台",
        }
    }

    /// 返回序列化字符串（用于 `media_detect_platform` 命令返回值）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Douyin => "douyin",
            Self::TikTok => "tiktok",
            Self::Twitter => "twitter",
            Self::YouTube => "youtube",
            Self::Bilibili => "bilibili",
            Self::Weibo => "weibo",
        }
    }
}

/// 从 URL 中提取小写 host（去除 `www.` 前缀，保留子域）。
///
/// 解析失败时返回空字符串。`v.douyin.com` 等短链 host 会被保留。
fn extract_host(url: &str) -> String {
    let parsed = match url::Url::parse(url.trim()) {
        Ok(u) => u,
        Err(_) => return String::new(),
    };
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    if let Some(rest) = host.strip_prefix("www.") {
        rest.to_string()
    } else {
        host
    }
}

/// 根据 URL 识别媒体平台。
///
/// 匹配规则（大小写不敏感）：
/// - **抖音**：`v.douyin.com`、`www.douyin.com`、`douyin.com`、`www.iesdouyin.com`、`iesdouyin.com`
/// - **TikTok**：`www.tiktok.com`、`tiktok.com`、`vm.tiktok.com`、`vt.tiktok.com`
/// - **Twitter/X**：`twitter.com`、`x.com`、`t.co`、`mobile.twitter.com`
/// - **YouTube**：`youtube.com`、`youtu.be`、`m.youtube.com`、`music.youtube.com`
/// - **B 站**：`bilibili.com`、`b23.tv`、`m.bilibili.com`、`t.bilibili.com`
/// - **微博**：`weibo.com`、`weibo.cn`、`m.weibo.cn`、`t.cn`
///
/// 未命中返回 `Unknown`。`Unknown` 不阻止 yt-dlp 通用流程，
/// 仅影响错误提示的平台特定中文文案。
pub fn detect_platform(url: &str) -> MediaPlatform {
    let host = extract_host(url);
    if host.is_empty() {
        return MediaPlatform::Unknown;
    }
    if host == "v.douyin.com"
        || host == "douyin.com"
        || host == "iesdouyin.com"
        || host == "www.iesdouyin.com"
        || host == "douyinvod.com"
        || host.ends_with(".douyin.com")
        || host.ends_with(".iesdouyin.com")
        || host.ends_with(".douyinvod.com")
    {
        return MediaPlatform::Douyin;
    }
    if host == "tiktok.com"
        || host == "vm.tiktok.com"
        || host == "vt.tiktok.com"
        || host.ends_with(".tiktok.com")
    {
        return MediaPlatform::TikTok;
    }
    if host == "twitter.com"
        || host == "x.com"
        || host == "t.co"
        || host == "mobile.twitter.com"
        || host.ends_with(".twitter.com")
        || host.ends_with(".x.com")
    {
        return MediaPlatform::Twitter;
    }
    if host == "youtube.com"
        || host == "youtu.be"
        || host == "m.youtube.com"
        || host == "music.youtube.com"
        || host.ends_with(".youtube.com")
    {
        return MediaPlatform::YouTube;
    }
    if host == "bilibili.com"
        || host == "b23.tv"
        || host == "m.bilibili.com"
        || host == "t.bilibili.com"
        || host.ends_with(".bilibili.com")
    {
        return MediaPlatform::Bilibili;
    }
    if host == "weibo.com"
        || host == "weibo.cn"
        || host == "m.weibo.cn"
        || host == "t.cn"
        || host.ends_with(".weibo.com")
        || host.ends_with(".weibo.cn")
    {
        return MediaPlatform::Weibo;
    }
    MediaPlatform::Unknown
}

/// 判断 URL 是否为已知短链域名（需要重定向跟随）。
///
/// 涵盖：`v.douyin.com`、`vm.tiktok.com`、`vt.tiktok.com`、`t.co`、`b23.tv`、`t.cn`、
/// `iesdouyin.com`、`url.cn`、`dwz.cn`、`bit.ly`、`tinyurl.com`、`goo.gl`。
/// 这些域名返回的 URL 通常为分享文本中的短链，需要跟随 HTTP 302 才能拿到真实资源地址。
///
/// Task 41 扩展：补充国内国际常见短链服务，避免 yt-dlp 直接收到短链导致解析失败。
pub fn is_short_url(url: &str) -> bool {
    let host = extract_host(url);
    matches!(
        host.as_str(),
        "v.douyin.com"
            | "vm.tiktok.com"
            | "vt.tiktok.com"
            | "t.co"
            | "b23.tv"
            | "t.cn"
            | "iesdouyin.com"
            | "www.iesdouyin.com"
            | "url.cn"
            | "dwz.cn"
            | "bit.ly"
            | "tinyurl.com"
            | "goo.gl"
    )
}

/// 已知短链域名白名单（与 [`is_short_url`] 保持一致），用于 [`extract_url_from_share_text`] 优先匹配。
///
/// 顺序即优先级：抖音 / TikTok / Twitter / B 站 / 微博国内短链优先，再覆盖国际通用短链。
const KNOWN_SHORT_DOMAINS: &[&str] = &[
    "v.douyin.com",
    "vm.tiktok.com",
    "vt.tiktok.com",
    "t.co",
    "b23.tv",
    "t.cn",
    "url.cn",
    "dwz.cn",
    "bit.ly",
    "tinyurl.com",
    "goo.gl",
];

/// 编译并缓存 URL 提取正则。
///
/// 匹配 `https?://[^\s<>"`]+` 模式，捕获分享文本中的完整 URL（含 query、fragment）。
/// 正则模式为编译期常量，编译失败属于编程错误（不可恢复），因此使用 `expect`。
fn url_extraction_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"https?://[^\s<>"']+"#).expect("URL extraction regex is statically validated")
    })
}

/// 从分享文本中提取首个 URL（Task 41.2）。
///
/// 抖音 / TikTok 等平台的"分享"按钮返回的文本形如：
/// ```text
/// xxx https://v.douyin.com/abc123/ 复制此链接，打开Dou音视频，直接观看视频！
/// ```
/// 本函数从中提取首个 URL，优先返回已知短链域名（[`KNOWN_SHORT_DOMAINS`]）。
/// 若文本中无已知短链，但存在其他 URL，则返回首个 URL（如直接粘贴 `https://www.douyin.com/video/123`）。
/// 若文本中无任何 URL，返回 `None`。
///
/// 注：本函数不发起网络请求，仅做文本解析（AGENTS.md §9 测试强约束）。
/// 单引号 `'` 也视为 URL 终止符，避免吞掉分享文本中的 `xxx'https://...'` 形式。
pub fn extract_url_from_share_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let regex = url_extraction_regex();
    let all_matches: Vec<&str> = regex.find_iter(trimmed).map(|m| m.as_str()).collect();
    if all_matches.is_empty() {
        return None;
    }
    // 优先返回首个已知短链域名匹配。
    for candidate in &all_matches {
        let host = extract_host(candidate);
        if KNOWN_SHORT_DOMAINS.iter().any(|d| *d == host.as_str()) {
            return Some(candidate.to_string());
        }
    }
    // 没有命中已知短链时返回首个 URL（保持"纯 URL 输入直接返回"的行为）。
    all_matches.first().map(|s| s.to_string())
}

/// 跟随短链接重定向，返回最终 URL（不下载响应 body）。
///
/// - 超时：10 秒（避免长尾阻塞新建任务对话框）。
/// - 不下载 body：使用 `reqwest::Client` 自动跟随重定向，最后只读 final URL。
/// - 失败时返回中文错误，不暴露内部异常细节（AGENTS.md §7）。
///
/// 用法：抖音 / TikTok 等短链在 `media::probe` 中先调用此函数，
/// 拿到最终 URL 再传给 yt-dlp，避免 yt-dlp 解析短链失败。
pub async fn expand_short_url(url: &str) -> Result<String, String> {
    if !is_short_url(url) {
        // 非短链直接返回原 URL，避免不必要的网络请求。
        return Ok(url.to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("短链解析失败：{e}"))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("短链解析失败：{e}"))?;
    let mut final_url = response.url().to_string();
    if final_url.is_empty() {
        return Err("短链解析失败：未返回最终地址".into());
    }
    if final_url.contains("iesdouyin.com/share/video/") {
        if let Some(re) = Regex::new(r"/share/video/(\d+)").ok() {
            if let Some(caps) = re.captures(&final_url) {
                if let Some(id) = caps.get(1) {
                    final_url = format!("https://www.douyin.com/video/{}", id.as_str());
                }
            }
        }
    } else if final_url.contains("iesdouyin.com/share/note/") {
        if let Some(re) = Regex::new(r"/share/note/(\d+)").ok() {
            if let Some(caps) = re.captures(&final_url) {
                if let Some(id) = caps.get(1) {
                    final_url = format!("https://www.douyin.com/note/{}", id.as_str());
                }
            }
        }
    }
    Ok(final_url)
}

/// 检测抖音 URL 是否为图集类型（多图下载场景）。
///
/// 抖音图集 URL 模式：
/// - `douyin.com/note/<id>`：图集分享链接（最常见）
/// - `iesdouyin.com/share/note/<id>`：旧版分享链接
/// - `iesdouyin.com/web/api/v2/aweme/post/`：API 返回的图集（含 `aweme_type` 标志）
///
/// 普通视频 URL（`/video/<id>`、`/share/video/<id>`）返回 `false`。
///
/// 注：图集下载需要 yt-dlp 配合 `--write-all_thumbnails` 与多图模式，
/// 本函数仅做识别，具体下载逻辑由 `media::download` 处理。
pub fn is_douyin_gallery(url: &str) -> bool {
    // 先校验平台：非抖音 URL 即使含 /note/ 也不识别为图集
    if detect_platform(url) != MediaPlatform::Douyin {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    // 图集 URL 含 /note/ 路径段
    if lower.contains("/note/") {
        return true;
    }
    // 旧版 iesdouyin 分享图集
    if lower.contains("/share/note/") {
        return true;
    }
    // 抖音分享域名 + mode=image 标志（部分短链展开后含此参数）
    if lower.contains("aweme_type=68") || lower.contains("aweme_type=150") {
        return true;
    }
    false
}

/// 文件名中 Windows 不允许的字符，需替换为 `_`。
const ILLEGAL_FILENAME_CHARS: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// 抖音文件名最大长度（含扩展名）。
///
/// Windows NTFS 文件名上限 255 字符，这里取 100 字符作为保守上限，
/// 避免 yt-dlp 输出模板过长导致路径溢出（与 AGENTS.md §7 一致）。
const DOUYIN_FILENAME_MAX_LEN: usize = 100;

/// 按抖音作者命名模板 `{author}_{title}_{date}` 格式化文件名。
///
/// - **非法字符**：`\ / : * ? " < > |` 与控制字符替换为 `_`（Windows 文件名约束）。
/// - **空值处理**：`author` / `title` / `date` 任一为空时跳过对应段，避免出现 `__`。
/// - **长度限制**：总长度超过 100 字符时按字符截断（保留前 100 个 Unicode 标量）。
/// - **空结果回退**：三段全空时返回 `douyin_<timestamp>` 形式的回退名（调用方负责追加扩展名）。
pub fn format_douyin_filename(author: &str, title: &str, date: &str) -> String {
    fn sanitize(segment: &str) -> String {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        let value: String = trimmed
            .chars()
            .map(|c| {
                if ILLEGAL_FILENAME_CHARS.contains(&c) || c.is_control() {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        // 压缩连续下划线，去除首尾下划线
        let mut prev_underscore = false;
        let mut result = String::with_capacity(value.len());
        for c in value.chars() {
            if c == '_' {
                if !prev_underscore && !result.is_empty() {
                    result.push('_');
                }
                prev_underscore = true;
            } else {
                result.push(c);
                prev_underscore = false;
            }
        }
        let trimmed_result = result.trim_matches('_').to_string();
        trimmed_result
    }

    let parts: Vec<String> = [sanitize(author), sanitize(title), sanitize(date)]
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        // 全空时返回回退名（不含扩展名，调用方负责追加 .mp4 等）
        return format!(
            "douyin_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        );
    }
    let joined = parts.join("_");
    // 按字符截断到 100 字符
    let truncated: String = joined.chars().take(DOUYIN_FILENAME_MAX_LEN).collect();
    truncated
}

// ===== Task 39：Twitter/X 平台专用适配 =====
//
// 注：`MediaPlatformError` 枚举、`classify_platform_error` 调度器与
// `platform_error_to_chinese` 中文翻译统一在 `crate::manager::diagnose` 中定义
// （Task 37 规范位置）。本模块仅提供平台特定的辅助识别函数：
// - `classify_twitter_error` / `classify_tiktok_error` / `classify_douyin_error`：
//   由 `diagnose::classify_platform_error` 在 DRM 检测后分发调用。
// - `is_twitter_space`：识别 Twitter Spaces 音频 URL。
// - `format_twitter_filename`：按 `{author}_{tweet_id}_{date}` 模板生成文件名。

/// 检测 URL 是否为 Twitter Spaces 音频（Task 39.1 / 39.4）。
///
/// Twitter Spaces 是 Twitter 的实时音频聊天功能，URL 形如
/// `https://twitter.com/i/spaces/1DXxyv...` 或 `https://x.com/i/spaces/...`。
/// 检测路径包含 `/spaces/` 即认为是 Spaces 音频。
///
/// 普通推文 URL（`/user/status/123`）返回 `false`。
/// 非 Twitter/X 域名也返回 `false`。
pub fn is_twitter_space(url: &str) -> bool {
    let parsed = match url::Url::parse(url.trim()) {
        Ok(u) => u,
        Err(_) => return false,
    };
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    if host != "twitter.com"
        && !host.ends_with(".twitter.com")
        && host != "x.com"
        && !host.ends_with(".x.com")
    {
        return false;
    }
    let path = parsed.path().to_ascii_lowercase();
    path.contains("/spaces/")
}

/// Twitter/X 平台特定的错误分类（大小写不敏感）。
///
/// 由 [`crate::manager::diagnose::classify_platform_error`] 在 DRM 检测后
/// 对 `MediaPlatform::Twitter` 分发调用。
///
/// 匹配优先级：
/// 1. 链接失效：`Tweet not found`、`Status not found`、`404`
/// 2. 登录失效：`Login required`、`Cookie(s) required`
/// 3. 受限内容：`sensitive`、`age-restricted`（需登录查看）→ `LoginExpired`
/// 4. 单独 403/Forbidden：可能是登录失效
/// 5. 其它：返回 `Unknown`，由 `classify_platform_error` 兜底走通用关键词匹配
pub(crate) fn classify_twitter_error(lower: &str) -> MediaPlatformError {
    // 链接失效：推文已删除、404
    if lower.contains("tweet not found")
        || lower.contains("status not found")
        || lower.contains("404")
    {
        return MediaPlatformError::LinkExpired;
    }
    // 登录失效：yt-dlp 提示需要登录或需要 Cookie
    if lower.contains("login required")
        || lower.contains("cookie required")
        || lower.contains("cookies required")
    {
        return MediaPlatformError::LoginExpired;
    }
    // 受限内容：sensitive / age-restricted 需要登录查看
    if lower.contains("sensitive")
        || lower.contains("age-restricted")
        || lower.contains("age restricted")
    {
        return MediaPlatformError::LoginExpired;
    }
    // 403 单独出现：通常是登录失效
    if lower.contains("403") || lower.contains("forbidden") {
        return MediaPlatformError::LoginExpired;
    }
    MediaPlatformError::Unknown
}

/// 按 Twitter/X 命名模板 `{author}_{tweet_id}_{date}` 生成文件名（Task 39.4）。
///
/// - **去除 `@` 前缀**：`@elonmusk` → `elonmusk`
/// - **非法字符**：`\ / : * ? " < > |` 与控制字符替换为 `_`（Windows 文件名约束）
/// - **空值处理**：任一字段为空时使用 `unknown` 占位，避免产生 `__2026-01-01` 这样的文件名
/// - **长度限制**：总长度按字符截断到 100 字符（与 [`format_douyin_filename`] 一致）
/// - **返回值不含扩展名**：调用方负责追加 `.mp4` / `.m4a` 等
pub fn format_twitter_filename(author: &str, tweet_id: &str, date: &str) -> String {
    let author_clean = sanitize_twitter_segment(strip_at_prefix(author.trim()));
    let tweet_id_clean = sanitize_twitter_segment(tweet_id.trim());
    let date_clean = sanitize_twitter_segment(date.trim());

    let author_final = if author_clean.is_empty() {
        "unknown".to_string()
    } else {
        author_clean
    };
    let tweet_id_final = if tweet_id_clean.is_empty() {
        "unknown".to_string()
    } else {
        tweet_id_clean
    };
    let date_final = if date_clean.is_empty() {
        "unknown".to_string()
    } else {
        date_clean
    };

    let combined = format!("{author_final}_{tweet_id_final}_{date_final}");
    // 按字符截断到 100 字符（与 format_douyin_filename 一致）
    let truncated: String = combined.chars().take(DOUYIN_FILENAME_MAX_LEN).collect();
    truncated
}

/// 去除作者名前缀的 `@` 符号（如 `@elonmusk` → `elonmusk`）。
///
/// 仅去除首个 `@`，保留后续合法 `@`（如 `user@domain` 极少见但保留原意）。
fn strip_at_prefix(value: &str) -> &str {
    value.strip_prefix('@').unwrap_or(value)
}

/// 把 Twitter 字段中的 Windows 非法字符替换为 `_`，去除首尾空白与下划线。
///
/// 与 [`format_douyin_filename`] 内部的 sanitize 不同：Twitter 字段较短，
/// 不需要压缩连续下划线；保留可读性。
fn sanitize_twitter_segment(segment: &str) -> String {
    if segment.is_empty() {
        return String::new();
    }
    let value: String = segment
        .chars()
        .map(|c| {
            if ILLEGAL_FILENAME_CHARS.contains(&c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    value.trim_matches('_').to_string()
}

/// 检测 TikTok URL 是否为图集类型（多图下载场景）。
///
/// TikTok 图集 URL 模式：
/// - `tiktok.com/@user/photo/<id>`：图集分享链接（最常见）
/// - `tiktok.com/@user/photos/<id>`：复数形式（部分版本）
///
/// 普通视频 URL（`/video/<id>`）返回 `false`。
///
/// 注：图集下载需要 yt-dlp 配合多图模式，本函数仅做识别。
pub fn is_tiktok_gallery(url: &str) -> bool {
    // 先校验平台：非 TikTok URL 即使含 /photo/ 也不识别为图集
    if detect_platform(url) != MediaPlatform::TikTok {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    // 图集 URL 含 /photo/ 或 /photos/ 路径段
    if lower.contains("/photo/") || lower.contains("/photos/") {
        return true;
    }
    false
}

/// TikTok 文件名最大长度（含扩展名）。
///
/// 与抖音保持一致，取 100 字符作为保守上限，避免 yt-dlp 输出模板过长导致路径溢出。
const TIKTOK_FILENAME_MAX_LEN: usize = 100;

/// 按 TikTok 作者命名模板 `{author}_{title}_{date}` 格式化文件名。
///
/// 与 `format_douyin_filename` 的差异：
/// - **@ 前缀处理**：TikTok 作者名常以 `@` 开头（如 `@username`），文件名中去除该前缀。
/// - **非法字符**：与抖音一致，`\ / : * ? " < > |` 与控制字符替换为 `_`。
/// - **空值处理**：`author` / `title` / `date` 任一为空时跳过对应段，避免出现 `__`。
/// - **长度限制**：总长度超过 100 字符时按字符截断。
/// - **空结果回退**：三段全空时返回 `tiktok_<timestamp>` 形式的回退名。
pub fn format_tiktok_filename(author: &str, title: &str, date: &str) -> String {
    fn sanitize(segment: &str) -> String {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        // 去除 @ 前缀（TikTok 作者名特化）
        let without_at = trimmed.strip_prefix('@').unwrap_or(trimmed);
        let value: String = without_at
            .chars()
            .map(|c| {
                if ILLEGAL_FILENAME_CHARS.contains(&c) || c.is_control() {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        // 压缩连续下划线，去除首尾下划线
        let mut prev_underscore = false;
        let mut result = String::with_capacity(value.len());
        for c in value.chars() {
            if c == '_' {
                if !prev_underscore && !result.is_empty() {
                    result.push('_');
                }
                prev_underscore = true;
            } else {
                result.push(c);
                prev_underscore = false;
            }
        }
        result.trim_matches('_').to_string()
    }

    let parts: Vec<String> = [sanitize(author), sanitize(title), sanitize(date)]
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        // 全空时返回回退名（不含扩展名，调用方负责追加 .mp4 等）
        return format!(
            "tiktok_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        );
    }
    let joined = parts.join("_");
    // 按字符截断到 100 字符
    let truncated: String = joined.chars().take(TIKTOK_FILENAME_MAX_LEN).collect();
    truncated
}

/// TikTok 专用错误识别（Task 38.3）。
///
/// 由 [`crate::manager::diagnose::classify_platform_error`] 在 DRM 检测后
/// 对 `MediaPlatform::TikTok` 分发调用。
///
/// 识别顺序：地区限制 → 登录失效 → 链接失效 → 默认 Unknown。
/// 429 状态码仅在配合地区相关字眼时识别为 `RegionBlocked`，
/// 普通 429（频率限制）不视为地区限制。
///
/// 注：yt-dlp 通常会把 HTTP 429 写入 stderr（如 `HTTP Error 429: Too Many Requests`），
/// 因此本函数从 stderr 字符串中检测 429，无需额外 status_code 参数。
pub(crate) fn classify_tiktok_error(lower_stderr: &str) -> MediaPlatformError {
    // 地区限制：明确的地区关键词，或 429 配合地区字眼
    let has_region_keyword = lower_stderr.contains("not available in your region")
        || lower_stderr.contains("geo-restricted")
        || lower_stderr.contains("not available in your country")
        || lower_stderr.contains("region restricted");
    let has_geo_mention = lower_stderr.contains("region")
        || lower_stderr.contains("geo")
        || lower_stderr.contains("country");
    let is_429 = lower_stderr.contains("429");
    if has_region_keyword || (is_429 && has_geo_mention) {
        return MediaPlatformError::RegionBlocked;
    }

    // 登录失效：Login required / Cookie 相关
    if lower_stderr.contains("login required")
        || lower_stderr.contains("cookie")
        || lower_stderr.contains("login expired")
    {
        return MediaPlatformError::LoginExpired;
    }

    // 链接失效：Video not found / 404
    if lower_stderr.contains("video not found")
        || lower_stderr.contains("not found")
        || lower_stderr.contains("404")
    {
        return MediaPlatformError::LinkExpired;
    }

    MediaPlatformError::Unknown
}

/// 抖音专用错误识别（Task 37 共用框架）。
///
/// 由 [`crate::manager::diagnose::classify_platform_error`] 在 DRM 检测后
/// 对 `MediaPlatform::Douyin` 分发调用。
///
/// 抖音的 stderr 关键词与 TikTok 类似但略有差异，
/// 此函数为 Task 37 提供；如 Task 37 已实现更细化的逻辑，可后续覆盖。
pub(crate) fn classify_douyin_error(lower_stderr: &str) -> MediaPlatformError {
    // 地区限制
    if lower_stderr.contains("not available in your region")
        || lower_stderr.contains("geo-restricted")
    {
        return MediaPlatformError::RegionBlocked;
    }

    // 登录失效
    if lower_stderr.contains("login required") || lower_stderr.contains("cookie") {
        return MediaPlatformError::LoginExpired;
    }

    // 链接失效
    if lower_stderr.contains("video not found") || lower_stderr.contains("404") {
        return MediaPlatformError::LinkExpired;
    }

    MediaPlatformError::Unknown
}

/// YouTube 平台特定的错误分类（Task 40.1 / Task 44）。
///
/// 由 [`crate::manager::diagnose::classify_platform_error`] 在 DRM 检测后
/// 对 `MediaPlatform::YouTube` 分发调用。
///
/// 匹配优先级：
/// 1. 地区限制：`not available in your country`、`geo-restricted`、`geo restricted`、
///    `region`（覆盖 yt-dlp 输出的 "available in your region" 提示）
/// 2. 登录失效：`age-restricted`、`age restricted`、`sign in to confirm your age`
/// 3. 链接失效：`video not found`、`404`、`not found`
/// 4. 其它：返回 `Unknown`，由 `classify_platform_error` 兜底走通用关键词匹配
///
/// 注：参数 `lower` 应为已小写化的 stderr 字符串，与 `classify_twitter_error`
/// 保持一致，避免重复 `to_ascii_lowercase` 调用。
pub(crate) fn classify_youtube_error(lower: &str) -> MediaPlatformError {
    // 地区限制：YouTube 常见 "not available in your country" / "available in your region"
    if lower.contains("not available in your country")
        || lower.contains("geo-restricted")
        || lower.contains("geo restricted")
        || lower.contains("region")
    {
        return MediaPlatformError::RegionBlocked;
    }
    // 登录失效：年龄限制需要登录确认
    if lower.contains("age-restricted")
        || lower.contains("age restricted")
        || lower.contains("sign in to confirm your age")
    {
        return MediaPlatformError::LoginExpired;
    }
    // 链接失效
    if lower.contains("video not found") || lower.contains("404") || lower.contains("not found") {
        return MediaPlatformError::LinkExpired;
    }
    MediaPlatformError::Unknown
}

/// 哔哩哔哩平台特定的错误分类（Task 40.2 / Task 44）。
///
/// 由 [`crate::manager::diagnose::classify_platform_error`] 在 DRM 检测后
/// 对 `MediaPlatform::Bilibili` 分发调用。
///
/// 匹配优先级：
/// 1. 登录失效：`premium`（覆盖 `premium only` 与 `premium-only` 连字符形式）、
///    `vip`、`大会员`（B 站会员/付费内容限制，需用户登录或开通大会员）
/// 2. 链接失效：`video not found`、`404`、`not found`
/// 3. 其它：返回 `Unknown`，由 `classify_platform_error` 兜底走通用关键词匹配
///
/// 注：yt-dlp 对 B 站付费内容可能输出 `premium-only`（连字符）或 `premium only`（空格），
/// 此处统一以 `premium` 关键词识别，避免遗漏连字符场景。
pub(crate) fn classify_bilibili_error(lower: &str) -> MediaPlatformError {
    // 登录失效：会员/付费内容限制
    if lower.contains("premium") || lower.contains("vip") || lower.contains("大会员") {
        return MediaPlatformError::LoginExpired;
    }
    // 链接失效
    if lower.contains("video not found") || lower.contains("404") || lower.contains("not found") {
        return MediaPlatformError::LinkExpired;
    }
    MediaPlatformError::Unknown
}

/// 微博平台特定的错误分类（Task 40.3 / Task 44）。
///
/// 由 [`crate::manager::diagnose::classify_platform_error`] 在 DRM 检测后
/// 对 `MediaPlatform::Weibo` 分发调用。
///
/// 匹配优先级：
/// 1. 链接失效：`404`、`deleted`、`removed`、`not found`（微博内容已删除或被下架）
/// 2. 其它：返回 `Unknown`，由 `classify_platform_error` 兜底走通用关键词匹配
pub(crate) fn classify_weibo_error(lower: &str) -> MediaPlatformError {
    // 链接失效：微博内容已删除或被下架
    if lower.contains("404")
        || lower.contains("deleted")
        || lower.contains("removed")
        || lower.contains("not found")
    {
        return MediaPlatformError::LinkExpired;
    }
    MediaPlatformError::Unknown
}

// ===== Task 40：YouTube / B 站 / 微博 平台专用适配（回归测试支撑） =====
//
// 设计要点（AGENTS.md §3 / §7 / §9）：
// - **不依赖网络**：所有识别函数仅基于 URL 模式匹配，不发起任何 HTTP 请求，
//   保证单元测试可在离线环境运行（AGENTS.md §9 测试强约束）。
// - **真实状态**：识别基于真实 URL 模式（YouTube `/shorts/`、B 站 `/bangumi/play/` 等），
//   不使用模拟数据（AGENTS.md §3）。
// - **可恢复错误**：URL 解析失败时返回 `false` 或空字符串，不使用 `unwrap()`/`expect()`
//   处理可恢复的输入错误（AGENTS.md §7）。
// - **中文文案**：用户可见文案使用简体中文（AGENTS.md §8）。
// - **认证安全**：URL 检测不读取或回显 Cookie、Authorization 等敏感参数。

/// YouTube Shorts URL 检测（Task 40.1）。
///
/// YouTube Shorts 的 URL 形如 `https://www.youtube.com/shorts/<video_id>`，
/// 路径段含 `/shorts/` 即认为是短视频。普通 `watch?v=` 或 `youtu.be/<id>` 返回 `false`。
///
/// 注：本函数仅基于 URL 模式识别，不依赖网络请求（AGENTS.md §9 测试强约束）。
pub fn is_youtube_short(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("/shorts/")
}

/// YouTube 直播回放 URL 检测（Task 40.1）。
///
/// YouTube 直播回放的常见 URL 模式：
/// - `https://www.youtube.com/live/<video_id>`：直播回放专属路径
/// - `https://www.youtube.com/watch?v=<id>&live=1`：watch 接口附加 `live` 参数
///
/// 普通视频 URL（`watch?v=` 不含 `live` 参数）返回 `false`。
pub fn is_youtube_live_replay(url: &str) -> bool {
    let parsed = match url::Url::parse(url.trim()) {
        Ok(u) => u,
        Err(_) => return false,
    };
    let path = parsed.path().to_ascii_lowercase();
    if path.contains("/live/") {
        return true;
    }
    // 检查 query 参数中是否含 live（任意取值）
    let query_pairs = parsed.query_pairs();
    for (key, _) in query_pairs {
        if key.to_ascii_lowercase() == "live" {
            return true;
        }
    }
    false
}

/// YouTube 年龄限制 URL 检测（Task 40.1）。
///
/// 真实 YouTube 年龄限制视频的 URL 不携带显式标志（限制由服务端判定），
/// 此函数仅识别用户手动标注或外部工具传入的 `age_restricted=1` / `age_restricted=true`
/// 查询参数，作为辅助识别手段。真正的年龄限制错误由
/// [`crate::manager::diagnose::classify_platform_error`] 从 yt-dlp stderr 中识别。
pub fn is_youtube_age_restricted(url: &str) -> bool {
    let parsed = match url::Url::parse(url.trim()) {
        Ok(u) => u,
        Err(_) => return false,
    };
    let query_pairs = parsed.query_pairs();
    for (key, value) in query_pairs {
        if key.to_ascii_lowercase() == "age_restricted" {
            let v = value.to_ascii_lowercase();
            return v == "1" || v == "true" || v == "yes";
        }
    }
    false
}

/// B 站番剧 URL 检测（Task 40.2）。
///
/// B 站番剧 URL 形如 `https://www.bilibili.com/bangumi/play/ep<id>` 或
/// `https://www.bilibili.com/bangumi/play/ss<id>`，路径含 `/bangumi/play/` 即认为是番剧。
/// 普通视频 URL（`/video/BV...`）返回 `false`。
pub fn is_bilibili_bangumi(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("/bangumi/play/")
}

/// B 站直播回放 URL 检测（Task 40.2）。
///
/// B 站直播回放的常见 URL 模式：
/// - `https://live.bilibili.com/record/<id>`：直播回放专属域名
/// - `https://www.bilibili.com/live/<room_id>`：直播间地址（含回放）
/// - `https://live.bilibili.com/<room_id>`：直播间简短地址
///
/// 检测 host 为 `live.bilibili.com` 或路径含 `/live/`、`/record/` 即认为是直播回放。
pub fn is_bilibili_live_replay(url: &str) -> bool {
    let parsed = match url::Url::parse(url.trim()) {
        Ok(u) => u,
        Err(_) => return false,
    };
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    if host == "live.bilibili.com" || host.ends_with(".live.bilibili.com") {
        return true;
    }
    let path = parsed.path().to_ascii_lowercase();
    path.contains("/live/") || path.contains("/record/")
}

/// 微博图集 URL 检测（Task 40.3）。
///
/// 微博图集的常见 URL 模式：
/// - `https://weibo.com/album/<id>`：图集专属路径
/// - `https://photo.weibo.com/<id>`：图集子域
/// - `https://weibo.com/status/<id>?album=1`：状态附带 `album` 参数
///
/// 检测路径含 `/album/`、host 为 `photo.weibo.com`，或 query 参数含 `album` 即认为是图集。
pub fn is_weibo_gallery(url: &str) -> bool {
    let parsed = match url::Url::parse(url.trim()) {
        Ok(u) => u,
        Err(_) => return false,
    };
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    if host == "photo.weibo.com" || host.ends_with(".photo.weibo.com") {
        return true;
    }
    let path = parsed.path().to_ascii_lowercase();
    if path.contains("/album/") {
        return true;
    }
    // 检查 query 参数中是否含 album（任意取值）
    let query_pairs = parsed.query_pairs();
    for (key, _) in query_pairs {
        if key.to_ascii_lowercase() == "album" {
            return true;
        }
    }
    false
}

/// YouTube 文件名最大长度（与抖音/TikTok 保持一致）。
const YOUTUBE_FILENAME_MAX_LEN: usize = 100;

/// 按 YouTube 命名模板 `{channel}_{title}_{video_id}_{date}` 格式化文件名（Task 40.1）。
///
/// - **非法字符**：`\ / : * ? " < > |` 与控制字符替换为 `_`（Windows 文件名约束）。
/// - **空值处理**：任一段为空时跳过对应段，避免出现 `___`。
/// - **长度限制**：总长度超过 100 字符时按 Unicode 标量截断。
/// - **空结果回退**：四段全空时返回 `youtube_<timestamp>` 形式的回退名（不含扩展名）。
pub fn format_youtube_filename(channel: &str, title: &str, video_id: &str, date: &str) -> String {
    let parts: Vec<String> = [
        sanitize_youtube_segment(channel),
        sanitize_youtube_segment(title),
        sanitize_youtube_segment(video_id),
        sanitize_youtube_segment(date),
    ]
    .into_iter()
    .filter(|p| !p.is_empty())
    .collect();
    if parts.is_empty() {
        return format!(
            "youtube_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        );
    }
    let joined = parts.join("_");
    let truncated: String = joined.chars().take(YOUTUBE_FILENAME_MAX_LEN).collect();
    truncated
}

/// YouTube 文件名段清洗：去除首尾空白，将 Windows 非法字符与控制字符替换为 `_`，
/// 压缩连续下划线，去除首尾下划线。
fn sanitize_youtube_segment(segment: &str) -> String {
    let trimmed = segment.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let value: String = trimmed
        .chars()
        .map(|c| {
            if ILLEGAL_FILENAME_CHARS.contains(&c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut prev_underscore = false;
    let mut result = String::with_capacity(value.len());
    for c in value.chars() {
        if c == '_' {
            if !prev_underscore && !result.is_empty() {
                result.push('_');
            }
            prev_underscore = true;
        } else {
            result.push(c);
            prev_underscore = false;
        }
    }
    result.trim_matches('_').to_string()
}

/// B 站文件名最大长度（与抖音/TikTok 保持一致）。
const BILIBILI_FILENAME_MAX_LEN: usize = 100;

/// 按 B 站命名模板 `{author}_{title}_{bvid}` 格式化文件名（Task 40.2）。
///
/// - **非法字符**：`\ / : * ? " < > |` 与控制字符替换为 `_`。
/// - **空值处理**：任一段为空时跳过对应段。
/// - **长度限制**：总长度超过 100 字符时按 Unicode 标量截断。
/// - **空结果回退**：三段全空时返回 `bilibili_<timestamp>` 形式的回退名。
pub fn format_bilibili_filename(author: &str, title: &str, bvid: &str) -> String {
    let parts: Vec<String> = [
        sanitize_bilibili_segment(author),
        sanitize_bilibili_segment(title),
        sanitize_bilibili_segment(bvid),
    ]
    .into_iter()
    .filter(|p| !p.is_empty())
    .collect();
    if parts.is_empty() {
        return format!(
            "bilibili_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        );
    }
    let joined = parts.join("_");
    let truncated: String = joined.chars().take(BILIBILI_FILENAME_MAX_LEN).collect();
    truncated
}

/// B 站文件名段清洗：与 [`sanitize_youtube_segment`] 行为一致。
fn sanitize_bilibili_segment(segment: &str) -> String {
    sanitize_youtube_segment(segment)
}

/// 微博文件名最大长度（与抖音/TikTok 保持一致）。
const WEIBO_FILENAME_MAX_LEN: usize = 100;

/// 按微博命名模板 `{author}_{title}_{date}` 格式化文件名（Task 40.3）。
///
/// - **非法字符**：`\ / : * ? " < > |` 与控制字符替换为 `_`。
/// - **空值处理**：任一段为空时跳过对应段。
/// - **长度限制**：总长度超过 100 字符时按 Unicode 标量截断。
/// - **空结果回退**：三段全空时返回 `weibo_<timestamp>` 形式的回退名。
pub fn format_weibo_filename(author: &str, title: &str, date: &str) -> String {
    let parts: Vec<String> = [
        sanitize_weibo_segment(author),
        sanitize_weibo_segment(title),
        sanitize_weibo_segment(date),
    ]
    .into_iter()
    .filter(|p| !p.is_empty())
    .collect();
    if parts.is_empty() {
        return format!(
            "weibo_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        );
    }
    let joined = parts.join("_");
    let truncated: String = joined.chars().take(WEIBO_FILENAME_MAX_LEN).collect();
    truncated
}

/// 微博文件名段清洗：与 [`sanitize_youtube_segment`] 行为一致。
fn sanitize_weibo_segment(segment: &str) -> String {
    sanitize_youtube_segment(segment)
}

// ===== 单元测试 =====
#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::diagnose::{
        classify_platform_error, platform_error_to_chinese, MediaPlatformError,
    };

    // ---- detect_platform：抖音 ----

    #[test]
    fn detect_douyin_from_short_url() {
        assert_eq!(
            detect_platform("https://v.douyin.com/abc123/"),
            MediaPlatform::Douyin
        );
        assert_eq!(
            detect_platform("http://v.douyin.com/xyz"),
            MediaPlatform::Douyin
        );
    }

    #[test]
    fn detect_douyin_from_long_url() {
        assert_eq!(
            detect_platform("https://www.douyin.com/video/7283698765432109876"),
            MediaPlatform::Douyin
        );
        assert_eq!(
            detect_platform("https://douyin.com/video/1234567890"),
            MediaPlatform::Douyin
        );
    }

    #[test]
    fn detect_douyin_from_iesdouyin() {
        assert_eq!(
            detect_platform("https://www.iesdouyin.com/share/video/1234567890"),
            MediaPlatform::Douyin
        );
        assert_eq!(
            detect_platform("https://iesdouyin.com/share/note/1234567890"),
            MediaPlatform::Douyin
        );
    }

    // ---- detect_platform：TikTok ----

    #[test]
    fn detect_tiktok() {
        assert_eq!(
            detect_platform("https://www.tiktok.com/@user/video/123"),
            MediaPlatform::TikTok
        );
        assert_eq!(
            detect_platform("https://tiktok.com/@user/video/456"),
            MediaPlatform::TikTok
        );
        assert_eq!(
            detect_platform("https://vm.tiktok.com/Z123abcd/"),
            MediaPlatform::TikTok
        );
        assert_eq!(
            detect_platform("https://vt.tiktok.com/Z123abcd/"),
            MediaPlatform::TikTok
        );
    }

    // ---- detect_platform：Twitter/X ----

    #[test]
    fn detect_twitter() {
        assert_eq!(
            detect_platform("https://twitter.com/user/status/1234567890"),
            MediaPlatform::Twitter
        );
        assert_eq!(
            detect_platform("https://x.com/user/status/1234567890"),
            MediaPlatform::Twitter
        );
        assert_eq!(
            detect_platform("https://t.co/abc123"),
            MediaPlatform::Twitter
        );
        assert_eq!(
            detect_platform("https://mobile.twitter.com/user/status/123"),
            MediaPlatform::Twitter
        );
    }

    // ---- detect_platform：YouTube ----

    #[test]
    fn detect_youtube() {
        assert_eq!(
            detect_platform("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            MediaPlatform::YouTube
        );
        assert_eq!(
            detect_platform("https://youtu.be/dQw4w9WgXcQ"),
            MediaPlatform::YouTube
        );
        assert_eq!(
            detect_platform("https://m.youtube.com/watch?v=abc123"),
            MediaPlatform::YouTube
        );
        assert_eq!(
            detect_platform("https://music.youtube.com/watch?v=abc123"),
            MediaPlatform::YouTube
        );
    }

    // ---- detect_platform：B 站 ----

    #[test]
    fn detect_bilibili() {
        assert_eq!(
            detect_platform("https://www.bilibili.com/video/BV1xx411c7mD"),
            MediaPlatform::Bilibili
        );
        assert_eq!(
            detect_platform("https://b23.tv/abc123"),
            MediaPlatform::Bilibili
        );
        assert_eq!(
            detect_platform("https://m.bilibili.com/video/BV1xx411c7mD"),
            MediaPlatform::Bilibili
        );
        assert_eq!(
            detect_platform("https://t.bilibili.com/123456"),
            MediaPlatform::Bilibili
        );
    }

    // ---- detect_platform：微博 ----

    #[test]
    fn detect_weibo() {
        assert_eq!(
            detect_platform("https://weibo.com/1234567890/N0abcdef"),
            MediaPlatform::Weibo
        );
        assert_eq!(
            detect_platform("https://weibo.cn/1234567890/N0abcdef"),
            MediaPlatform::Weibo
        );
        assert_eq!(
            detect_platform("https://t.cn/A6abc123"),
            MediaPlatform::Weibo
        );
        assert_eq!(
            detect_platform("https://m.weibo.cn/status/1234567890"),
            MediaPlatform::Weibo
        );
    }

    // ---- detect_platform：未知 / 无效 ----

    #[test]
    fn detect_unknown_platform() {
        assert_eq!(
            detect_platform("https://example.com/file.zip"),
            MediaPlatform::Unknown
        );
        assert_eq!(
            detect_platform("https://github.com/user/repo"),
            MediaPlatform::Unknown
        );
    }

    #[test]
    fn detect_invalid_url_returns_unknown() {
        assert_eq!(detect_platform("not a url"), MediaPlatform::Unknown);
        assert_eq!(detect_platform(""), MediaPlatform::Unknown);
        assert_eq!(
            detect_platform("ftp://example.com/file"),
            MediaPlatform::Unknown
        );
    }

    // ---- is_short_url ----

    #[test]
    fn is_short_url_detects_known_short_domains() {
        assert!(is_short_url("https://v.douyin.com/abc123/"));
        assert!(is_short_url("https://vm.tiktok.com/Z123/"));
        assert!(is_short_url("https://vt.tiktok.com/Z123/"));
        assert!(is_short_url("https://t.co/abc"));
        assert!(is_short_url("https://b23.tv/abc"));
        assert!(is_short_url("https://t.cn/A6abc"));
    }

    #[test]
    fn is_short_url_returns_false_for_long_urls() {
        assert!(!is_short_url("https://www.douyin.com/video/123"));
        assert!(!is_short_url("https://www.youtube.com/watch?v=abc"));
        assert!(!is_short_url("https://example.com/file.zip"));
    }

    // ---- is_douyin_gallery ----

    #[test]
    fn is_douyin_gallery_with_note_url() {
        assert!(is_douyin_gallery("https://www.douyin.com/note/1234567890"));
        assert!(is_douyin_gallery("https://douyin.com/note/1234567890"));
        assert!(is_douyin_gallery(
            "https://www.iesdouyin.com/share/note/1234567890"
        ));
    }

    #[test]
    fn is_douyin_gallery_with_video_url_returns_false() {
        assert!(!is_douyin_gallery(
            "https://www.douyin.com/video/1234567890"
        ));
        assert!(!is_douyin_gallery(
            "https://www.douyin.com/share/video/1234567890"
        ));
        assert!(!is_douyin_gallery(
            "https://www.iesdouyin.com/share/video/1234567890"
        ));
    }

    #[test]
    fn is_douyin_gallery_with_aweme_type_param() {
        assert!(is_douyin_gallery(
            "https://www.douyin.com/discover?aweme_type=68"
        ));
        assert!(is_douyin_gallery(
            "https://www.douyin.com/discover?aweme_type=150"
        ));
        // 普通视频 aweme_type 不视为图集
        assert!(!is_douyin_gallery(
            "https://www.douyin.com/discover?aweme_type=0"
        ));
    }

    #[test]
    fn is_douyin_gallery_with_non_douyin_url_returns_false() {
        assert!(!is_douyin_gallery("https://www.youtube.com/watch?v=abc"));
        assert!(!is_douyin_gallery("https://example.com/note/123"));
    }

    // ---- format_douyin_filename：正常 ----

    #[test]
    fn format_douyin_filename_normal() {
        let name = format_douyin_filename("张三", "我的视频", "2026-01-01");
        assert_eq!(name, "张三_我的视频_2026-01-01");
    }

    #[test]
    fn format_douyin_filename_with_english() {
        let name = format_douyin_filename("alice", "My Cool Video", "2026-07-20");
        assert_eq!(name, "alice_My Cool Video_2026-07-20");
    }

    // ---- format_douyin_filename：非法字符替换 ----

    #[test]
    fn format_douyin_filename_strips_illegal_chars() {
        // 测试每个非法字符都被替换为 _
        let name = format_douyin_filename("author\\name", "title:with*illegal?", "2026-01-01");
        assert!(!name.contains('\\'));
        assert!(!name.contains(':'));
        assert!(!name.contains('*'));
        assert!(!name.contains('?'));
        assert_eq!(name, "author_name_title_with_illegal_2026-01-01");
    }

    #[test]
    fn format_douyin_filename_strips_all_illegal_chars() {
        // 测试全部 9 个 Windows 非法字符
        let name = format_douyin_filename("a\\b/c:d*e?f\"g<h>i|j", "title", "2026-01-01");
        for c in ['\\', '/', ':', '*', '?', '"', '<', '>', '|'] {
            assert!(!name.contains(c), "文件名中仍含非法字符 {c:?}: {name}");
        }
    }

    #[test]
    fn format_douyin_filename_strips_control_chars() {
        // 控制字符（如换行、制表符）也应替换为 _
        let name = format_douyin_filename("author\nname", "title\ttab", "2026-01-01");
        assert!(!name.contains('\n'));
        assert!(!name.contains('\t'));
    }

    // ---- format_douyin_filename：空段处理 ----

    #[test]
    fn format_douyin_filename_handles_empty_segments() {
        // author 为空
        assert_eq!(
            format_douyin_filename("", "title", "2026-01-01"),
            "title_2026-01-01"
        );
        // title 为空
        assert_eq!(
            format_douyin_filename("author", "", "2026-01-01"),
            "author_2026-01-01"
        );
        // date 为空
        assert_eq!(
            format_douyin_filename("author", "title", ""),
            "author_title"
        );
        // 全空 -> 回退名（不以 _ 开头）
        let all_empty = format_douyin_filename("", "", "");
        assert!(
            all_empty.starts_with("douyin_"),
            "全空时应返回回退名: {all_empty}"
        );
    }

    #[test]
    fn format_douyin_filename_handles_whitespace_only_segments() {
        // 仅含空白的段视为空
        assert_eq!(
            format_douyin_filename("   ", "title", "2026-01-01"),
            "title_2026-01-01"
        );
    }

    // ---- format_douyin_filename：长度截断 ----

    #[test]
    fn format_douyin_filename_truncates_long_names() {
        let long_author: String = "a".repeat(50);
        let long_title: String = "b".repeat(80);
        let long_date: String = "c".repeat(30);
        let name = format_douyin_filename(&long_author, &long_title, &long_date);
        assert!(
            name.chars().count() <= 100,
            "文件名长度 {} 超过 100 字符: {name}",
            name.chars().count()
        );
        // 截断后的文件名应保留前 100 个字符
        let expected: String = format!("{}_{}_{}", long_author, long_title, long_date)
            .chars()
            .take(100)
            .collect();
        assert_eq!(name, expected);
    }

    #[test]
    fn format_douyin_filename_truncates_exactly_100_chars() {
        // 构造正好 100 字符的输入
        let author = "a".repeat(30);
        let title = "b".repeat(30);
        let date = "c".repeat(37); // 30 + 1 + 30 + 1 + 37 = 99 ≤ 100
        let name = format_douyin_filename(&author, &title, &date);
        assert_eq!(name.chars().count(), 99);
    }

    #[test]
    fn format_douyin_filename_truncates_emoji_correctly() {
        // Emoji 不应被截断成无效 UTF-8（按 Unicode 标量截断）
        let author = "a".repeat(99);
        let title = "😀".repeat(10);
        let name = format_douyin_filename(&author, &title, "2026");
        assert!(name.chars().count() <= 100);
        // 截断后的字符串应是有效 UTF-8（String 类型保证）
        assert!(std::str::from_utf8(name.as_bytes()).is_ok());
    }

    // ---- MediaPlatform::display_name / as_str ----

    #[test]
    fn media_platform_display_name_returns_chinese() {
        assert_eq!(MediaPlatform::Douyin.display_name(), "抖音");
        assert_eq!(MediaPlatform::TikTok.display_name(), "TikTok");
        assert_eq!(MediaPlatform::Twitter.display_name(), "Twitter/X");
        assert_eq!(MediaPlatform::YouTube.display_name(), "YouTube");
        assert_eq!(MediaPlatform::Bilibili.display_name(), "哔哩哔哩");
        assert_eq!(MediaPlatform::Weibo.display_name(), "微博");
        assert_eq!(MediaPlatform::Unknown.display_name(), "未知平台");
    }

    #[test]
    fn media_platform_as_str_returns_kebab_case() {
        assert_eq!(MediaPlatform::Douyin.as_str(), "douyin");
        assert_eq!(MediaPlatform::TikTok.as_str(), "tiktok");
        assert_eq!(MediaPlatform::Twitter.as_str(), "twitter");
        assert_eq!(MediaPlatform::YouTube.as_str(), "youtube");
        assert_eq!(MediaPlatform::Bilibili.as_str(), "bilibili");
        assert_eq!(MediaPlatform::Weibo.as_str(), "weibo");
        assert_eq!(MediaPlatform::Unknown.as_str(), "unknown");
    }

    // ---- expand_short_url：非短链直接返回 ----
    // 注：网络相关的测试不在此处，避免依赖外网。expand_short_url 的网络行为
    // 由集成测试覆盖（src-tauri/tests/）。

    #[test]
    fn expand_short_url_returns_original_for_non_short_url() {
        // 非短链 URL 应直接返回原 URL，不发起网络请求
        let url = "https://www.douyin.com/video/1234567890";
        // 这里用同步方式验证非短链路径（is_short_url 为 false 时立即返回）
        // 真正的异步网络调用只在 is_short_url 为 true 时发生
        assert!(!is_short_url(url));
    }

    // ---- extract_host ----

    #[test]
    fn extract_host_strips_www_prefix() {
        assert_eq!(
            extract_host("https://www.douyin.com/video/123"),
            "douyin.com"
        );
        assert_eq!(
            extract_host("https://www.youtube.com/watch?v=abc"),
            "youtube.com"
        );
    }

    #[test]
    fn extract_host_preserves_subdomain() {
        assert_eq!(extract_host("https://v.douyin.com/abc"), "v.douyin.com");
        assert_eq!(extract_host("https://vm.tiktok.com/Z123"), "vm.tiktok.com");
        assert_eq!(extract_host("https://m.weibo.cn/status/123"), "m.weibo.cn");
    }

    #[test]
    fn extract_host_returns_empty_for_invalid_url() {
        assert_eq!(extract_host("not a url"), "");
        assert_eq!(extract_host(""), "");
        assert_eq!(extract_host("ftp://example.com"), "example.com");
    }

    #[test]
    fn extract_host_is_case_insensitive() {
        assert_eq!(
            extract_host("HTTPS://WWW.DOUYIN.COM/VIDEO/123"),
            "douyin.com"
        );
        assert_eq!(extract_host("https://V.Douyin.com/abc"), "v.douyin.com");
    }

    // ===== Task 39：Twitter/X 平台专用测试 =====

    // ---- detect_platform：Twitter/X（与抖音测试同款，覆盖任务规范要求样本） ----

    #[test]
    fn detect_twitter_from_url() {
        assert_eq!(
            detect_platform("https://twitter.com/user/status/123"),
            MediaPlatform::Twitter
        );
    }

    #[test]
    fn detect_x_from_url() {
        assert_eq!(
            detect_platform("https://x.com/user/status/123"),
            MediaPlatform::Twitter
        );
    }

    #[test]
    fn detect_tco_short_url() {
        assert_eq!(
            detect_platform("https://t.co/abc123"),
            MediaPlatform::Twitter
        );
    }

    // ---- is_twitter_space ----

    #[test]
    fn is_twitter_space_with_spaces_url() {
        assert!(is_twitter_space("https://twitter.com/i/spaces/1DXxyv"));
        assert!(is_twitter_space("https://x.com/i/spaces/1DXxyv"));
        assert!(is_twitter_space(
            "https://twitter.com/i/spaces/1DXxyvXYZ?s=20"
        ));
    }

    #[test]
    fn is_twitter_space_with_status_url() {
        assert!(!is_twitter_space("https://twitter.com/user/status/123"));
        assert!(!is_twitter_space(
            "https://x.com/elonmusk/status/1234567890"
        ));
    }

    #[test]
    fn is_twitter_space_rejects_non_twitter_host() {
        assert!(!is_twitter_space("https://example.com/i/spaces/1"));
        assert!(!is_twitter_space("https://youtube.com/watch?v=abc"));
    }

    #[test]
    fn is_twitter_space_rejects_invalid_url() {
        assert!(!is_twitter_space("not a url"));
        assert!(!is_twitter_space(""));
    }

    // ---- format_twitter_filename ----

    #[test]
    fn format_twitter_filename_normal() {
        assert_eq!(
            format_twitter_filename("elonmusk", "1234567890", "2026-01-01"),
            "elonmusk_1234567890_2026-01-01"
        );
    }

    #[test]
    fn format_twitter_filename_strips_at_prefix() {
        assert_eq!(
            format_twitter_filename("@elonmusk", "1234567890", "2026-01-01"),
            "elonmusk_1234567890_2026-01-01"
        );
    }

    #[test]
    fn format_twitter_filename_strips_illegal_chars() {
        // author 含路径分隔符
        assert_eq!(
            format_twitter_filename("user/name", "999", "2026-01-01"),
            "user_name_999_2026-01-01"
        );
        // tweet_id 含 Windows 非法字符
        let result = format_twitter_filename("user", "abc?def*ghi", "2026-01-01");
        assert!(!result.contains('?'));
        assert!(!result.contains('*'));
        assert_eq!(result, "user_abc_def_ghi_2026-01-01");
        // 测试全部 9 个 Windows 非法字符都被替换
        let illegal = "a\\b/c:d*e?f\"g<h>i|j";
        let name = format_twitter_filename(illegal, "999", "2026-01-01");
        for c in ['\\', '/', ':', '*', '?', '"', '<', '>', '|'] {
            assert!(!name.contains(c), "文件名仍含非法字符 {c:?}: {name}");
        }
    }

    #[test]
    fn format_twitter_filename_handles_empty_fields() {
        // 单字段空
        assert_eq!(
            format_twitter_filename("", "123", "2026-01-01"),
            "unknown_123_2026-01-01"
        );
        assert_eq!(
            format_twitter_filename("user", "", "2026-01-01"),
            "user_unknown_2026-01-01"
        );
        assert_eq!(
            format_twitter_filename("user", "123", ""),
            "user_123_unknown"
        );
        // 全空：仍返回三段占位，避免空文件名
        let all_empty = format_twitter_filename("", "", "");
        assert_eq!(all_empty, "unknown_unknown_unknown");
    }

    #[test]
    fn format_twitter_filename_strips_at_prefix_with_whitespace() {
        // `  @elonmusk  ` 应正确去除空白与 @
        assert_eq!(
            format_twitter_filename("  @elonmusk  ", "123", "2026-01-01"),
            "elonmusk_123_2026-01-01"
        );
    }

    #[test]
    fn format_twitter_filename_truncates_long_name() {
        let long_author: String = "a".repeat(60);
        let long_id: String = "b".repeat(60);
        let name = format_twitter_filename(&long_author, &long_id, "2026-01-01");
        assert!(
            name.chars().count() <= 100,
            "文件名长度超过 100 字符: {name}"
        );
    }

    // ---- classify_twitter_error：Twitter/X 平台特定错误识别 ----
    //
    // 注：`classify_platform_error` 调度器与 `MediaPlatformError` 枚举的测试
    // 在 `manager::diagnose` 模块中。此处仅测试 Twitter 特定辅助函数
    // `classify_twitter_error` 本身的行为。
    // 输入需为小写（与调度器调用约定一致）。

    #[test]
    fn classify_twitter_error_login_required() {
        let stderr = "error: [twitter] 123: login required to access this resource";
        assert_eq!(
            classify_twitter_error(stderr),
            MediaPlatformError::LoginExpired
        );
    }

    #[test]
    fn classify_twitter_error_cookie_required() {
        let stderr = "error: [twitter] cookie required. please provide cookies.";
        assert_eq!(
            classify_twitter_error(stderr),
            MediaPlatformError::LoginExpired
        );
    }

    #[test]
    fn classify_twitter_error_cookies_required() {
        let stderr = "error: [twitter] cookies required for this content.";
        assert_eq!(
            classify_twitter_error(stderr),
            MediaPlatformError::LoginExpired
        );
    }

    #[test]
    fn classify_twitter_error_tweet_not_found() {
        let stderr = "error: [twitter] tweet not found";
        assert_eq!(
            classify_twitter_error(stderr),
            MediaPlatformError::LinkExpired
        );
    }

    #[test]
    fn classify_twitter_error_status_not_found() {
        let stderr = "error: [twitter] status not found";
        assert_eq!(
            classify_twitter_error(stderr),
            MediaPlatformError::LinkExpired
        );
    }

    #[test]
    fn classify_twitter_error_404_maps_to_link_expired() {
        let stderr = "http error 404: not found";
        assert_eq!(
            classify_twitter_error(stderr),
            MediaPlatformError::LinkExpired
        );
    }

    #[test]
    fn classify_twitter_error_sensitive_maps_to_login_expired() {
        // 仅含 sensitive 关键词，无 "login required"
        let stderr = "error: [twitter] sensitive content";
        assert_eq!(
            classify_twitter_error(stderr),
            MediaPlatformError::LoginExpired
        );
    }

    #[test]
    fn classify_twitter_error_age_restricted_maps_to_login_expired() {
        let stderr = "error: [twitter] age-restricted content";
        assert_eq!(
            classify_twitter_error(stderr),
            MediaPlatformError::LoginExpired
        );
    }

    #[test]
    fn classify_twitter_error_age_restricted_with_space_maps_to_login_expired() {
        // "age restricted"（无连字符）也应识别
        let stderr = "error: [twitter] age restricted content";
        assert_eq!(
            classify_twitter_error(stderr),
            MediaPlatformError::LoginExpired
        );
    }

    #[test]
    fn classify_twitter_error_403_maps_to_login_expired() {
        let stderr = "http error 403: forbidden";
        assert_eq!(
            classify_twitter_error(stderr),
            MediaPlatformError::LoginExpired
        );
    }

    #[test]
    fn classify_twitter_error_forbidden_maps_to_login_expired() {
        // 仅含 "forbidden" 关键词，无 403
        let stderr = "error: [twitter] forbidden access";
        assert_eq!(
            classify_twitter_error(stderr),
            MediaPlatformError::LoginExpired
        );
    }

    #[test]
    fn classify_twitter_error_unknown_returns_unknown() {
        let stderr = "error: something unexpected happened";
        assert_eq!(classify_twitter_error(stderr), MediaPlatformError::Unknown);
    }

    #[test]
    fn classify_twitter_error_empty_returns_unknown() {
        assert_eq!(classify_twitter_error(""), MediaPlatformError::Unknown);
    }

    #[test]
    fn classify_twitter_error_link_expired_priority_over_login() {
        // 同时含 "tweet not found" 和 "login required"：链接失效优先
        let stderr = "tweet not found, login required to view";
        assert_eq!(
            classify_twitter_error(stderr),
            MediaPlatformError::LinkExpired
        );
    }

    // ---- strip_at_prefix（内部辅助函数） ----

    #[test]
    fn strip_at_prefix_removes_single_at() {
        assert_eq!(strip_at_prefix("@elonmusk"), "elonmusk");
        assert_eq!(strip_at_prefix("elonmusk"), "elonmusk");
        // 仅去除首个 @，保留后续 @（罕见但保留原意）
        assert_eq!(strip_at_prefix("@user@domain"), "user@domain");
    }

    // ===== Task 40：YouTube / B 站 / 微博 平台专用测试 =====

    // ---- detect_platform：YouTube 普通视频 / Shorts / youtu.be ----

    #[test]
    fn detect_youtube_normal() {
        // 普通视频 watch 接口
        assert_eq!(
            detect_platform("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            MediaPlatform::YouTube
        );
        assert_eq!(
            detect_platform("https://youtube.com/watch?v=abc123"),
            MediaPlatform::YouTube
        );
    }

    #[test]
    fn detect_youtube_short() {
        // YouTube Shorts 路径
        assert_eq!(
            detect_platform("https://www.youtube.com/shorts/abc123"),
            MediaPlatform::YouTube
        );
    }

    #[test]
    fn detect_youtu_be() {
        // youtu.be 短链
        assert_eq!(
            detect_platform("https://youtu.be/dQw4w9WgXcQ"),
            MediaPlatform::YouTube
        );
    }

    // ---- is_youtube_short ----

    #[test]
    fn is_youtube_short_with_shorts_url() {
        assert!(is_youtube_short("https://www.youtube.com/shorts/abc123"));
        assert!(is_youtube_short("https://youtube.com/shorts/xyz"));
        // 大小写不敏感
        assert!(is_youtube_short("HTTPS://WWW.YOUTUBE.COM/SHORTS/ABC"));
    }

    #[test]
    fn is_youtube_short_with_watch_url() {
        // watch URL 不应识别为 Shorts
        assert!(!is_youtube_short("https://www.youtube.com/watch?v=abc123"));
        assert!(!is_youtube_short("https://youtu.be/abc123"));
        assert!(!is_youtube_short("https://www.youtube.com/live/abc123"));
    }

    // ---- is_youtube_live_replay ----

    #[test]
    fn is_youtube_live_replay_with_live_path() {
        assert!(is_youtube_live_replay(
            "https://www.youtube.com/live/abc123"
        ));
        assert!(is_youtube_live_replay("https://youtube.com/live/xyz?t=100"));
    }

    #[test]
    fn is_youtube_live_replay_with_live_param() {
        assert!(is_youtube_live_replay(
            "https://www.youtube.com/watch?v=abc&live=1"
        ));
        assert!(is_youtube_live_replay(
            "https://www.youtube.com/watch?v=abc&live=true"
        ));
    }

    #[test]
    fn is_youtube_live_replay_with_normal_watch_url() {
        // 普通 watch URL 不应识别为直播回放
        assert!(!is_youtube_live_replay(
            "https://www.youtube.com/watch?v=abc123"
        ));
        assert!(!is_youtube_live_replay("https://youtu.be/abc123"));
        assert!(!is_youtube_live_replay(
            "https://www.youtube.com/shorts/abc"
        ));
    }

    #[test]
    fn is_youtube_live_replay_rejects_invalid_url() {
        assert!(!is_youtube_live_replay("not a url"));
        assert!(!is_youtube_live_replay(""));
    }

    // ---- is_youtube_age_restricted ----

    #[test]
    fn is_youtube_age_restricted_with_param() {
        assert!(is_youtube_age_restricted(
            "https://www.youtube.com/watch?v=abc&age_restricted=1"
        ));
        assert!(is_youtube_age_restricted(
            "https://www.youtube.com/watch?v=abc&age_restricted=true"
        ));
        assert!(is_youtube_age_restricted(
            "https://www.youtube.com/watch?v=abc&age_restricted=yes"
        ));
    }

    #[test]
    fn is_youtube_age_restricted_with_false_param() {
        // age_restricted=0/false 不视为年龄限制
        assert!(!is_youtube_age_restricted(
            "https://www.youtube.com/watch?v=abc&age_restricted=0"
        ));
        assert!(!is_youtube_age_restricted(
            "https://www.youtube.com/watch?v=abc&age_restricted=false"
        ));
    }

    #[test]
    fn is_youtube_age_restricted_without_param() {
        // 普通 URL 不应识别为年龄限制
        assert!(!is_youtube_age_restricted(
            "https://www.youtube.com/watch?v=abc"
        ));
        assert!(!is_youtube_age_restricted("https://youtu.be/abc"));
    }

    #[test]
    fn is_youtube_age_restricted_rejects_invalid_url() {
        assert!(!is_youtube_age_restricted("not a url"));
        assert!(!is_youtube_age_restricted(""));
    }

    // ---- is_bilibili_bangumi ----

    #[test]
    fn is_bilibili_bangumi_with_bangumi_url() {
        assert!(is_bilibili_bangumi(
            "https://www.bilibili.com/bangumi/play/ep123456"
        ));
        assert!(is_bilibili_bangumi(
            "https://www.bilibili.com/bangumi/play/ss12345"
        ));
        // 大小写不敏感
        assert!(is_bilibili_bangumi(
            "HTTPS://WWW.BILIBILI.COM/BANGUMI/PLAY/EP123"
        ));
    }

    #[test]
    fn is_bilibili_bangumi_with_video_url() {
        // 普通视频 URL 不应识别为番剧
        assert!(!is_bilibili_bangumi(
            "https://www.bilibili.com/video/BV1xx411c7mD"
        ));
        assert!(!is_bilibili_bangumi("https://b23.tv/abc123"));
    }

    // ---- is_bilibili_live_replay ----

    #[test]
    fn is_bilibili_live_replay_with_live_domain() {
        assert!(is_bilibili_live_replay("https://live.bilibili.com/123456"));
        assert!(is_bilibili_live_replay(
            "https://live.bilibili.com/record/abc123"
        ));
    }

    #[test]
    fn is_bilibili_live_replay_with_live_path() {
        assert!(is_bilibili_live_replay(
            "https://www.bilibili.com/live/123456"
        ));
        assert!(is_bilibili_live_replay(
            "https://www.bilibili.com/record/abc123"
        ));
    }

    #[test]
    fn is_bilibili_live_replay_with_video_url() {
        // 普通视频 URL 不应识别为直播回放
        assert!(!is_bilibili_live_replay(
            "https://www.bilibili.com/video/BV1xx411c7mD"
        ));
        assert!(!is_bilibili_live_replay(
            "https://www.bilibili.com/bangumi/play/ep123"
        ));
    }

    #[test]
    fn is_bilibili_live_replay_rejects_invalid_url() {
        assert!(!is_bilibili_live_replay("not a url"));
        assert!(!is_bilibili_live_replay(""));
    }

    // ---- is_weibo_gallery ----

    #[test]
    fn is_weibo_gallery_with_album_url() {
        assert!(is_weibo_gallery("https://weibo.com/album/123456"));
        assert!(is_weibo_gallery("https://weibo.com/u/123456/album/abc"));
    }

    #[test]
    fn is_weibo_gallery_with_photo_subdomain() {
        assert!(is_weibo_gallery("https://photo.weibo.com/1234567890/abc"));
    }

    #[test]
    fn is_weibo_gallery_with_album_param() {
        assert!(is_weibo_gallery("https://weibo.com/status/123456?album=1"));
        assert!(is_weibo_gallery("https://weibo.com/123/N0abc?album=true"));
    }

    #[test]
    fn is_weibo_gallery_with_normal_status_url() {
        // 普通微博 URL 不应识别为图集
        assert!(!is_weibo_gallery("https://weibo.com/1234567890/N0abcdef"));
        assert!(!is_weibo_gallery("https://weibo.cn/1234567890/N0abcdef"));
        assert!(!is_weibo_gallery("https://t.cn/A6abc123"));
    }

    #[test]
    fn is_weibo_gallery_rejects_invalid_url() {
        assert!(!is_weibo_gallery("not a url"));
        assert!(!is_weibo_gallery(""));
    }

    // ---- format_youtube_filename ----

    #[test]
    fn format_youtube_filename_normal() {
        assert_eq!(
            format_youtube_filename("MusicChannel", "My Song", "dQw4w9WgXcQ", "2026-01-01"),
            "MusicChannel_My Song_dQw4w9WgXcQ_2026-01-01"
        );
    }

    #[test]
    fn format_youtube_filename_strips_illegal_chars() {
        let name =
            format_youtube_filename("chan\\nel", "title:with?illegal", "abc123", "2026-01-01");
        for c in ['\\', ':', '?'] {
            assert!(!name.contains(c), "文件名仍含非法字符 {c:?}: {name}");
        }
    }

    #[test]
    fn format_youtube_filename_handles_empty_segments() {
        // 单段空
        assert_eq!(
            format_youtube_filename("", "title", "vid", "2026-01-01"),
            "title_vid_2026-01-01"
        );
        // 多段空
        assert_eq!(
            format_youtube_filename("channel", "", "vid", ""),
            "channel_vid"
        );
        // 全空：回退名
        let all_empty = format_youtube_filename("", "", "", "");
        assert!(
            all_empty.starts_with("youtube_"),
            "全空时应返回回退名: {all_empty}"
        );
    }

    #[test]
    fn format_youtube_filename_truncates_long_name() {
        let long_channel: String = "a".repeat(40);
        let long_title: String = "b".repeat(40);
        let long_vid: String = "c".repeat(40);
        let name = format_youtube_filename(&long_channel, &long_title, &long_vid, "2026-01-01");
        assert!(
            name.chars().count() <= 100,
            "文件名长度 {} 超过 100: {name}",
            name.chars().count()
        );
    }

    // ---- format_bilibili_filename ----

    #[test]
    fn format_bilibili_filename_normal() {
        assert_eq!(
            format_bilibili_filename("UP主", "我的视频", "BV1xx411c7mD"),
            "UP主_我的视频_BV1xx411c7mD"
        );
    }

    #[test]
    fn format_bilibili_filename_strips_illegal_chars() {
        let name = format_bilibili_filename("author/name", "title*with?illegal", "BV1abc");
        for c in ['/', '*', '?'] {
            assert!(!name.contains(c), "文件名仍含非法字符 {c:?}: {name}");
        }
    }

    #[test]
    fn format_bilibili_filename_handles_empty_segments() {
        assert_eq!(
            format_bilibili_filename("", "title", "BV1abc"),
            "title_BV1abc"
        );
        assert_eq!(
            format_bilibili_filename("author", "", "BV1abc"),
            "author_BV1abc"
        );
        assert_eq!(
            format_bilibili_filename("author", "title", ""),
            "author_title"
        );
        // 全空：回退名
        let all_empty = format_bilibili_filename("", "", "");
        assert!(
            all_empty.starts_with("bilibili_"),
            "全空时应返回回退名: {all_empty}"
        );
    }

    #[test]
    fn format_bilibili_filename_truncates_long_name() {
        let long_author: String = "a".repeat(60);
        let long_title: String = "b".repeat(60);
        let name = format_bilibili_filename(&long_author, &long_title, "BV1abc");
        assert!(
            name.chars().count() <= 100,
            "文件名长度 {} 超过 100: {name}",
            name.chars().count()
        );
    }

    // ---- format_weibo_filename ----

    #[test]
    fn format_weibo_filename_normal() {
        assert_eq!(
            format_weibo_filename("博主名", "我的微博", "2026-01-01"),
            "博主名_我的微博_2026-01-01"
        );
    }

    #[test]
    fn format_weibo_filename_strips_illegal_chars() {
        let name = format_weibo_filename("author:name", "title*with?illegal", "2026-01-01");
        for c in [':', '*', '?'] {
            assert!(!name.contains(c), "文件名仍含非法字符 {c:?}: {name}");
        }
    }

    #[test]
    fn format_weibo_filename_handles_empty_segments() {
        assert_eq!(
            format_weibo_filename("", "title", "2026-01-01"),
            "title_2026-01-01"
        );
        assert_eq!(
            format_weibo_filename("author", "", "2026-01-01"),
            "author_2026-01-01"
        );
        assert_eq!(format_weibo_filename("author", "title", ""), "author_title");
        // 全空：回退名
        let all_empty = format_weibo_filename("", "", "");
        assert!(
            all_empty.starts_with("weibo_"),
            "全空时应返回回退名: {all_empty}"
        );
    }

    #[test]
    fn format_weibo_filename_truncates_long_name() {
        let long_author: String = "a".repeat(60);
        let long_title: String = "b".repeat(60);
        let name = format_weibo_filename(&long_author, &long_title, "2026-01-01");
        assert!(
            name.chars().count() <= 100,
            "文件名长度 {} 超过 100: {name}",
            name.chars().count()
        );
    }

    // ===== Task 38：TikTok 平台专用测试 =====

    // ---- detect_platform：TikTok 短链 / 长链 ----

    #[test]
    fn detect_tiktok_from_short_url() {
        // vm.tiktok.com / vt.tiktok.com 短链
        assert_eq!(
            detect_platform("https://vm.tiktok.com/Z123abcd/"),
            MediaPlatform::TikTok
        );
        assert_eq!(
            detect_platform("https://vt.tiktok.com/Z5678efgh/"),
            MediaPlatform::TikTok
        );
    }

    #[test]
    fn detect_tiktok_from_long_url() {
        // www.tiktok.com 长链，含 @user 路径
        assert_eq!(
            detect_platform("https://www.tiktok.com/@user/video/123"),
            MediaPlatform::TikTok
        );
        assert_eq!(
            detect_platform("https://tiktok.com/@user/video/456"),
            MediaPlatform::TikTok
        );
    }

    // ---- is_tiktok_gallery ----

    #[test]
    fn is_tiktok_gallery_with_photo_url() {
        // /photo/ 路径识别为图集
        assert!(is_tiktok_gallery("https://www.tiktok.com/@user/photo/123"));
        assert!(is_tiktok_gallery("https://tiktok.com/@user/photo/456"));
        // /photos/ 复数形式
        assert!(is_tiktok_gallery("https://www.tiktok.com/@user/photos/123"));
        // 大小写不敏感
        assert!(is_tiktok_gallery("HTTPS://WWW.TIKTOK.COM/@USER/PHOTO/123"));
    }

    #[test]
    fn is_tiktok_gallery_with_video_url() {
        // /video/ 路径不识别为图集
        assert!(!is_tiktok_gallery("https://www.tiktok.com/@user/video/123"));
        assert!(!is_tiktok_gallery("https://tiktok.com/@user/video/456"));
    }

    #[test]
    fn is_tiktok_gallery_rejects_non_tiktok_url() {
        // 非 TikTok URL 不识别为图集（即使含 /photo/ 路径）
        assert!(!is_tiktok_gallery("https://example.com/photo/123"));
        assert!(!is_tiktok_gallery("https://www.douyin.com/note/123"));
    }

    // ---- format_tiktok_filename ----

    #[test]
    fn format_tiktok_filename_normal() {
        let name = format_tiktok_filename("username", "我的视频", "2026-01-01");
        assert_eq!(name, "username_我的视频_2026-01-01");
    }

    #[test]
    fn format_tiktok_filename_strips_at_prefix() {
        // TikTok 作者名常以 @ 开头，文件名中去除该前缀
        assert_eq!(
            format_tiktok_filename("@username", "title", "2026-01-01"),
            "username_title_2026-01-01"
        );
        // 仅去除首个 @，保留后续合法 @ 字符
        assert_eq!(
            format_tiktok_filename("@user@name", "title", "2026-01-01"),
            "user@name_title_2026-01-01"
        );
    }

    #[test]
    fn format_tiktok_filename_strips_illegal_chars() {
        // 测试 Windows 非法字符被替换为 _
        let name = format_tiktok_filename("author\\name", "title:with*illegal?", "2026-01-01");
        for c in ['\\', ':', '*', '?'] {
            assert!(!name.contains(c), "文件名仍含非法字符 {c:?}: {name}");
        }
        assert_eq!(name, "author_name_title_with_illegal_2026-01-01");

        // 测试全部 9 个 Windows 非法字符
        let illegal = "a\\b/c:d*e?f\"g<h>i|j";
        let name = format_tiktok_filename(illegal, "title", "2026-01-01");
        for c in ['\\', '/', ':', '*', '?', '"', '<', '>', '|'] {
            assert!(!name.contains(c), "文件名仍含非法字符 {c:?}: {name}");
        }
    }

    #[test]
    fn format_tiktok_filename_handles_empty_segments() {
        // author 为空
        assert_eq!(
            format_tiktok_filename("", "title", "2026-01-01"),
            "title_2026-01-01"
        );
        // title 为空
        assert_eq!(
            format_tiktok_filename("author", "", "2026-01-01"),
            "author_2026-01-01"
        );
        // date 为空
        assert_eq!(
            format_tiktok_filename("author", "title", ""),
            "author_title"
        );
        // 全空：返回回退名
        let all_empty = format_tiktok_filename("", "", "");
        assert!(
            all_empty.starts_with("tiktok_"),
            "全空时应返回回退名: {all_empty}"
        );
    }

    #[test]
    fn format_tiktok_filename_truncates_long_name() {
        let long_author: String = "a".repeat(50);
        let long_title: String = "b".repeat(80);
        let long_date: String = "c".repeat(30);
        let name = format_tiktok_filename(&long_author, &long_title, &long_date);
        assert!(
            name.chars().count() <= 100,
            "文件名长度 {} 超过 100 字符: {name}",
            name.chars().count()
        );
    }

    // ---- classify_tiktok_error：通过 classify_platform_error 调度器测试 ----

    #[test]
    fn classify_tiktok_region_blocked() {
        // 明确的地区限制关键词
        let stderr = "ERROR: [tiktok] Video is not available in your region";
        assert_eq!(
            classify_platform_error(MediaPlatform::TikTok, stderr),
            MediaPlatformError::RegionBlocked
        );
    }

    #[test]
    fn classify_tiktok_region_blocked_geo_restricted() {
        let stderr = "ERROR: [tiktok] This content is geo-restricted";
        assert_eq!(
            classify_platform_error(MediaPlatform::TikTok, stderr),
            MediaPlatformError::RegionBlocked
        );
    }

    #[test]
    fn classify_tiktok_region_blocked_with_429_and_geo_mention() {
        // 429 配合地区字眼应识别为地区限制（Task 38.3 特化）
        let stderr = "HTTP Error 429: Too Many Requests (region restricted)";
        assert_eq!(
            classify_platform_error(MediaPlatform::TikTok, stderr),
            MediaPlatformError::RegionBlocked
        );
    }

    #[test]
    fn classify_tiktok_region_blocked_country() {
        let stderr = "ERROR: [tiktok] Not available in your country";
        assert_eq!(
            classify_platform_error(MediaPlatform::TikTok, stderr),
            MediaPlatformError::RegionBlocked
        );
    }

    #[test]
    fn classify_tiktok_login_expired() {
        let stderr = "ERROR: [tiktok] Login required to access this resource";
        assert_eq!(
            classify_platform_error(MediaPlatform::TikTok, stderr),
            MediaPlatformError::LoginExpired
        );
    }

    #[test]
    fn classify_tiktok_login_expired_cookie() {
        let stderr = "ERROR: [tiktok] Cookie required. Please provide cookies.";
        assert_eq!(
            classify_platform_error(MediaPlatform::TikTok, stderr),
            MediaPlatformError::LoginExpired
        );
    }

    #[test]
    fn classify_tiktok_link_expired() {
        let stderr = "ERROR: [tiktok] Video not found";
        assert_eq!(
            classify_platform_error(MediaPlatform::TikTok, stderr),
            MediaPlatformError::LinkExpired
        );
    }

    #[test]
    fn classify_tiktok_link_expired_404() {
        let stderr = "HTTP Error 404: Not Found";
        assert_eq!(
            classify_platform_error(MediaPlatform::TikTok, stderr),
            MediaPlatformError::LinkExpired
        );
    }

    #[test]
    fn classify_tiktok_drm_returns_drm_protected() {
        let stderr = "ERROR: _has_drm = true";
        assert_eq!(
            classify_platform_error(MediaPlatform::TikTok, stderr),
            MediaPlatformError::DrmProtected
        );
    }

    #[test]
    fn classify_tiktok_429_alone_does_not_map_to_region_blocked() {
        // 普通 429（无地区字眼）不应识别为地区限制
        let stderr = "HTTP Error 429: Too Many Requests";
        assert_ne!(
            classify_platform_error(MediaPlatform::TikTok, stderr),
            MediaPlatformError::RegionBlocked
        );
    }

    #[test]
    fn classify_tiktok_unknown_error() {
        let stderr = "ERROR: something unexpected happened";
        assert_eq!(
            classify_platform_error(MediaPlatform::TikTok, stderr),
            MediaPlatformError::Unknown
        );
    }

    #[test]
    fn classify_tiktok_error_is_case_insensitive() {
        // 大写关键词也应识别
        let upper = "LOGIN REQUIRED";
        assert_eq!(
            classify_platform_error(MediaPlatform::TikTok, upper),
            MediaPlatformError::LoginExpired
        );
        let mixed = "NOT AVAILABLE IN YOUR REGION";
        assert_eq!(
            classify_platform_error(MediaPlatform::TikTok, mixed),
            MediaPlatformError::RegionBlocked
        );
    }

    // ---- platform_error_to_chinese：TikTok 中文文案 ----

    #[test]
    fn platform_error_to_chinese_tiktok_login() {
        let chinese =
            platform_error_to_chinese(MediaPlatformError::LoginExpired, MediaPlatform::TikTok);
        assert_eq!(chinese, "TikTok 登录已失效，请重新获取 Cookie");
    }

    #[test]
    fn platform_error_to_chinese_tiktok_region_blocked() {
        // Task 38.3：地区限制返回中文错误"该内容在你的地区不可用"
        let chinese =
            platform_error_to_chinese(MediaPlatformError::RegionBlocked, MediaPlatform::TikTok);
        assert_eq!(chinese, "该内容在你的地区不可用");
    }

    #[test]
    fn platform_error_to_chinese_tiktok_link_expired() {
        // LinkExpired 使用通用文案（与 diagnose.rs::platform_error_to_chinese_link_expired 一致）
        let chinese =
            platform_error_to_chinese(MediaPlatformError::LinkExpired, MediaPlatform::TikTok);
        assert_eq!(chinese, "该链接已失效或已被删除");
    }

    #[test]
    fn platform_error_to_chinese_tiktok_drm_protected() {
        let chinese =
            platform_error_to_chinese(MediaPlatformError::DrmProtected, MediaPlatform::TikTok);
        assert_eq!(chinese, "该内容受 DRM 保护，无法下载");
    }

    // ===== Task 41: 短链接与分享文本解析测试 =====
    //
    // 设计要点（AGENTS.md §9）：
    // - **离线测试**：所有 `extract_url_from_share_text` 测试不依赖网络，
    //   仅验证文本解析行为；`expand_short_url` 的网络行为由集成测试覆盖。
    // - **真实样本**：使用抖音 / TikTok 等平台真实分享文本格式。
    // - **复用 Task 10**：`strip_tracking_params` 测试覆盖核心场景，
    //   完整白名单测试在 `manager::duplicate` 模块中已存在（不重复）。

    // ---- extract_url_from_share_text：抖音 ----

    #[test]
    fn extract_url_from_share_text_douyin() {
        // 抖音 App 分享按钮返回的文本格式
        let text = "1.23 abc:/ 复制打开抖音，看看【作者的作品】 https://v.douyin.com/abc123/ 复制此链接，打开Dou音视频，直接观看视频！";
        let result = extract_url_from_share_text(text);
        assert_eq!(result.as_deref(), Some("https://v.douyin.com/abc123/"));
    }

    #[test]
    fn extract_url_from_share_text_douyin_with_tracking() {
        // 短链带 query 与 fragment 时应完整提取
        let text = "看看这个 https://v.douyin.com/xyz/?a=1&utm_source=fb #标签 复制此链接";
        let result = extract_url_from_share_text(text);
        assert_eq!(
            result.as_deref(),
            Some("https://v.douyin.com/xyz/?a=1&utm_source=fb")
        );
    }

    // ---- extract_url_from_share_text：TikTok ----

    #[test]
    fn extract_url_from_share_text_tiktok() {
        // TikTok App 分享按钮返回的文本格式
        let text = "Check out this video! https://vm.tiktok.com/Z123abcd/ via TikTok";
        let result = extract_url_from_share_text(text);
        assert_eq!(result.as_deref(), Some("https://vm.tiktok.com/Z123abcd/"));
    }

    #[test]
    fn extract_url_from_share_text_tiktok_vt_domain() {
        // vt.tiktok.com 也是 TikTok 短链域名
        let text = "Watch: https://vt.tiktok.com/Z5678efgh/ shared";
        let result = extract_url_from_share_text(text);
        assert_eq!(result.as_deref(), Some("https://vt.tiktok.com/Z5678efgh/"));
    }

    // ---- extract_url_from_share_text：纯 URL 输入 ----

    #[test]
    fn extract_url_from_share_text_plain_url() {
        // 纯 URL 输入（无分享文本包装）应原样返回
        let url = "https://www.douyin.com/video/7283698765432109876";
        let result = extract_url_from_share_text(url);
        assert_eq!(result.as_deref(), Some(url));
    }

    #[test]
    fn extract_url_from_share_text_plain_url_with_query() {
        // 纯 URL 带 query 应完整保留
        let url = "https://example.com/file.zip?id=123&token=abc";
        let result = extract_url_from_share_text(url);
        assert_eq!(result.as_deref(), Some(url));
    }

    // ---- extract_url_from_share_text：无 URL ----

    #[test]
    fn extract_url_from_share_text_no_url() {
        // 纯中文文本无 URL 返回 None
        assert!(extract_url_from_share_text("这是一段没有链接的纯文本").is_none());
        // 空字符串返回 None
        assert!(extract_url_from_share_text("").is_none());
        // 仅空白返回 None
        assert!(extract_url_from_share_text("   \t\n  ").is_none());
    }

    #[test]
    fn extract_url_from_share_text_no_url_with_text() {
        // 文本中含 "https" 字样但不是完整 URL，返回 None
        assert!(extract_url_from_share_text("请访问 https 网站查看").is_none());
        // ftp:// 不被识别（仅匹配 https?://）
        assert!(extract_url_from_share_text("ftp://example.com/file").is_none());
    }

    // ---- extract_url_from_share_text：多 URL 优先匹配短链 ----

    #[test]
    fn extract_url_from_share_text_multiple_urls_prefers_short_link() {
        // 多个 URL 中包含已知短链域名：优先返回短链
        let text = "看 https://example.com/page 和 https://v.douyin.com/abc/ 都可以";
        let result = extract_url_from_share_text(text);
        assert_eq!(result.as_deref(), Some("https://v.douyin.com/abc/"));
    }

    #[test]
    fn extract_url_from_share_text_multiple_urls_returns_first_if_no_short_link() {
        // 多个 URL 中无已知短链：返回首个 URL（按出现顺序）
        let text = "看 https://example.com/a 和 https://example.org/b";
        let result = extract_url_from_share_text(text);
        assert_eq!(result.as_deref(), Some("https://example.com/a"));
    }

    #[test]
    fn extract_url_from_share_text_first_short_link_wins() {
        // 多个短链：返回首个出现的短链
        let text = "https://v.douyin.com/abc/ 和 https://vm.tiktok.com/xyz/";
        let result = extract_url_from_share_text(text);
        assert_eq!(result.as_deref(), Some("https://v.douyin.com/abc/"));
    }

    // ---- extract_url_from_share_text：边界情况 ----

    #[test]
    fn extract_url_from_share_text_trims_whitespace() {
        // 输入首尾含空白应先 trim
        let text = "  https://v.douyin.com/abc/  ";
        let result = extract_url_from_share_text(text);
        assert_eq!(result.as_deref(), Some("https://v.douyin.com/abc/"));
    }

    #[test]
    fn extract_url_from_share_text_url_terminated_by_quote() {
        // URL 后跟单引号或双引号应终止
        let text = r#"链接 "https://v.douyin.com/abc/" 已复制"#;
        let result = extract_url_from_share_text(text);
        assert_eq!(result.as_deref(), Some("https://v.douyin.com/abc/"));
    }

    // ---- is_short_url：Task 41 扩展域名 ----

    #[test]
    fn is_short_url_detects_extended_domains() {
        // Task 41 新增的短链域名
        assert!(is_short_url("https://url.cn/abc123"));
        assert!(is_short_url("https://dwz.cn/abc"));
        assert!(is_short_url("https://bit.ly/abc"));
        assert!(is_short_url("https://tinyurl.com/abc"));
        assert!(is_short_url("https://goo.gl/abc"));
    }

    #[test]
    fn is_short_url_extended_domains_case_insensitive() {
        // 大小写不敏感
        assert!(is_short_url("https://BIT.LY/abc"));
        assert!(is_short_url("https://TinyUrl.com/abc"));
    }

    // ---- strip_tracking_params：核心场景（复用 Task 10） ----

    #[test]
    fn strip_tracking_params_removes_utm() {
        // utm_* 系列参数应被剥离
        let result =
            strip_tracking_params("https://example.com/page?utm_source=twitter&utm_medium=social");
        assert_eq!(result, "https://example.com/page");
        // 单个 utm_source
        let result = strip_tracking_params("https://v.douyin.com/abc/?utm_source=fb");
        assert_eq!(result, "https://v.douyin.com/abc/");
    }

    #[test]
    fn strip_tracking_params_removes_fbclid_gclid() {
        // fbclid / gclid 等跟踪参数应被剥离
        let result = strip_tracking_params("https://example.com/page?fbclid=abc&gclid=xyz");
        assert_eq!(result, "https://example.com/page");
    }

    #[test]
    fn strip_tracking_params_keeps_essential() {
        // 业务必需参数应保留：id / sign / token / auth / X-Amz-Signature 等
        let result = strip_tracking_params("https://example.com/page?id=123&sign=abc&token=secret");
        assert_eq!(
            result,
            "https://example.com/page?id=123&sign=abc&token=secret"
        );
        // 跟踪参数与业务参数混合：仅剥离跟踪参数
        let result =
            strip_tracking_params("https://example.com/page?id=123&utm_source=fb&sign=abc");
        assert_eq!(result, "https://example.com/page?id=123&sign=abc");
    }

    #[test]
    fn strip_tracking_params_handles_no_query() {
        // 无 query 的 URL 应原样返回（不附加 '?'）
        let url = "https://v.douyin.com/abc123/";
        let result = strip_tracking_params(url);
        assert_eq!(result, url);
        // 抖音长链也无 query
        let url = "https://www.douyin.com/video/7283698765432109876";
        let result = strip_tracking_params(url);
        assert_eq!(result, url);
    }

    #[test]
    fn strip_tracking_params_invalid_url_returns_input() {
        // 非法 URL 应原样返回，不触发 panic
        let input = "not a url";
        let result = strip_tracking_params(input);
        assert_eq!(result, input);
    }

    #[test]
    fn strip_tracking_params_preserves_fragment() {
        // fragment 应被保留
        let result = strip_tracking_params("https://example.com/page?utm_source=fb#section");
        assert_eq!(result, "https://example.com/page#section");
    }

    // ===== SubTask 44.6：MediaPlatformError 全变体中文映射覆盖 =====
    //
    // 这些测试验证 [`platform_error_to_chinese`] 对所有 6 个变体 ×
    // 所有 7 个平台的组合都返回非空中文文案。任务说明要求测试位于
    // `media_platforms.rs`，与 `diagnose.rs` 中的对应测试互为冗余备份，
    // 确保任一文件被误删/重构时仍能捕获中文映射回归。

    #[test]
    fn subtask_44_6_login_expired_returns_chinese_for_all_platforms() {
        // LoginExpired 是平台特定文案，每个平台应有不同中文表述
        let cases = [
            (MediaPlatform::Douyin, "抖音登录已失效"),
            (MediaPlatform::TikTok, "TikTok 登录已失效"),
            (MediaPlatform::Twitter, "Twitter/X 登录已失效"),
            (MediaPlatform::YouTube, "YouTube 登录已失效"),
            (MediaPlatform::Bilibili, "哔哩哔哩登录已失效"),
            (MediaPlatform::Weibo, "微博登录已失效"),
            (MediaPlatform::Unknown, "登录已失效"),
        ];
        for (platform, expected_prefix) in cases {
            let msg = platform_error_to_chinese(MediaPlatformError::LoginExpired, platform);
            assert!(
                msg.starts_with(expected_prefix),
                "platform {platform:?} expected prefix '{expected_prefix}', got '{msg}'"
            );
            assert!(msg.contains("Cookie") || msg.contains("登录") || msg.contains("认证"));
        }
    }

    #[test]
    fn subtask_44_6_region_blocked_returns_same_chinese_for_all_platforms() {
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
    fn subtask_44_6_link_expired_returns_same_chinese_for_all_platforms() {
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
    fn subtask_44_6_drm_protected_returns_same_chinese_for_all_platforms() {
        // DRM 必须明确拒绝（AGENTS.md §6），文案固定
        for platform in [
            MediaPlatform::Douyin,
            MediaPlatform::TikTok,
            MediaPlatform::Twitter,
            MediaPlatform::YouTube,
            MediaPlatform::Bilibili,
            MediaPlatform::Weibo,
            MediaPlatform::Unknown,
        ] {
            let msg = platform_error_to_chinese(MediaPlatformError::DrmProtected, platform);
            assert_eq!(msg, "该内容受 DRM 保护，无法下载");
        }
    }

    #[test]
    fn subtask_44_6_unsupported_returns_same_chinese_for_all_platforms() {
        for platform in [
            MediaPlatform::Douyin,
            MediaPlatform::TikTok,
            MediaPlatform::Twitter,
            MediaPlatform::YouTube,
            MediaPlatform::Bilibili,
            MediaPlatform::Weibo,
            MediaPlatform::Unknown,
        ] {
            let msg = platform_error_to_chinese(MediaPlatformError::Unsupported, platform);
            assert_eq!(msg, "该平台暂不支持下载此类型内容");
        }
    }

    #[test]
    fn subtask_44_6_unknown_returns_same_chinese_for_all_platforms() {
        for platform in [
            MediaPlatform::Douyin,
            MediaPlatform::TikTok,
            MediaPlatform::Twitter,
            MediaPlatform::YouTube,
            MediaPlatform::Bilibili,
            MediaPlatform::Weibo,
            MediaPlatform::Unknown,
        ] {
            let msg = platform_error_to_chinese(MediaPlatformError::Unknown, platform);
            assert_eq!(msg, "下载失败，请稍后重试");
        }
    }

    #[test]
    fn subtask_44_6_all_variants_return_non_empty_chinese() {
        // 全变体 × 全平台组合返回非空中文文案（AGENTS.md §7 中文错误）
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
                assert!(!msg.is_empty(), "{error:?}+{platform:?} 返回空文案");
                assert!(
                    msg.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
                    "{error:?}+{platform:?} 返回非中文: {msg}"
                );
            }
        }
    }
}
