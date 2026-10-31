use super::{auth_scope, command_config, detect_platform, portable, MediaPlatform};
use crate::media_cookies::{write_cookie_file_in_dir, CookieFileGuard};
use std::path::Path;
use tauri::AppHandle;
use tokio::process::Command;

/// 从 `task.headers` 中提取并移除认证相关头（Cookie/Referer/User-Agent）。
///
/// 这些头不得通过 yt-dlp `--add-header` 传递（命令行对同用户其它进程可见），
/// 必须通过 `--cookies`/`--referer`/`--user-agent` 安全参数传递。
///
/// 返回 (cookie, referer, user_agent, 剩余 headers)。
/// 比较使用大小写不敏感的 header name。
pub(super) fn split_auth_headers(
    headers: &std::collections::HashMap<String, String>,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Vec<(String, String)>,
) {
    let mut cookie = None;
    let mut referer = None;
    let mut user_agent = None;
    let mut rest = Vec::new();
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "cookie" => cookie = Some(value.clone()),
            "referer" | "referrer" => referer = Some(value.clone()),
            "user-agent" => user_agent = Some(value.clone()),
            _ => rest.push((name.clone(), value.clone())),
        }
    }
    (cookie, referer, user_agent, rest)
}

/// 把认证参数附加到 yt-dlp 命令行。
///
/// - Cookie 通过临时文件传递（`--cookies <path>`），不写入命令行
/// - Referer 通过 `--referer` 传递
/// - User-Agent 通过 `--user-agent` 传递
///
/// 返回 `CookieFileGuard` 用于自动清理临时文件。即使 cookie 为空也会返回 guard
/// （内部 path 为 None），调用方可统一 consume。
///
/// 路径解析（Task 34）：优先使用 `portable::resolve_data_dir` 推导的 `cookies_tmp`
/// 子目录，便携模式下数据写入 EXE 同目录的 `data/cookies_tmp/`。
pub(super) async fn attach_auth_args(
    command: &mut Command,
    app: &AppHandle,
    url: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
) -> Result<command_config::AuthFiles, String> {
    let base_dir = portable::resolve_data_dir(app).join("cookies_tmp");
    attach_auth_args_in_dir(command, &base_dir, url, cookie, referer, user_agent).await
}

/// YouTube 反爬虫对策（AGENTS.md §3）：
/// 根据是否有 Cookie 和 PO Token 动态匹配最佳 player_client 组合。
///
/// PO Token 与 Cookie 是两层独立的反爬虫：
/// - Cookie 过"登录墙"（部分视频需要登录才能观看，如年龄限制视频）
/// - PO Token 过"机器人验证墙"（2024 年起 YouTube 强制，yt-dlp 无法自行生成）
///
/// `android_vr` 客户端不需要 PO Token，是绕过 PO Token 墙的最后回退手段。
/// 无论是否有 Cookie，都必须把 `android_vr` 放在客户端列表末尾作为回退，
/// 否则用户配置 Cookie 后反而无法下载（因为 web/mweb/ios 都需要 PO Token）。
/// android_vr 的限制："Made for Kids" 视频不可用（这类视频需要登录态）。
///
/// 参考：https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide
pub(crate) fn apply_youtube_extractor_args(
    command: &mut Command,
    po_token: &str,
    has_cookie: bool,
) {
    let extractor_arg = build_youtube_extractor_arg(po_token, has_cookie);
    command.args(["--extractor-args", &extractor_arg]);
}

/// 纯函数版本：构建 `--extractor-args "youtube:..."` 的参数值。
/// 抽出来便于单元测试（tokio::process::Command 不支持读取已设置参数）。
pub(crate) fn build_youtube_extractor_arg(po_token: &str, has_cookie: bool) -> String {
    let po = po_token.trim();
    let has_po = !po.is_empty();
    match (has_cookie, has_po) {
        (true, true) => format!("youtube:player_client=web,mweb,ios,android_vr;po_token={po}"),
        (true, false) => "youtube:player_client=web,mweb,ios,android_vr".to_string(),
        (false, true) => format!("youtube:player_client=web,mweb,ios,android_vr;po_token={po}"),
        (false, false) => "youtube:player_client=default,mweb,web,android_vr,ios".to_string(),
    }
}

/// 与 `attach_auth_args` 相同，但接受明确的 `base_dir`。
///
/// 用于测试场景（不需要 AppHandle）。生产代码应使用 `attach_auth_args`。
pub(crate) async fn attach_auth_args_in_dir(
    command: &mut Command,
    base_dir: &Path,
    url: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
) -> Result<command_config::AuthFiles, String> {
    auth_scope::validate_referer(referer)?;
    let is_douyin = detect_platform(url) == MediaPlatform::Douyin
        || url.contains("douyin.com")
        || url.contains("douyincdn.com")
        || url.contains("iesdouyin.com");

    let effective_referer = referer
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            if is_douyin {
                Some("https://live.douyin.com/")
            } else {
                None
            }
        });

    let effective_ua = user_agent.filter(|value| !value.trim().is_empty()).or_else(|| {
        Some("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
    });

    let mut options = Vec::new();
    if let Some(value) = effective_referer {
        options.push(("--referer", value.to_owned()));
    }
    if let Some(value) = effective_ua {
        options.push(("--user-agent", value.to_owned()));
    }
    let guard = if let Some(cookie) = cookie.filter(|value| !value.trim().is_empty()) {
        // Header cookies are scoped to the authenticated page URL, never an unrelated Referer.
        write_cookie_file_in_dir(base_dir, cookie, url, None).await?
    } else {
        CookieFileGuard::empty()
    };
    if let Some(path) = guard.path() {
        command.arg("--cookies").arg(path);
    }
    let config = command_config::PrivateConfig::create(base_dir, &options, None).await?;
    config.attach(command);
    Ok(command_config::AuthFiles::new(guard, config))
}
