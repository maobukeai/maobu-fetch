use crate::models::{AppSettings, ToolPhase, ToolStatus};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex, RwLock},
};
use tokio_util::sync::CancellationToken;

const VERSION: &str = "yt-dlp 2026.06.09 · FFmpeg 8.1.2";
const DIRECTORY: &str = "2026.06.09-ffmpeg-8.1.2";
const YT_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/download/2026.06.09/yt-dlp.exe";
const YT_HASH: &str = "3a48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a27";
const FF_URL: &str =
    "https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-8.1.2-essentials_build.zip";
const FF_HASH: &str = "db580001caa24ac104c8cb856cd113a87b0a443f7bdf47d8c12b1d740584a2ec";
const DOWNLOAD_BYTES: u64 = 122 * 1024 * 1024;
const INSTALL_BYTES: u64 = 216 * 1024 * 1024;

#[derive(Clone)]
pub struct MediaTools {
    status: Arc<RwLock<ToolStatus>>,
    cancellation: Arc<Mutex<Option<CancellationToken>>>,
}

impl MediaTools {
    pub fn new(app: &AppHandle) -> Self {
        let ready =
            tool_path(app, "yt-dlp.exe").is_some() && tool_path(app, "ffmpeg.exe").is_some();
        Self {
            status: Arc::new(RwLock::new(ToolStatus {
                state: if ready {
                    ToolPhase::Ready
                } else {
                    ToolPhase::Missing
                },
                version: VERSION.into(),
                downloaded_bytes: 0,
                total_bytes: DOWNLOAD_BYTES,
                installed_bytes: if ready { INSTALL_BYTES } else { 0 },
                error: None,
                yt_dlp_available: tool_path(app, "yt-dlp.exe").is_some(),
                ffmpeg_available: tool_path(app, "ffmpeg.exe").is_some(),
            })),
            cancellation: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn status(&self) -> ToolStatus {
        self.status.read().await.clone()
    }

    pub async fn start_install(&self, app: AppHandle, settings: AppSettings) -> Result<(), String> {
        if matches!(
            self.status.read().await.state,
            ToolPhase::Downloading | ToolPhase::Verifying | ToolPhase::Extracting
        ) {
            return Err("媒体工具正在安装".into());
        }
        let data = app.path().app_data_dir().map_err(|e| e.to_string())?;
        if fs2::available_space(&data).unwrap_or(u64::MAX) < 350 * 1024 * 1024 {
            return Err("MEDIA_TOOLS_NO_SPACE: 至少需要 350 MB 可用空间".into());
        }
        let token = CancellationToken::new();
        *self.cancellation.lock().await = Some(token.clone());
        self.set(&app, ToolPhase::Downloading, 0, None).await;
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = this.install(&app, &settings, token).await {
                let state = if error == "已取消安装" {
                    ToolPhase::Missing
                } else {
                    ToolPhase::Failed
                };
                this.set(
                    &app,
                    state,
                    0,
                    if error == "已取消安装" {
                        None
                    } else {
                        Some(error)
                    },
                )
                .await;
            }
            *this.cancellation.lock().await = None;
        });
        Ok(())
    }

    pub async fn cancel(&self) {
        if let Some(token) = self.cancellation.lock().await.as_ref() {
            token.cancel();
        }
    }

    pub async fn uninstall(&self, app: &AppHandle) -> Result<(), String> {
        if matches!(
            self.status.read().await.state,
            ToolPhase::Downloading | ToolPhase::Verifying | ToolPhase::Extracting
        ) {
            return Err("请先取消安装".into());
        }
        let path = tools_root(app)?.join(DIRECTORY);
        if path.exists() {
            tokio::fs::remove_dir_all(path)
                .await
                .map_err(|e| e.to_string())?;
        }
        let staging = tools_root(app)?.join(format!(".{DIRECTORY}.installing"));
        if staging.exists() {
            tokio::fs::remove_dir_all(staging)
                .await
                .map_err(|e| e.to_string())?;
        }
        self.set(app, ToolPhase::Missing, 0, None).await;
        Ok(())
    }

    async fn install(
        &self,
        app: &AppHandle,
        settings: &AppSettings,
        token: CancellationToken,
    ) -> Result<(), String> {
        let root = tools_root(app)?;
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(|e| e.to_string())?;
        let staging = root.join(format!(".{DIRECTORY}.installing"));
        tokio::fs::create_dir_all(&staging)
            .await
            .map_err(|e| e.to_string())?;
        let client = client(settings)?;
        let yt = staging.join("yt-dlp.exe.download");
        let ff = staging.join("ffmpeg.zip.download");
        let result: Result<(), String> = async {
            let yt_size = download(&client, YT_URL, &yt, &token, |n| async move {
                self.set(app, ToolPhase::Downloading, n, None).await
            })
            .await?;
            download(&client, FF_URL, &ff, &token, |n| async move {
                self.set(app, ToolPhase::Downloading, yt_size + n, None)
                    .await
            })
            .await?;
            self.set(app, ToolPhase::Verifying, DOWNLOAD_BYTES, None)
                .await;
            verify(&yt, YT_HASH).await?;
            verify(&ff, FF_HASH).await?;
            if token.is_cancelled() {
                return Err("已取消安装".into());
            }
            self.set(app, ToolPhase::Extracting, DOWNLOAD_BYTES, None)
                .await;
            let yt_final = staging.join("yt-dlp.exe");
            tokio::fs::rename(&yt, &yt_final)
                .await
                .map_err(|e| e.to_string())?;
            let ff_copy = ff.clone();
            let stage_copy = staging.clone();
            tokio::task::spawn_blocking(move || extract_ffmpeg(&ff_copy, &stage_copy))
                .await
                .map_err(|e| e.to_string())??;
            tokio::fs::remove_file(&ff).await.ok();
            if token.is_cancelled() {
                return Err("已取消安装".into());
            }
            let final_dir = root.join(DIRECTORY);
            if final_dir.exists() {
                tokio::fs::remove_dir_all(&final_dir)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            tokio::fs::rename(&staging, &final_dir)
                .await
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        .await;
        if result.as_ref().is_err_and(|error| {
            error.starts_with("MEDIA_TOOLS_CHECKSUM")
                || error.starts_with("MEDIA_TOOLS_ARCHIVE")
                || error == "已取消安装"
        }) && staging.exists()
        {
            tokio::fs::remove_dir_all(&staging).await.ok();
        }
        result?;
        {
            let mut status = self.status.write().await;
            status.state = ToolPhase::Ready;
            status.downloaded_bytes = status.total_bytes;
            status.installed_bytes = INSTALL_BYTES;
            status.error = None;
            status.yt_dlp_available = true;
            status.ffmpeg_available = true;
            let _ = app.emit("media-tools-progress", status.clone());
        }
        Ok(())
    }

    async fn set(&self, app: &AppHandle, state: ToolPhase, downloaded: u64, error: Option<String>) {
        let mut status = self.status.write().await;
        status.state = state;
        status.downloaded_bytes = downloaded.min(status.total_bytes);
        status.error = error;
        if status.state != ToolPhase::Ready {
            status.installed_bytes = 0;
            status.yt_dlp_available = false;
            status.ffmpeg_available = false;
        }
        let _ = app.emit("media-tools-progress", status.clone());
    }
}

pub fn tool_path(app: &AppHandle, name: &str) -> Option<PathBuf> {
    let path = tools_root(app).ok()?.join(DIRECTORY).join(name);
    path.exists().then_some(path)
}
fn tools_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|p| p.join("tools"))
        .map_err(|e| e.to_string())
}
fn client(settings: &AppSettings) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder().user_agent(&settings.user_agent);
    if settings.proxy_mode == "manual" && !settings.proxy_url.is_empty() {
        builder =
            builder.proxy(reqwest::Proxy::all(&settings.proxy_url).map_err(|e| e.to_string())?);
    }
    if settings.proxy_mode == "none" {
        builder = builder.no_proxy();
    }
    builder.build().map_err(|e| e.to_string())
}
async fn download<F, Fut>(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    token: &CancellationToken,
    progress: F,
) -> Result<u64, String>
where
    F: Fn(u64) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let existing = tokio::fs::metadata(path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    let mut request = client.get(url);
    if existing > 0 {
        request = request.header("Range", format!("bytes={existing}-"));
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("MEDIA_TOOLS_NETWORK: {e}"))?;
    let append = existing > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if !response.status().is_success() {
        return Err(format!("MEDIA_TOOLS_NETWORK: HTTP {}", response.status()));
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(path)
        .await
        .map_err(|e| e.to_string())?;
    let mut received = if append { existing } else { 0 };
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if token.is_cancelled() {
            return Err("已取消安装".into());
        }
        let chunk = chunk.map_err(|e| format!("MEDIA_TOOLS_NETWORK: {e}"))?;
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        received += chunk.len() as u64;
        progress(received).await;
    }
    file.flush().await.map_err(|e| e.to_string())?;
    Ok(received)
}
async fn verify(path: &Path, expected: &str) -> Result<(), String> {
    let path = path.to_path_buf();
    let expected = expected.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 1024 * 1024];
        loop {
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        let actual = hex::encode(hash.finalize());
        if actual != expected {
            Err("MEDIA_TOOLS_CHECKSUM: 文件校验失败".into())
        } else {
            Ok(())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
fn extract_ffmpeg(archive: &Path, target: &Path) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("MEDIA_TOOLS_ARCHIVE: {e}"))?;
    let mut found = 0;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        let Some(enclosed) = entry.enclosed_name() else {
            return Err("MEDIA_TOOLS_ARCHIVE: 非法压缩路径".into());
        };
        let Some(name) = enclosed.file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        if name != "ffmpeg.exe" && name != "ffprobe.exe" {
            continue;
        }
        let mut output = File::create(target.join(name)).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut output).map_err(|e| e.to_string())?;
        output.flush().map_err(|e| e.to_string())?;
        found += 1;
    }
    if found != 2 {
        return Err("MEDIA_TOOLS_ARCHIVE: 缺少 FFmpeg 文件".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_zip() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.zip");
        std::fs::write(&bad, b"bad").unwrap();
        assert!(extract_ffmpeg(&bad, dir.path())
            .unwrap_err()
            .starts_with("MEDIA_TOOLS_ARCHIVE"));
    }

    #[test]
    fn verifies_sha256_and_rejects_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sample.bin");
        std::fs::write(&file, b"abc").unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            verify(
                &file,
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            )
            .await
            .unwrap();
            assert!(verify(&file, "0000")
                .await
                .unwrap_err()
                .starts_with("MEDIA_TOOLS_CHECKSUM"));
        });
    }
}
