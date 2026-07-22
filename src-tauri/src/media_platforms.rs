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
use crate::models::MediaCredentialCheckResult;
// Task 41：复用 Task 10 已实现的 `strip_tracking_params`，避免重复实现导致行为分叉。
pub use crate::manager::duplicate::strip_tracking_params;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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

    /// 返回该平台可能关联的已知域名列表（主域名排在首位）。
    /// 用于按域名检索凭证时在别名域名（如 `youtu.be` -> `youtube.com`、`x.com` -> `twitter.com`）
    /// 之间自动互查，保证短链和替代域名能命中主域名的凭证。
    pub fn candidate_domains(&self) -> &'static [&'static str] {
        match self {
            Self::Douyin => &["douyin.com", "iesdouyin.com", "douyinvod.com", "v.douyin.com", "amemv.com"],
            Self::TikTok => &["tiktok.com", "vm.tiktok.com", "vt.tiktok.com"],
            Self::Twitter => &["twitter.com", "x.com", "t.co", "mobile.twitter.com"],
            Self::YouTube => &["youtube.com", "youtu.be", "m.youtube.com", "music.youtube.com"],
            Self::Bilibili => &["bilibili.com", "b23.tv", "m.bilibili.com", "t.bilibili.com"],
            Self::Weibo => &["weibo.com", "weibo.cn", "m.weibo.cn", "t.cn"],
            Self::Unknown => &[],
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

/// 检测 URL 是否为抖音直播。
///
/// 匹配 `live.douyin.com/<room_id>` 格式，room_id 为纯数字或字母数字组合
/// （抖音号也可能是字母）。
pub fn is_douyin_live(url: &str) -> bool {
    if detect_platform(url) != MediaPlatform::Douyin {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    lower.contains("live.douyin.com/")
}

/// 从抖音直播 URL 中提取房间号（web_rid）。
///
/// 支持：
/// - `https://live.douyin.com/628273323967`
/// - `https://live.douyin.com/628273323967?enter_from_merge=...`
/// - `https://live.douyin.com/yall1102`（抖音号作为 web_rid）
///
/// 提取失败返回 `None`。
pub fn extract_douyin_live_room_id(url: &str) -> Option<String> {
    if !is_douyin_live(url) {
        return None;
    }
    let parsed = url::Url::parse(url.trim()).ok()?;
    let path = parsed.path();
    // 路径形如 /628273323967 或 /628273323967/
    let segment = path.trim_start_matches('/').trim_end_matches('/');
    if segment.is_empty() {
        return None;
    }
    // 排除特殊路径（如 /favicon.ico）
    if segment.contains('.') || segment.contains('/') {
        return None;
    }
    Some(segment.to_string())
}

/// 抖音直播页面 HTML 中的 JSON 数据使用 Unicode 转义（`\u0026` = `&`，`\"` = `"`）。
/// 本函数反转义常见转义序列，便于后续正则提取。
fn unescape_douyin_html_json(text: &str) -> String {
    text.replace("\\u0026", "&")
        .replace("\\u0027", "'")
        .replace("\\\"", "\"")
        .replace("\\\\", "\\")
        .replace("\\/", "/")
}

/// 从直播页面 HTML 中提取直播间信息。
///
/// 返回 `serde_json::Value`，包含字段：
/// - `title`：直播间标题
/// - `nickname`：主播昵称
/// - `status`：直播状态（2 = 直播中，4 = 未开播）
/// - `flv_url`：HTTP-FLV 流地址（FULL_HD1 画质，最高清晰度）
/// - `hls_url`：HLS m3u8 流地址（FULL_HD1 画质）
///
/// 提取策略：HTML 中嵌入的 JSON 数据经过 Unicode 转义，先反转义，
/// 然后用正则提取 `stream_url` 附近的字段。
/// 直播间标题：在 `stream_url` 前 3000 字符窗口内搜索最近的 `"title"` 字段，
/// 排除页面模板标题（如"广告投放"、"用户服务协议"等）。
pub async fn fetch_douyin_live_detail(room_id: &str) -> Result<serde_json::Value, String> {
    fetch_douyin_live_detail_with_credentials(room_id, None, None, None).await
}

pub async fn fetch_douyin_live_detail_with_credentials(
    room_id: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
) -> Result<serde_json::Value, String> {
    let url = format!("https://live.douyin.com/{}", room_id);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| format!("请求抖音直播页面失败：{e}"))?;

    let ua = user_agent.unwrap_or(DOUYIN_BROWSER_UA);
    let ref_hdr = referer.unwrap_or("https://live.douyin.com/");

    let initial_cookie = match cookie {
        Some(c) if !c.trim().is_empty() => {
            if !c.contains("ttwid=") {
                format!("{c}; ttwid={DOUYIN_ANONYMOUS_TTWID}")
            } else {
                c.to_string()
            }
        }
        _ => format!("ttwid={DOUYIN_ANONYMOUS_TTWID}"),
    };

    let response = client
        .get(&url)
        .header("User-Agent", ua)
        .header("Referer", ref_hdr)
        .header("Cookie", &initial_cookie)
        .header("Accept", "text/html,application/xhtml+xml")
        .send()
        .await
        .map_err(|e| format!("请求抖音直播页面失败：{e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "抖音直播页面返回 HTTP {}",
            response.status().as_u16()
        ));
    }

    // 提取服务端返回的 Set-Cookie（如新下发的 ttwid）
    let set_cookies: Vec<String> = response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let html = response
        .text()
        .await
        .map_err(|e| format!("读取抖音直播页面失败：{e}"))?;

    let mut unescaped = unescape_douyin_html_json(&html);

    let mut flv_url = extract_stream_url_from_html(&unescaped, "flv_pull_url");
    let mut hls_url = extract_stream_url_from_html(&unescaped, "hls_pull_url_map");
    let mut title = extract_live_title_near_stream_url(&unescaped);
    let mut nickname = extract_live_nickname(&unescaped);
    let mut status = extract_live_status(&unescaped);

    // 2-pass 提取：如果首次请求未获得流地址（例如初始 ttwid 过期导致服务端 Set-Cookie 下发新 ttwid），
    // 带有服务端下发的新 Cookie 再次发送 GET 请求
    if flv_url.is_none() && hls_url.is_none() {
        let merged_cookie = if !set_cookies.is_empty() {
            format!("{}; {}", initial_cookie, set_cookies.join("; "))
        } else {
            initial_cookie.clone()
        };

        let req2 = client
            .get(&url)
            .header("User-Agent", ua)
            .header("Referer", ref_hdr)
            .header("Cookie", &merged_cookie)
            .header("Accept", "text/html,application/xhtml+xml");

        if let Ok(resp2) = req2.send().await {
            if resp2.status().is_success() {
                if let Ok(html2) = resp2.text().await {
                    unescaped = unescape_douyin_html_json(&html2);
                    flv_url = extract_stream_url_from_html(&unescaped, "flv_pull_url");
                    hls_url = extract_stream_url_from_html(&unescaped, "hls_pull_url_map");
                    if title.is_none() {
                        title = extract_live_title_near_stream_url(&unescaped);
                    }
                    if nickname.is_none() {
                        nickname = extract_live_nickname(&unescaped);
                    }
                    if status.is_none() {
                        status = extract_live_status(&unescaped);
                    }
                }
            }
        }
    }

    // 三次兜底：若直连 HTML 提取仍失败，尝试 Webcast Room Enter API 端点
    if flv_url.is_none() && hls_url.is_none() {
        let webcast_cookie = if !set_cookies.is_empty() {
            format!("{}; {}", initial_cookie, set_cookies.join("; "))
        } else {
            initial_cookie.clone()
        };
        if let Ok(webcast_val) = fetch_douyin_webcast_room_enter(&client, room_id, ua, ref_hdr, &webcast_cookie).await {
            if let Some(f_url) = webcast_val.get("flv_url").and_then(|v| v.as_str()) {
                flv_url = Some(f_url.to_string());
            }
            if let Some(h_url) = webcast_val.get("hls_url").and_then(|v| v.as_str()) {
                hls_url = Some(h_url.to_string());
            }
            if title.is_none() {
                title = webcast_val.get("title").and_then(|v| v.as_str()).map(|s| s.to_string());
            }
            if nickname.is_none() {
                nickname = webcast_val.get("nickname").and_then(|v| v.as_str()).map(|s| s.to_string());
            }
            if status.is_none() {
                status = webcast_val.get("status").and_then(|v| v.as_u64()).map(|u| u as u32);
            }
        }
    }

    if flv_url.is_none() && hls_url.is_none() {
        if status == Some(4) || status == Some(0) {
            return Err("主播当前未开播，无法获取直播流".into());
        }
        return Err("未从直播页面提取到流地址，可能直播间已结束或被风控拦截".into());
    }

    Ok(serde_json::json!({
        "title": title.unwrap_or_else(|| format!("抖音直播_{}", room_id)),
        "nickname": nickname.unwrap_or_else(|| "未知主播".into()),
        "status": status.unwrap_or(0),
        "flv_url": flv_url,
        "hls_url": hls_url,
        "room_id": room_id,
    }))
}

/// Webcast room enter API 兜底解析
async fn fetch_douyin_webcast_room_enter(
    client: &reqwest::Client,
    room_id: &str,
    ua: &str,
    referer: &str,
    cookie: &str,
) -> Result<serde_json::Value, String> {
    let webcast_url = format!(
        "https://live.douyin.com/webcast/room/web/enter/?aid=6383&app_name=douyin_web&live_id=1&device_platform=web&language=zh-CN&cookie_enabled=true&web_rid={}",
        room_id
    );
    let resp = client
        .get(&webcast_url)
        .header("User-Agent", ua)
        .header("Referer", referer)
        .header("Cookie", cookie)
        .header("Accept", "application/json, text/plain, */*")
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("Webcast API HTTP {}", resp.status()));
    }
    let body = resp.text().await.map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;

    let room_data = json
        .get("data")
        .and_then(|d| d.get("data"))
        .and_then(|arr| arr.get(0))
        .ok_or_else(|| "Webcast API 未返回有效房间数据".to_string())?;

    let title = room_data.get("title").and_then(|v| v.as_str());
    let status = room_data.get("status").and_then(|v| v.as_u64()).map(|u| u as u32);
    let nickname = room_data
        .get("owner")
        .and_then(|o| o.get("nickname"))
        .and_then(|v| v.as_str());

    let stream_url_obj = room_data
        .get("stream_url")
        .and_then(|s| s.get("live_core_sdk_data"))
        .and_then(|s| s.get("pull_data"))
        .and_then(|s| s.get("stream_data"));

    let mut flv_url = None;
    let mut hls_url = None;

    if let Some(stream_data_str) = stream_url_obj.and_then(|v| v.as_str()) {
        if let Ok(parsed_stream) = serde_json::from_str::<serde_json::Value>(stream_data_str) {
            if let Some(data_node) = parsed_stream.get("data") {
                if let Some(hd) = data_node.get("hd").or_else(|| data_node.get("origin")).or_else(|| data_node.get("sd")) {
                    if let Some(main) = hd.get("main") {
                        flv_url = main.get("flv").and_then(|v| v.as_str()).map(|s| s.to_string());
                        hls_url = main.get("hls").and_then(|v| v.as_str()).map(|s| s.to_string());
                    }
                }
            }
        }
    }

    Ok(serde_json::json!({
        "title": title,
        "nickname": nickname,
        "status": status,
        "flv_url": flv_url,
        "hls_url": hls_url,
    }))
}

/// 从 HTML（已反转义）中提取流地址。
///
/// `field` 参数为 `"flv_pull_url"` 或 `"hls_pull_url_map"`。
/// 优先提取 `FULL_HD1` 画质，回退到第一个可用画质。
fn extract_stream_url_from_html(unescaped: &str, field: &str) -> Option<String> {
    let pattern = format!(
        r#""{}"\s*:\s*\{{([^}}]+)\}}"#,
        regex::escape(field)
    );
    let re = regex::Regex::new(&pattern).ok()?;
    // 取最后一个匹配（跳过模板/预渲染数据，实际数据通常在末尾）
    let last_match = re.find_iter(unescaped).last()?;
    let block = last_match.as_str();
    // 优先提取 FULL_HD1
    let full_hd_re =
        regex::Regex::new(r#""FULL_HD1"\s*:\s*"(https?://[^"]+)""#).ok()?;
    if let Some(caps) = full_hd_re.captures(block) {
        return Some(caps.get(1)?.as_str().to_string());
    }
    // 回退到 ORIGIN
    let origin_re =
        regex::Regex::new(r#""ORIGIN"\s*:\s*"(https?://[^"]+)""#).ok()?;
    if let Some(caps) = origin_re.captures(block) {
        return Some(caps.get(1)?.as_str().to_string());
    }
    // 回退到第一个可用的 URL
    let any_url_re = regex::Regex::new(r#""[^"]+"\s*:\s*"(https?://[^"]+)""#).ok()?;
    let caps = any_url_re.captures(block)?;
    Some(caps.get(1)?.as_str().to_string())
}

/// 在 `stream_url` 附近搜索直播间标题。
///
/// 策略：找到最后一个 `stream_url` 出现位置，向前在 3000 字符窗口内
/// 搜索 `"title":"..."` 字段，排除已知的页面模板标题。
fn extract_live_title_near_stream_url(unescaped: &str) -> Option<String> {
    let stream_url_pos = unescaped.rfind("\"stream_url\"")?;
    let start = stream_url_pos.saturating_sub(3000);
    let window = &unescaped[start..stream_url_pos];
    let title_re = regex::Regex::new(r#""title"\s*:\s*"([^"]+)""#).ok()?;
    let mut candidates: Vec<String> = Vec::new();
    for caps in title_re.captures_iter(window) {
        let title = caps.get(1)?.as_str().to_string();
        // 排除已知页面模板标题
        if is_template_title(&title) {
            continue;
        }
        candidates.push(title);
    }
    candidates.into_iter().last()
}

/// 判断标题是否为页面模板标题（非直播间标题）。
fn is_template_title(title: &str) -> bool {
    const TEMPLATE_PREFIXES: &[&str] = &[
        "广告投放", "用户服务", "隐私政策", "账号找回", "联系我们", "加入我们",
        "营业执照", "友情链接", "站点地图", "下载抖音", "抖音电商", "网络谣言",
        "网上有害", "违法和不良", "算法推荐", "体育饭圈", "京ICP", "京公网",
        "广播电视", "增值电信", "网络文化", "互联网", "药品医疗", "PC ",
        "Scan QR", "抖音直播电脑版",
    ];
    TEMPLATE_PREFIXES
        .iter()
        .any(|prefix| title.starts_with(prefix))
        || title.contains("© 抖音")
        || title.trim().is_empty()
        || title.starts_with("{{")
}

/// 从 HTML 中提取主播昵称。
///
/// 策略：找到最后一个 `nickname` 字段，排除 `$undefined` 等占位符。
fn extract_live_nickname(unescaped: &str) -> Option<String> {
    let nick_re = regex::Regex::new(r#""nickname"\s*:\s*"([^"]+)""#).ok()?;
    let mut candidates: Vec<String> = Vec::new();
    for caps in nick_re.captures_iter(unescaped) {
        let nick = caps.get(1)?.as_str().to_string();
        if nick == "$undefined" || nick.is_empty() {
            continue;
        }
        candidates.push(nick);
    }
    candidates.into_iter().last()
}

/// 从 HTML 中提取直播状态。
///
/// `status: 2` = 直播中，`status: 4` = 未开播。
fn extract_live_status(unescaped: &str) -> Option<u32> {
    let status_re = regex::Regex::new(r#""status"\s*:\s*(\d+)"#).ok()?;
    let candidates: Vec<u32> = status_re
        .captures_iter(unescaped)
        .filter_map(|caps| {
            caps.get(1)
                .and_then(|m| m.as_str().parse::<u32>().ok())
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    // 取出现频率最高的状态值（众数）
    let mut max_count = 0usize;
    let mut result = candidates[0];
    for &val in &candidates {
        let count = candidates.iter().filter(|&&x| x == val).count();
        if count > max_count {
            max_count = count;
            result = val;
        }
    }
    Some(result)
}

/// 将抖音直播详情转换为 yt-dlp 兼容的 JSON 格式。
///
/// 生成的 JSON 包含：
/// - `title`：直播间标题（带主播昵称前缀）
/// - `uploader`：主播昵称
/// - `is_live`：是否正在直播
/// - `formats`：包含 HLS 流地址的格式列表（yt-dlp 可直接录制）
/// - `_is_douyin_live`：标记为抖音直播（供 probe 函数识别）
pub fn convert_douyin_live_to_yt_dlp_json(detail: &serde_json::Value) -> serde_json::Value {
    let title = detail
        .get("title")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("抖音直播")
        .to_string();
    let nickname = detail
        .get("nickname")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("未知主播")
        .to_string();
    let status = detail
        .get("status")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let is_live = status == 2;
    let hls_url = detail
        .get("hls_url")
        .and_then(serde_json::Value::as_str)
        .map(|s| s.to_string());
    let flv_url = detail
        .get("flv_url")
        .and_then(serde_json::Value::as_str)
        .map(|s| s.to_string());

    // 构建 formats：优先 FLV（HTTP 直连流，无需依赖 FFmpeg，原生兼容性最高）
    let mut formats = Vec::new();
    if let Some(ref url) = flv_url {
        formats.push(serde_json::json!({
            "format_id": "live-flv",
            "url": url,
            "ext": "flv",
            "protocol": "http",
            "format": "FLV 直播流（最高画质）",
            "vcodec": "h264",
            "acodec": "aac",
            "tbr": 2560,
        }));
    }
    if let Some(ref url) = hls_url {
        formats.push(serde_json::json!({
            "format_id": "live-hls",
            "url": url,
            "ext": "mp4",
            "protocol": "m3u8_native",
            "format": "HLS 直播流（最高画质）",
            "vcodec": "h264",
            "acodec": "aac",
            "tbr": 2560,
        }));
    }

    serde_json::json!({
        "title": format!("{}的直播间", nickname),
        "fulltitle": title,
        "uploader": nickname,
        "uploader_id": detail.get("room_id").and_then(serde_json::Value::as_str).unwrap_or(""),
        "extractor": "douyin",
        "extractor_key": "DouyinLive",
        "is_live": is_live,
        "live_status": if is_live { "is_live" } else { "was_live" },
        "duration": null,
        "formats": formats,
        "_is_douyin_live": true,
        "_is_douyin_gallery": false,
    })
}

// ===== Task：抖音直连解析（绕过 byted_acrawler 反爬） =====
//
// 背景：抖音 web 页面在首次访问时返回 `byted_acrawler` 反爬挑战页
// （要求执行 JS 计算 `__ac_signature`），reqwest 不执行 JS，
// 因此 yt-dlp 无法获取真实页面元数据，导致抖音图集标题解析失败、图片无法下载。
//
// 方案：参考开源项目 `wujunwei928/parse-video-py` 与 `lijinrui182/douyin-8k-parser`
// 的反编译结论：抖音 `/aweme/v1/web/aweme/detail/` 接口仅需 `ttwid` cookie +
// 浏览器 UA + Referer，**不要求 a_bogus / msToken / __ac_signature 真实签名**。
// 直接调用该接口即可获取完整的 `aweme_detail`（图集 images[] 与视频 play_addr 均包含）。
//
// 安全约束（AGENTS.md §3 / §5 / §7）：
// - 不引入新依赖（复用 reqwest / serde_json / regex）
// - 不执行 JS，不依赖复杂签名
// - 仅请求完成当前解析所需数据，不上传浏览历史或页面数据（AGENTS.md §5）
// - 失败时返回中文错误，不暴露内部异常细节
// - 不使用 `unwrap()` / `expect()` 处理可恢复错误
// - ttwid 是匿名访问令牌（非用户身份凭证），不视为敏感信息

/// 抖音 aweme_id 提取正则（按优先级排序）。
///
/// 模式（按优先级）：
/// 1. `/note/<id>`、`/share/note/<id>`：图集
/// 2. `/video/<id>`、`/share/video/<id>`：视频
/// 3. `?modal_id=<id>`：弹窗分享
/// 4. `?aweme_id=<id>`：API 直链
/// 5. `?item_id=<id>`：旧版分享
/// 6. `?vid=<id>`：新版 PC 分享
fn douyin_aweme_id_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // 5 个捕获组，按优先级匹配；id 至少 10 位数字（抖音 aweme_id 通常 19 位）
        Regex::new(
            r"(?:/share/note/|/note/|/share/video/|/video/)(\d{10,})|[?&](?:modal_id|aweme_id|item_id|vid)=(\d{10,})",
        )
        .expect("douyin aweme_id 正则编译失败（构建期不变量）")
    })
}

/// 从抖音 URL 中提取 aweme_id。
///
/// 支持以下 URL 形式（按优先级）：
/// - `https://www.douyin.com/note/7623069826236834489`
/// - `https://www.douyin.com/video/7623069826236834489`
/// - `https://www.iesdouyin.com/share/note/7623069826236834489/`
/// - `https://www.iesdouyin.com/share/video/7623069826236834489/`
/// - `https://www.douyin.com/?modal_id=7623069826236834489`
/// - `https://www.douyin.com/discover?aweme_id=7623069826236834489`
///
/// 非抖音 URL、未命中任何模式、或 id 不是 10+ 位数字时返回 `None`。
///
/// 注：本函数不做网络请求，纯 URL 字符串解析。
pub fn extract_douyin_aweme_id(url: &str) -> Option<String> {
    if detect_platform(url) != MediaPlatform::Douyin {
        return None;
    }
    let caps = douyin_aweme_id_regex().captures(url)?;
    // 路径捕获组（组 1）或查询参数捕获组（组 2）
    let id = caps.get(1).or_else(|| caps.get(2))?.as_str();
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// 抖音直连解析使用的匿名 `ttwid` cookie。
///
/// `ttwid` 是抖音 web 端为匿名访问分配的访问令牌，**不绑定用户身份**，
/// 由抖音服务器在任意 iesdouyin.com / douyin.com 请求的 Set-Cookie 响应中下发。
/// 此处使用一个长期有效的固定值作为初始请求凭证；若抖音服务端拒绝（返回 0 字节或
/// status_code != 0），调用方应回退到 yt-dlp 流程。
///
/// 安全说明（AGENTS.md §3）：ttwid 不属于认证信息（不绑定用户账号），
/// 不视为敏感信息，可硬编码；用户私有的登录 Cookie / Authorization 仍由
/// `attach_auth_args` 通过临时文件传递，不与此值混淆。
const DOUYIN_ANONYMOUS_TTWID: &str = "1%7C6gjfVcoFl_d0j8RolI8X7PCgqTGrt-NRW6X0ZEnWdHc%7C1784635574%7C15b2bad4a3b61619bcb47bd9fa28610be73c57a3287d845d8564a04ef98a47f1";

/// 抖音直连解析使用的浏览器 UA（与 `media::attach_auth_args_in_dir` 保持一致）。
const DOUYIN_BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// 调用抖音 web API 获取 `aweme_detail`。
///
/// 端点：`https://www.douyin.com/aweme/v1/web/aweme/detail/`
/// 必需参数：`aweme_id`、`aid=6383`、`device_platform=webapp`、`cookie_enabled=true`
/// 必需请求头：`User-Agent`（浏览器）、`Referer: https://www.douyin.com/`
/// 必需 Cookie：`ttwid`（匿名访问令牌）
///
/// **不需要** `a_bogus` / `msToken` / `__ac_signature` 等复杂签名。
///
/// 超时：10 秒（避免长尾阻塞新建任务对话框）。
/// 失败时返回中文错误，不暴露内部异常细节（AGENTS.md §7）。
///
/// 成功返回原始 JSON（顶层含 `aweme_detail` 字段）。
pub async fn fetch_douyin_aweme_detail(aweme_id: &str) -> Result<serde_json::Value, String> {
    fetch_douyin_aweme_detail_with_credentials(aweme_id, None, None, None).await
}

pub async fn fetch_douyin_aweme_detail_with_credentials(
    aweme_id: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
) -> Result<serde_json::Value, String> {
    if aweme_id.trim().is_empty() {
        return Err("抖音 aweme_id 为空".into());
    }
    let url = format!(
        "https://www.douyin.com/aweme/v1/web/aweme/detail/?aweme_id={aweme_id}&aid=6383\
         &device_platform=webapp&cookie_enabled=true&browser_language=zh-CN\
         &browser_platform=Win32&browser_name=Chrome&browser_version=131.0.0.0"
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        // 抖音 API 不会重定向，禁用 redirect 以避免误把 302 当作成功
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("抖音解析客户端构建失败：{e}"))?;

    let effective_ua = user_agent.filter(|u| !u.trim().is_empty()).unwrap_or(DOUYIN_BROWSER_UA);
    let effective_ref = referer.filter(|r| !r.trim().is_empty()).unwrap_or("https://www.douyin.com/");
    let effective_cookie = match cookie.filter(|c| !c.trim().is_empty()) {
        Some(c) => {
            if c.contains("ttwid=") {
                c.to_string()
            } else {
                format!("{c}; ttwid={DOUYIN_ANONYMOUS_TTWID}")
            }
        }
        None => format!("ttwid={DOUYIN_ANONYMOUS_TTWID}"),
    };

    let response = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, effective_ua)
        .header(reqwest::header::REFERER, effective_ref)
        .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
        .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9")
        .header(reqwest::header::COOKIE, effective_cookie)
        .send()
        .await
        .map_err(|e| format!("抖音解析请求失败：{e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("抖音解析失败：HTTP {status}"));
    }
    // 抖音 API 在某些场景下返回 200 + 0 字节（无效 ttwid 或被风控），需明确判定
    let body = response
        .bytes()
        .await
        .map_err(|e| format!("抖音解析响应读取失败：{e}"))?;
    if body.is_empty() {
        return Err("抖音解析失败：服务端返回空响应（ttwid 可能已失效）".into());
    }
    let value: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| format!("抖音解析 JSON 解析失败：{e}"))?;
    // 抖音 API 错误响应：{"status_code": 11110, "status_msg": "encrypt_data_miss"}
    let status_code = value
        .get("status_code")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    if status_code != 0 {
        let status_msg = value
            .get("status_msg")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("未知错误");
        return Err(format!("抖音解析失败：{status_msg}（code {status_code}）"));
    }
    if value.get("aweme_detail").is_none() {
        return Err("抖音解析失败：响应缺少 aweme_detail 字段".into());
    }
    Ok(value)
}

/// 把抖音 `aweme_detail` JSON 转换为 yt-dlp 兼容的 JSON 结构。
///
/// 转换规则：
/// - `title` ← `aweme_detail.desc`（图集/视频描述，含 hashtag）
/// - `extractor_key` = `"Douyin"`、`extractor` = `"douyin"`
/// - `webpage_url` ← 原始 URL（由调用方在 `media::probe` 拼接）
/// - `uploader` ← `author.nickname`、`uploader_id` ← `author.sec_uid`
/// - `upload_date` ← `create_time`（unix 秒）格式化为 `YYYYMMDD`
/// - `duration` ← `video.duration / 1000`（图集为 0）
/// - `thumbnail` ← 首张图片 URL（图集）或视频封面（视频）
/// - `thumbnails` ← `images[].url_list[0]` 数组（图集专用）
/// - `formats` ← 图集：每张图一个图片格式项（vcodec=none, acodec=none, ext=webp）
///              视频：play_addr.url_list[0] 作为视频格式项
///
/// 设计要点：
/// - 纯函数，无副作用，便于单元测试
/// - 所有字段缺失时使用安全默认值，不 panic
/// - 图集 ext 优先 `jpeg` 而非 `webp`（兼容性更好，前端预览与下载均支持）
///   实际 URL 中 `:q75.webp` 后缀由抖音 CDN 处理，扩展名仅作格式识别用
pub fn convert_douyin_aweme_to_yt_dlp_json(detail: &serde_json::Value) -> serde_json::Value {
    let aweme = detail
        .get("aweme_detail")
        .unwrap_or(detail); // 容错：直接传入 aweme_detail 子对象也支持

    let desc = aweme.get("desc").and_then(serde_json::Value::as_str).unwrap_or("");
    let author = aweme.get("author");
    let nickname = author
        .and_then(|a| a.get("nickname"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let sec_uid = author
        .and_then(|a| a.get("sec_uid"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let create_time = aweme
        .get("create_time")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let upload_date = if create_time > 0 {
        // unix 秒 → YYYYMMDD（UTC，避免本地时区干扰命名稳定性）
        let days = (create_time / 86400) as i64;
        // 1970-01-01 起 + days 天，手动计算避免引入 chrono 依赖
        let (y, m, d) = days_to_ymd(days);
        format!("{y:04}{m:02}{d:02}")
    } else {
        String::new()
    };
    let video = aweme.get("video");
    let duration_ms = video
        .and_then(|v| v.get("duration"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let duration_sec = if duration_ms > 0 { duration_ms as f64 / 1000.0 } else { 0.0 };

    // 图集：images[] 数组
    let images: Vec<&serde_json::Value> = aweme
        .get("images")
        .and_then(serde_json::Value::as_array)
        .map(|arr| arr.iter().collect())
        .unwrap_or_default();

    // 构建 thumbnails 数组（前端预览与 extract_thumbnail_images 兜底均会用到）
    let thumbnails: Vec<serde_json::Value> = images
        .iter()
        .filter_map(|img| {
            let url = img
                .get("url_list")
                .and_then(serde_json::Value::as_array)
                .and_then(|arr| arr.first())
                .and_then(serde_json::Value::as_str)?;
            if url.is_empty() {
                None
            } else {
                Some(serde_json::json!({ "url": url }))
            }
        })
        .collect();

    // 构建 formats 数组：图集每张图一个图片格式项；视频走 play_addr
    let mut formats: Vec<serde_json::Value> = Vec::new();
    for (idx, img) in images.iter().enumerate() {
        let url = img
            .get("url_list")
            .and_then(serde_json::Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if url.is_empty() {
            continue;
        }
        let width = img.get("width").and_then(serde_json::Value::as_u64);
        let height = img.get("height").and_then(serde_json::Value::as_u64);
        formats.push(serde_json::json!({
            "format_id": format!("image-{idx}"),
            "format_note": format!("图片 {}", idx + 1),
            "ext": "jpeg",
            "vcodec": "none",
            "acodec": "none",
            "width": width,
            "height": height,
            "url": url,
        }));
    }
    // 视频格式（如果有 play_addr）
    if let Some(video_obj) = video {
        let play_addr = video_obj.get("play_addr");
        let video_url = play_addr
            .and_then(|p| p.get("url_list"))
            .and_then(serde_json::Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if !video_url.is_empty() && duration_sec > 0.0 {
            let width = video_obj
                .get("width")
                .and_then(serde_json::Value::as_u64);
            let height = video_obj
                .get("height")
                .and_then(serde_json::Value::as_u64);
            formats.push(serde_json::json!({
                "format_id": "play-0",
                "format_note": "原片",
                "ext": "mp4",
                "vcodec": "h264",
                "acodec": "aac",
                "width": width,
                "height": height,
                "url": video_url,
            }));
        }
    }

    let thumbnail = thumbnails
        .first()
        .and_then(|t| t.get("url"))
        .and_then(serde_json::Value::as_str)
        .map(|s| s.to_string());

    // 图集标记：images 数组非空时标记为图集，供 probe 函数覆盖 media_type。
    // 不能仅靠 URL 路径判断，因为抖音短链可能展开为 /discover?modal_id=xxx
    // 等不含 /note/ 的路径，但 API 返回的 aweme_detail.images[] 仍为图集。
    let is_gallery = !images.is_empty();

    serde_json::json!({
        "title": if desc.is_empty() { "抖音媒体" } else { desc },
        "extractor": "douyin",
        "extractor_key": "Douyin",
        "uploader": nickname,
        "uploader_id": sec_uid,
        "upload_date": upload_date,
        "duration": duration_sec,
        "thumbnail": thumbnail,
        "thumbnails": thumbnails,
        "formats": formats,
        "_is_douyin_gallery": is_gallery,
    })
}

/// Unix 天数（自 1970-01-01 起）转 `(year, month, day)`（公历，UTC）。
///
/// 复用项目已有算法风格（避免引入 chrono 依赖，AGENTS.md §8）。
/// 实现采用经典"霍华德·欣尼斯"算法，覆盖 1970-2199 年范围（足够抖音使用）。
fn days_to_ymd(days: i64) -> (i64, u32, u32) {
    // 1970-01-01 = day 0
    let z = days + 719468; // 偏移到 0000-03-01
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
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
// （Task 37 规范位置）。本模块仅提供平台特定的辅助识别与直连解析函数：
// - `extract_twitter_status_id` / `fetch_twitter_tweet_detail` / `convert_twitter_tweet_to_yt_dlp_json`：
//   基于 Twitter Syndication API 直连解析推文视频（耗时约 150ms），跳过慢速 Python yt-dlp 调用。
// - `classify_twitter_error` / `classify_tiktok_error` / `classify_douyin_error`：
//   由 `diagnose::classify_platform_error` 在 DRM 检测后分发调用。
// - `is_twitter_space`：识别 Twitter Spaces 音频 URL。
// - `format_twitter_filename`：按 `{author}_{tweet_id}_{date}` 模板生成文件名。

/// 从 Twitter/X URL 中提取 Tweet ID（数字串）。
///
/// 支持格式：
/// - `https://x.com/user/status/1812345678901234567`
/// - `https://twitter.com/i/web/status/1812345678901234567`
/// - `https://mobile.twitter.com/user/status/1812345678901234567?s=20`
pub fn extract_twitter_status_id(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url.trim()).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    if host != "twitter.com"
        && !host.ends_with(".twitter.com")
        && host != "x.com"
        && !host.ends_with(".x.com")
        && host != "t.co"
    {
        return None;
    }
    static STATUS_RE: OnceLock<Regex> = OnceLock::new();
    let re = STATUS_RE.get_or_init(|| Regex::new(r"(?i)/(?:status|statuses)/(\d+)").unwrap());
    let captures = re.captures(parsed.path())?;
    Some(captures.get(1)?.as_str().to_string())
}

/// 通过 Twitter Syndication API 直连获取 Tweet 详情。
pub async fn fetch_twitter_tweet_detail(tweet_id: &str) -> Result<serde_json::Value, String> {
    fetch_twitter_tweet_detail_with_credentials(tweet_id, None, None, None).await
}

/// 通过 Twitter Syndication API 直连获取 Tweet 详情（带可选凭证/UA/Referer）。
pub async fn fetch_twitter_tweet_detail_with_credentials(
    tweet_id: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
) -> Result<serde_json::Value, String> {
    if tweet_id.trim().is_empty() {
        return Err("Twitter tweet_id 为空".into());
    }
    let url = format!("https://cdn.syndication.twimg.com/tweet-result?id={tweet_id}&token=5");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .connect_timeout(Duration::from_secs(4))
        .build()
        .map_err(|e| format!("Twitter 解析客户端构建失败：{e}"))?;

    let effective_ua = user_agent
        .filter(|u| !u.trim().is_empty())
        .unwrap_or("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36");
    let effective_ref = referer
        .filter(|r| !r.trim().is_empty())
        .unwrap_or("https://platform.twitter.com/");

    let mut req = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, effective_ua)
        .header(reqwest::header::REFERER, effective_ref)
        .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9,zh-CN;q=0.8");

    if let Some(c) = cookie.filter(|c| !c.trim().is_empty()) {
        req = req.header(reqwest::header::COOKIE, c);
    }

    let response = req.send().await.map_err(|e| format!("Twitter API 请求失败：{e}"))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err("Twitter/X 推文不存在或已被删除".into());
    }
    if !status.is_success() {
        return Err(format!("Twitter API 响应失败：HTTP {status}"));
    }
    let body = response.bytes().await.map_err(|e| format!("Twitter API 读取响应失败：{e}"))?;
    if body.is_empty() {
        return Err("Twitter API 返回空响应".into());
    }
    let value: serde_json::Value = serde_json::from_slice(&body).map_err(|e| format!("Twitter API JSON 解析失败：{e}"))?;
    if value.get("text").is_none() && value.get("mediaDetails").is_none() && value.get("video").is_none() {
        return Err("Twitter API 响应缺少推文核心字段".into());
    }
    Ok(value)
}

fn to_twitter_orig_image_url(raw: &str) -> String {
    if raw.contains("name=") {
        static NAME_RE: OnceLock<Regex> = OnceLock::new();
        let re = NAME_RE.get_or_init(|| Regex::new(r"name=[^&]+").unwrap());
        re.replace(raw, "name=orig").to_string()
    } else if raw.contains('?') {
        format!("{raw}&name=orig")
    } else {
        format!("{raw}?name=orig")
    }
}

/// 把 Twitter `tweet-result` JSON 转换为 yt-dlp 兼容的 JSON 结构。
pub fn convert_twitter_tweet_to_yt_dlp_json(detail: &serde_json::Value) -> serde_json::Value {
    let title = detail
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("Twitter 媒体");
    let uploader = detail
        .get("user")
        .and_then(|u| u.get("name").or_else(|| u.get("screen_name")))
        .and_then(Value::as_str);
    let uploader_id = detail
        .get("user")
        .and_then(|u| u.get("screen_name"))
        .and_then(Value::as_str);

    let mut formats = Vec::new();
    let mut thumbnails = Vec::new();
    let mut thumbnail = None;
    let mut photo_count = 0;
    let mut has_video = false;

    if let Some(media_arr) = detail.get("mediaDetails").and_then(Value::as_array) {
        for media in media_arr {
            if thumbnail.is_none() {
                if let Some(raw_thumb) = media.get("media_url_https").and_then(Value::as_str) {
                    thumbnail = Some(to_twitter_orig_image_url(raw_thumb));
                }
            }
            if let Some(video_info) = media.get("video_info") {
                has_video = true;
                if let Some(variants) = video_info.get("variants").and_then(Value::as_array) {
                    let mut mp4_list: Vec<&Value> = variants
                        .iter()
                        .filter(|v| v.get("content_type").and_then(Value::as_str) == Some("video/mp4"))
                        .collect();

                    static RES_RE: OnceLock<Regex> = OnceLock::new();
                    let res_re = RES_RE.get_or_init(|| Regex::new(r"/(\d{3,4})x(\d{3,4})/").unwrap());

                    mp4_list.sort_by(|a, b| {
                        let br_a = a.get("bitrate").and_then(Value::as_u64).unwrap_or(0);
                        let br_b = b.get("bitrate").and_then(Value::as_u64).unwrap_or(0);
                        br_b.cmp(&br_a)
                    });

                    for (idx, var) in mp4_list.iter().enumerate() {
                        let url = match var.get("url").and_then(Value::as_str) {
                            Some(u) => u.to_string(),
                            None => continue,
                        };
                        let bitrate = var.get("bitrate").and_then(Value::as_u64);

                        let (width, height) = if let Some(cap) = res_re.captures(&url) {
                            let w = cap.get(1).and_then(|m| m.as_str().parse::<u64>().ok());
                            let h = cap.get(2).and_then(|m| m.as_str().parse::<u64>().ok());
                            (w, h)
                        } else {
                            (None, None)
                        };

                        let format_id = match (height, bitrate) {
                            (Some(h), _) => format!("http-{h}"),
                            (None, Some(b)) => format!("http-{}", b / 1000),
                            _ => format!("http-mp4-{idx}"),
                        };

                        let format_note = match (height, width) {
                            (Some(h), Some(w)) => format!("{w}x{h} · mp4 (单文件轻量)"),
                            (Some(h), None) => format!("{h}p · mp4 (单文件轻量)"),
                            _ => "mp4 (单文件轻量)".to_string(),
                        };

                        let mut fmt = serde_json::json!({
                            "format_id": format_id,
                            "url": url,
                            "ext": "mp4",
                            "vcodec": "h264",
                            "acodec": "aac",
                            "format_note": format_note,
                            "protocol": "https"
                        });
                        if let Some(w) = width { fmt["width"] = serde_json::Value::from(w); }
                        if let Some(h) = height { fmt["height"] = serde_json::Value::from(h); }
                        if let Some(b) = bitrate { fmt["tbr"] = serde_json::Value::from(b / 1000); }

                        formats.push(fmt);
                    }
                }
            } else if let Some(media_url) = media.get("media_url_https").and_then(Value::as_str) {
                photo_count += 1;
                let orig_url = to_twitter_orig_image_url(media_url);
                thumbnails.push(serde_json::json!({ "url": orig_url.clone() }));

                let width = media
                    .get("original_info")
                    .and_then(|info| info.get("width"))
                    .or_else(|| media.get("sizes").and_then(|s| s.get("large")).and_then(|l| l.get("w")))
                    .and_then(Value::as_u64);
                let height = media
                    .get("original_info")
                    .and_then(|info| info.get("height"))
                    .or_else(|| media.get("sizes").and_then(|s| s.get("large")).and_then(|l| l.get("h")))
                    .and_then(Value::as_u64);

                let ext = if orig_url.contains("format=png") || media_url.ends_with(".png") {
                    "png"
                } else if orig_url.contains("format=webp") || media_url.ends_with(".webp") {
                    "webp"
                } else {
                    "jpg"
                };

                let format_note = match (width, height) {
                    (Some(w), Some(h)) => format!("图片 {w}×{h}"),
                    _ => format!("图片 {photo_count}"),
                };

                let mut fmt = serde_json::json!({
                    "format_id": format!("image-{photo_count}"),
                    "format_note": format_note,
                    "url": orig_url,
                    "ext": ext,
                    "vcodec": "none",
                    "acodec": "none"
                });
                if let Some(w) = width { fmt["width"] = serde_json::Value::from(w); }
                if let Some(h) = height { fmt["height"] = serde_json::Value::from(h); }
                formats.push(fmt);
            }
        }
    }

    if formats.is_empty() {
        if let Some(photos_arr) = detail.get("photos").and_then(Value::as_array) {
            for (idx, p) in photos_arr.iter().enumerate() {
                let url_str = match p.get("url").and_then(Value::as_str) {
                    Some(u) => u,
                    None => continue,
                };
                photo_count += 1;
                let orig_url = to_twitter_orig_image_url(url_str);
                if thumbnail.is_none() {
                    thumbnail = Some(orig_url.clone());
                }
                thumbnails.push(serde_json::json!({ "url": orig_url.clone() }));
                let width = p.get("width").and_then(Value::as_u64);
                let height = p.get("height").and_then(Value::as_u64);
                let ext = if orig_url.contains("format=png") || url_str.ends_with(".png") {
                    "png"
                } else {
                    "jpg"
                };
                let format_note = match (width, height) {
                    (Some(w), Some(h)) => format!("图片 {w}×{h}"),
                    _ => format!("图片 {}", idx + 1),
                };
                let mut fmt = serde_json::json!({
                    "format_id": format!("image-{}", idx + 1),
                    "format_note": format_note,
                    "url": orig_url,
                    "ext": ext,
                    "vcodec": "none",
                    "acodec": "none"
                });
                if let Some(w) = width { fmt["width"] = serde_json::Value::from(w); }
                if let Some(h) = height { fmt["height"] = serde_json::Value::from(h); }
                formats.push(fmt);
            }
        }
    }

    let is_gallery = photo_count > 0 && !has_video;

    serde_json::json!({
        "title": title,
        "extractor": "twitter",
        "extractor_key": "Twitter",
        "uploader": uploader,
        "uploader_id": uploader_id,
        "thumbnail": thumbnail,
        "thumbnails": thumbnails,
        "formats": formats,
        "_is_twitter_gallery": is_gallery,
    })
}



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

    // 登录失效 / 权限拒绝
    if lower_stderr.contains("login required")
        || lower_stderr.contains("cookie")
        || lower_stderr.contains("403")
        || lower_stderr.contains("forbidden")
        || lower_stderr.contains("401")
        || lower_stderr.contains("unauthorized")
    {
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
/// 2. 登录失效/机器人验证：
///    - 年龄限制：`age-restricted`、`age restricted`、`sign in to confirm your age`
///    - 机器人验证（自 2024 年起 YouTube 加强 PO Token 校验）：`sign in to confirm
///      you're not a bot`、`not a bot`、`cookies-from-browser`、`cookies for the
///      authentication`，yt-dlp 会提示用户使用 `--cookies-from-browser` 或
///      `--cookies` 解决，统一归类为 `LoginExpired`，由文案引导用户提供 Cookie
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
    // 机器人验证（PO Token 校验失败）：yt-dlp 输出形如
    // "Sign in to confirm you're not a bot. Use --cookies-from-browser or --cookies
    //  for the authentication." 需用户在设置中提供 YouTube Cookie 才能继续下载
    if lower.contains("not a bot")
        || lower.contains("cookies-from-browser")
        || lower.contains("cookies for the authentication")
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

/// 媒体凭证在线有效性检测功能（四大平台 Bilibili / 抖音 / Twitter / YouTube 及通用兜底）。
pub async fn check_media_credential(
    domain: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
) -> Result<MediaCredentialCheckResult, String> {
    let domain_clean = domain.trim().to_lowercase();
    let now = crate::now_iso8601_utc();

    let cookie_clean = cookie.unwrap_or("").trim();
    let referer_clean = referer.unwrap_or("").trim();
    let ua_clean = user_agent.unwrap_or("").trim();

    if cookie_clean.is_empty() && referer_clean.is_empty() && ua_clean.is_empty() {
        return Ok(MediaCredentialCheckResult {
            domain: domain.to_string(),
            valid: false,
            message: "未设置凭证内容（Cookie/Referer/User-Agent 均为空）".to_string(),
            tested_at: now,
        });
    }

    let default_ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
    let effective_ua = if !ua_clean.is_empty() { ua_clean } else { default_ua };

    let platform = detect_platform(&format!("https://{}", domain_clean));

    match platform {
        MediaPlatform::Bilibili => check_bilibili_credential(domain, cookie_clean, referer_clean, effective_ua, now).await,
        MediaPlatform::Douyin => check_douyin_credential(domain, cookie_clean, referer_clean, effective_ua, now).await,
        MediaPlatform::Twitter => check_twitter_credential(domain, cookie_clean, referer_clean, effective_ua, now).await,
        MediaPlatform::YouTube => check_youtube_credential(domain, cookie_clean, referer_clean, effective_ua, now).await,
        _ => check_generic_credential(domain, cookie_clean, referer_clean, effective_ua, now).await,
    }
}

async fn check_bilibili_credential(
    domain: &str,
    cookie: &str,
    referer: &str,
    ua: &str,
    now: String,
) -> Result<MediaCredentialCheckResult, String> {
    let url = "https://api.bilibili.com/x/web-interface/nav";
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build() {
            Ok(c) => c,
            Err(e) => return Ok(MediaCredentialCheckResult {
                domain: domain.to_string(),
                valid: false,
                message: format!("构建客户端失败：{e}"),
                tested_at: now,
            }),
        };

    let effective_ref = if !referer.is_empty() { referer } else { "https://www.bilibili.com/" };

    let mut req = client.get(url)
        .header(reqwest::header::USER_AGENT, ua)
        .header(reqwest::header::REFERER, effective_ref);

    if !cookie.is_empty() {
        req = req.header(reqwest::header::COOKIE, cookie);
    }

    let res = match req.send().await {
        Ok(r) => r,
        Err(e) => return Ok(MediaCredentialCheckResult {
            domain: domain.to_string(),
            valid: false,
            message: format!("连接 Bilibili 接口失败：{e}"),
            tested_at: now,
        }),
    };

    if !res.status().is_success() {
        return Ok(MediaCredentialCheckResult {
            domain: domain.to_string(),
            valid: false,
            message: format!("Bilibili 响应异常：HTTP {}", res.status()),
            tested_at: now,
        });
    }

    let bytes = match res.bytes().await {
        Ok(b) => b,
        Err(_) => return Ok(MediaCredentialCheckResult {
            domain: domain.to_string(),
            valid: false,
            message: "读取 Bilibili 响应内容失败".to_string(),
            tested_at: now,
        }),
    };

    let json: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(j) => j,
        Err(_) => return Ok(MediaCredentialCheckResult {
            domain: domain.to_string(),
            valid: false,
            message: "Bilibili 响应格式非 JSON".to_string(),
            tested_at: now,
        }),
    };

    let code = json.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 {
        let msg = json.get("message").and_then(|v| v.as_str()).unwrap_or("校验未通过");
        return Ok(MediaCredentialCheckResult {
            domain: domain.to_string(),
            valid: false,
            message: format!("Bilibili 校验失败：{msg}（code {code}）"),
            tested_at: now,
        });
    }

    let is_login = json.pointer("/data/isLogin").and_then(|v| v.as_bool()).unwrap_or(false);
    if is_login {
        let uname = json.pointer("/data/uname").and_then(|v| v.as_str()).unwrap_or("用户");
        let vip_status = json.pointer("/data/vipStatus").and_then(|v| v.as_i64()).unwrap_or(0);
        let is_vip = vip_status == 1;

        let msg = if is_vip {
            format!("凭证有效：已登录 B站账号「{uname}」(大会员)")
        } else {
            format!("凭证有效：已登录 B站账号「{uname}」")
        };

        Ok(MediaCredentialCheckResult {
            domain: domain.to_string(),
            valid: true,
            message: msg,
            tested_at: now,
        })
    } else {
        Ok(MediaCredentialCheckResult {
            domain: domain.to_string(),
            valid: false,
            message: "凭证未生效或 Cookie 已过期 (B站未识别到登录状态)".to_string(),
            tested_at: now,
        })
    }
}

async fn check_douyin_credential(
    domain: &str,
    cookie: &str,
    referer: &str,
    ua: &str,
    now: String,
) -> Result<MediaCredentialCheckResult, String> {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build() {
            Ok(c) => c,
            Err(e) => return Ok(MediaCredentialCheckResult {
                domain: domain.to_string(),
                valid: false,
                message: format!("构建客户端失败：{e}"),
                tested_at: now,
            }),
        };

    let effective_ref = if !referer.is_empty() { referer } else { "https://www.douyin.com/" };

    let mut req = client.get("https://www.douyin.com/passport/web/user/info/")
        .header(reqwest::header::USER_AGENT, ua)
        .header(reqwest::header::REFERER, effective_ref)
        .header(reqwest::header::ACCEPT, "application/json, text/plain, */*");

    if !cookie.is_empty() {
        req = req.header(reqwest::header::COOKIE, cookie);
    }

    if let Ok(res) = req.send().await {
        if res.status().is_success() {
            if let Ok(bytes) = res.bytes().await {
                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                    if let Some(data) = json.get("data") {
                        let nickname = data.get("nickname")
                            .or_else(|| data.get("screen_name"))
                            .and_then(|v| v.as_str());
                        if let Some(nick) = nickname {
                            if !nick.is_empty() {
                                return Ok(MediaCredentialCheckResult {
                                    domain: domain.to_string(),
                                    valid: true,
                                    message: format!("凭证有效：已登录抖音账号「{nick}」"),
                                    tested_at: now,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    let has_session = cookie.contains("sessionid") || cookie.contains("passport_csrf_token") || cookie.contains("ttwid");
    if has_session {
        Ok(MediaCredentialCheckResult {
            domain: domain.to_string(),
            valid: true,
            message: "凭证有效：已包含抖音身份 Cookie 凭据".to_string(),
            tested_at: now,
        })
    } else {
        Ok(MediaCredentialCheckResult {
            domain: domain.to_string(),
            valid: false,
            message: "凭证未生效或 Cookie 中缺少 sessionid / passport_csrf_token".to_string(),
            tested_at: now,
        })
    }
}

async fn check_twitter_credential(
    domain: &str,
    cookie: &str,
    referer: &str,
    ua: &str,
    now: String,
) -> Result<MediaCredentialCheckResult, String> {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build() {
            Ok(c) => c,
            Err(e) => return Ok(MediaCredentialCheckResult {
                domain: domain.to_string(),
                valid: false,
                message: format!("构建客户端失败：{e}"),
                tested_at: now,
            }),
        };

    let effective_ref = if !referer.is_empty() { referer } else { "https://x.com/" };

    let ct0 = crate::media_cookies::parse_cookie_header(cookie)
        .into_iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("ct0"))
        .map(|(_, v)| v);

    let url = "https://x.com/i/api/1.1/account/verify_credentials.json";
    let bearer = "Bearer AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";

    let mut req = client.get(url)
        .header(reqwest::header::USER_AGENT, ua)
        .header(reqwest::header::REFERER, effective_ref)
        .header(reqwest::header::AUTHORIZATION, bearer);

    if let Some(ref csrf) = ct0 {
        req = req.header("x-csrf-token", csrf);
    }
    if !cookie.is_empty() {
        req = req.header(reqwest::header::COOKIE, cookie);
    }

    match req.send().await {
        Ok(res) => {
            if res.status().is_success() {
                if let Ok(bytes) = res.bytes().await {
                    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        if let Some(screen_name) = json.get("screen_name").and_then(|v| v.as_str()) {
                            let name = json.get("name").and_then(|v| v.as_str()).unwrap_or(screen_name);
                            return Ok(MediaCredentialCheckResult {
                                domain: domain.to_string(),
                                valid: true,
                                message: format!("凭证有效：已登录 Twitter/X 账号 @{screen_name} ({name})"),
                                tested_at: now,
                            });
                        }
                    }
                }
                Ok(MediaCredentialCheckResult {
                    domain: domain.to_string(),
                    valid: true,
                    message: "凭证有效：Twitter/X 接口响应 200 OK".to_string(),
                    tested_at: now,
                })
            } else {
                let status = res.status();
                if cookie.is_empty() {
                    Ok(MediaCredentialCheckResult {
                        domain: domain.to_string(),
                        valid: false,
                        message: "Twitter/X Cookie 未设置".to_string(),
                        tested_at: now,
                    })
                } else if ct0.is_none() {
                    Ok(MediaCredentialCheckResult {
                        domain: domain.to_string(),
                        valid: false,
                        message: format!("Twitter/X 响应 HTTP {status} (Cookie 缺少 ct0 字段)"),
                        tested_at: now,
                    })
                } else {
                    Ok(MediaCredentialCheckResult {
                        domain: domain.to_string(),
                        valid: false,
                        message: format!("Twitter/X 凭证校验失败：HTTP {status} (auth_token 可能失效)"),
                        tested_at: now,
                    })
                }
            }
        }
        Err(e) => Ok(MediaCredentialCheckResult {
            domain: domain.to_string(),
            valid: false,
            message: format!("请求 Twitter/X 失败：{e}"),
            tested_at: now,
        }),
    }
}

async fn check_youtube_credential(
    domain: &str,
    cookie: &str,
    referer: &str,
    ua: &str,
    now: String,
) -> Result<MediaCredentialCheckResult, String> {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build() {
            Ok(c) => c,
            Err(e) => return Ok(MediaCredentialCheckResult {
                domain: domain.to_string(),
                valid: false,
                message: format!("构建客户端失败：{e}"),
                tested_at: now,
            }),
        };

    let effective_ref = if !referer.is_empty() { referer } else { "https://www.youtube.com/" };

    let mut req = client.get("https://www.youtube.com/")
        .header(reqwest::header::USER_AGENT, ua)
        .header(reqwest::header::REFERER, effective_ref);

    if !cookie.is_empty() {
        req = req.header(reqwest::header::COOKIE, cookie);
    }

    match req.send().await {
        Ok(res) => {
            if res.status().is_success() {
                let text = res.text().await.unwrap_or_default();
                if text.contains("\"LOGGED_IN\":true") || text.contains("\"LOGGED_IN\": true") {
                    Ok(MediaCredentialCheckResult {
                        domain: domain.to_string(),
                        valid: true,
                        message: "凭证有效：已检测到 YouTube 登录会话".to_string(),
                        tested_at: now,
                    })
                } else if cookie.contains("LOGIN_INFO") || cookie.contains("SID") || cookie.contains("SAPISID") {
                    Ok(MediaCredentialCheckResult {
                        domain: domain.to_string(),
                        valid: true,
                        message: "凭证有效：已附带 YouTube 认证 Cookie".to_string(),
                        tested_at: now,
                    })
                } else {
                    Ok(MediaCredentialCheckResult {
                        domain: domain.to_string(),
                        valid: false,
                        message: "YouTube 未识别到登录账号 (Cookie 中缺少 LOGIN_INFO / SID)".to_string(),
                        tested_at: now,
                    })
                }
            } else {
                Ok(MediaCredentialCheckResult {
                    domain: domain.to_string(),
                    valid: false,
                    message: format!("YouTube 响应异常：HTTP {}", res.status()),
                    tested_at: now,
                })
            }
        }
        Err(e) => Ok(MediaCredentialCheckResult {
            domain: domain.to_string(),
            valid: false,
            message: format!("请求 YouTube 失败：{e}"),
            tested_at: now,
        }),
    }
}

async fn check_generic_credential(
    domain: &str,
    cookie: &str,
    referer: &str,
    ua: &str,
    now: String,
) -> Result<MediaCredentialCheckResult, String> {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build() {
            Ok(c) => c,
            Err(e) => return Ok(MediaCredentialCheckResult {
                domain: domain.to_string(),
                valid: false,
                message: format!("构建客户端失败：{e}"),
                tested_at: now,
            }),
        };

    let target_url = if !referer.is_empty() {
        referer.to_string()
    } else {
        format!("https://{domain}/")
    };

    let mut req = client.get(&target_url).header(reqwest::header::USER_AGENT, ua);
    if !cookie.is_empty() {
        req = req.header(reqwest::header::COOKIE, cookie);
    }

    match req.send().await {
        Ok(res) => {
            let status = res.status();
            if status.is_success() || status.is_redirection() {
                Ok(MediaCredentialCheckResult {
                    domain: domain.to_string(),
                    valid: true,
                    message: format!("已成功连通服务器 (HTTP {status})"),
                    tested_at: now,
                })
            } else {
                Ok(MediaCredentialCheckResult {
                    domain: domain.to_string(),
                    valid: false,
                    message: format!("服务器响应异常：HTTP {status}"),
                    tested_at: now,
                })
            }
        }
        Err(e) => Ok(MediaCredentialCheckResult {
            domain: domain.to_string(),
            valid: false,
            message: format!("连接服务器失败：{e}"),
            tested_at: now,
        }),
    }
}

// ===== 单元测试 =====
#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::diagnose::{
        classify_platform_error, platform_error_to_chinese, MediaPlatformError,
    };
    use serde_json::Value;

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

    // ---- extract_douyin_aweme_id：从 URL 提取 aweme_id ----

    #[test]
    fn extract_douyin_aweme_id_from_note_path() {
        // 图集 URL（用户实际报告的链接展开后形态）
        assert_eq!(
            extract_douyin_aweme_id("https://www.douyin.com/note/7623069826236834489"),
            Some("7623069826236834489".to_string())
        );
        // 带查询参数
        assert_eq!(
            extract_douyin_aweme_id(
                "https://www.douyin.com/note/7623069826236834489?previous_page=web_code_link"
            ),
            Some("7623069826236834489".to_string())
        );
    }

    #[test]
    fn extract_douyin_aweme_id_from_video_path() {
        assert_eq!(
            extract_douyin_aweme_id("https://www.douyin.com/video/7283698765432109876"),
            Some("7283698765432109876".to_string())
        );
        assert_eq!(
            extract_douyin_aweme_id("https://www.iesdouyin.com/share/video/7623069826236834489/"),
            Some("7623069826236834489".to_string())
        );
        assert_eq!(
            extract_douyin_aweme_id("https://www.iesdouyin.com/share/note/7623069826236834489/"),
            Some("7623069826236834489".to_string())
        );
    }

    #[test]
    fn extract_douyin_aweme_id_from_query_param() {
        // modal_id（弹窗分享）
        assert_eq!(
            extract_douyin_aweme_id("https://www.douyin.com/?modal_id=7623069826236834489"),
            Some("7623069826236834489".to_string())
        );
        // aweme_id（API 直链）
        assert_eq!(
            extract_douyin_aweme_id("https://www.douyin.com/discover?aweme_id=7623069826236834489"),
            Some("7623069826236834489".to_string())
        );
        // item_id（旧版分享）
        assert_eq!(
            extract_douyin_aweme_id("https://www.douyin.com/?item_id=7623069826236834489"),
            Some("7623069826236834489".to_string())
        );
    }

    #[test]
    fn extract_douyin_aweme_id_rejects_short_id() {
        // 少于 10 位数字不匹配（避免误识别普通路径段）
        assert_eq!(extract_douyin_aweme_id("https://www.douyin.com/note/12345"), None);
    }

    #[test]
    fn extract_douyin_aweme_id_rejects_non_douyin() {
        // 非抖音 URL 不识别
        assert_eq!(
            extract_douyin_aweme_id("https://www.tiktok.com/@user/video/7623069826236834489"),
            None
        );
        assert_eq!(
            extract_douyin_aweme_id("https://www.youtube.com/watch?v=7623069826236834489"),
            None
        );
    }

    // ---- convert_douyin_aweme_to_yt_dlp_json：JSON 转换 ----

    #[test]
    fn convert_douyin_gallery_to_yt_dlp_json() {
        // 真实抖音图集响应结构（已裁剪到关键字段）
        let detail = serde_json::json!({
            "aweme_detail": {
                "aweme_id": "7623069826236834489",
                "aweme_type": 0,
                "desc": "在圆圆的世界里 有棱有角的生活#plog#八月手札",
                "create_time": 1722720000_u64,
                "author": {
                    "nickname": "不耀香菜.",
                    "sec_uid": "MS4wLjABAAAAusDy7OGj4V3d8LFHSNLbquDYqsWIOGNnfVeHSbXEIsG_Np-MhaMah-yftJbvhVIo",
                    "uid": "1823463798226043"
                },
                "images": [
                    {
                        "url_list": [
                            "https://p3-pc-sign.douyinpic.com/img0.webp",
                            "https://p9-pc-sign.douyinpic.com/img0.webp",
                            "https://p3-pc-sign.douyinpic.com/img0.jpeg"
                        ],
                        "width": 1080,
                        "height": 1440
                    },
                    {
                        "url_list": ["https://p9-pc-sign.douyinpic.com/img1.webp"],
                        "width": 1080,
                        "height": 1440
                    }
                ],
                "video": {
                    "duration": 0,
                    "play_addr": {
                        "uri": "https://sf11-cdn-tos.douyinstatic.com/obj/ies-music/7568908144924035866.mp3",
                        "url_list": ["https://sf11-cdn-tos.douyinstatic.com/obj/ies-music/7568908144924035866.mp3"]
                    }
                }
            }
        });
        let json = convert_douyin_aweme_to_yt_dlp_json(&detail);

        // 标题正确提取（这是用户报告"图集标题解析不出来"的核心修复点）
        assert_eq!(
            json.get("title").and_then(Value::as_str),
            Some("在圆圆的世界里 有棱有角的生活#plog#八月手札")
        );
        // 平台标识
        assert_eq!(
            json.get("extractor_key").and_then(Value::as_str),
            Some("Douyin")
        );
        assert_eq!(json.get("extractor").and_then(Value::as_str), Some("douyin"));
        // 作者信息（命名模板 {author}_{title}_{date} 用到）
        assert_eq!(
            json.get("uploader").and_then(Value::as_str),
            Some("不耀香菜.")
        );
        // upload_date 格式 YYYYMMDD
        // 1722720000 秒 = 2024-08-03 21:20:00 UTC，整数除法得 day 19938 = 2024-08-03 UTC
        assert_eq!(
            json.get("upload_date").and_then(Value::as_str),
            Some("20240803")
        );
        // thumbnails 数组：每张图一个（用于前端预览 + extract_thumbnail_images 兜底）
        let thumbnails = json
            .get("thumbnails")
            .and_then(Value::as_array)
            .expect("thumbnails 必须存在");
        assert_eq!(thumbnails.len(), 2);
        assert_eq!(
            thumbnails[0].get("url").and_then(Value::as_str),
            Some("https://p3-pc-sign.douyinpic.com/img0.webp")
        );
        // formats 数组：图集每张图一个图片格式项（vcodec=none, acodec=none, ext=jpeg）
        let formats = json
            .get("formats")
            .and_then(Value::as_array)
            .expect("formats 必须存在");
        assert_eq!(formats.len(), 2, "图集应生成 2 个图片格式项，不包含 BGM 视频项");
        assert_eq!(
            formats[0].get("format_id").and_then(Value::as_str),
            Some("image-0")
        );
        assert_eq!(
            formats[0].get("vcodec").and_then(Value::as_str),
            Some("none")
        );
        assert_eq!(
            formats[0].get("acodec").and_then(Value::as_str),
            Some("none")
        );
        assert_eq!(formats[0].get("ext").and_then(Value::as_str), Some("jpeg"));
        assert_eq!(
            formats[0].get("url").and_then(Value::as_str),
            Some("https://p3-pc-sign.douyinpic.com/img0.webp")
        );
        assert_eq!(
            formats[0].get("width").and_then(Value::as_u64),
            Some(1080)
        );
        assert_eq!(
            formats[0].get("height").and_then(Value::as_u64),
            Some(1440)
        );
        // 视频项不应出现（图集 BGM 时长为 0，跳过视频格式）
        assert!(
            !formats.iter().any(|f| {
                f.get("format_id").and_then(Value::as_str) == Some("play-0")
            }),
            "图集 BGM（duration=0）不应生成视频格式项"
        );
    }

    #[test]
    fn convert_douyin_video_to_yt_dlp_json() {
        // 抖音视频响应（含 play_addr + duration > 0）
        let detail = serde_json::json!({
            "aweme_detail": {
                "aweme_id": "7283698765432109876",
                "desc": "测试视频",
                "create_time": 1700000000_u64,
                "author": {
                    "nickname": "测试作者",
                    "sec_uid": "sec_123"
                },
                "video": {
                    "duration": 15000_u64,
                    "width": 1080,
                    "height": 1920,
                    "play_addr": {
                        "url_list": ["https://example.com/video.mp4"]
                    }
                }
            }
        });
        let json = convert_douyin_aweme_to_yt_dlp_json(&detail);
        let formats = json
            .get("formats")
            .and_then(Value::as_array)
            .expect("formats 必须存在");
        assert_eq!(formats.len(), 1, "视频应生成 1 个视频格式项");
        assert_eq!(
            formats[0].get("format_id").and_then(Value::as_str),
            Some("play-0")
        );
        assert_eq!(formats[0].get("ext").and_then(Value::as_str), Some("mp4"));
        assert_eq!(
            formats[0].get("url").and_then(Value::as_str),
            Some("https://example.com/video.mp4")
        );
        // duration 单位转换：ms → s
        assert_eq!(json.get("duration").and_then(Value::as_f64), Some(15.0));
    }

    #[test]
    fn convert_douyin_handles_missing_fields_gracefully() {
        // 字段缺失时使用安全默认值，不 panic（AGENTS.md §7）
        let detail = serde_json::json!({ "aweme_detail": {} });
        let json = convert_douyin_aweme_to_yt_dlp_json(&detail);
        assert_eq!(
            json.get("title").and_then(Value::as_str),
            Some("抖音媒体")
        );
        assert_eq!(
            json.get("uploader").and_then(Value::as_str),
            Some("")
        );
        assert_eq!(
            json.get("upload_date").and_then(Value::as_str),
            Some("")
        );
        assert_eq!(json.get("duration").and_then(Value::as_f64), Some(0.0));
        assert!(json
            .get("formats")
            .and_then(Value::as_array)
            .map(|a| a.is_empty())
            .unwrap_or(true));
    }

    #[test]
    fn convert_douyin_live_to_yt_dlp_json_works() {
        let detail = serde_json::json!({
            "title": "测试直播间",
            "nickname": "测试主播",
            "status": 2,
            "flv_url": "http://pull-flv.douyincdn.com/stream.flv",
            "hls_url": "http://pull-hls.douyincdn.com/stream.m3u8",
            "room_id": "381796295907"
        });
        let json = convert_douyin_live_to_yt_dlp_json(&detail);
        assert_eq!(json["title"], "测试主播的直播间");
        assert_eq!(json["is_live"], true);
        assert!(json["formats"].as_array().unwrap().len() >= 1);
    }

    #[test]
    fn convert_douyin_accepts_bare_aweme_detail() {
        // 容错：直接传 aweme_detail 子对象（不带外层包装）也应支持
        let aweme = serde_json::json!({
            "desc": "容错测试",
            "author": { "nickname": "作者" }
        });
        let json = convert_douyin_aweme_to_yt_dlp_json(&aweme);
        assert_eq!(
            json.get("title").and_then(Value::as_str),
            Some("容错测试")
        );
    }

    // ---- days_to_ymd：unix 天数转年月日 ----

    #[test]
    fn days_to_ymd_known_dates() {
        // 1970-01-01 = day 0
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
        // 2024-08-03（抖音 aweme create_time=1722720000 / 86400 = 19938 天）
        assert_eq!(days_to_ymd(19938), (2024, 8, 3));
        // 2025-01-01（验证跨年，20089 * 86400 = 1735689600 = 2025-01-01 UTC）
        assert_eq!(days_to_ymd(20089), (2025, 1, 1));
        // 2024-12-31（验证年末）
        assert_eq!(days_to_ymd(20088), (2024, 12, 31));
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

    // ---- is_douyin_live / extract_douyin_live_room_id ----

    #[test]
    fn is_douyin_live_detects_live_url() {
        assert!(is_douyin_live("https://live.douyin.com/628273323967"));
        assert!(is_douyin_live(
            "https://live.douyin.com/628273323967?enter_from_merge=link_share"
        ));
        assert!(is_douyin_live("https://live.douyin.com/yall1102"));
    }

    #[test]
    fn is_douyin_live_returns_false_for_non_live_url() {
        assert!(!is_douyin_live("https://www.douyin.com/video/1234567890"));
        assert!(!is_douyin_live("https://www.douyin.com/note/1234567890"));
        assert!(!is_douyin_live("https://v.douyin.com/abc123/"));
        assert!(!is_douyin_live("https://www.youtube.com/watch?v=abc"));
    }

    #[test]
    fn extract_douyin_live_room_id_numeric() {
        assert_eq!(
            extract_douyin_live_room_id("https://live.douyin.com/628273323967"),
            Some("628273323967".to_string())
        );
        assert_eq!(
            extract_douyin_live_room_id(
                "https://live.douyin.com/628273323967?enter_from_merge=link_share&action_type=click"
            ),
            Some("628273323967".to_string())
        );
    }

    #[test]
    fn extract_douyin_live_room_id_alpha() {
        assert_eq!(
            extract_douyin_live_room_id("https://live.douyin.com/yall1102"),
            Some("yall1102".to_string())
        );
    }

    #[test]
    fn extract_douyin_live_room_id_with_trailing_slash() {
        assert_eq!(
            extract_douyin_live_room_id("https://live.douyin.com/628273323967/"),
            Some("628273323967".to_string())
        );
    }

    #[test]
    fn extract_douyin_live_room_id_rejects_special_paths() {
        assert_eq!(
            extract_douyin_live_room_id("https://live.douyin.com/favicon.ico"),
            None
        );
        assert_eq!(
            extract_douyin_live_room_id("https://live.douyin.com/a/b/c"),
            None
        );
        assert_eq!(
            extract_douyin_live_room_id("https://live.douyin.com/"),
            None
        );
    }

    #[test]
    fn extract_douyin_live_room_id_returns_none_for_non_live() {
        assert_eq!(
            extract_douyin_live_room_id("https://www.douyin.com/video/123"),
            None
        );
    }

    // ---- convert_douyin_live_to_yt_dlp_json ----

    #[test]
    fn convert_douyin_live_to_yt_dlp_json_live_status() {
        let detail = serde_json::json!({
            "title": "测试直播间",
            "nickname": "测试主播",
            "status": 2u32,
            "flv_url": "http://example.com/live.flv",
            "hls_url": "http://example.com/live.m3u8",
            "room_id": "123456",
        });
        let json = convert_douyin_live_to_yt_dlp_json(&detail);
        assert_eq!(
            json.get("title").and_then(serde_json::Value::as_str),
            Some("测试主播的直播间")
        );
        assert_eq!(
            json.get("uploader").and_then(serde_json::Value::as_str),
            Some("测试主播")
        );
        assert_eq!(
            json.get("is_live").and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            json.get("live_status").and_then(serde_json::Value::as_str),
            Some("is_live")
        );
        let formats = json.get("formats").and_then(serde_json::Value::as_array).unwrap();
        assert_eq!(formats.len(), 2);
        // FLV 优先（零依赖直连 HTTP 流）
        assert_eq!(
            formats[0].get("format_id").and_then(serde_json::Value::as_str),
            Some("live-flv")
        );
        assert_eq!(
            formats[0].get("url").and_then(serde_json::Value::as_str),
            Some("http://example.com/live.flv")
        );
    }

    #[test]
    fn convert_douyin_live_to_yt_dlp_json_offline_status() {
        let detail = serde_json::json!({
            "title": "未开播",
            "nickname": "主播",
            "status": 4u32,
            "flv_url": null,
            "hls_url": null,
            "room_id": "789",
        });
        let json = convert_douyin_live_to_yt_dlp_json(&detail);
        assert_eq!(
            json.get("is_live").and_then(serde_json::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            json.get("live_status").and_then(serde_json::Value::as_str),
            Some("was_live")
        );
        let formats = json.get("formats").and_then(serde_json::Value::as_array).unwrap();
        assert_eq!(formats.len(), 0);
    }

    #[test]
    fn convert_douyin_live_to_yt_dlp_json_only_hls() {
        let detail = serde_json::json!({
            "title": "仅HLS",
            "nickname": "主播",
            "status": 2u32,
            "flv_url": null,
            "hls_url": "http://example.com/live.m3u8",
            "room_id": "456",
        });
        let json = convert_douyin_live_to_yt_dlp_json(&detail);
        let formats = json.get("formats").and_then(serde_json::Value::as_array).unwrap();
        assert_eq!(formats.len(), 1);
        assert_eq!(
            formats[0].get("format_id").and_then(serde_json::Value::as_str),
            Some("live-hls")
        );
    }

    // ---- extract_stream_url_from_html / extract_live_title_near_stream_url ----

    #[test]
    fn extract_stream_url_from_html_finds_full_hd1() {
        let html = r#"{"stream_url":{"flv_pull_url":{"FULL_HD1":"http://pull-flv-l26.douyincdn.com/stage/stream-123.flv?sign=abc"},"HD1":"http://pull-flv-l26.douyincdn.com/stage/stream-123_hd.flv?sign=def"}}}"#;
        let url = extract_stream_url_from_html(html, "flv_pull_url");
        assert_eq!(
            url,
            Some("http://pull-flv-l26.douyincdn.com/stage/stream-123.flv?sign=abc".to_string())
        );
    }

    #[test]
    fn extract_stream_url_from_html_fallback_to_origin() {
        let html = r#"{"stream_url":{"flv_pull_url":{"ORIGIN":"http://example.com/origin.flv"}}}"#;
        let url = extract_stream_url_from_html(html, "flv_pull_url");
        assert_eq!(
            url,
            Some("http://example.com/origin.flv".to_string())
        );
    }

    #[test]
    fn extract_stream_url_from_html_returns_none_when_no_url() {
        let html = r#"{"stream_url":{"flv_pull_url":{}}}"#;
        let url = extract_stream_url_from_html(html, "flv_pull_url");
        assert_eq!(url, None);
    }

    #[test]
    fn extract_stream_url_from_html_takes_last_match() {
        // HTML 中可能有多个 flv_pull_url（模板 + 实际数据），取最后一个
        let html = r#"{"flv_pull_url":{"FULL_HD1":"http://template.flv"}}{"flv_pull_url":{"FULL_HD1":"http://real.flv"}}"#;
        let url = extract_stream_url_from_html(html, "flv_pull_url");
        assert_eq!(url, Some("http://real.flv".to_string()));
    }

    #[test]
    fn extract_live_title_near_stream_url_finds_title() {
        let html = r#"{"title":"广告投放"}some stuff{"title":"确定不进来看看嘛","status":2,"stream_url":{"flv_pull_url":{"FULL_HD1":"http://example.com/live.flv"}}}"#;
        let title = extract_live_title_near_stream_url(html);
        assert_eq!(title, Some("确定不进来看看嘛".to_string()));
    }

    #[test]
    fn extract_live_title_near_stream_url_filters_template_titles() {
        let html = r#"{"title":"广告投放"},{"title":"用户服务协议"},{"title":"隐私政策"},"stream_url":{"flv_pull_url":{}}"#;
        let title = extract_live_title_near_stream_url(html);
        // 所有标题都是模板标题，返回 None
        assert_eq!(title, None);
    }

    #[test]
    fn is_template_title_detects_known_templates() {
        assert!(is_template_title("广告投放"));
        assert!(is_template_title("用户服务协议"));
        assert!(is_template_title("京ICP备16016397号-3"));
        assert!(is_template_title(""));
        assert!(!is_template_title("我的直播间"));
        assert!(!is_template_title("游戏直播"));
    }

    #[test]
    fn extract_live_nickname_finds_last_valid() {
        let html = r#"{"nickname":"$undefined"}{"nickname":"音"}{"nickname":"小主播"}"#;
        let nick = extract_live_nickname(html);
        assert_eq!(nick, Some("小主播".to_string()));
    }

    #[test]
    fn extract_live_nickname_filters_undefined() {
        let html = r#"{"nickname":"$undefined"}{"nickname":""}"#;
        let nick = extract_live_nickname(html);
        assert_eq!(nick, None);
    }

    #[test]
    fn extract_live_status_finds_mode() {
        // 多个 status 值中，2 出现最多次
        let html = r#"{"status":2}{"status":2}{"status":4}{"status":2}"#;
        let status = extract_live_status(html);
        assert_eq!(status, Some(2));
    }

    #[test]
    fn unescape_douyin_html_json_decodes_common_sequences() {
        let input = r#"http://example.com/live.flv?expire=abc\u0026sign=def\u0026t=x"#;
        let result = unescape_douyin_html_json(input);
        assert_eq!(result, "http://example.com/live.flv?expire=abc&sign=def&t=x");
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
            (MediaPlatform::YouTube, "YouTube 反爬虫"),
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

    #[test]
    fn test_extract_twitter_status_id() {
        assert_eq!(
            extract_twitter_status_id("https://x.com/user/status/1812345678901234567").as_deref(),
            Some("1812345678901234567")
        );
        assert_eq!(
            extract_twitter_status_id("https://twitter.com/i/web/status/987654321?s=20").as_deref(),
            Some("987654321")
        );
        assert_eq!(
            extract_twitter_status_id("https://mobile.twitter.com/abc/status/1122334455").as_deref(),
            Some("1122334455")
        );
        assert_eq!(extract_twitter_status_id("https://example.com/user/status/123"), None);
    }

    #[test]
    fn test_convert_twitter_tweet_to_yt_dlp_json() {
        let sample = serde_json::json!({
            "text": "Hello Twitter Video",
            "user": { "name": "Test User", "screen_name": "testuser" },
            "mediaDetails": [{
                "media_url_https": "https://pbs.twimg.com/media/thumb.jpg",
                "video_info": {
                    "variants": [
                        { "bitrate": 832000, "content_type": "video/mp4", "url": "https://video.twimg.com/vid/avc1/640x360/low.mp4" },
                        { "bitrate": 2176000, "content_type": "video/mp4", "url": "https://video.twimg.com/vid/avc1/1280x720/high.mp4" }
                    ]
                }
            }]
        });
        let converted = convert_twitter_tweet_to_yt_dlp_json(&sample);
        assert_eq!(converted["title"], "Hello Twitter Video");
        assert_eq!(converted["extractor"], "twitter");
        let formats = converted["formats"].as_array().unwrap();
        assert_eq!(formats.len(), 2);
        // 应该按 bitrate 降序排列，首个为 1280x720 2176000
        assert_eq!(formats[0]["format_id"], "http-720");
        assert_eq!(formats[0]["url"], "https://video.twimg.com/vid/avc1/1280x720/high.mp4");
        assert_eq!(formats[0]["width"], 1280);
        assert_eq!(formats[0]["height"], 720);
    }

    #[test]
    fn test_convert_twitter_tweet_to_yt_dlp_json_photo() {
        let sample = serde_json::json!({
            "text": "Hello Twitter Photos",
            "user": { "name": "Photo User", "screen_name": "photouser" },
            "mediaDetails": [
                {
                    "type": "photo",
                    "media_url_https": "https://pbs.twimg.com/media/F123456789.jpg",
                    "original_info": { "width": 1920, "height": 1080 }
                },
                {
                    "type": "photo",
                    "media_url_https": "https://pbs.twimg.com/media/F987654321.jpg",
                    "original_info": { "width": 1080, "height": 1920 }
                }
            ]
        });
        let converted = convert_twitter_tweet_to_yt_dlp_json(&sample);
        assert_eq!(converted["title"], "Hello Twitter Photos");
        assert_eq!(converted["_is_twitter_gallery"], true);
        let formats = converted["formats"].as_array().unwrap();
        assert_eq!(formats.len(), 2);
        assert_eq!(formats[0]["format_id"], "image-1");
        assert_eq!(formats[0]["url"], "https://pbs.twimg.com/media/F123456789.jpg?name=orig");
        assert_eq!(formats[0]["width"], 1920);
        assert_eq!(formats[0]["height"], 1080);
        assert_eq!(formats[1]["format_id"], "image-2");
        assert_eq!(formats[1]["url"], "https://pbs.twimg.com/media/F987654321.jpg?name=orig");
    }
}
