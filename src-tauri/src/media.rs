use crate::models::{DownloadTask, MediaFormat, MediaProbeResult, TaskStatus, ToolStatus};
use serde_json::Value;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

pub async fn tool_status(app: &AppHandle) -> Vec<ToolStatus> {
    let yt = locate_tool(app, "yt-dlp.exe").await;
    let ffmpeg = locate_tool(app, "ffmpeg.exe").await;
    vec![
        status_for("yt-dlp", yt).await,
        status_for("FFmpeg", ffmpeg).await,
    ]
}

pub async fn probe(app: &AppHandle, url: &str) -> Result<MediaProbeResult, String> {
    let parsed = url::Url::parse(url).map_err(|_| "媒体地址无效".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("媒体地址无效".into());
    }
    let yt = locate_tool(app, "yt-dlp.exe")
        .await
        .ok_or("yt-dlp 尚未安装")?;
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
    let formats = value
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
            })
        })
        .collect();
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
    mut task: DownloadTask,
    token: CancellationToken,
) -> Result<DownloadTask, String> {
    let media = task.media.clone().ok_or("缺少媒体格式")?;
    let yt = locate_tool(app, "yt-dlp.exe")
        .await
        .ok_or("yt-dlp 尚未安装")?;
    let ffmpeg = locate_tool(app, "ffmpeg.exe").await;
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
    let mut command = Command::new(yt);
    command.args([
        "--newline",
        "--no-playlist",
        "--no-part",
        "-f",
        &format,
        "--merge-output-format",
        "mp4",
        "-o",
        &template,
    ]);
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

async fn locate_tool(app: &AppHandle, name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(resource) = app.path().resource_dir() {
        candidates.push(resource.join("tools").join(name));
    }
    if let Ok(data) = app.path().app_data_dir() {
        candidates.push(data.join("tools").join(name));
    }
    for candidate in candidates {
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if Command::new(name).arg("--version").output().await.is_ok() {
        Some(PathBuf::from(name))
    } else {
        None
    }
}
async fn status_for(name: &str, path: Option<PathBuf>) -> ToolStatus {
    let version = if let Some(path) = &path {
        Command::new(path)
            .arg("--version")
            .output()
            .await
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|v| v.lines().next().unwrap_or_default().trim().to_string())
    } else {
        None
    };
    ToolStatus {
        name: name.into(),
        available: path.is_some(),
        version,
        path: path.map(|p| p.to_string_lossy().to_string()),
    }
}
