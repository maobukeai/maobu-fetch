use crate::{
    manager::{
        diagnose::{classify_platform_error, platform_error_to_chinese},
        naming_template::{apply_naming_template, find_template_for_platform, NamingVars},
    },
    media_cookies::{write_cookie_file_in_dir, CookieFileGuard},
    media_platforms::{
        convert_douyin_aweme_to_yt_dlp_json, convert_douyin_live_to_yt_dlp_json, detect_platform,
        expand_short_url, extract_douyin_aweme_id, extract_douyin_live_room_id,
        extract_url_from_share_text, fetch_douyin_aweme_detail_with_credentials,
        fetch_douyin_live_detail, fetch_douyin_live_detail_with_credentials, is_douyin_gallery, is_douyin_live, is_tiktok_gallery,
        is_twitter_space, is_weibo_gallery, strip_tracking_params, MediaPlatform,
    },
    media_tools::{resolve_ffmpeg, resolve_yt_dlp},
    models::AppSettings,
    models::{
        DownloadTask, MediaFormat, MediaProbeResult, MediaType, PlatformNamingTemplate, TaskStatus,
    },
    portable,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

/// 从 `task.headers` 中提取并移除认证相关头（Cookie/Referer/User-Agent）。
///
/// 这些头不得通过 yt-dlp `--add-header` 传递（命令行对同用户其它进程可见），
/// 必须通过 `--cookies`/`--referer`/`--user-agent` 安全参数传递。
///
/// 返回 (cookie, referer, user_agent, 剩余 headers)。
/// 比较使用大小写不敏感的 header name。
fn split_auth_headers(
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
async fn attach_auth_args(
    command: &mut Command,
    app: &AppHandle,
    url: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
) -> Result<CookieFileGuard, String> {
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
pub(crate) fn apply_youtube_extractor_args(command: &mut Command, po_token: &str, has_cookie: bool) {
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
) -> Result<CookieFileGuard, String> {
    let is_douyin = detect_platform(url) == MediaPlatform::Douyin || url.contains("douyin.com") || url.contains("douyincdn.com") || url.contains("iesdouyin.com");

    let effective_referer = referer.filter(|value| !value.trim().is_empty()).or_else(|| {
        if is_douyin {
            Some("https://live.douyin.com/")
        } else {
            None
        }
    });

    let effective_ua = user_agent.filter(|value| !value.trim().is_empty()).or_else(|| {
        Some("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
    });

    let effective_cookie = cookie.filter(|value| !value.trim().is_empty()).map(|s| s.to_string()).or_else(|| {
        if is_douyin {
            Some("passport_csrf_token=43b4f6208a54173872591b6197368d18; passport_csrf_token_default=43b4f6208a54173872591b6197368d18; ttwid=1%7CXFBh1bjNbUX5px8paL7ryFXgrs_rMmh_KQ_SJPKJLUo%7C1784608893%7C6c4fb3d007dd68448ed303b5110aa80cdc48bbfeefddd93e50c1489e59079adb".to_string())
        } else {
            None
        }
    });

    if let Some(ref_val) = effective_referer {
        command.arg("--referer").arg(ref_val);
    }
    if let Some(ua_val) = effective_ua {
        command.arg("--user-agent").arg(ua_val);
    }
    if let Some(cookie_val) = effective_cookie {
        let guard = write_cookie_file_in_dir(base_dir, &cookie_val, url, effective_referer).await?;
        if let Some(path) = guard.path() {
            command.arg("--cookies").arg(path);
        }
        Ok(guard)
    } else {
        Ok(CookieFileGuard::empty())
    }
}

pub async fn probe(
    app: &AppHandle,
    settings: &AppSettings,
    url: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
) -> Result<MediaProbeResult, String> {
    // Task 41.2：用户可能粘贴分享文本（如"xxx https://v.douyin.com/yyy 复制此链接..."），
    // 先从中提取首个 URL。纯 URL 输入会被原样返回；无 URL 时回退到原输入，
    // 让后续 url::Url::parse 给出"媒体地址无效"中文错误（AGENTS.md §7）。
    let extracted_url = extract_url_from_share_text(url).unwrap_or_else(|| url.to_string());
    let parsed = url::Url::parse(&extracted_url).map_err(|_| "媒体地址无效".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("媒体地址无效".into());
    }
    // Task 37.3：识别平台，用于失败时返回平台特定的中文错误。
    let platform = detect_platform(&extracted_url);
    // Task 37.3 / Task 41.1：短链先跟随重定向到最终 URL，再传给 yt-dlp。
    // 失败时不阻断流程（yt-dlp 自身也能处理部分短链），仅在错误信息中体现。
    let expanded = match expand_short_url(&extracted_url).await {
        Ok(final_url) => final_url,
        Err(_) => extracted_url.clone(),
    };
    // Task 41.3：剥离跟踪参数（utm_* / fbclid / gclid 等白名单），
    // 保留业务必需参数（sign / token / auth / X-Amz-Signature 等）。
    let effective_url = strip_tracking_params(&expanded);
    // 抖音优先走直连解析（绕过 byted_acrawler 反爬）。
    // 抖音 web 页面首次访问会返回 JS 挑战页，yt-dlp 无法获取元数据；
    // 而抖音 aweme_detail API 仅需 ttwid + UA + Referer，无需复杂签名。
    // 失败时回退到 yt-dlp 流程，保证用户已安装的旧版 yt-dlp 仍可用。
    //
    // 抖音直播（live.douyin.com）yt-dlp 官方不支持，需要直连请求直播页面 HTML，
    // 从中提取 HLS/FLV 流地址和元数据（标题、主播名），交给 yt-dlp 录制 HLS 流。
    let value: Value = if platform == MediaPlatform::Douyin {
        // 优先检查抖音直播 URL
        if is_douyin_live(&effective_url) {
            if let Some(room_id) = extract_douyin_live_room_id(&effective_url) {
                match crate::media_platforms::fetch_douyin_live_detail_with_credentials(&room_id, cookie, referer, user_agent).await {
                    Ok(detail) => {
                        let mut json = convert_douyin_live_to_yt_dlp_json(&detail);
                        json["webpage_url"] = Value::String(effective_url.clone());
                        tracing::info!(
                            room_id = %room_id,
                            "抖音直播直连解析成功，跳过 yt-dlp 调用"
                        );
                        json
                    }
                    Err(e) => {
                        tracing::warn!(
                            room_id = %room_id,
                            error = %e,
                            "抖音直播直连解析失败，回退到 yt-dlp 流程"
                        );
                        probe_via_yt_dlp(
                            app,
                            settings,
                            &effective_url,
                            platform,
                            cookie,
                            referer,
                            user_agent,
                        )
                        .await?
                    }
                }
            } else {
                probe_via_yt_dlp(
                    app,
                    settings,
                    &effective_url,
                    platform,
                    cookie,
                    referer,
                    user_agent,
                )
                .await?
            }
        } else if let Some(id) = extract_douyin_aweme_id(&effective_url) {
            match fetch_douyin_aweme_detail_with_credentials(&id, cookie, referer, user_agent).await {
                Ok(detail) => {
                    let mut json = convert_douyin_aweme_to_yt_dlp_json(&detail);
                    // 保留原始 URL 给后续命名模板与下载流程使用
                    json["webpage_url"] = Value::String(effective_url.clone());
                    tracing::info!(
                        aweme_id = %id,
                        "抖音直连解析成功，跳过 yt-dlp 调用"
                    );
                    json
                }
                Err(e) => {
                    tracing::warn!(
                        aweme_id = %id,
                        error = %e,
                        "抖音直连解析失败，回退到 yt-dlp 流程"
                    );
                    probe_via_yt_dlp(
                        app,
                        settings,
                        &effective_url,
                        platform,
                        cookie,
                        referer,
                        user_agent,
                    )
                    .await?
                }
            }
        } else {
            probe_via_yt_dlp(
                app,
                settings,
                &effective_url,
                platform,
                cookie,
                referer,
                user_agent,
            )
            .await?
        }
    } else if platform == MediaPlatform::Twitter && !is_twitter_space(&effective_url) {
        if let Some(tweet_id) = crate::media_platforms::extract_twitter_status_id(&effective_url) {
            match crate::media_platforms::fetch_twitter_tweet_detail_with_credentials(&tweet_id, cookie, referer, user_agent).await {
                Ok(detail) => {
                    let mut json = crate::media_platforms::convert_twitter_tweet_to_yt_dlp_json(&detail);
                    json["webpage_url"] = Value::String(effective_url.clone());
                    tracing::info!(
                        tweet_id = %tweet_id,
                        "Twitter 直连解析成功，跳过 yt-dlp 调用"
                    );
                    json
                }
                Err(e) => {
                    tracing::warn!(
                        tweet_id = %tweet_id,
                        error = %e,
                        "Twitter 直连解析失败，回退到 yt-dlp 流程"
                    );
                    probe_via_yt_dlp(
                        app,
                        settings,
                        &effective_url,
                        platform,
                        cookie,
                        referer,
                        user_agent,
                    )
                    .await?
                }
            }
        } else {
            probe_via_yt_dlp(
                app,
                settings,
                &effective_url,
                platform,
                cookie,
                referer,
                user_agent,
            )
            .await?
        }
    } else {
        probe_via_yt_dlp(
            app,
            settings,
            &effective_url,
            platform,
            cookie,
            referer,
            user_agent,
        )
        .await?
    };
    let drm = value
        .get("_has_drm")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // Task 37.4 / Task 38.1：识别抖音与 TikTok 图集类型，记录日志供后续处理参考。
    // yt-dlp 对图集的支持有限，识别后由前端据 extractor 字段决定后续处理。
    // Task 38：图集类型映射到 `MediaType::Gallery`，前端据此选择多图下载策略。
    // Task 39.1：识别 Twitter Spaces 音频 URL，映射到 `MediaType::Audio`，
    // 前端据此展示音频流选择，并提醒用户 Spaces 通常需要登录态。
    // Task 42：识别逻辑提取为 `determine_media_type` 纯函数，便于单元测试，
    // 补充微博图集识别（`is_weibo_gallery`）。
    let media_type = determine_media_type(&effective_url);
    // 抖音直连解析后，API 返回的 aweme_detail.images[] 是图集的真实判定依据。
    // 不能仅靠 URL 路径判断，因为抖音短链可能展开为 /discover?modal_id=xxx
    // 等不含 /note/ 的路径，但 API 返回的 aweme_detail.images[] 仍为图集。
    // convert_douyin_aweme_to_yt_dlp_json 会设置 _is_douyin_gallery 标记。
    let media_type = if (value
        .get("_is_douyin_gallery")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || value
            .get("_is_twitter_gallery")
            .and_then(Value::as_bool)
            .unwrap_or(false))
        && media_type == MediaType::Video
    {
        MediaType::Gallery
    } else {
        media_type
    };
    match media_type {
        MediaType::Gallery => {
            tracing::info!(platform = ?platform, "检测到图集类型 URL");
        }
        MediaType::Audio => {
            tracing::info!(platform = ?platform, "检测到音频类型 URL");
        }
        _ => {}
    }
    // Task 37.6：若 yt-dlp 探测到 DRM，按平台错误返回中文文案（AGENTS.md §6 必须拒绝）。
    if drm {
        let chinese = platform_error_to_chinese(
            crate::manager::diagnose::MediaPlatformError::DrmProtected,
            platform,
        );
        return Err(format!("MEDIA_DRM_PROTECTED: {chinese}"));
    }
    // Task 42：formats 解析提取为 `parse_formats` 纯函数，便于单元测试。
    // 同时识别图片格式项（vcodec=none, acodec=none, ext 为图片扩展名），
    // 填充 `image_url` 字段供图集场景前端展示。
    let mut formats: Vec<MediaFormat> = parse_formats(&value);
    let has_separate_video = formats
        .iter()
        .any(|format| format.has_video && !format.has_audio);
    let has_separate_audio = formats
        .iter()
        .any(|format| !format.has_video && format.has_audio);
    if platform != MediaPlatform::Twitter && has_separate_video && has_separate_audio {
        // 插入合并格式：FFmpeg 可用时由 yt-dlp 调用 FFmpeg 合并；
        // FFmpeg 不可用时由后端内置 media_muxer 合并（仅支持 fMP4，适用于 Twitter/X 等 HLS 场景）。
        // 因此 label 不再强调"需要 FFmpeg"，而是说明会合并音视频。
        // 取分离视频流中最高画质作为合成格式的 width/height，便于前端按高度排序选中。
        let best_video = formats
            .iter()
            .filter(|f| f.has_video && !f.has_audio)
            .max_by_key(|f| f.height.unwrap_or(0));
        let best_height = best_video.and_then(|f| f.height);
        let best_width = best_video.and_then(|f| f.width);
        // 取最高码率音频流 filesize 作为音频大小参考（仅供前端展示，实际合并后大小未知）
        let audio_size = formats
            .iter()
            .filter(|f| !f.has_video && f.has_audio)
            .max_by_key(|f| f.file_size.unwrap_or(0))
            .and_then(|f| f.file_size);
        formats.insert(
            0,
            MediaFormat {
                id: "bestvideo*+bestaudio/best".into(),
                label: "最高画质（合并音视频）".into(),
                extension: Some("mp4".into()),
                width: best_width,
                height: best_height,
                file_size: audio_size,
                has_video: true,
                has_audio: true,
                requires_ffmpeg: true,
                image_url: None,
                url: None,
            },
        );
    }
    // Task 42：图集类型且 `formats` 中未含图片项时，从 `thumbnails` 数组补充。
    // 部分图集 URL（如抖音 note）yt-dlp 把图片放在 thumbnails 而非 formats，
    // 这里做兜底提取，保证前端始终能拿到图片直链列表。
    if media_type == MediaType::Gallery {
        let has_images = formats.iter().any(|f| f.image_url.is_some());
        if !has_images {
            formats.extend(extract_thumbnail_images(&value));
        }
    }
    let subtitles = value
        .get("subtitles")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    Ok(MediaProbeResult {
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("媒体下载")
            .to_string(),
        thumbnail: value
            .get("thumbnail")
            .and_then(Value::as_str)
            .map(str::to_owned),
        extractor: value
            .get("extractor_key")
            .or_else(|| value.get("extractor"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        duration: value.get("duration").and_then(Value::as_f64),
        formats,
        subtitles,
        drm,
        media_type,
        episodes: Vec::new(),
    })
}

/// 通过 yt-dlp 子进程探测媒体元数据（probe 的 yt-dlp 路径）。
///
/// 抽出为独立函数以便抖音直连解析失败时复用同一段 yt-dlp 调用逻辑。
///
/// 安全约束（AGENTS.md §3 / §7）：
/// - Cookie / Referer / User-Agent 通过 `attach_auth_args` 安全传递（不写入命令行）
/// - stderr 在分类前由 `redact_sensitive` 脱敏，不泄露认证信息
/// - 错误按平台分类后返回中文文案
async fn probe_via_yt_dlp(
    app: &AppHandle,
    settings: &AppSettings,
    effective_url: &str,
    platform: MediaPlatform,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
) -> Result<Value, String> {
    let yt = resolve_yt_dlp(app, settings)
        .ok_or("MEDIA_YT_DLP_MISSING: 分析媒体需要先安装 yt-dlp 基础组件")?;
    let mut command = Command::new(yt);
    command.env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1");
    command.args([
        "--dump-single-json",
        "--no-playlist",
        "--no-warnings",
        "--no-check-formats",
    ]);
    // YouTube 反爬虫对策（AGENTS.md §3：不得绕过 DRM，但 PO Token 不是 DRM）：
    // YouTube 自 2024 年起强制 PO Token 验证，yt-dlp 无法自行生成，
    // 普通浏览器 Cookie 会被 YouTube 频繁轮换失效。
    // 使用 player_client 按序回退：默认客户端优先（格式最全），
    // android_vr 客户端不需要 PO Token（限制："Made for kids" 视频不可用），
    // web_safari 客户端的 HLS 格式不需要 GVS PO Token。
    // 参考：https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide
    let cookie_guard = attach_auth_args(
        &mut command,
        app,
        effective_url,
        cookie,
        referer,
        user_agent,
    )
    .await?;
    if platform == MediaPlatform::YouTube {
        apply_youtube_extractor_args(&mut command, &settings.youtube_po_token, cookie_guard.path().is_some());
    }
    command.arg(effective_url);
    let output = command.output().await.map_err(|e| e.to_string())?;
    // 显式删除临时 cookie 文件（即使分析失败也会通过 drop 删除，这里主动清理）
    cookie_guard.consume().await;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        // Task 37.6：按平台分类错误并返回中文文案。
        // AGENTS.md §3：认证信息不得泄露；stderr 在分类前由 redact_sensitive 脱敏。
        let redacted = crate::manager::redact_sensitive(&stderr);
        tracing::error!("yt-dlp 执行失败。脱敏后的 Stderr: \n{}", redacted);
        let platform_error = classify_platform_error(platform, &redacted);
        let chinese = platform_error_to_chinese(platform_error, platform);
        // DrmProtected 错误前缀加 MEDIA_DRM_ 以便前端识别并拒绝（AGENTS.md §6）
        if platform_error == crate::manager::diagnose::MediaPlatformError::DrmProtected {
            return Err(format!("MEDIA_DRM_PROTECTED: {chinese}"));
        }
        return Err(chinese);
    }
    serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())
}

pub async fn download(
    app: &AppHandle,
    settings: &AppSettings,
    mut task: DownloadTask,
    token: CancellationToken,
    naming_templates: Vec<PlatformNamingTemplate>,
) -> Result<DownloadTask, String> {
    let media = task.media.clone().ok_or("缺少媒体格式")?;
    let yt = resolve_yt_dlp(app, settings)
        .ok_or("MEDIA_YT_DLP_MISSING: 下载媒体需要先安装 yt-dlp 基础组件")?;
    let ffmpeg = resolve_ffmpeg(app, settings).map(|tools| tools.ffmpeg);
    let output = PathBuf::from(&task.destination).join(&task.file_name);
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?
    }
    let template = output.to_string_lossy().to_string();
    // 默认格式：用户未通过"解析媒体"选择格式时，使用 `best`（最高画质轻量单文件）。
    // `best` 选择最高码率的 progressive 流（音视频合一的单文件），无需合并，
    // 下载速度快、兼容性好。Twitter/X 场景下为 http-2176（720p MP4）。
    // 用户如需更高画质（HLS 原始流合并），需主动点击"解析媒体"选择合并格式。
    let format = media
        .format_id
        .unwrap_or_else(|| "best".into());
    let requires_ffmpeg = media.requires_ffmpeg || format.contains('+');
    // 合并格式（如 bestvideo*+bestaudio/best）：yt-dlp 输出视频和音频两个独立文件。
    // FFmpeg 可用时由 yt-dlp 调用 FFmpeg 合并；FFmpeg 不可用时由内置 media_muxer 合并
    // （仅支持 fragmented MP4，适用于 Twitter/X 等 HLS 切片场景）。
    // 非合并格式但需要 FFmpeg（如直播流转码）仍必须安装 FFmpeg。
    let is_merge_format = format.contains('+');
    let use_internal_muxer = is_merge_format && ffmpeg.is_none();
    if requires_ffmpeg && ffmpeg.is_none() && !is_merge_format {
        return Err("MEDIA_FFMPEG_MISSING: 当前格式需要 FFmpeg 合并组件".into());
    }
    // 下载前记录输出目录的文件列表，用于下载后识别 yt-dlp 输出的视频/音频文件。
    // 仅在 use_internal_muxer 时需要（FFmpeg 可用时 yt-dlp 直接输出合并后的单文件）。
    let output_dir = output
        .parent()
        .ok_or_else(|| "输出路径无父目录".to_string())?
        .to_path_buf();
    let before_files: Vec<std::path::PathBuf> = if use_internal_muxer {
        match std::fs::read_dir(&output_dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .collect(),
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };
    // 从 task.headers 中分离认证头，避免通过 --add-header 传递 Cookie
    let (cookie, referer, user_agent, safe_headers) = split_auth_headers(&task.headers);
    let mut command = Command::new(yt);
    command.env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1");
    command.args(media_arguments(&format, &template, ffmpeg.is_some()));
    if let Some(path) = ffmpeg {
        command.arg("--ffmpeg-location").arg(path);
    }
    // 应用单任务限速或全局限速 (AGENTS.md §3)
    let speed_limit = if task.per_task_speed_limit > 0 {
        task.per_task_speed_limit
    } else if settings.speed_limit_kbps > 0 {
        settings.speed_limit_kbps * 1024
    } else {
        0
    };
    if speed_limit > 0 {
        command.arg("--limit-rate").arg(format!("{speed_limit}"));
    }
    let target_conn = if task.connection_count > 1 { task.connection_count } else { settings.connections_per_download.max(8) };
    let conn_count = if task.total_bytes == 0 { 1 } else { target_conn };
    task.connection_count = conn_count;
    task.active_connections = conn_count;
    command.arg("--concurrent-fragments").arg(format!("{conn_count}"));
    for (name, value) in &safe_headers {
        command.arg("--add-header").arg(format!("{name}:{value}"));
    }

    let download_url = media.url.as_deref().unwrap_or(&task.url);
    let cookie_guard = attach_auth_args(
        &mut command,
        app,
        download_url,
        cookie.as_deref(),
        referer.as_deref(),
        user_agent.as_deref(),
    )
    .await?;
    let download_platform = detect_platform(&task.url);
    if download_platform == MediaPlatform::YouTube {
        apply_youtube_extractor_args(&mut command, &settings.youtube_po_token, cookie_guard.path().is_some());
    }
    for language in media.subtitles {
        command.args(["--write-subs", "--sub-langs", &language]);
    }
    command.arg(download_url);

    command.stdout(std::process::Stdio::piped());
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("无法获取子进程 stdout")?;

    use tauri::Manager;
    let manager = app.state::<crate::manager::SharedManager>();

    // 启动 yt-dlp 子进程后立即 emit_task 一次，确保前端收到 active_connections > 0 的状态。
    // 否则对于直播流等没有立即输出进度行的场景，前端会因 downloaded_bytes=0 && active_connections=0
    // 误显示"解析中"，直到第一个进度行被解析后才切换为"下载中"，造成状态来回闪烁。
    {
        let _ = manager.store.upsert_task(&task).await;
        manager.emit_task("updated", &task);
    }

    let mut reader = tokio::io::BufReader::new(stdout);
    let mut lines = tokio::io::AsyncBufReadExt::lines(&mut reader);

    let mut last_update = std::time::Instant::now();
    let update_interval = std::time::Duration::from_millis(500);
    // 心跳间隔：即使 yt-dlp 没有输出可解析的进度行（如直播流录制中的 [hls]/[info] 行），
    // 也定期 emit_task，保持前端"下载中"状态稳定，避免回退到"解析中"。
    let heartbeat_interval = std::time::Duration::from_secs(1);
    let mut last_heartbeat = std::time::Instant::now();

    // 直播任务（抖音直播 HLS/FLV）yt-dlp 不输出标准 [download] X% of ... at ... 进度行，
    // 而是输出 [hlsnative] Downloading fragment N 等不可解析的行，导致前端显示 0 字节和 0 速度。
    // 参考本项目 B 站直播等场景的处理：轮询输出文件大小计算已下载字节数和实时速度。
    // --no-part 模式下 yt-dlp 直接写入 output 文件，可安全轮询其大小。
    // 同时检查 format 和 URL，与 manager.rs 中 is_live_task 判断保持一致。
    let is_live = format == "live-hls"
        || format == "live-flv"
        || format.starts_with("live-")
        || crate::media_platforms::is_douyin_live(&task.url)
        || task.url.contains("pull-hls-")
        || task.url.contains("pull-flv-")
        || task.url.contains(".m3u8");
    let mut last_file_size: u64 = 0;
    let mut last_speed_tick = std::time::Instant::now();
    tracing::info!(
        task_id = %task.id,
        format = %format,
        is_live = is_live,
        output = ?output,
        output_dir = ?output_dir,
        download_url = %download_url,
        "直播下载诊断: 进入下载循环"
    );

    let mut exit_status = None;
    loop {
        tokio::select! {
            line_res = lines.next_line() => {
                match line_res {
                    Ok(Some(line)) => {
                        // 直播任务跳过 yt-dlp 进度行解析：
                        // 1. yt-dlp 对直播 HLS/FLV 流可能输出不准确的进度行（值为 0）
                        // 2. 如果 parse_yt_dlp_progress 返回 Some，会重置 last_heartbeat，
                        //    导致文件大小轮询代码永远不执行
                        // 直播任务始终使用文件大小轮询获取真实下载字节数和速度。
                        if let Some((_percent, downloaded, total, speed)) = parse_yt_dlp_progress(&line) {
                            if downloaded > 0 {
                                task.downloaded_bytes = downloaded;
                            }
                            if total > 0 {
                                task.total_bytes = total;
                            }
                            if speed > 0 {
                                task.speed = speed;
                            }
                            if speed > 0 && task.total_bytes > task.downloaded_bytes {
                                task.eta_seconds = Some((task.total_bytes - task.downloaded_bytes) / speed);
                            } else {
                                task.eta_seconds = None;
                            }

                            if last_update.elapsed() >= update_interval {
                                let _ = manager.store.upsert_task(&task).await;
                                manager.emit_task("updated", &task);
                                last_update = std::time::Instant::now();
                                last_heartbeat = std::time::Instant::now();
                            }
                        }
                        // 心跳检查：yt-dlp 持续输出非进度行时，select! 的 sleep 分支会被饿死
                        //（每次循环重新创建 sleep，但 lines.next_line() 总是先就绪）。
                        // 在这里检查心跳间隔，确保即使 yt-dlp 持续输出非进度行，也定期 emit_task，
                        // 保持前端"下载中"状态稳定。
                        // 直播任务也在这里执行文件大小轮询（与 sleep 分支共用 last_speed_tick，
                        // 避免重复计算）。
                        if last_heartbeat.elapsed() >= heartbeat_interval {
                            if is_live {
                                update_live_progress(
                                    &mut task,
                                    &output,
                                    &output_dir,
                                    &mut last_file_size,
                                    &mut last_speed_tick,
                                )
                                .await;
                            }
                            let _ = manager.store.upsert_task(&task).await;
                            manager.emit_task("updated", &task);
                            last_heartbeat = std::time::Instant::now();
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
            status = child.wait() => {
                exit_status = Some(status.map_err(|e| e.to_string())?);
                break;
            }
            _ = tokio::time::sleep(heartbeat_interval) => {
                // 兜底心跳：yt-dlp 完全没有输出（如网络等待）时，定期 emit_task。
                // 注意：此分支在 yt-dlp 持续输出非进度行时可能被饿死，
                // 主要靠 lines.next_line() 分支中的心跳检查兜底。
                if is_live {
                    update_live_progress(
                        &mut task,
                        &output,
                        &output_dir,
                        &mut last_file_size,
                        &mut last_speed_tick,
                    )
                    .await;
                }
                let _ = manager.store.upsert_task(&task).await;
                manager.emit_task("updated", &task);
                last_heartbeat = std::time::Instant::now();
            }
            _ = token.cancelled() => {
                // 直播录制暂停 = 结束录制。参考 B 站直播流程：
                // 这里只 kill yt-dlp 子进程并释放文件句柄，返回 Err("任务已暂停")。
                // 暂停保存逻辑（重命名 + remux + Completed）由 manager.rs 的调用点
                // 统一处理（与 B 站直播 download_stream 暂停后的逻辑一致）。
                let _ = child.kill().await;
                let _ = child.wait().await; // 确保子进程完全退出，释放文件句柄
                cookie_guard.consume().await;
                return Err("任务已暂停".into());
            }
        }
    }

    let status = match exit_status {
        Some(s) => s,
        None => child.wait().await.map_err(|e| e.to_string())?,
    };
    if !status.success() {
        cookie_guard.consume().await;
        return Err(format!("yt-dlp 退出码：{}", status.code().unwrap_or(-1)));
    }

    // 下载完成，清理临时 cookie 文件
    cookie_guard.consume().await;

    // 内置合并器：yt-dlp 在无 FFmpeg 时对合并格式输出视频和音频两个独立文件，
    // 这里用纯 Rust 实现的 media_muxer 合并为单个 fMP4 文件。
    // 仅支持 fragmented MP4（Twitter/X 等 HLS 切片场景）；其它格式会返回错误。
    if use_internal_muxer {
        if let Err(e) = merge_split_tracks(&output_dir, &before_files, &output).await {
            tracing::warn!(task_id = %task.id, error = %e, "内置音视频合并失败");
            return Err(format!("内置音视频合并失败：{e}"));
        }
    }

    let metadata = tokio::fs::metadata(&output)
        .await
        .map_err(|e| e.to_string())?;
    task.total_bytes = metadata.len();
    task.downloaded_bytes = metadata.len();
    task.status = TaskStatus::Completed;
    // Task 43：下载完成后按平台命名模板重命名文件。
    // 失败时不阻塞下载完成（仅记录 tracing 警告），保留 yt-dlp 原始输出文件名。
    // 命名模板只对新建任务的下载文件生效（AGENTS.md §3），此处 task.file_name
    // 已是 yt-dlp 写入磁盘的真实文件名（可能因合并音视频等改为 .mp4 扩展名）。
    if let Err(e) = apply_platform_naming_template(
        app,
        settings,
        &mut task,
        &output,
        cookie.as_deref(),
        referer.as_deref(),
        user_agent.as_deref(),
        &naming_templates,
    )
    .await
    {
        tracing::warn!(task_id = %task.id, error = %e, "平台命名模板应用失败，保留原始文件名");
    }
    Ok(task)
}

fn media_arguments(format: &str, template: &str, has_ffmpeg: bool) -> Vec<String> {
    let effective_format = if format == "live-hls" || format == "live-flv" || format.starts_with("live-") {
        "b/best"
    } else {
        format
    };
    let mut arguments = vec![
        "--newline".into(),
        "--no-colors".into(),
        "--no-playlist".into(),
        "--no-part".into(),
        "-f".into(),
        effective_format.into(),
    ];
    if has_ffmpeg {
        arguments.extend(["--merge-output-format".into(), "mp4".into()]);
    }
    arguments.extend(["-o".into(), template.into()]);
    arguments
}

/// 内置音视频合并：扫描下载目录，识别 yt-dlp 在无 FFmpeg 时输出的两个独立文件
/// （视频轨 + 音频轨），调用 `media_muxer::merge_fragmented_mp4` 合并为单个 fMP4 文件。
///
/// 识别规则：对比 `before_files`（下载前的目录文件列表），找出下载后新增的文件。
/// 新增文件恰好 2 个时，按文件名启发式识别视频和音频（文件名包含 "audio" 的为音频轨）。
///
/// 合并成功后删除原始的两个文件。合并失败时保留原始文件，由调用方决定如何处理。
///
/// 安全约束（AGENTS.md §3 / §7）：
/// - 不使用 `unwrap()` / `expect()` 处理可恢复错误
/// - 合并使用临时文件 + 原子重命名（由 `media_muxer` 内部实现）
/// - 不删除用户已有文件（仅删除本次下载产生的新文件）
async fn merge_split_tracks(
    output_dir: &Path,
    before_files: &[std::path::PathBuf],
    output: &Path,
) -> Result<(), String> {
    // 扫描下载后的目录文件
    let after_entries = std::fs::read_dir(output_dir)
        .map_err(|e| format!("读取输出目录失败：{e}"))?;
    let after_files: Vec<std::path::PathBuf> = after_entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();

    // 找出新增文件（下载产生）
    let new_files: Vec<&std::path::PathBuf> = after_files
        .iter()
        .filter(|p| !before_files.contains(p))
        .collect();

    if new_files.is_empty() {
        return Err("未找到 yt-dlp 输出的文件".into());
    }

    // 如果只有一个新文件，可能 yt-dlp 已经合并（不应该发生，但兜底处理）
    if new_files.len() == 1 {
        let src = new_files[0];
        if src != output {
            tokio::fs::rename(src, output)
                .await
                .map_err(|e| format!("重命名单文件失败：{e}"))?;
        }
        return Ok(());
    }

    // 期望恰好 2 个文件：视频 + 音频
    if new_files.len() != 2 {
        return Err(format!(
            "期望 2 个输出文件（视频+音频），实际找到 {} 个：{:?}",
            new_files.len(),
            new_files
        ));
    }

    // 启发式识别：文件名包含 "audio" 的为音频轨，另一个为视频轨
    let (video_path, audio_path) = {
        let first = new_files[0];
        let second = new_files[1];
        let first_name = first.to_string_lossy().to_lowercase();
        let second_name = second.to_string_lossy().to_lowercase();
        if first_name.contains("audio") && !second_name.contains("audio") {
            (second, first)
        } else if second_name.contains("audio") && !first_name.contains("audio") {
            (first, second)
        } else {
            // 启发式失败：按文件大小判断（视频通常大于音频）
            let first_size = std::fs::metadata(first).map(|m| m.len()).unwrap_or(0);
            let second_size = std::fs::metadata(second).map(|m| m.len()).unwrap_or(0);
            if first_size >= second_size {
                (first, second)
            } else {
                (second, first)
            }
        }
    };

    tracing::info!(
        video = %video_path.display(),
        audio = %audio_path.display(),
        output = %output.display(),
        "开始内置合并音视频轨"
    );

    // 调用 media_muxer 合并
    crate::media_muxer::merge_fragmented_mp4(video_path, audio_path, output)
        .await
        .map_err(|e| format!("合并 fMP4 失败：{e}"))?;

    // 合并成功后删除原始文件（仅删除本次下载产生的，不影响用户已有文件）
    let _ = tokio::fs::remove_file(video_path).await;
    let _ = tokio::fs::remove_file(audio_path).await;

    Ok(())
}

/// Task 43：下载完成后按平台命名模板重命名文件。
///
/// 流程：
/// 1. 检测 URL 对应的 `MediaPlatform`，跳过 `Unknown` 平台（无内置模板）。
/// 2. 从 `naming_templates` 中找出该平台第一条启用的模板；无则跳过。
/// 3. 调用 yt-dlp `--dump-single-json --skip-download` 获取媒体元数据
///    （不复用下载命令的输出，避免改变下载/取消流程）。
/// 4. 从元数据构建 `NamingVars`，套用模板生成新文件名（不含扩展名）。
/// 5. 拼接原扩展名（保留 yt-dlp 输出的实际扩展名，如合并后的 `.mp4`），
///    重命名磁盘文件并更新 `task.file_name`。
///
/// 失败语义：任一步骤失败返回 `Err`，由调用方记录警告但不阻塞下载完成。
/// 不删除原文件、不修改 `task.file_name`（保留 yt-dlp 默认命名）。
///
/// 安全约束（AGENTS.md §3 / §7）：
/// - 不使用 `unwrap()` / `expect()` 处理可恢复错误
/// - Cookie 通过临时文件传递（与下载流程一致的 `attach_auth_args`）
/// - 重命名使用 `tokio::fs::rename`（同卷原子替换）；目标已存在时返回错误，
///   不静默覆盖用户文件（AGENTS.md §7 重名策略）
pub(crate) async fn apply_platform_naming_template(
    app: &AppHandle,
    settings: &AppSettings,
    task: &mut DownloadTask,
    current_path: &Path,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
    naming_templates: &[PlatformNamingTemplate],
) -> Result<(), String> {
    let mut platform = detect_platform(&task.url);
    if matches!(platform, MediaPlatform::Unknown) {
        if let Some(referer) = task.headers.get("Referer") {
            platform = detect_platform(referer);
        }
    }
    if matches!(platform, MediaPlatform::Unknown) {
        if let Some(final_url) = &task.final_url {
            platform = detect_platform(final_url);
        }
    }
    if matches!(platform, MediaPlatform::Unknown) {
        return Ok(());
    }
    let default_template = PlatformNamingTemplate {
        id: "default_fallback".into(),
        platform: platform.as_str().into(),
        template: "{title}".into(),
        enabled: true,
        is_builtin: true,
    };
    let template = find_template_for_platform(naming_templates, platform.as_str())
        .unwrap_or(&default_template);
    let yt = resolve_yt_dlp(app, settings)
        .ok_or_else(|| "MEDIA_YT_DLP_MISSING: 应用命名模板需要 yt-dlp".to_string())?;
    // 短链先跟随重定向（与 probe 一致），避免 yt-dlp 解析短链失败。
    let target_url = if let Some(referer) = task.headers.get("Referer") {
        if detect_platform(referer) != MediaPlatform::Unknown || referer.contains("douyin.com") {
            referer.as_str()
        } else {
            task.final_url.as_deref().unwrap_or(&task.url)
        }
    } else {
        task.final_url.as_deref().unwrap_or(&task.url)
    };
    let effective_url = match expand_short_url(target_url).await {
        Ok(final_url) => final_url,
        Err(_) => target_url.to_string(),
    };
    let mut command = Command::new(yt);
    command.env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1");
    command.args([
        "--dump-single-json",
        "--no-playlist",
        "--no-warnings",
        "--skip-download",
        "--no-check-formats",
        "--no-call-home",
    ]);
    let cookie_guard = attach_auth_args(
        &mut command,
        app,
        &effective_url,
        cookie,
        referer,
        user_agent,
    )
    .await?;
    if platform == MediaPlatform::YouTube {
        apply_youtube_extractor_args(&mut command, &settings.youtube_po_token, cookie_guard.path().is_some());
    }
    command.arg(&effective_url);
    let output = command.output().await.map_err(|e| e.to_string())?;
    cookie_guard.consume().await;
    if !output.status.success() {
        return Err("yt-dlp 元数据探测失败".into());
    }
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    let vars = build_naming_vars(&value, platform);
    let mut new_stem = apply_naming_template(&template.template, &vars);

    // 应用文件名清理规则（如去除 #话题 标签）
    use tauri::Manager;
    let manager = app.state::<crate::manager::SharedManager>();
    if let Ok(rules) = manager.store.filename_cleanup_rule_list().await {
        let after = crate::manager::apply_filename_cleanup(&new_stem, &rules);
        if !after.is_empty() {
            new_stem = after;
        }
    }

    // 优先从 yt-dlp 元数据 JSON 中提取格式扩展名，无则使用 "mp4" 兜底
    let ext_from_json = value
        .get("ext")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "mp4".to_string())
        .replace(".", "");
    // 拼接原文件扩展名：保留 yt-dlp 实际输出的扩展名（合并后可能是 .mp4）。
    // 若原文件名无扩展名，这里从元数据中恢复扩展名。
    let original_extension = current_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_string())
        .unwrap_or(ext_from_json);
    let new_file_name = format!("{new_stem}.{original_extension}");
    // 新旧文件名相同时跳过重命名（避免无意义的 IO）。
    if new_file_name == task.file_name {
        return Ok(());
    }
    let target_dir = current_path
        .parent()
        .ok_or_else(|| "目标路径无父目录".to_string())?;
    let new_path = target_dir.join(&new_file_name);

    let final_new_path = if new_path.exists() && new_path != current_path {
        let stem = new_stem.clone();
        let ext = original_extension.clone();
        let mut count = 1;
        let mut candidate = target_dir.join(format!("{stem} ({count}).{ext}"));
        while candidate.exists() {
            count += 1;
            candidate = target_dir.join(format!("{stem} ({count}).{ext}"));
        }
        candidate
    } else {
        new_path
    };

    tokio::fs::rename(current_path, &final_new_path)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(final_name) = final_new_path.file_name().and_then(|s| s.to_str()) {
        task.file_name = final_name.to_string();
    }
    Ok(())
}

/// 从 yt-dlp `--dump-single-json` 输出构建命名模板变量集合。
///
/// 字段映射（AGENTS.md §3：所有变量来自真实状态）：
/// - `author`：`uploader` > `channel` > `uploader_id`（优先级回退）
/// - `title`：`title`
/// - `date`：`upload_date`（yt-dlp 已格式化为 `YYYYMMDD`）
/// - `platform`：`MediaPlatform::as_str()`
/// - `id`：`id`（站点视频 ID）
/// - `channel`：`channel`
/// - `bvid`：`display_id`（B 站场景下为 BV 号；非 B 站通常与 `id` 相同或缺失，
///   模板不使用 `{bvid}` 时不影响结果）
fn build_naming_vars(value: &Value, platform: MediaPlatform) -> NamingVars {
    let str_field = |key: &str| -> Option<String> {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let author = str_field("uploader")
        .or_else(|| str_field("channel"))
        .or_else(|| str_field("uploader_id"));
    NamingVars {
        author,
        title: str_field("title"),
        date: str_field("upload_date"),
        platform: Some(platform.as_str().to_string()),
        id: str_field("id"),
        channel: str_field("channel"),
        bvid: str_field("display_id"),
    }
}

/// Task 42：根据 URL 模式判定媒体内容类型（纯函数）。
///
/// 识别规则：
/// - `Gallery`：抖音 note / TikTok photo / 微博 album 图集 URL
/// - `Audio`：Twitter Spaces 音频 URL
/// - `Video`：其它所有 URL（默认值，向后兼容）
///
/// 提取为纯函数以便单元测试，无需启动 yt-dlp 子进程即可验证识别逻辑。
/// AGENTS.md §3：识别基于真实 URL 模式，不使用模拟数据。
fn determine_media_type(url: &str) -> MediaType {
    if is_douyin_gallery(url) || is_tiktok_gallery(url) || is_weibo_gallery(url) {
        MediaType::Gallery
    } else if is_twitter_space(url) {
        MediaType::Audio
    } else {
        MediaType::Video
    }
}

/// Task 42：从 yt-dlp JSON 输出中解析格式列表（纯函数）。
///
/// 同时识别视频/音频格式与图片格式（图集场景）。图片项的判定：
/// - `vcodec == "none"` 且 `acodec == "none"`
/// - `ext` 为 jpg/jpeg/png/webp/gif/bmp 之一（大小写不敏感）
///
/// 图片项填充 `image_url` 字段；视频/音频项的 `image_url` 始终为 `None`。
/// 提取为纯函数便于单元测试，不依赖网络或子进程。
fn parse_formats(value: &Value) -> Vec<MediaFormat> {
    value
        .get("formats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(parse_single_format)
        .collect()
}

/// Task 42：解析单个 yt-dlp format 项为 `MediaFormat`。
///
/// 同时识别图片项并填充 `image_url`，便于图集场景前端展示。
/// `vcodec` / `acodec` 默认 `"none"`（与 yt-dlp 习惯一致）；
/// `format_id` 缺失时返回 `None`，跳过该格式项。
///
/// **Twitter/X progressive 流处理**：yt-dlp 对 Twitter `http-*` 格式（progressive
/// 音视频合一）返回的 `vcodec`/`acodec` 为 JSON `null`（而非 `"none"`）。
/// 旧逻辑把 null 当 `"none"` 处理，导致最高画质的有声直链被误判为"无视频无音频"，
/// 前端无法正确选择。本函数对 null/空字符串做如下推断：
/// - 有 `width`/`height` → 一定有视频
/// - `vcodec` 和 `acodec` 都未知（null/空）→ progressive 流，既有视频又有音频
/// - `vcodec` 为 `"none"` 且无 `width`/`height` → 纯音频流（`acodec` 可能为 null）
fn parse_single_format(item: &Value) -> Option<MediaFormat> {
    let id = item.get("format_id")?.as_str()?.to_string();
    // vcodec/acodec 保留 Option 以区分三种状态：
    //   Some("none") = 明确无此轨；Some("avc1...") = 有此轨；None = JSON null/缺失（未知）
    // yt-dlp 对 Twitter http-* progressive 流返回 vcodec/acodec 为 null（而非 "none"），
    // 旧逻辑把 null 当 "none" 处理，导致最高画质有声直链被误判为"无视频无音频"。
    let vcodec: Option<&str> = item.get("vcodec").and_then(Value::as_str);
    let acodec: Option<&str> = item.get("acodec").and_then(Value::as_str);
    let width = item.get("width").and_then(Value::as_u64);
    let height = item.get("height").and_then(Value::as_u64);
    let ext = item.get("ext").and_then(Value::as_str).map(str::to_owned);
    let size = item
        .get("filesize")
        .or_else(|| item.get("filesize_approx"))
        .and_then(Value::as_u64);
    let url = item.get("url").and_then(Value::as_str).map(str::to_owned);
    let ext_lower = ext.as_deref().map(|e| e.to_ascii_lowercase());
    let format_note = item.get("format_note").and_then(Value::as_str);
    // 过滤 YouTube 预览图/底片流（storyboard）：
    // yt-dlp 对 YouTube 序列帧图片格式返回 format_id 为 sb3/sb2/sb1/sb0 或 format_note 为 "storyboard"
    if format_note == Some("storyboard")
        || id.starts_with("sb0")
        || id.starts_with("sb1")
        || id.starts_with("sb2")
        || id.starts_with("sb3")
        || ext_lower.as_deref() == Some("mhtml")
    {
        return None;
    }
    // 明确无此轨（yt-dlp 返回字符串 "none"）
    let vcodec_is_explicit_none = vcodec == Some("none");
    let acodec_is_explicit_none = acodec == Some("none");
    // 未知：JSON null/缺失/空字符串（yt-dlp 对 progressive 流不返回编解码信息）
    let vcodec_is_unknown = vcodec.map_or(true, |s| s.is_empty());
    let acodec_is_unknown = acodec.map_or(true, |s| s.is_empty());
    let has_dimensions = width.is_some() || height.is_some();
    // 图片项：vcodec/acodec 均明确为 "none" 且扩展名为图片格式（优先判定，避免被 has_dimensions 误判为视频）
    let is_image = vcodec_is_explicit_none
        && acodec_is_explicit_none
        && matches!(
            ext_lower.as_deref(),
            Some("jpg") | Some("jpeg") | Some("png") | Some("webp") | Some("gif") | Some("bmp")
        );
    // has_video: 明确有编解码，或有画面尺寸（排除图片）
    let has_video = !is_image
        && ((!vcodec_is_explicit_none && !vcodec_is_unknown) || has_dimensions);
    // has_audio: 明确有编解码；或编解码未知但视频也未知（progressive 流）；
    // 或编解码未知且无视频（纯音频流，acodec 可能为 null）
    let has_audio = !is_image
        && ((!acodec_is_explicit_none && !acodec_is_unknown)
            || (acodec_is_unknown && vcodec_is_unknown)
            || (acodec_is_unknown && !has_video));
    let label = if is_image {
        match (width, height) {
            (Some(w), Some(h)) => format!("图片 {w}×{h}"),
            _ => "图片".to_string(),
        }
    } else {
        item.get("format_note")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| match (height, ext.as_deref()) {
                (Some(h), Some(ext)) => format!("{h}p · {ext}"),
                (None, Some(ext)) => ext.to_string(),
                _ => id.clone(),
            })
    };
    Some(MediaFormat {
        id,
        label,
        extension: ext,
        width,
        height,
        file_size: size,
        has_video,
        has_audio,
        requires_ffmpeg: false,
        image_url: if is_image { url.clone() } else { None },
        url,
    })
}

/// Task 42：从 yt-dlp `thumbnails` 数组中提取图片格式项（纯函数）。
///
/// 仅在 `formats` 中未含图片项时（图集类型 URL 但 yt-dlp 把图片放在 thumbnails）使用。
/// 每个含非空 `url` 字段 of thumbnail 转换为一个 `MediaFormat`，`id` 形如 `image-<idx>`，
/// `extension` 默认 `"jpg"`（与抖音/TikTok 图集常见格式一致）。
fn extract_thumbnail_images(value: &Value) -> Vec<MediaFormat> {
    let mut result = Vec::new();
    let thumbnails = match value.get("thumbnails").and_then(Value::as_array) {
        Some(arr) => arr,
        None => return result,
    };
    for (idx, thumb) in thumbnails.iter().enumerate() {
        let url = match thumb.get("url").and_then(Value::as_str) {
            Some(u) if !u.is_empty() => u.to_owned(),
            _ => continue,
        };
        let width = thumb.get("width").and_then(Value::as_u64);
        let height = thumb.get("height").and_then(Value::as_u64);
        let label = match (width, height) {
            (Some(w), Some(h)) => format!("图片 {w}×{h}"),
            _ => format!("图片 {}", idx + 1),
        };
        result.push(MediaFormat {
            id: format!("image-{idx}"),
            label,
            extension: Some("jpg".to_string()),
            width,
            height,
            file_size: None,
            has_video: false,
            has_audio: false,
            requires_ffmpeg: false,
            image_url: Some(url.clone()),
            url: Some(url),
        });
    }
    result
}

/// 更新直播任务的下载进度（downloaded_bytes 和 speed）。
///
/// 通过扫描输出目录找到 yt-dlp 实际写入的文件（扩展名可能与 template 不同），
/// 读取文件大小作为 downloaded_bytes，用两次轮询的文件大小增量除以时间差计算 speed。
/// `last_file_size` 和 `last_speed_tick` 由调用方持有，确保两个心跳分支共用同一基准。
async fn update_live_progress(
    task: &mut DownloadTask,
    output: &Path,
    output_dir: &Path,
    last_file_size: &mut u64,
    last_speed_tick: &mut std::time::Instant,
) {
    let stem = output
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if stem.is_empty() {
        return;
    }
    let live_file = match find_live_output_file(output_dir, stem).await {
        Some(f) => f,
        None => return,
    };
    let meta = match tokio::fs::metadata(&live_file).await {
        Ok(m) => m,
        Err(_) => return,
    };
    let current_size = meta.len();
    let elapsed = last_speed_tick.elapsed().as_secs_f64();
    // 第一次轮询时 last_file_size 为 0，delta 会等于整个文件大小，
    // 导致 speed 等于 downloaded_bytes（前端显示"网速=文件大小"）。
    // 第一次只记录基准值，不计算速度。
    if *last_file_size > 0 && elapsed > 0.0 {
        let delta = current_size.saturating_sub(*last_file_size);
        task.speed = (delta as f64 / elapsed) as u64;
    }
    task.downloaded_bytes = current_size;
    *last_file_size = current_size;
    *last_speed_tick = std::time::Instant::now();
}

/// 在输出目录中查找直播录制输出的文件。
///
/// yt-dlp 录制直播 HLS/FLV 流时，实际输出的文件扩展名可能与 template 指定的不同
/// （例如 template 是 `xxx.mp4`，但 yt-dlp 实际写入 `xxx.ts` 或 `xxx.mkv`）。
/// `--no-part` 对某些协议可能无效，yt-dlp 仍可能使用 `.part` 文件。
///
/// 查找策略：
/// 1. 优先匹配以 stem 开头的文件（最可靠）
/// 2. 如果 stem 匹配不到，回退到目录中最大的非 cookie 临时文件
pub async fn find_live_output_file(output_dir: &Path, raw_stem_or_filename: &str) -> Option<PathBuf> {
    let clean_stem = std::path::Path::new(raw_stem_or_filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(raw_stem_or_filename);
    let mut entries = tokio::fs::read_dir(output_dir).await.ok()?;
    let mut stem_match: Option<(PathBuf, u64)> = None;
    let mut fallback: Option<(PathBuf, u64)> = None;
    while let Some(entry) = entries.next_entry().await.ok()? {
        if let Some(name) = entry.file_name().to_str() {
            // 排除 cookie 临时文件和 .tmp 文件
            if name.ends_with(".tmp") || name.contains("cookies") {
                continue;
            }
            if let Ok(meta) = entry.metadata().await {
                let size = meta.len();
                // stem 匹配优先
                if name.starts_with(clean_stem) {
                    match &stem_match {
                        Some((_, s)) if size <= *s => {}
                        _ => stem_match = Some((entry.path(), size)),
                    }
                }
                // 回退：所有非 cookie 文件中最大的（排除 0 字节文件）
                if size > 0 {
                    match &fallback {
                        Some((_, s)) if size <= *s => {}
                        _ => fallback = Some((entry.path(), size)),
                    }
                }
            }
        }
    }
    stem_match
        .or(fallback)
        .map(|(p, _)| p)
}

fn parse_yt_dlp_progress(line: &str) -> Option<(f64, u64, u64, u64)> {
    let trimmed = line.trim_matches(|c: char| c == '\r' || c == '\n' || c == ' ' || c == '\t');
    let clean = if let Some(idx) = trimmed.find("[download]") {
        &trimmed[idx..]
    } else {
        return None;
    };
    let parts: Vec<&str> = clean.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    if let Some(pct_idx) = parts.iter().position(|p| p.ends_with('%')) {
        let pct_str = parts[pct_idx].strip_suffix('%')?;
        let percent: f64 = pct_str.parse().ok()?;

        let of_idx = parts.iter().position(|p| *p == "of")?;
        // yt-dlp 对 HLS fragment 进度输出形如：
        //   [download] 10.0% of ~   8.82KiB at 412.33B/s ETA Unknown (frag 0/10)
        // `~` 是独立 token（表示估算值），需要跳过取下一个 token 作为 size。
        let size_idx = of_idx + 1;
        let size_str = if parts.get(size_idx) == Some(&"~") {
            parts.get(size_idx + 1)?
        } else {
            parts.get(size_idx)?
        };
        let total_bytes = parse_size_to_bytes(size_str)?;

        let speed = parts.iter().position(|p| *p == "at")
            .and_then(|at_idx| parts.get(at_idx + 1))
            .and_then(|s| parse_speed_to_bytes(s))
            .unwrap_or(0);

        let downloaded_bytes = ((percent / 100.0) * (total_bytes as f64)) as u64;
        return Some((percent, downloaded_bytes, total_bytes, speed));
    }
    if let Some(at_idx) = parts.iter().position(|p| *p == "at") {
        if at_idx >= 2 {
            let downloaded = parse_size_to_bytes(parts[at_idx - 1])?;
            let speed = parts.get(at_idx + 1).and_then(|s| parse_speed_to_bytes(s)).unwrap_or(0);
            return Some((0.0, downloaded, 0, speed));
        }
    }
    None
}

fn parse_size_to_bytes(s: &str) -> Option<u64> {
    let clean = s.trim_start_matches('~');
    let (num_part, unit) = if clean.ends_with("KiB") || clean.ends_with("KIB") || clean.ends_with("kb") || clean.ends_with("KB") {
        (clean.get(..clean.len()-3).or_else(|| clean.get(..clean.len()-2))?, 1024_f64)
    } else if clean.ends_with("MiB") || clean.ends_with("MIB") || clean.ends_with("mb") || clean.ends_with("MB") {
        (clean.get(..clean.len()-3).or_else(|| clean.get(..clean.len()-2))?, 1024.0 * 1024.0)
    } else if clean.ends_with("GiB") || clean.ends_with("GIB") || clean.ends_with("gb") || clean.ends_with("GB") {
        (clean.get(..clean.len()-3).or_else(|| clean.get(..clean.len()-2))?, 1024.0 * 1024.0 * 1024.0)
    } else if clean.ends_with("B") {
        (clean.get(..clean.len()-1)?, 1.0)
    } else {
        (clean, 1.0)
    };
    let val: f64 = num_part.trim().parse().ok()?;
    Some((val * unit) as u64)
}

fn parse_speed_to_bytes(s: &str) -> Option<u64> {
    let clean = s.strip_suffix("/s")?;
    parse_size_to_bytes(clean)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn lightweight_media_download_does_not_require_ffmpeg_arguments() {
        let arguments = media_arguments("18", "video.mp4", false);
        assert!(!arguments
            .iter()
            .any(|value| value == "--merge-output-format"));
        assert!(arguments.windows(2).any(|pair| pair == ["-f", "18"]));
    }

    /// 验证 build_youtube_extractor_arg 在所有 (has_cookie, has_po) 组合下
    /// 都把 android_vr 作为最后回退客户端纳入列表。
    #[test]
    fn youtube_extractor_args_always_includes_android_vr_as_fallback() {
        // 关键回归测试：android_vr 是唯一不需要 PO Token 的客户端，
        // 必须在所有 4 种 (has_cookie, has_po) 组合中都出现作为最后回退。
        // 否则用户配置 Cookie 后反而无法下载（因为 web/mweb/ios 都需要 PO Token）。
        let cases = [
            ("token-abc", true),
            ("", true),
            ("token-abc", false),
            ("", false),
        ];
        for (po_token, has_cookie) in cases {
            let arg = build_youtube_extractor_arg(po_token, has_cookie);
            assert!(
                arg.contains("android_vr"),
                "android_vr must be in client list for (has_cookie={}, po_token={:?}). Got: {}",
                has_cookie,
                po_token,
                arg
            );
        }
    }

    #[test]
    fn youtube_extractor_args_includes_po_token_when_provided() {
        let arg = build_youtube_extractor_arg("my-po-token", false);
        assert!(arg.contains("po_token=my-po-token"), "po_token must be in args. Got: {}", arg);
    }

    #[test]
    fn youtube_extractor_args_omits_po_token_when_empty() {
        let arg = build_youtube_extractor_arg("", true);
        assert!(!arg.contains("po_token="), "po_token must NOT be in args when empty. Got: {}", arg);
    }

    #[test]
    fn youtube_extractor_args_with_cookie_uses_web_clients_first() {
        // 有 Cookie 时优先用 web 系客户端（带登录态），android_vr 作为回退
        let arg = build_youtube_extractor_arg("", true);
        let clients_part = arg.split(';').next().unwrap_or("");
        assert!(clients_part.contains("web"), "web client must be first when has_cookie. Got: {}", arg);
        // android_vr 应该在 web 系之后
        let web_pos = clients_part.find("web").unwrap();
        let vr_pos = clients_part.find("android_vr").unwrap();
        assert!(vr_pos > web_pos, "android_vr must come after web clients. Got: {}", arg);
    }

    #[test]
    fn test_parse_yt_dlp_progress() {
        // Test case 1: Standard progress with percent, total size, speed and ETA
        let line = "[download]   5.2% of  201.95MiB at    8.43MiB/s ETA 00:22";
        let res = parse_yt_dlp_progress(line);
        assert!(res.is_some());
        let (pct, downloaded, total, speed) = res.unwrap();
        assert!((pct - 5.2).abs() < 1e-6);
        assert_eq!(total, (201.95 * 1024.0 * 1024.0) as u64);
        assert_eq!(speed, (8.43 * 1024.0 * 1024.0) as u64);
        assert_eq!(downloaded, ((5.2 / 100.0) * (total as f64)) as u64);

        // Test case 2: Estimated size progress (with ~)
        let line_est = "[download]  50.2% of ~100.00MB at  1.50MB/s ETA 00:30";
        let res_est = parse_yt_dlp_progress(line_est);
        assert!(res_est.is_some());
        let (pct, downloaded, total, speed) = res_est.unwrap();
        assert!((pct - 50.2).abs() < 1e-6);
        assert_eq!(total, 100 * 1024 * 1024);
        assert_eq!(speed, (1.5 * 1024.0 * 1024.0) as u64);

        // Test case 3: Unknown total size progress (DASH/HLS stream)
        let line_unk = "[download]   50.2MiB at    8.43MiB/s ETA --:--";
        let res_unk = parse_yt_dlp_progress(line_unk);
        assert!(res_unk.is_some());
        let (pct, downloaded, total, speed) = res_unk.unwrap();
        assert!((pct - 0.0).abs() < 1e-6);
        assert_eq!(downloaded, (50.2 * 1024.0 * 1024.0) as u64);
        assert_eq!(total, 0);
        assert_eq!(speed, (8.43 * 1024.0 * 1024.0) as u64);

        // Test case 4: Non-download line
        let line_other = "[youtube] abc123xyz: Downloading webpage";
        assert!(parse_yt_dlp_progress(line_other).is_none());

        // Test case 5: Twitter HLS fragment 进度（`~` 是独立 token）
        // 实际 yt-dlp 输出：`[download]  10.0% of ~   8.82KiB at 412.33B/s ETA Unknown (frag 0/10)`
        // `~` 和 size 之间有多个空格，split_whitespace 会把它们分开。
        let line_frag = "[download]  10.0% of ~   8.82KiB at    412.33B/s ETA Unknown (frag 0/10)";
        let res_frag = parse_yt_dlp_progress(line_frag);
        assert!(res_frag.is_some(), "HLS fragment 进度应能解析");
        let (pct, _downloaded, total, speed) = res_frag.unwrap();
        assert!((pct - 10.0).abs() < 1e-6);
        assert_eq!(total, (8.82 * 1024.0) as u64);
        assert_eq!(speed, 412);

        // Test case 6: Twitter HLS fragment 后续进度（~ 后跟 MiB）
        let line_frag2 = "[download] 100.0% of ~   5.14MiB at  275.66KiB/s ETA 00:29 (frag 10/10)";
        let res_frag2 = parse_yt_dlp_progress(line_frag2);
        assert!(res_frag2.is_some());
        let (pct, _downloaded, total, speed) = res_frag2.unwrap();
        assert!((pct - 100.0).abs() < 1e-6);
        assert_eq!(total, (5.14 * 1024.0 * 1024.0) as u64);
        assert_eq!(speed, (275.66 * 1024.0) as u64);
    }

    #[test]
    fn merged_media_download_enables_ffmpeg_output_format() {
        let arguments = media_arguments("bestvideo+bestaudio", "video.mp4", true);
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--merge-output-format", "mp4"]));
    }

    #[test]
    fn split_auth_headers_separates_cookie_referer_user_agent() {
        let mut headers = HashMap::new();
        headers.insert("Cookie".to_string(), "session=abc".to_string());
        headers.insert(
            "Referer".to_string(),
            "https://example.com/page".to_string(),
        );
        headers.insert("User-Agent".to_string(), "TestUA/1.0".to_string());
        headers.insert("X-Custom".to_string(), "keep-me".to_string());

        let (cookie, referer, user_agent, rest) = split_auth_headers(&headers);
        assert_eq!(cookie.as_deref(), Some("session=abc"));
        assert_eq!(referer.as_deref(), Some("https://example.com/page"));
        assert_eq!(user_agent.as_deref(), Some("TestUA/1.0"));
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].0, "X-Custom");
        assert_eq!(rest[0].1, "keep-me");
    }

    #[test]
    fn split_auth_headers_is_case_insensitive() {
        let mut headers = HashMap::new();
        headers.insert("cookie".to_string(), "v1".to_string());
        headers.insert("REFERER".to_string(), "v2".to_string());
        headers.insert("user-AGENT".to_string(), "v3".to_string());

        let (cookie, referer, user_agent, rest) = split_auth_headers(&headers);
        assert_eq!(cookie.as_deref(), Some("v1"));
        assert_eq!(referer.as_deref(), Some("v2"));
        assert_eq!(user_agent.as_deref(), Some("v3"));
        assert!(rest.is_empty());
    }

    #[test]
    fn split_auth_headers_handles_empty_map() {
        let headers = HashMap::new();
        let (cookie, referer, user_agent, rest) = split_auth_headers(&headers);
        assert!(cookie.is_none());
        assert!(referer.is_none());
        assert!(user_agent.is_none());
        assert!(rest.is_empty());
    }

    /// 验证 yt-dlp 子进程命令行不包含 Cookie 原文。
    ///
    /// 这是 SubTask 4.6 的核心安全测试：通过启动一个真实子进程
    /// （Windows 用 cmd.exe /c echo，Unix 用 /bin/echo）打印其命令行参数，
    /// 然后断言：
    /// - Cookie 原文（如 `secret_value_abc123`）不出现在命令行
    /// - `--cookies` 标志出现在命令行（指向临时文件）
    /// - 不存在 `--add-header Cookie:` 模式
    #[tokio::test]
    async fn attach_auth_args_does_not_leak_cookie_to_command_line() {
        let temp_dir = tempfile::tempdir().unwrap();
        let base_dir = temp_dir.path().to_path_buf();

        // 选择平台对应的 echo 可执行文件
        #[cfg(windows)]
        let mut command = Command::new("cmd.exe");
        #[cfg(windows)]
        {
            command.args(["/c", "echo"]);
        }
        #[cfg(not(windows))]
        let mut command = Command::new("/bin/echo");

        let secret_cookie = "session=secret_value_abc123; token=sensitive_xyz";
        let guard = attach_auth_args_in_dir(
            &mut command,
            &base_dir,
            "https://www.example.com/video",
            Some(secret_cookie),
            Some("https://www.example.com/page"),
            Some("TestUA/2.0"),
        )
        .await
        .expect("attach_auth_args_in_dir 应成功");

        // 验证临时 Cookie 文件已创建
        let cookie_path = guard.path().expect("应返回 cookie 文件路径").to_path_buf();
        assert!(
            cookie_path.exists(),
            "Cookie 临时文件应存在：{}",
            cookie_path.display()
        );

        // 启动子进程并捕获其 stdout（即命令行参数回显）
        let output = command.output().await.expect("echo 子进程应可启动");
        let stdout = String::from_utf8_lossy(&output.stdout);

        // 断言 1：Cookie 原文不得出现在命令行
        assert!(
            !stdout.contains("secret_value_abc123"),
            "Cookie 原文泄露到命令行：{stdout}"
        );
        assert!(
            !stdout.contains("sensitive_xyz"),
            "Cookie token 值泄露到命令行：{stdout}"
        );
        assert!(
            !stdout.contains("session=secret_value_abc123"),
            "完整 Cookie 字符串泄露到命令行：{stdout}"
        );

        // 断言 2：`--cookies` 标志应出现（指向临时文件）
        assert!(
            stdout.contains("--cookies"),
            "命令行应包含 --cookies 标志：{stdout}"
        );

        // 断言 3：不得出现 `--add-header Cookie:` 危险模式
        let stdout_lower = stdout.to_lowercase();
        assert!(
            !(stdout_lower.contains("--add-header") && stdout_lower.contains("cookie")),
            "命令行不得通过 --add-header 传递 Cookie：{stdout}"
        );

        // 断言 4：Referer 和 User-Agent 通过安全参数传递
        assert!(
            stdout.contains("--referer"),
            "命令行应包含 --referer 标志：{stdout}"
        );
        assert!(
            stdout.contains("https://www.example.com/page"),
            "Referer 值应通过 --referer 传递：{stdout}"
        );
        assert!(
            stdout.contains("--user-agent"),
            "命令行应包含 --user-agent 标志：{stdout}"
        );

        // 显式 consume，删除临时文件
        guard.consume().await;
        assert!(
            !cookie_path.exists(),
            "Cookie 临时文件应在 consume 后被删除"
        );
    }

    /// 验证无认证参数时不创建临时文件也不添加 `--cookies` 参数。
    #[tokio::test]
    async fn attach_auth_args_without_cookie_is_noop() {
        let temp_dir = tempfile::tempdir().unwrap();
        let base_dir = temp_dir.path().to_path_buf();

        #[cfg(windows)]
        let mut command = Command::new("cmd.exe");
        #[cfg(windows)]
        {
            command.args(["/c", "echo"]);
        }
        #[cfg(not(windows))]
        let mut command = Command::new("/bin/echo");

        let guard = attach_auth_args_in_dir(
            &mut command,
            &base_dir,
            "https://example.com/video",
            None,
            None,
            None,
        )
        .await
        .expect("无认证参数应成功");

        assert!(guard.path().is_none(), "无 cookie 时不应创建文件");

        let output = command.output().await.expect("echo 子进程应可启动");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains("--cookies"),
            "无 cookie 时不应添加 --cookies 标志：{stdout}"
        );
        assert!(
            !stdout.contains("--referer"),
            "无 referer 时不应添加 --referer 标志：{stdout}"
        );

        // consume 无文件情况应正常
        guard.consume().await;
    }

    /// 验证空 Cookie 字符串被当作无 Cookie 处理（不创建文件）。
    #[tokio::test]
    async fn attach_auth_args_with_empty_cookie_is_noop() {
        let temp_dir = tempfile::tempdir().unwrap();
        let base_dir = temp_dir.path().to_path_buf();

        #[cfg(windows)]
        let mut command = Command::new("cmd.exe");
        #[cfg(windows)]
        {
            command.args(["/c", "echo"]);
        }
        #[cfg(not(windows))]
        let mut command = Command::new("/bin/echo");

        let guard = attach_auth_args_in_dir(
            &mut command,
            &base_dir,
            "https://example.com/video",
            Some("   "),
            None,
            None,
        )
        .await
        .expect("空 cookie 应成功");

        assert!(guard.path().is_none(), "空 cookie 时不应创建文件");
        guard.consume().await;
    }

    /// 验证临时 Cookie 文件使用 Netscape 格式且包含正确的域名。
    #[tokio::test]
    async fn cookie_file_uses_netscape_format_with_correct_domain() {
        let temp_dir = tempfile::tempdir().unwrap();
        let base_dir = temp_dir.path().to_path_buf();

        let mut command = Command::new("echo");
        command.arg("noop");

        let guard = attach_auth_args_in_dir(
            &mut command,
            &base_dir,
            "https://www.example.com/video",
            Some("session=abc; theme=dark"),
            None,
            None,
        )
        .await
        .expect("应成功创建 cookie 文件");

        let cookie_path = guard.path().unwrap().to_path_buf();
        let content = tokio::fs::read_to_string(&cookie_path)
            .await
            .expect("应能读取 cookie 文件");

        // Netscape 格式首行
        assert!(
            content.starts_with("# Netscape HTTP Cookie File"),
            "Cookie 文件应以 Netscape 头行开始：{content}"
        );
        // Netscape 格式：domain\tFALSE\t/\tTRUE\t0\tname\tvalue
        // 域名按 exact host 保留 www.example.com，secure 匹配 https
        assert!(
            content.contains("www.example.com\tFALSE\t/\tTRUE\t0\tsession\tabc"),
            "Cookie 文件应包含 www.example.com 域名和 session 条目：{content}"
        );
        assert!(
            content.contains("www.example.com\tFALSE\t/\tTRUE\t0\ttheme\tdark"),
            "Cookie 文件应包含 theme 条目：{content}"
        );

        guard.consume().await;
        assert!(!cookie_path.exists(), "文件应在 consume 后删除");
    }

    // ===== Task 42：图集/音频类型识别与解析 =====

    /// 普通视频 URL 应识别为 `MediaType::Video`。
    ///
    /// 覆盖 YouTube / B 站 / 抖音视频 / TikTok 视频 / Twitter 推文 / 微博状态，
    /// 这些 URL 都不含图集或音频专有路径段，应回退到默认的 Video 类型。
    #[test]
    fn media_type_video_for_normal_video() {
        // YouTube 普通视频
        assert_eq!(
            determine_media_type("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            MediaType::Video
        );
        // B 站普通视频
        assert_eq!(
            determine_media_type("https://www.bilibili.com/video/BV1xx411c7mD"),
            MediaType::Video
        );
        // 抖音普通视频（/video/，非 /note/）
        assert_eq!(
            determine_media_type("https://www.douyin.com/video/7283698765432109876"),
            MediaType::Video
        );
        // TikTok 普通视频
        assert_eq!(
            determine_media_type("https://www.tiktok.com/@user/video/123"),
            MediaType::Video
        );
        // Twitter 普通推文（非 Spaces）
        assert_eq!(
            determine_media_type("https://twitter.com/user/status/1234567890"),
            MediaType::Video
        );
        // 微博普通状态（非 album）
        assert_eq!(
            determine_media_type("https://weibo.com/1234567890/N0abcdef"),
            MediaType::Video
        );
    }

    /// Twitter Spaces 音频 URL 应识别为 `MediaType::Audio`。
    ///
    /// 覆盖 `twitter.com/i/spaces/` 与 `x.com/i/spaces/` 两种 host。
    #[test]
    fn media_type_audio_for_twitter_space() {
        assert_eq!(
            determine_media_type("https://twitter.com/i/spaces/1DXxyv"),
            MediaType::Audio
        );
        assert_eq!(
            determine_media_type("https://x.com/i/spaces/1DXxyvXYZ?s=20"),
            MediaType::Audio
        );
    }

    /// 抖音 note 图集 URL 应识别为 `MediaType::Gallery`。
    ///
    /// 覆盖 `/note/` 路径与 iesdouyin 旧版 `/share/note/` 路径。
    /// 同时验证 TikTok `/photo/` 与微博 `/album/` 也识别为 Gallery。
    #[test]
    fn media_type_gallery_for_douyin_note() {
        // 抖音 note 图集
        assert_eq!(
            determine_media_type("https://www.douyin.com/note/1234567890"),
            MediaType::Gallery
        );
        assert_eq!(
            determine_media_type("https://www.iesdouyin.com/share/note/1234567890"),
            MediaType::Gallery
        );
        // TikTok photo 图集
        assert_eq!(
            determine_media_type("https://www.tiktok.com/@user/photo/123"),
            MediaType::Gallery
        );
        // 微博 album 图集
        assert_eq!(
            determine_media_type("https://weibo.com/album/123456"),
            MediaType::Gallery
        );
    }

    #[test]
    fn parse_formats_filters_out_youtube_storyboard_formats() {
        let json = serde_json::json!({
            "formats": [
                {
                    "format_id": "sb3",
                    "format_note": "storyboard",
                    "vcodec": "none",
                    "acodec": "none",
                    "ext": "mhtml",
                    "width": 168,
                    "height": 94,
                    "url": "https://i.ytimg.com/sb/pZHf0913SGI/storyboard3_L0/default.jpg"
                },
                {
                    "format_id": "18",
                    "format_note": "360p",
                    "vcodec": "avc1.42001E",
                    "acodec": "mp4a.40.2",
                    "ext": "mp4",
                    "width": 640,
                    "height": 360,
                    "url": "https://rr3---sn-oguelnze.googlevideo.com/videoplayback?id=123"
                }
            ]
        });
        let formats = parse_formats(&json);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].id, "18");
        assert!(formats[0].has_video);
        assert!(formats[0].has_audio);
    }

    /// Task 42：`parse_formats` 应正确识别图片格式项并填充 `image_url`。
    ///
    /// 模拟 yt-dlp 返回的图集 JSON：formats 中含 2 个图片项（vcodec=none, acodec=none,
    /// ext=jpg/jpeg），应被解析为 `MediaFormat`，`has_video=false`、`has_audio=false`、
    /// `image_url` 非空。
    #[test]
    fn parse_formats_extracts_image_items_from_yt_dlp_json() {
        let json = serde_json::json!({
            "formats": [
                {
                    "format_id": "image-0",
                    "vcodec": "none",
                    "acodec": "none",
                    "ext": "jpeg",
                    "width": 1080,
                    "height": 1440,
                    "url": "https://p3-sign.douyinpic.com/image0.jpg"
                },
                {
                    "format_id": "image-1",
                    "vcodec": "none",
                    "acodec": "none",
                    "ext": "jpg",
                    "url": "https://p3-sign.douyinpic.com/image1.jpg"
                }
            ]
        });
        let formats = parse_formats(&json);
        assert_eq!(formats.len(), 2, "应解析出 2 个图片项");
        // 第一个图片项：含宽高，label 应显示尺寸
        let first = &formats[0];
        assert_eq!(first.id, "image-0");
        assert!(!first.has_video);
        assert!(!first.has_audio);
        assert_eq!(first.extension.as_deref(), Some("jpeg"));
        assert_eq!(first.width, Some(1080));
        assert_eq!(first.height, Some(1440));
        assert_eq!(
            first.image_url.as_deref(),
            Some("https://p3-sign.douyinpic.com/image0.jpg")
        );
        assert!(
            first.label.contains("1080"),
            "label 应包含宽度信息：{}",
            first.label
        );
        // 第二个图片项：无宽高，label 应为 "图片"
        let second = &formats[1];
        assert_eq!(second.id, "image-1");
        assert_eq!(second.extension.as_deref(), Some("jpg"));
        assert_eq!(second.width, None);
        assert_eq!(second.height, None);
        assert_eq!(
            second.image_url.as_deref(),
            Some("https://p3-sign.douyinpic.com/image1.jpg")
        );
    }

    /// Task 42：视频/音频格式项的 `image_url` 应始终为 `None`。
    ///
    /// 即使 format 项含 `url` 字段，只要 vcodec/acodec 非 none 或 ext 非图片扩展名，
    /// `image_url` 字段都不应被填充。保证图集前端不会把视频流误识别为图片。
    #[test]
    fn parse_formats_does_not_set_image_url_for_video_audio_items() {
        let json = serde_json::json!({
            "formats": [
                {
                    "format_id": "137",
                    "vcodec": "avc1.640028",
                    "acodec": "none",
                    "ext": "mp4",
                    "height": 1080,
                    "url": "https://example.com/video.mp4"
                },
                {
                    "format_id": "140",
                    "vcodec": "none",
                    "acodec": "mp4a.40.2",
                    "ext": "m4a",
                    "url": "https://example.com/audio.m4a"
                }
            ]
        });
        let formats = parse_formats(&json);
        assert_eq!(formats.len(), 2);
        for format in &formats {
            assert!(
                format.image_url.is_none(),
                "视频/音频格式项不应填充 image_url：{:?}",
                format.id
            );
        }
        // 视频项 has_video=true，音频项 has_audio=true
        assert!(formats[0].has_video);
        assert!(!formats[0].has_audio);
        assert!(!formats[1].has_video);
        assert!(formats[1].has_audio);
    }

    /// Twitter progressive 流：yt-dlp 对 http-* 格式返回 vcodec/acodec 为 null，
    /// 应正确推断为有声视频（has_video=true, has_audio=true），而非误判为"无视频无音频"。
    #[test]
    fn parse_single_format_twitter_progressive_null_codecs() {
        let json = serde_json::json!({
            "formats": [
                {
                    "format_id": "http-2176",
                    "vcodec": null,
                    "acodec": null,
                    "ext": "mp4",
                    "width": 1280,
                    "height": 720,
                    "filesize": 8115664
                },
                {
                    "format_id": "http-832",
                    "vcodec": null,
                    "acodec": null,
                    "ext": "mp4",
                    "width": 640,
                    "height": 360
                }
            ]
        });
        let formats = parse_formats(&json);
        assert_eq!(formats.len(), 2);
        // 两个 progressive 格式都应识别为有声视频
        for f in &formats {
            assert!(f.has_video, "Twitter progressive 流应有视频：{}", f.id);
            assert!(f.has_audio, "Twitter progressive 流应有音频：{}", f.id);
            assert!(!f.requires_ffmpeg);
        }
        // 按 height 排序时 720p 应在前
        assert_eq!(formats[0].height, Some(720));
        assert_eq!(formats[1].height, Some(360));
    }

    /// Twitter HLS 分离流：hls-* 视频轨 acodec="none" 应识别为无声视频；
    /// hls-audio-* 音频轨 vcodec="none" + acodec=null 应识别为纯音频。
    #[test]
    fn parse_single_format_twitter_hls_split_streams() {
        let json = serde_json::json!({
            "formats": [
                {
                    "format_id": "hls-1570",
                    "vcodec": "avc1.64001F",
                    "acodec": "none",
                    "ext": "mp4",
                    "width": 1280,
                    "height": 720
                },
                {
                    "format_id": "hls-audio-128000-Audio",
                    "vcodec": "none",
                    "acodec": null,
                    "ext": "mp4"
                }
            ]
        });
        let formats = parse_formats(&json);
        assert_eq!(formats.len(), 2);
        // hls-1570: 无声视频
        assert!(formats[0].has_video, "hls 视频轨应有视频");
        assert!(!formats[0].has_audio, "hls 视频轨应无音频");
        // hls-audio-128000: 纯音频（acodec=null 应被推断为有音频）
        assert!(!formats[1].has_video, "hls 音频轨应无视频");
        assert!(formats[1].has_audio, "hls 音频轨应有音频（acodec=null 推断为纯音频）");
    }
    /// 合成格式（bestvideo*+bestaudio/best）应携带最高视频流的 height，
    /// 便于前端按高度排序正确选中默认格式。Twitter 场景：hls-1570 (720p) + hls-audio。
    #[test]
    fn merged_format_carries_best_video_height() {
        let json = serde_json::json!({
            "formats": [
                {
                    "format_id": "hls-audio-128000-Audio",
                    "vcodec": "none",
                    "acodec": null,
                    "ext": "mp4"
                },
                {
                    "format_id": "hls-522",
                    "vcodec": "avc1.4D401E",
                    "acodec": "none",
                    "ext": "mp4",
                    "width": 640,
                    "height": 360
                },
                {
                    "format_id": "hls-1570",
                    "vcodec": "avc1.64001F",
                    "acodec": "none",
                    "ext": "mp4",
                    "width": 1280,
                    "height": 720
                }
            ]
        });
        // 模拟 probe 函数中插入合成格式的逻辑
        let mut formats: Vec<MediaFormat> = parse_formats(&json);
        let has_separate_video = formats.iter().any(|f| f.has_video && !f.has_audio);
        let has_separate_audio = formats.iter().any(|f| !f.has_video && f.has_audio);
        assert!(has_separate_video && has_separate_audio);
        // 复用 probe 中的合成格式插入逻辑
        let best_video = formats
            .iter()
            .filter(|f| f.has_video && !f.has_audio)
            .max_by_key(|f| f.height.unwrap_or(0));
        let best_height = best_video.and_then(|f| f.height);
        let best_width = best_video.and_then(|f| f.width);
        formats.insert(
            0,
            MediaFormat {
                id: "bestvideo*+bestaudio/best".into(),
                label: "最高画质（合并音视频）".into(),
                extension: Some("mp4".into()),
                width: best_width,
                height: best_height,
                file_size: None,
                has_video: true,
                has_audio: true,
                requires_ffmpeg: true,
                image_url: None,
                url: None,
            },
        );
        // 合成格式应在第一位，且 height=720（最高视频流）
        assert_eq!(formats[0].id, "bestvideo*+bestaudio/best");
        assert_eq!(formats[0].height, Some(720), "合成格式应携带最高视频流的 height");
        assert_eq!(formats[0].width, Some(1280));
        assert!(formats[0].has_video);
        assert!(formats[0].has_audio);
        assert!(formats[0].requires_ffmpeg);
    }

    /// Task 42：图集类型 URL 在 formats 不含图片项时，应从 thumbnails 兜底提取。
    /// 模拟抖音 note URL 的 yt-dlp 输出：formats 为空数组，thumbnails 含 3 个图片。
    /// `extract_thumbnail_images` 应返回 3 个 `MediaFormat`，每个 `image_url` 非空。
    #[test]
    fn extract_thumbnail_images_fallback_when_formats_has_no_image() {
        let json = serde_json::json!({
            "formats": [],
            "thumbnails": [
                {"id": "0", "url": "https://example.com/thumb0.jpg", "width": 720, "height": 1280},
                {"id": "1", "url": "https://example.com/thumb1.jpg"},
                {"id": "2", "url": ""}
            ]
        });
        let images = extract_thumbnail_images(&json);
        // 第 3 项 url 为空，应被跳过；最终解析出 2 个图片
        assert_eq!(images.len(), 2, "应跳过 url 为空的 thumbnail");
        assert_eq!(images[0].id, "image-0");
        assert_eq!(images[0].width, Some(720));
        assert_eq!(images[0].height, Some(1280));
        assert_eq!(
            images[0].image_url.as_deref(),
            Some("https://example.com/thumb0.jpg")
        );
        assert_eq!(images[1].id, "image-1");
        assert_eq!(
            images[1].image_url.as_deref(),
            Some("https://example.com/thumb1.jpg")
        );
        // 第 2 项 label 应显示 "图片 2"（idx+1）
        assert_eq!(images[1].label, "图片 2");
    }

    /// Task 42：`extract_thumbnail_images` 在 thumbnails 缺失时应返回空列表。
    #[test]
    fn extract_thumbnail_images_returns_empty_when_no_thumbnails() {
        let json = serde_json::json!({"formats": []});
        let images = extract_thumbnail_images(&json);
        assert!(images.is_empty(), "无 thumbnails 时应返回空列表");

        // thumbnails 为非数组类型时也应安全返回空
        let json2 = serde_json::json!({"thumbnails": "not-an-array"});
        let images2 = extract_thumbnail_images(&json2);
        assert!(images2.is_empty(), "thumbnails 非数组时应返回空列表");
    }
}
