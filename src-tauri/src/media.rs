use crate::{
    manager::{
        diagnose::{classify_platform_error, platform_error_to_chinese},
        naming_template::{apply_naming_template, find_template_for_platform, NamingVars},
    },
    media_cookies::{write_cookie_file_in_dir, CookieFileGuard},
    media_platforms::{
        detect_platform, expand_short_url, extract_url_from_share_text, is_douyin_gallery,
        is_tiktok_gallery, is_twitter_space, is_weibo_gallery, strip_tracking_params,
        MediaPlatform,
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
    if let Some(referer) = referer.filter(|value| !value.trim().is_empty()) {
        command.arg("--referer").arg(referer);
    }
    if let Some(ua) = user_agent.filter(|value| !value.trim().is_empty()) {
        command.arg("--user-agent").arg(ua);
    }
    if let Some(cookie) = cookie.filter(|value| !value.trim().is_empty()) {
        let guard = write_cookie_file_in_dir(base_dir, cookie, url, referer).await?;
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
    let yt = resolve_yt_dlp(app, settings)
        .ok_or("MEDIA_YT_DLP_MISSING: 分析媒体需要先安装 yt-dlp 基础组件")?;
    let mut command = Command::new(yt);
    command.args(["--dump-single-json", "--no-playlist", "--no-warnings"]);
    let cookie_guard = attach_auth_args(
        &mut command,
        app,
        &effective_url,
        cookie,
        referer,
        user_agent,
    )
    .await?;
    command.arg(&effective_url);
    let output = command.output().await.map_err(|e| e.to_string())?;
    // 显式删除临时 cookie 文件（即使分析失败也会通过 drop 删除，这里主动清理）
    cookie_guard.consume().await;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        // Task 37.6：按平台分类错误并返回中文文案。
        // AGENTS.md §3：认证信息不得泄露；stderr 在分类前由 redact_sensitive 脱敏。
        let redacted = crate::manager::redact_sensitive(&stderr);
        let platform_error = classify_platform_error(platform, &redacted);
        let chinese = platform_error_to_chinese(platform_error, platform);
        // DrmProtected 错误前缀加 MEDIA_DRM_ 以便前端识别并拒绝（AGENTS.md §6）
        if platform_error == crate::manager::diagnose::MediaPlatformError::DrmProtected {
            return Err(format!("MEDIA_DRM_PROTECTED: {chinese}"));
        }
        return Err(chinese);
    }
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
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
    if has_separate_video && has_separate_audio {
        formats.insert(
            0,
            MediaFormat {
                id: "bestvideo*+bestaudio/best".into(),
                label: "最高画质（需要 FFmpeg 合并音视频）".into(),
                extension: Some("mp4".into()),
                width: None,
                height: None,
                file_size: None,
                has_video: true,
                has_audio: true,
                requires_ffmpeg: true,
                image_url: None,
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
    })
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
    let format = media
        .format_id
        .unwrap_or_else(|| "bestvideo*+bestaudio/best".into());
    let requires_ffmpeg = media.requires_ffmpeg || format.contains('+');
    if requires_ffmpeg && ffmpeg.is_none() {
        return Err("MEDIA_FFMPEG_MISSING: 当前格式需要 FFmpeg 合并组件".into());
    }
    // 从 task.headers 中分离认证头，避免通过 --add-header 传递 Cookie
    let (cookie, referer, user_agent, safe_headers) = split_auth_headers(&task.headers);
    let mut command = Command::new(yt);
    command.args(media_arguments(&format, &template, ffmpeg.is_some()));
    if let Some(path) = ffmpeg {
        command.arg("--ffmpeg-location").arg(path);
    }
    for (name, value) in &safe_headers {
        command.arg("--add-header").arg(format!("{name}:{value}"));
    }
    let cookie_guard = attach_auth_args(
        &mut command,
        app,
        &task.url,
        cookie.as_deref(),
        referer.as_deref(),
        user_agent.as_deref(),
    )
    .await?;
    for language in media.subtitles {
        command.args(["--write-subs", "--sub-langs", &language]);
    }
    command.arg(&task.url);
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    tokio::select! {status=child.wait()=>{let status=status.map_err(|e|e.to_string())?;if !status.success(){// 下载失败也要清理临时 cookie 文件
    cookie_guard.consume().await;return Err(format!("yt-dlp 退出码：{}",status.code().unwrap_or(-1)))}} _=token.cancelled()=>{let _=child.kill().await;cookie_guard.consume().await;return Err("任务已暂停".into())}}
    // 下载完成，清理临时 cookie 文件
    cookie_guard.consume().await;
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
    let mut arguments = vec![
        "--newline".into(),
        "--no-playlist".into(),
        "--no-part".into(),
        "-f".into(),
        format.into(),
    ];
    if has_ffmpeg {
        arguments.extend(["--merge-output-format".into(), "mp4".into()]);
    }
    arguments.extend(["-o".into(), template.into()]);
    arguments
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
async fn apply_platform_naming_template(
    app: &AppHandle,
    settings: &AppSettings,
    task: &mut DownloadTask,
    current_path: &Path,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
    naming_templates: &[PlatformNamingTemplate],
) -> Result<(), String> {
    if naming_templates.is_empty() {
        return Ok(());
    }
    let platform = detect_platform(&task.url);
    if matches!(platform, MediaPlatform::Unknown) {
        return Ok(());
    }
    let Some(template) = find_template_for_platform(naming_templates, platform.as_str()) else {
        return Ok(());
    };
    let yt = resolve_yt_dlp(app, settings)
        .ok_or_else(|| "MEDIA_YT_DLP_MISSING: 应用命名模板需要 yt-dlp".to_string())?;
    // 短链先跟随重定向（与 probe 一致），避免 yt-dlp 解析短链失败。
    let effective_url = match expand_short_url(&task.url).await {
        Ok(final_url) => final_url,
        Err(_) => task.url.clone(),
    };
    let mut command = Command::new(yt);
    command.args([
        "--dump-single-json",
        "--no-playlist",
        "--no-warnings",
        "--skip-download",
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
    command.arg(&effective_url);
    let output = command.output().await.map_err(|e| e.to_string())?;
    cookie_guard.consume().await;
    if !output.status.success() {
        return Err("yt-dlp 元数据探测失败".into());
    }
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    let vars = build_naming_vars(&value, platform);
    let new_stem = apply_naming_template(&template.template, &vars);
    // 拼接原文件扩展名：保留 yt-dlp 实际输出的扩展名（合并后可能是 .mp4）。
    // 若原文件名无扩展名（如 `.mp4` 在 `task.file_name` 中存在但磁盘文件无扩展名），
    // 这里以磁盘文件实际扩展名为准。
    let original_extension = current_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_string());
    let new_file_name = match original_extension {
        Some(ext) => format!("{new_stem}.{ext}"),
        None => new_stem,
    };
    // 新旧文件名相同时跳过重命名（避免无意义的 IO）。
    if new_file_name == task.file_name {
        return Ok(());
    }
    let new_path = current_path
        .parent()
        .ok_or_else(|| "目标路径无父目录".to_string())?
        .join(&new_file_name);
    // AGENTS.md §7：重名策略严格执行，不静默覆盖用户文件。
    // 新路径已存在时返回错误，调用方应保留原文件名并记录警告。
    if new_path.exists() {
        return Err(format!("目标文件已存在：{}", new_path.display()));
    }
    tokio::fs::rename(current_path, &new_path)
        .await
        .map_err(|e| e.to_string())?;
    task.file_name = new_file_name;
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
fn parse_single_format(item: &Value) -> Option<MediaFormat> {
    let id = item.get("format_id")?.as_str()?.to_string();
    let vcodec = item.get("vcodec").and_then(Value::as_str).unwrap_or("none");
    let acodec = item.get("acodec").and_then(Value::as_str).unwrap_or("none");
    let width = item.get("width").and_then(Value::as_u64);
    let height = item.get("height").and_then(Value::as_u64);
    let ext = item.get("ext").and_then(Value::as_str).map(str::to_owned);
    let size = item
        .get("filesize")
        .or_else(|| item.get("filesize_approx"))
        .and_then(Value::as_u64);
    let url = item.get("url").and_then(Value::as_str).map(str::to_owned);
    let ext_lower = ext.as_deref().map(|e| e.to_ascii_lowercase());
    let is_image = vcodec == "none"
        && acodec == "none"
        && matches!(
            ext_lower.as_deref(),
            Some("jpg") | Some("jpeg") | Some("png") | Some("webp") | Some("gif") | Some("bmp")
        );
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
        has_video: vcodec != "none",
        has_audio: acodec != "none",
        requires_ffmpeg: false,
        image_url: if is_image { url } else { None },
    })
}

/// Task 42：从 yt-dlp `thumbnails` 数组中提取图片格式项（纯函数）。
///
/// 仅在 `formats` 中未含图片项时（图集类型 URL 但 yt-dlp 把图片放在 thumbnails）使用。
/// 每个含非空 `url` 字段的 thumbnail 转换为一个 `MediaFormat`，`id` 形如 `image-<idx>`，
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
            image_url: Some(url),
        });
    }
    result
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

    /// Task 42：图集类型 URL 在 formats 不含图片项时，应从 thumbnails 兜底提取。
    ///
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
