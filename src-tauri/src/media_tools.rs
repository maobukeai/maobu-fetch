use crate::models::{AppSettings, DetectedMediaTools, ToolComponent, ToolPhase, ToolStatus};
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

const DIRECTORY: &str = "2026.06.09-ffmpeg-8.1.2";
const YT_VERSION: &str = "2026.06.09";
const FF_VERSION: &str = "8.1.2 essentials";
const YT_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/download/2026.06.09/yt-dlp.exe";
const YT_HASH: &str = "3a48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a27";
const FF_URL: &str =
    "https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-8.1.2-essentials_build.zip";
const FF_HASH: &str = "db580001caa24ac104c8cb856cd113a87b0a443f7bdf47d8c12b1d740584a2ec";
const YT_DOWNLOAD_BYTES: u64 = 18_202_192;
const FF_DOWNLOAD_BYTES: u64 = 109_728_040;
const YT_INSTALL_BYTES: u64 = 18_202_192;
const FF_INSTALL_BYTES: u64 = 199 * 1024 * 1024;

#[derive(Clone)]
pub struct MediaTools {
    status: Arc<RwLock<ToolStatus>>,
    cancellation: Arc<Mutex<Option<CancellationToken>>>,
}

impl MediaTools {
    pub fn new(app: &AppHandle, settings: &AppSettings) -> Self {
        Self {
            status: Arc::new(RwLock::new(status_from_disk(app, settings))),
            cancellation: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn status(&self, app: &AppHandle, settings: &AppSettings) -> ToolStatus {
        let mut status = self.status.write().await;
        refresh_disk_fields(app, settings, &mut status);
        if status.active_component.is_none() {
            status.state = if status.yt_dlp_available && status.ffmpeg_available {
                ToolPhase::Ready
            } else {
                ToolPhase::Missing
            };
        }
        status.clone()
    }

    pub async fn start_install(
        &self,
        app: AppHandle,
        settings: AppSettings,
        component: ToolComponent,
    ) -> Result<(), String> {
        let mut cancellation = self.cancellation.lock().await;
        if cancellation.is_some() {
            return Err("另一个媒体组件正在安装".into());
        }
        if component_available(&self.status(&app, &settings).await, component) {
            return Ok(());
        }
        ensure_space(&app, component)?;
        let token = CancellationToken::new();
        *cancellation = Some(token.clone());
        drop(cancellation);
        self.set_operation(&app, &settings, component, ToolPhase::Downloading, 0, None)
            .await;
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let result = this
                .install_component(&app, &settings, component, token)
                .await;
            match result {
                Ok(()) => this.finish_operation(&app, &settings).await,
                Err(error) if error == "已取消安装" => {
                    this.finish_operation(&app, &settings).await
                }
                Err(error) => {
                    this.set_operation(
                        &app,
                        &settings,
                        component,
                        ToolPhase::Failed,
                        0,
                        Some(error),
                    )
                    .await
                }
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

    pub async fn uninstall(
        &self,
        app: &AppHandle,
        settings: &AppSettings,
        component: ToolComponent,
    ) -> Result<(), String> {
        if self.cancellation.lock().await.is_some() {
            return Err("请先取消正在进行的安装".into());
        }
        let directory = tools_directory(app)?;
        for name in component_files(component) {
            let path = directory.join(name);
            if path.exists() {
                tokio::fs::remove_file(path)
                    .await
                    .map_err(|error| error.to_string())?;
            }
        }
        cleanup_staging(app, component).await;
        self.finish_operation(app, settings).await;
        Ok(())
    }

    async fn install_component(
        &self,
        app: &AppHandle,
        settings: &AppSettings,
        component: ToolComponent,
        token: CancellationToken,
    ) -> Result<(), String> {
        match component {
            ToolComponent::YtDlp => self.install_yt_dlp(app, settings, token).await,
            ToolComponent::Ffmpeg => self.install_ffmpeg(app, settings, token).await,
        }
    }

    async fn install_yt_dlp(
        &self,
        app: &AppHandle,
        settings: &AppSettings,
        token: CancellationToken,
    ) -> Result<(), String> {
        let staging = staging_directory(app, ToolComponent::YtDlp)?;
        tokio::fs::create_dir_all(&staging)
            .await
            .map_err(|error| error.to_string())?;
        let download_path = staging.join("yt-dlp.exe.download");
        let client = client(settings)?;
        let result = async {
            download(
                &client,
                YT_URL,
                &download_path,
                &token,
                |received| async move {
                    self.set_operation(
                        app,
                        settings,
                        ToolComponent::YtDlp,
                        ToolPhase::Downloading,
                        received,
                        None,
                    )
                    .await;
                },
            )
            .await?;
            self.set_operation(
                app,
                settings,
                ToolComponent::YtDlp,
                ToolPhase::Verifying,
                YT_DOWNLOAD_BYTES,
                None,
            )
            .await;
            verify(&download_path, YT_HASH).await?;
            check_cancelled(&token)?;
            let directory = tools_directory(app)?;
            tokio::fs::create_dir_all(&directory)
                .await
                .map_err(|error| error.to_string())?;
            replace_file(download_path, directory.join("yt-dlp.exe")).await
        }
        .await;
        handle_staging_result(&staging, &result).await;
        result
    }

    async fn install_ffmpeg(
        &self,
        app: &AppHandle,
        settings: &AppSettings,
        token: CancellationToken,
    ) -> Result<(), String> {
        let staging = staging_directory(app, ToolComponent::Ffmpeg)?;
        tokio::fs::create_dir_all(&staging)
            .await
            .map_err(|error| error.to_string())?;
        let archive = staging.join("ffmpeg.zip.download");
        let client = client(settings)?;
        let result = async {
            download(&client, FF_URL, &archive, &token, |received| async move {
                self.set_operation(
                    app,
                    settings,
                    ToolComponent::Ffmpeg,
                    ToolPhase::Downloading,
                    received,
                    None,
                )
                .await;
            })
            .await?;
            self.set_operation(
                app,
                settings,
                ToolComponent::Ffmpeg,
                ToolPhase::Verifying,
                FF_DOWNLOAD_BYTES,
                None,
            )
            .await;
            verify(&archive, FF_HASH).await?;
            check_cancelled(&token)?;
            self.set_operation(
                app,
                settings,
                ToolComponent::Ffmpeg,
                ToolPhase::Extracting,
                FF_DOWNLOAD_BYTES,
                None,
            )
            .await;
            let archive_copy = archive.clone();
            let staging_copy = staging.clone();
            tokio::task::spawn_blocking(move || extract_ffmpeg(&archive_copy, &staging_copy))
                .await
                .map_err(|error| error.to_string())??;
            check_cancelled(&token)?;
            let directory = tools_directory(app)?;
            tokio::fs::create_dir_all(&directory)
                .await
                .map_err(|error| error.to_string())?;
            replace_file(staging.join("ffmpeg.exe"), directory.join("ffmpeg.exe")).await?;
            replace_file(staging.join("ffprobe.exe"), directory.join("ffprobe.exe")).await
        }
        .await;
        handle_staging_result(&staging, &result).await;
        result
    }

    async fn set_operation(
        &self,
        app: &AppHandle,
        settings: &AppSettings,
        component: ToolComponent,
        phase: ToolPhase,
        downloaded: u64,
        error: Option<String>,
    ) {
        let mut status = self.status.write().await;
        refresh_disk_fields(app, settings, &mut status);
        status.active_component = Some(component);
        status.state = phase;
        status.total_bytes = component_download_bytes(component);
        status.downloaded_bytes = downloaded.min(status.total_bytes);
        status.error = error;
        let _ = app.emit("media-tools-progress", status.clone());
    }

    async fn finish_operation(&self, app: &AppHandle, settings: &AppSettings) {
        let mut status = self.status.write().await;
        refresh_disk_fields(app, settings, &mut status);
        status.active_component = None;
        status.state = if status.yt_dlp_available && status.ffmpeg_available {
            ToolPhase::Ready
        } else {
            ToolPhase::Missing
        };
        status.downloaded_bytes = 0;
        status.total_bytes = 0;
        status.error = None;
        let _ = app.emit("media-tools-progress", status.clone());
    }
}

fn bundled_tool_path(app: &AppHandle, name: &str) -> Option<PathBuf> {
    let path = tools_directory(app).ok()?.join(name);
    path.is_file().then_some(path)
}

#[derive(Clone)]
struct ResolvedTool {
    path: PathBuf,
    source: &'static str,
}

#[derive(Clone)]
pub struct ResolvedFfmpeg {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    source: &'static str,
}

pub fn resolve_yt_dlp(app: &AppHandle, settings: &AppSettings) -> Option<PathBuf> {
    resolve_yt_dlp_with_source(app, settings).map(|tool| tool.path)
}

pub fn resolve_ffmpeg(app: &AppHandle, settings: &AppSettings) -> Option<ResolvedFfmpeg> {
    if !settings.ffmpeg_path.is_empty() || !settings.ffprobe_path.is_empty() {
        return Some(ResolvedFfmpeg {
            ffmpeg: existing_file(&settings.ffmpeg_path)?,
            ffprobe: existing_file(&settings.ffprobe_path)?,
            source: "custom",
        });
    }
    if let (Some(ffmpeg), Some(ffprobe)) = (
        bundled_tool_path(app, "ffmpeg.exe"),
        bundled_tool_path(app, "ffprobe.exe"),
    ) {
        return Some(ResolvedFfmpeg {
            ffmpeg,
            ffprobe,
            source: "bundled",
        });
    }
    Some(ResolvedFfmpeg {
        ffmpeg: find_system_tool("ffmpeg.exe")?,
        ffprobe: find_system_tool("ffprobe.exe")?,
        source: "system",
    })
}

fn resolve_yt_dlp_with_source(app: &AppHandle, settings: &AppSettings) -> Option<ResolvedTool> {
    if !settings.yt_dlp_path.is_empty() {
        return Some(ResolvedTool {
            path: existing_file(&settings.yt_dlp_path)?,
            source: "custom",
        });
    }
    if let Some(path) = bundled_tool_path(app, "yt-dlp.exe") {
        return Some(ResolvedTool {
            path,
            source: "bundled",
        });
    }
    Some(ResolvedTool {
        path: find_system_tool("yt-dlp.exe")?,
        source: "system",
    })
}

fn existing_file(value: &str) -> Option<PathBuf> {
    let path = PathBuf::from(value);
    path.is_file().then(|| path.canonicalize().unwrap_or(path))
}

fn find_system_tool(name: &str) -> Option<PathBuf> {
    find_in_directories(name, system_tool_directories())
}

pub fn detect_system_tools() -> DetectedMediaTools {
    let directories = system_tool_directories();
    detect_tools_in_directories(&directories)
}

fn system_tool_directories() -> Vec<PathBuf> {
    let mut directories = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();

    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        add_python_script_directories(
            &local_app_data.join("Programs").join("Python"),
            &mut directories,
        );
        add_directory(
            &mut directories,
            local_app_data
                .join("Microsoft")
                .join("WinGet")
                .join("Links"),
        );
        add_winget_package_directories(
            &local_app_data
                .join("Microsoft")
                .join("WinGet")
                .join("Packages"),
            &mut directories,
        );
        add_directory(
            &mut directories,
            local_app_data.join("Programs").join("ffmpeg").join("bin"),
        );
    }
    if let Some(app_data) = std::env::var_os("APPDATA").map(PathBuf::from) {
        add_python_script_directories(&app_data.join("Python"), &mut directories);
    }
    if let Some(user_profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        let scoop = std::env::var_os("SCOOP")
            .map(PathBuf::from)
            .unwrap_or_else(|| user_profile.join("scoop"));
        for relative in [
            PathBuf::from("shims"),
            PathBuf::from("apps/yt-dlp/current"),
            PathBuf::from("apps/ffmpeg/current/bin"),
            PathBuf::from("apps/ffmpeg-shared/current/bin"),
        ] {
            add_directory(&mut directories, scoop.join(relative));
        }
        add_directory(&mut directories, user_profile.join(".local").join("bin"));
        add_directory(&mut directories, user_profile.join("bin"));
        add_directory(&mut directories, user_profile.join("ffmpeg").join("bin"));
    }
    if let Some(program_files) = std::env::var_os("ProgramFiles").map(PathBuf::from) {
        add_directory(&mut directories, program_files.join("ffmpeg").join("bin"));
    }
    let chocolatey = std::env::var_os("ChocolateyInstall")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("ProgramData").map(|root| PathBuf::from(root).join("chocolatey"))
        });
    if let Some(chocolatey) = chocolatey {
        add_directory(&mut directories, chocolatey.join("bin"));
    }
    directories
}

fn add_python_script_directories(root: &Path, directories: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .to_ascii_lowercase()
            .starts_with("python")
        {
            add_directory(directories, entry.path().join("Scripts"));
        }
    }
}

fn add_winget_package_directories(root: &Path, directories: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.starts_with("yt-dlp.yt-dlp") || name.starts_with("gyan.ffmpeg") {
            add_descendant_directories(&entry.path(), 3, directories);
        }
    }
}

fn add_descendant_directories(root: &Path, remaining_depth: u8, directories: &mut Vec<PathBuf>) {
    add_directory(directories, root.to_path_buf());
    if remaining_depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten().filter(|entry| entry.path().is_dir()) {
        add_descendant_directories(&entry.path(), remaining_depth - 1, directories);
    }
}

fn add_directory(directories: &mut Vec<PathBuf>, directory: PathBuf) {
    if directory.is_absolute() && !directories.iter().any(|existing| existing == &directory) {
        directories.push(directory);
    }
}

fn detect_tools_in_directories(directories: &[PathBuf]) -> DetectedMediaTools {
    DetectedMediaTools {
        yt_dlp_path: detected_path_in_directories("yt-dlp.exe", directories),
        ffmpeg_path: detected_path_in_directories("ffmpeg.exe", directories),
        ffprobe_path: detected_path_in_directories("ffprobe.exe", directories),
    }
}

fn detected_path_in_directories(name: &str, directories: &[PathBuf]) -> Option<String> {
    find_in_directories(name, directories.iter().cloned()).map(display_path)
}

fn display_path(path: PathBuf) -> String {
    let value = path.to_string_lossy();
    if let Some(network_path) = value.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{network_path}");
    }
    value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned()
}

fn find_in_directories(
    name: &str,
    directories: impl IntoIterator<Item = PathBuf>,
) -> Option<PathBuf> {
    directories
        .into_iter()
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
        .map(|candidate| candidate.canonicalize().unwrap_or(candidate))
}

fn status_from_disk(app: &AppHandle, settings: &AppSettings) -> ToolStatus {
    let mut status = ToolStatus {
        state: ToolPhase::Missing,
        version: format!("yt-dlp {YT_VERSION} · FFmpeg {FF_VERSION}"),
        downloaded_bytes: 0,
        total_bytes: 0,
        installed_bytes: 0,
        error: None,
        yt_dlp_available: false,
        ffmpeg_available: false,
        active_component: None,
        yt_dlp_version: YT_VERSION.into(),
        ffmpeg_version: FF_VERSION.into(),
        yt_dlp_download_bytes: YT_DOWNLOAD_BYTES,
        ffmpeg_download_bytes: FF_DOWNLOAD_BYTES,
        yt_dlp_installed_bytes: 0,
        ffmpeg_installed_bytes: 0,
        yt_dlp_source: "missing".into(),
        ffmpeg_source: "missing".into(),
        yt_dlp_resolved_path: None,
        ffmpeg_resolved_path: None,
    };
    refresh_disk_fields(app, settings, &mut status);
    if status.yt_dlp_available && status.ffmpeg_available {
        status.state = ToolPhase::Ready;
    }
    status
}

fn refresh_disk_fields(app: &AppHandle, settings: &AppSettings, status: &mut ToolStatus) {
    let yt_dlp = resolve_yt_dlp_with_source(app, settings);
    let ffmpeg = resolve_ffmpeg(app, settings);
    status.yt_dlp_available = yt_dlp.is_some();
    status.ffmpeg_available = ffmpeg.is_some();
    status.yt_dlp_source = yt_dlp
        .as_ref()
        .map(|tool| tool.source)
        .unwrap_or("missing")
        .into();
    status.ffmpeg_source = ffmpeg
        .as_ref()
        .map(|tool| tool.source)
        .unwrap_or("missing")
        .into();
    status.yt_dlp_resolved_path = yt_dlp
        .as_ref()
        .map(|tool| tool.path.to_string_lossy().into_owned());
    status.ffmpeg_resolved_path = ffmpeg
        .as_ref()
        .map(|tools| tools.ffmpeg.to_string_lossy().into_owned());
    status.yt_dlp_installed_bytes = file_size(yt_dlp.map(|tool| tool.path));
    status.ffmpeg_installed_bytes = ffmpeg
        .map(|tools| file_size(Some(tools.ffmpeg)) + file_size(Some(tools.ffprobe)))
        .unwrap_or(0);
    status.installed_bytes = status
        .yt_dlp_installed_bytes
        .saturating_add(status.ffmpeg_installed_bytes);
}

fn file_size(path: Option<PathBuf>) -> u64 {
    path.and_then(|value| std::fs::metadata(value).ok())
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn component_available(status: &ToolStatus, component: ToolComponent) -> bool {
    match component {
        ToolComponent::YtDlp => status.yt_dlp_available,
        ToolComponent::Ffmpeg => status.ffmpeg_available,
    }
}

fn component_download_bytes(component: ToolComponent) -> u64 {
    match component {
        ToolComponent::YtDlp => YT_DOWNLOAD_BYTES,
        ToolComponent::Ffmpeg => FF_DOWNLOAD_BYTES,
    }
}

fn component_files(component: ToolComponent) -> &'static [&'static str] {
    match component {
        ToolComponent::YtDlp => &["yt-dlp.exe"],
        ToolComponent::Ffmpeg => &["ffmpeg.exe", "ffprobe.exe"],
    }
}

fn ensure_space(app: &AppHandle, component: ToolComponent) -> Result<(), String> {
    let root = tools_root(app)?;
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let required = match component {
        ToolComponent::YtDlp => YT_DOWNLOAD_BYTES + YT_INSTALL_BYTES + 32 * 1024 * 1024,
        ToolComponent::Ffmpeg => FF_DOWNLOAD_BYTES + FF_INSTALL_BYTES + 32 * 1024 * 1024,
    };
    let available = fs2::available_space(&root).map_err(|error| error.to_string())?;
    if available < required {
        Err(format!(
            "MEDIA_TOOLS_NO_SPACE: 安装{}至少需要 {} MB 可用空间",
            match component {
                ToolComponent::YtDlp => " yt-dlp ",
                ToolComponent::Ffmpeg => " FFmpeg ",
            },
            required.div_ceil(1024 * 1024)
        ))
    } else {
        Ok(())
    }
}

fn tools_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("tools"))
        .map_err(|error| error.to_string())
}

fn tools_directory(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(tools_root(app)?.join(DIRECTORY))
}

fn staging_directory(app: &AppHandle, component: ToolComponent) -> Result<PathBuf, String> {
    let name = match component {
        ToolComponent::YtDlp => ".yt-dlp.installing",
        ToolComponent::Ffmpeg => ".ffmpeg.installing",
    };
    Ok(tools_root(app)?.join(name))
}

async fn cleanup_staging(app: &AppHandle, component: ToolComponent) {
    if let Ok(path) = staging_directory(app, component) {
        if path.exists() {
            let _ = tokio::fs::remove_dir_all(path).await;
        }
    }
}

async fn handle_staging_result(staging: &Path, result: &Result<(), String>) {
    let keep_for_resume = result
        .as_ref()
        .err()
        .is_some_and(|error| error.starts_with("MEDIA_TOOLS_NETWORK"));
    if !keep_for_resume && staging.exists() {
        let _ = tokio::fs::remove_dir_all(staging).await;
    }
}

fn check_cancelled(token: &CancellationToken) -> Result<(), String> {
    if token.is_cancelled() {
        Err("已取消安装".into())
    } else {
        Ok(())
    }
}

async fn replace_file(source: PathBuf, target: PathBuf) -> Result<(), String> {
    let backup = target.with_extension("exe.backup");
    if backup.exists() {
        tokio::fs::remove_file(&backup)
            .await
            .map_err(|error| error.to_string())?;
    }
    if target.exists() {
        tokio::fs::rename(&target, &backup)
            .await
            .map_err(|error| error.to_string())?;
    }
    if let Err(error) = tokio::fs::rename(&source, &target).await {
        if backup.exists() {
            let _ = tokio::fs::rename(&backup, &target).await;
        }
        return Err(error.to_string());
    }
    if backup.exists() {
        let _ = tokio::fs::remove_file(backup).await;
    }
    Ok(())
}

fn client(settings: &AppSettings) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder().user_agent(&settings.user_agent);
    if settings.proxy_mode == "manual" && !settings.proxy_url.is_empty() {
        let mut proxy = reqwest::Proxy::all(&settings.proxy_url).map_err(|e| e.to_string())?;
        if !settings.proxy_username.is_empty() {
            proxy = proxy.basic_auth(&settings.proxy_username, &settings.proxy_password);
        }
        builder = builder.proxy(proxy);
    }
    if settings.proxy_mode == "none" {
        builder = builder.no_proxy();
    }
    builder.build().map_err(|error| error.to_string())
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
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let mut request = client.get(url);
    if existing > 0 {
        request = request.header("Range", format!("bytes={existing}-"));
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("MEDIA_TOOLS_NETWORK: {error}"))?;
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
        .map_err(|error| error.to_string())?;
    let mut received = if append { existing } else { 0 };
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        check_cancelled(token)?;
        let chunk = chunk.map_err(|error| format!("MEDIA_TOOLS_NETWORK: {error}"))?;
        file.write_all(&chunk)
            .await
            .map_err(|error| error.to_string())?;
        received += chunk.len() as u64;
        progress(received).await;
    }
    file.flush().await.map_err(|error| error.to_string())?;
    Ok(received)
}

async fn verify(path: &Path, expected: &str) -> Result<(), String> {
    let path = path.to_path_buf();
    let expected = expected.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut file = File::open(path).map_err(|error| error.to_string())?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 1024 * 1024];
        loop {
            let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        if hex::encode(hash.finalize()) == expected {
            Ok(())
        } else {
            Err("MEDIA_TOOLS_CHECKSUM: 文件校验失败".into())
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

fn extract_ffmpeg(archive: &Path, target: &Path) -> Result<(), String> {
    let file = File::open(archive).map_err(|error| error.to_string())?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|error| format!("MEDIA_TOOLS_ARCHIVE: {error}"))?;
    let mut found = 0;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|error| error.to_string())?;
        let Some(enclosed) = entry.enclosed_name() else {
            return Err("MEDIA_TOOLS_ARCHIVE: 非法压缩路径".into());
        };
        let Some(name) = enclosed.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if name != "ffmpeg.exe" && name != "ffprobe.exe" {
            continue;
        }
        let mut output = File::create(target.join(name)).map_err(|error| error.to_string())?;
        std::io::copy(&mut entry, &mut output).map_err(|error| error.to_string())?;
        output.flush().map_err(|error| error.to_string())?;
        found += 1;
    }
    if found == 2 {
        Ok(())
    } else {
        Err("MEDIA_TOOLS_ARCHIVE: 缺少 FFmpeg 文件".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    #[test]
    fn components_have_independent_files_and_sizes() {
        assert_eq!(component_files(ToolComponent::YtDlp), &["yt-dlp.exe"]);
        assert_eq!(
            component_files(ToolComponent::Ffmpeg),
            &["ffmpeg.exe", "ffprobe.exe"]
        );
        assert!(
            component_download_bytes(ToolComponent::Ffmpeg)
                > component_download_bytes(ToolComponent::YtDlp)
        );
    }

    #[test]
    fn finds_existing_tool_in_system_directories() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("yt-dlp.exe");
        std::fs::write(&executable, b"tool").unwrap();
        assert_eq!(
            find_in_directories("yt-dlp.exe", [directory.path().to_path_buf()]),
            executable.canonicalize().ok()
        );
        assert!(find_in_directories("ffmpeg.exe", [directory.path().to_path_buf()]).is_none());
    }

    #[test]
    fn detects_available_system_media_tool_paths() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["yt-dlp.exe", "ffmpeg.exe", "ffprobe.exe"] {
            std::fs::write(directory.path().join(name), b"tool").unwrap();
        }
        let detected = detect_tools_in_directories(&[directory.path().to_path_buf()]);
        assert!(detected
            .yt_dlp_path
            .as_deref()
            .unwrap()
            .ends_with("yt-dlp.exe"));
        assert!(detected
            .ffmpeg_path
            .as_deref()
            .unwrap()
            .ends_with("ffmpeg.exe"));
        assert!(detected
            .ffprobe_path
            .as_deref()
            .unwrap()
            .ends_with("ffprobe.exe"));
    }

    #[test]
    fn discovers_python_scripts_outside_path() {
        let directory = tempfile::tempdir().unwrap();
        let scripts = directory.path().join("Python312").join("Scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        std::fs::write(scripts.join("yt-dlp.exe"), b"tool").unwrap();
        let mut directories = Vec::new();
        add_python_script_directories(directory.path(), &mut directories);
        let detected = detect_tools_in_directories(&directories);
        assert!(detected
            .yt_dlp_path
            .as_deref()
            .unwrap()
            .ends_with("yt-dlp.exe"));
    }

    #[test]
    fn discovers_ffmpeg_inside_winget_package() {
        let directory = tempfile::tempdir().unwrap();
        let bin = directory
            .path()
            .join("Gyan.FFmpeg.Essentials_test")
            .join("ffmpeg-build")
            .join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("ffmpeg.exe"), b"tool").unwrap();
        std::fs::write(bin.join("ffprobe.exe"), b"tool").unwrap();
        let mut directories = Vec::new();
        add_winget_package_directories(directory.path(), &mut directories);
        let detected = detect_tools_in_directories(&directories);
        assert!(detected.ffmpeg_path.is_some());
        assert!(detected.ffprobe_path.is_some());
    }

    #[test]
    fn rejects_invalid_zip() {
        let directory = tempfile::tempdir().unwrap();
        let bad = directory.path().join("bad.zip");
        std::fs::write(&bad, b"bad").unwrap();
        assert!(extract_ffmpeg(&bad, directory.path())
            .unwrap_err()
            .starts_with("MEDIA_TOOLS_ARCHIVE"));
    }

    #[test]
    fn extracts_only_required_ffmpeg_executables() {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("ffmpeg.zip");
        let file = File::create(&archive).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("ffmpeg/bin/ffmpeg.exe", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"ffmpeg").unwrap();
        writer
            .start_file("ffmpeg/bin/ffprobe.exe", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"ffprobe").unwrap();
        writer
            .start_file("ffmpeg/doc/readme.txt", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"ignore").unwrap();
        writer.finish().unwrap();

        extract_ffmpeg(&archive, directory.path()).unwrap();
        assert_eq!(
            std::fs::read(directory.path().join("ffmpeg.exe")).unwrap(),
            b"ffmpeg"
        );
        assert_eq!(
            std::fs::read(directory.path().join("ffprobe.exe")).unwrap(),
            b"ffprobe"
        );
        assert!(!directory.path().join("readme.txt").exists());
    }

    #[test]
    fn verifies_sha256_and_rejects_mismatch() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("sample.bin");
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
