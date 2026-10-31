use super::*;
use std::time::Duration;

/// Validate/remux an already downloaded local container. No network access or tool installation.
pub async fn prepare_native_media_file(
    app: &AppHandle,
    settings: &AppSettings,
    task: &mut DownloadTask,
    source: &Path,
    cancel: &CancellationToken,
) -> Result<PathBuf, String> {
    if cancel.is_cancelled() {
        return Err("任务已暂停".into());
    }
    let metadata = tokio::fs::symlink_metadata(source)
        .await
        .map_err(|_| "媒体暂存文件缺失")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() == 0 {
        return Err("媒体暂存路径或长度无效，未发布文件".into());
    }
    let Some(tools) = resolve_ffmpeg(app, settings) else {
        let extension = live_output::validated_container(source, cancel).await?;
        set_extension(task, extension)?;
        return Ok(source.to_owned());
    };
    download::verify_streams(&tools.ffprobe, source, false, cancel).await?;
    let parent = source.parent().ok_or("媒体暂存文件目录无效")?;
    let candidate = parent.join(format!(
        ".maobu-remux-{}.mp4",
        uuid::Uuid::new_v4().simple()
    ));
    let mut command = create_hidden_tokio_command(&tools.ffmpeg);
    command
        .args([
            "-hide_banner",
            "-nostdin",
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-fflags",
            "+genpts",
            "-i",
        ])
        .arg(source)
        .args([
            "-map",
            "0:v?",
            "-map",
            "0:a?",
            "-c",
            "copy",
            "-avoid_negative_ts",
            "make_zero",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
            "-n",
        ])
        .arg(&candidate);
    let timeout = crate::manager::effective_retry_policy(task, settings)
        .task_timeout_secs
        .unwrap_or(300)
        .clamp(1, 300);
    let result = process::capture(
        &mut command,
        cancel,
        Duration::from_secs(timeout),
        1024 * 1024,
    )
    .await?;
    if !result.status.success() {
        return Err("媒体转封装失败，完整原始暂存文件已保留，请检查 FFmpeg 或稍后重试".into());
    }
    download::verify_streams(&tools.ffprobe, &candidate, false, cancel).await?;
    if cancel.is_cancelled() {
        return Err("任务已暂停".into());
    }
    set_extension(task, "mp4")?;
    Ok(candidate)
}

fn set_extension(task: &mut DownloadTask, extension: &str) -> Result<(), String> {
    let mut name = PathBuf::from(&task.file_name);
    if name.file_name().and_then(|name| name.to_str()) != Some(task.file_name.as_str()) {
        return Err("媒体目标文件名无效".into());
    }
    name.set_extension(extension);
    task.file_name = name
        .into_os_string()
        .into_string()
        .map_err(|_| "媒体目标文件名编码无效")?;
    Ok(())
}
