use crate::{
    media_tools::{resolve_ffmpeg, resolve_yt_dlp},
    models::AppSettings,
    models::{DownloadTask, MediaFormat, MediaProbeResult, TaskStatus},
};
use serde_json::Value;
use std::path::PathBuf;
use tauri::AppHandle;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

pub async fn probe(
    app: &AppHandle,
    settings: &AppSettings,
    url: &str,
) -> Result<MediaProbeResult, String> {
    let parsed = url::Url::parse(url).map_err(|_| "媒体地址无效".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("媒体地址无效".into());
    }
    let yt = resolve_yt_dlp(app, settings)
        .ok_or("MEDIA_YT_DLP_MISSING: 分析媒体需要先安装 yt-dlp 基础组件")?;
    let output = Command::new(yt)
        .args(["--dump-single-json", "--no-playlist", "--no-warnings", url])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    let drm = value
        .get("_has_drm")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut formats: Vec<MediaFormat> = value
        .get("formats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
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
            let label = item
                .get("format_note")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| match (height, ext.as_deref()) {
                    (Some(h), Some(ext)) => format!("{h}p · {ext}"),
                    (None, Some(ext)) => ext.to_string(),
                    _ => id.clone(),
                });
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
            })
        })
        .collect();
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
            },
        );
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
    })
}

pub async fn download(
    app: &AppHandle,
    settings: &AppSettings,
    mut task: DownloadTask,
    token: CancellationToken,
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
    let mut command = Command::new(yt);
    command.args(media_arguments(&format, &template, ffmpeg.is_some()));
    if let Some(path) = ffmpeg {
        command.arg("--ffmpeg-location").arg(path);
    }
    for (name, value) in &task.headers {
        command.arg("--add-header").arg(format!("{name}:{value}"));
    }
    for language in media.subtitles {
        command.args(["--write-subs", "--sub-langs", &language]);
    }
    command.arg(&task.url);
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    tokio::select! {status=child.wait()=>{let status=status.map_err(|e|e.to_string())?;if !status.success(){return Err(format!("yt-dlp 退出码：{}",status.code().unwrap_or(-1)))}} _=token.cancelled()=>{let _=child.kill().await;return Err("任务已暂停".into())}}
    let metadata = tokio::fs::metadata(&output)
        .await
        .map_err(|e| e.to_string())?;
    task.total_bytes = metadata.len();
    task.downloaded_bytes = metadata.len();
    task.status = TaskStatus::Completed;
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
