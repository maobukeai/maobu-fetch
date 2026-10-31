use super::{
    command_config::PrivateConfig,
    download_process, download_progress,
    output::{OutputManifest, Workspace},
    traffic_proxy::{TrafficBudget, TrafficProxy},
    *,
};
use std::{sync::Arc, time::Duration};
use tauri::Manager;

pub async fn download(
    app: &AppHandle,
    settings: &AppSettings,
    task: DownloadTask,
    token: CancellationToken,
    naming_templates: Vec<PlatformNamingTemplate>,
) -> Result<DownloadTask, String> {
    let manager = app
        .try_state::<crate::manager::SharedManager>()
        .ok_or("下载管理器尚未就绪")?
        .inner()
        .clone();
    download_with_manager(app, settings, task, token, naming_templates, &manager).await
}

pub(super) async fn download_with_manager(
    app: &AppHandle,
    settings: &AppSettings,
    mut task: DownloadTask,
    token: CancellationToken,
    naming_templates: Vec<PlatformNamingTemplate>,
    manager: &crate::manager::SharedManager,
) -> Result<DownloadTask, String> {
    let (cookie, referer, user_agent, headers) = split_auth_headers(&task.headers);
    auth_scope::validate_external_headers(
        headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
    )?;
    let media = task.media.clone().ok_or("缺少媒体格式")?;
    let yt = resolve_yt_dlp(app, settings)
        .ok_or("MEDIA_YT_DLP_MISSING: 下载媒体需要先安装 yt-dlp 基础组件")?;
    let tools = resolve_ffmpeg(app, settings);
    let format = media
        .format_id
        .as_deref()
        .unwrap_or("best[vcodec!=none][acodec!=none]");
    let merge = format.contains('+');
    if media.requires_ffmpeg && !merge && tools.is_none() {
        return Err("MEDIA_FFMPEG_MISSING: 当前格式需要 FFmpeg 组件".into());
    }
    let workspace = Workspace::open(&task, format).await?;
    workspace.record_metadata(&serde_json::json!({})).await?;
    let runtime = manager
        .task_runtime
        .read()
        .await
        .get(&task.id)
        .cloned()
        .unwrap_or_else(|| Arc::new(crate::manager::RuntimeTaskOptions::new(&task)));
    let budget = TrafficBudget::new(
        task.id.clone(),
        manager.bandwidth_scheduler.clone(),
        runtime,
    );
    let proxy = TrafficProxy::start(
        network_policy::NetworkPolicy::resolve(settings, Some(&task))?,
        Some(budget),
        &token,
    )
    .await?;
    let mut command = create_hidden_tokio_command(yt);
    traffic_proxy::configure_command(&mut command, proxy.address());
    command
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .current_dir(&workspace.root);
    command.args(arguments(format, "payload.%(ext)s", tools.is_some()));
    if merge {
        command.arg("--keep-video");
    }
    command.args([
        "--progress-template",
        download_progress::PROGRESS_TEMPLATE,
        "--print",
        download_progress::METADATA_TEMPLATE,
        "--print",
        download_progress::OUTPUT_TEMPLATE,
    ]);
    let connection_count = match task.connection_count {
        1 | 2 | 4 | 8 | 16 | 32 => task.connection_count,
        _ => 1,
    };
    command
        .arg("--concurrent-fragments")
        .arg(connection_count.to_string());
    if let Some(tools) = &tools {
        command.arg("--ffmpeg-location").arg(&tools.ffmpeg);
    } else {
        // An explicit unavailable location prevents yt-dlp from silently using a PATH
        // installation outside the component choice. Keep separate tracks for our muxer.
        command.arg("--ffmpeg-location").arg(
            workspace
                .root
                .join(format!("unavailable-ffmpeg-{}", uuid::Uuid::new_v4())),
        );
    }
    if !media.subtitles.is_empty() {
        command.arg("--write-subs").arg("--sub-langs").arg(
            media
                .subtitles
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    let private_base = manager.store.data_dir().join("cookies_tmp");
    let auth = attach_auth_args_in_dir(
        &mut command,
        &private_base,
        &task.url,
        cookie.as_deref(),
        referer.as_deref(),
        user_agent.as_deref(),
    )
    .await?;
    let mut options = vec![
        ("--proxy", proxy.address().to_owned()),
        ("--socket-timeout", "30".into()),
    ];
    for (name, value) in headers {
        if !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || value.chars().any(char::is_control)
        {
            return Err("媒体认证请求头无效".into());
        }
        options.push(("--add-headers", format!("{name}:{value}")));
    }
    if detect_platform(&task.url) == MediaPlatform::YouTube {
        options.push((
            "--extractor-args",
            build_youtube_extractor_arg(&settings.youtube_po_token, auth.path().is_some()),
        ));
    }
    let source = crate::media_platforms::normalize_bilibili_video_url(
        media.url.as_deref().unwrap_or(&task.url),
    );
    let config = PrivateConfig::create(&private_base, &options, Some(&source)).await?;
    config.attach(&mut command);
    task.active_connections = 0;
    let timeout = crate::manager::effective_retry_policy(&task, settings)
        .task_timeout_secs
        .map(Duration::from_secs);
    let result = download_process::run(
        &mut command,
        &mut task,
        &manager,
        &proxy.stats,
        &workspace,
        &token,
        timeout,
    )
    .await;
    proxy.shutdown().await;
    auth.consume().await;
    let manifest = result?;
    if token.is_cancelled() {
        return Err("任务已暂停".into());
    }
    let output = resolve_output(&workspace, &manifest, merge && tools.is_none(), &token).await?;
    if let Some(tools) = tools {
        verify_streams(&tools.ffprobe, &output, merge, &token).await?;
    }
    // Reuse extraction metadata for naming; never make a second network request after download.
    super::cached_naming::apply(
        &mut task,
        manifest.metadata.as_ref(),
        &naming_templates,
        &manager.store,
    )
    .await?;
    if token.is_cancelled() {
        return Err("任务已暂停".into());
    }
    super::subtitles::validate(&workspace, &manifest).await?;
    let length = workspace
        .publish_with_subtitles(&output, &mut task, &token, &manifest)
        .await?;
    task.total_bytes = length;
    task.downloaded_bytes = length;
    task.active_connections = 0;
    task.speed = 0;
    task.status = TaskStatus::Completed;
    Ok(task)
}

pub(super) fn arguments(format: &str, template: &str, has_ffmpeg: bool) -> Vec<String> {
    let effective = if format.starts_with("live-") {
        "best[vcodec!=none][acodec!=none]/best"
    } else {
        format
    };
    let mut args = vec![
        "--newline".into(),
        "--no-colors".into(),
        "--no-playlist".into(),
        "--part".into(),
        "--no-simulate".into(),
        "--progress".into(),
        "--progress-delta".into(),
        "0.25".into(),
        "-f".into(),
        effective.into(),
        "-o".into(),
        template.into(),
    ];
    if has_ffmpeg {
        args.extend(["--merge-output-format".into(), "mp4".into()]);
    }
    args
}

async fn resolve_output(
    workspace: &Workspace,
    manifest: &OutputManifest,
    internal_muxer: bool,
    cancel: &CancellationToken,
) -> Result<PathBuf, String> {
    if !internal_muxer {
        if let Some(file) = &manifest.file {
            return workspace.validate_file(file).await;
        }
    }
    if !internal_muxer || manifest.tracks.len() != 2 {
        return Err("媒体工具未返回完整输出清单，文件已保留在任务临时目录".into());
    }
    let mut video = None;
    let mut audio = None;
    for track in &manifest.tracks {
        let path = workspace.validate_file(track).await?;
        match crate::media_muxer::probe_fragmented_mp4_track_kind_with_cancel(&path, cancel).await?
        {
            crate::media_muxer::TrackKind::Video if video.is_none() => video = Some(path),
            crate::media_muxer::TrackKind::Audio if audio.is_none() => audio = Some(path),
            _ => return Err("媒体轨道类型重复或不完整，请安装 FFmpeg 后重试".into()),
        }
    }
    let output = workspace
        .root
        .join(format!("merged-{}.mp4", uuid::Uuid::new_v4()));
    crate::media_muxer::merge_fragmented_mp4_with_cancel(
        video.as_deref().ok_or("缺少视频轨道")?,
        audio.as_deref().ok_or("缺少音频轨道")?,
        &output,
        cancel,
    )
    .await?;
    workspace.validate_file(&output).await
}

pub(super) async fn verify_streams(
    ffprobe: &Path,
    output: &Path,
    require_both: bool,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let mut command = create_hidden_tokio_command(ffprobe);
    command
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-show_entries",
            "stream=codec_type,codec_tag_string",
            "-of",
            "json",
            "-i",
        ])
        .arg(output);
    let result = process::capture(
        &mut command,
        cancel,
        Duration::from_secs(30),
        2 * 1024 * 1024,
    )
    .await?;
    if !result.status.success() {
        return Err("媒体文件容器校验失败，未发布目标文件".into());
    }
    let metadata: Value =
        serde_json::from_slice(&result.stdout).map_err(|_| "媒体文件轨道信息无效")?;
    let streams = metadata
        .get("streams")
        .and_then(Value::as_array)
        .ok_or("媒体文件没有可识别轨道")?;
    if streams.iter().any(|stream| {
        matches!(
            stream.get("codec_tag_string").and_then(Value::as_str),
            Some("encv" | "enca")
        )
    }) {
        return Err("MEDIA_DRM_PROTECTED: 媒体轨道声明加密，已拒绝处理且保留原文件".into());
    }
    let has = |kind| {
        streams
            .iter()
            .any(|stream| stream.get("codec_type").and_then(Value::as_str) == Some(kind))
    };
    if !(has("video") || has("audio")) || require_both && !(has("video") && has("audio")) {
        return Err("媒体输出缺少所需音视频轨道，原始轨道已保留".into());
    }
    Ok(())
}
