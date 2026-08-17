use crate::models::{AppSettings, DetectedMediaTools, ToolComponent, ToolPhase, ToolStatus, YtDlpUpdateInfo};
use crate::updater::version_compare;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::{
    cmp::Ordering,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex, RwLock},
};
use tokio_util::sync::CancellationToken;

const DIRECTORY: &str = "2026.07.04-ffmpeg-8.1.2";
const YT_VERSION: &str = "2026.07.04";
const FF_VERSION: &str = "8.1.2 essentials";
const YT_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/download/2026.07.04/yt-dlp.exe";
const YT_HASH: &str = "52fe3c26dcf71fbdc85b528589020bb0b8e383155cfa81b64dd447bbe35e24b8";
const FF_URL: &str =
    "https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-8.1.2-essentials_build.zip";
const FF_HASH: &str = "db580001caa24ac104c8cb856cd113a87b0a443f7bdf47d8c12b1d740584a2ec";
const YT_DOWNLOAD_BYTES: u64 = 18_226_085;
const FF_DOWNLOAD_BYTES: u64 = 109_728_040;
const YT_INSTALL_BYTES: u64 = 18_226_085;
const FF_INSTALL_BYTES: u64 = 199 * 1024 * 1024;
/// 2026-08-16 BT 批准：aria2 固定版本安装规格（AGENTS.md §6）。
/// 官方未修改构建（GPLv2），随附 COPYING 许可证文本与源码链接文件。
const ARIA2_VERSION: &str = "1.37.0";
const ARIA2_URL: &str =
    "https://github.com/aria2/aria2/releases/download/release-1.37.0/aria2-1.37.0-win-64bit-build1.zip";
const ARIA2_HASH: &str =
    "67d015301eef0b612191212d564c5bb0a14b5b9c4796b76454276a4d28d9b288";
const ARIA2_DOWNLOAD_BYTES: u64 = 2_475_379;
const ARIA2_INSTALL_BYTES: u64 = 5_649_408 + 32 * 1024; // aria2c.exe + 许可证与源码链接文本
/// GPLv2 源码获取链接（§6：必须随附源码获取方式）。
const ARIA2_SOURCE_URL: &str = "https://github.com/aria2/aria2/tree/release-1.37.0";
/// GitHub Releases API（yt-dlp 官方仓库最新 release）。
const YT_RELEASES_LATEST_API: &str = "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest";
/// 官方 release 资产下载地址必须以此前缀开头，防止解析被劫持的响应。
const YT_ASSET_URL_PREFIX: &str = "https://github.com/yt-dlp/yt-dlp/releases/download/";
/// yt-dlp 版本记录文件名（与 yt-dlp.exe 同目录），内容为已安装版本号。
const YT_VERSION_MARKER: &str = "yt-dlp.exe.version";
/// GitHub API 要求显式 User-Agent，否则返回 403。
const API_USER_AGENT: &str = concat!(
    "MaobuFetch/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/maobukeai/maobu-fetch)"
);

/// yt-dlp 安装规格：固定版本 + 官方下载地址 + 官方 SHA-256（AGENTS.md §6）。
#[derive(Clone, Debug)]
struct YtDlpInstallSpec {
    version: String,
    url: String,
    sha256: String,
    bytes: u64,
}

/// 编译期内置版本规格：来源与校验值随源码固定，作为首次安装与回退路径。
fn pinned_yt_dlp_spec() -> YtDlpInstallSpec {
    YtDlpInstallSpec {
        version: YT_VERSION.into(),
        url: YT_URL.into(),
        sha256: YT_HASH.into(),
        bytes: YT_DOWNLOAD_BYTES,
    }
}

#[derive(Clone)]
pub struct MediaTools {
    status: Arc<RwLock<ToolStatus>>,
    cancellation: Arc<Mutex<Option<CancellationToken>>>,
}

pub fn create_hidden_tokio_command<P: AsRef<std::ffi::OsStr>>(program: P) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd
}

pub fn create_hidden_std_command<P: AsRef<std::ffi::OsStr>>(program: P) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd
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
        // 首次安装走编译期内置规格（固定版本 + 已验证校验值）；
        // 已安装组件的更新/重装由 update_yt_dlp_latest 走在线最新版本。
        let yt_spec = match component {
            ToolComponent::YtDlp => Some(pinned_yt_dlp_spec()),
            ToolComponent::Ffmpeg | ToolComponent::Aria2 => None,
        };
        self.spawn_install(app, settings, component, yt_spec).await
    }

    /// 检查 yt-dlp 官方最新版本（仅检查，不下载任何内容）。
    ///
    /// 本地版本来自版本记录文件（缺失时回退编译期内置版本），
    /// 与 GitHub 最新 release 比较后返回 `YtDlpUpdateInfo`。
    /// 网络失败返回中文错误，不 panic。
    pub async fn check_yt_dlp_update(
        &self,
        app: &AppHandle,
        settings: &AppSettings,
    ) -> Result<YtDlpUpdateInfo, String> {
        let spec = fetch_latest_yt_dlp_spec(settings).await?;
        let installed_version = installed_yt_dlp_version(app, settings);
        let has_update =
            version_compare(&spec.version, &installed_version) == Ordering::Greater;
        // 先构造 release_url 再移动 spec.version（tag 与版本号一致，可能带 v 前缀）。
        let release_url =
            format!("https://github.com/yt-dlp/yt-dlp/releases/tag/{version}", version = spec.version);
        Ok(YtDlpUpdateInfo {
            installed_version,
            latest_version: spec.version,
            has_update,
            size_bytes: spec.bytes,
            release_url,
        })
    }

    /// 用户确认后把 yt-dlp 更新到官方最新版本。
    ///
    /// 安装时重新拉取最新规格（不信任调用方缓存），下载地址与 SHA-256
    /// 均来自 GitHub 官方 API 的资产 digest，校验失败不落盘为可用版本。
    pub async fn update_yt_dlp_latest(
        &self,
        app: AppHandle,
        settings: AppSettings,
    ) -> Result<(), String> {
        // 网络错误在启动安装前直接返回给调用方，避免后台任务静默失败。
        let spec = fetch_latest_yt_dlp_spec(&settings).await?;
        self.spawn_install(app, settings, ToolComponent::YtDlp, Some(spec))
            .await
    }

    /// 统一的安装任务启动入口：独占安装锁、空间检查后异步执行安装流水线。
    async fn spawn_install(
        &self,
        app: AppHandle,
        settings: AppSettings,
        component: ToolComponent,
        yt_spec: Option<YtDlpInstallSpec>,
    ) -> Result<(), String> {
        let mut cancellation = self.cancellation.lock().await;
        if cancellation.is_some() {
            return Err("另一个媒体组件正在安装".into());
        }
        match component {
            ToolComponent::YtDlp => {
                let bytes = yt_spec
                    .as_ref()
                    .map(|spec| spec.bytes)
                    .unwrap_or(YT_DOWNLOAD_BYTES);
                ensure_space_bytes(&app, bytes, YT_INSTALL_BYTES, " yt-dlp ")?;
            }
            ToolComponent::Ffmpeg => {
                ensure_space(&app, component)?;
            }
            ToolComponent::Aria2 => {
                ensure_space_bytes(
                    &app,
                    ARIA2_DOWNLOAD_BYTES,
                    ARIA2_INSTALL_BYTES,
                    " aria2 ",
                )?;
            }
        }
        let token = CancellationToken::new();
        *cancellation = Some(token.clone());
        drop(cancellation);
        self.set_operation(&app, &settings, component, ToolPhase::Downloading, 0, yt_spec
                .as_ref()
                .map(|spec| spec.bytes)
                .unwrap_or_else(|| component_download_bytes(component)), None)
            .await;
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let result = this
                .install_component(&app, &settings, component, yt_spec, token)
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
                        component_download_bytes(component),
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
        yt_spec: Option<YtDlpInstallSpec>,
        token: CancellationToken,
    ) -> Result<(), String> {
        match component {
            ToolComponent::YtDlp => {
                let spec = yt_spec.unwrap_or_else(pinned_yt_dlp_spec);
                self.install_yt_dlp(app, settings, token, &spec).await
            }
            ToolComponent::Ffmpeg => self.install_ffmpeg(app, settings, token).await,
            ToolComponent::Aria2 => self.install_aria2(app, settings, token).await,
        }
    }

    /// aria2 按需安装（BT-01）：下载官方 zip → SHA-256 校验 → 仅提取
    /// `aria2c.exe` 与 `COPYING`（GPLv2 许可证）→ 写入源码链接文件。
    /// 任何一步失败都不落盘为可用版本（§6）。
    async fn install_aria2(
        &self,
        app: &AppHandle,
        settings: &AppSettings,
        token: CancellationToken,
    ) -> Result<(), String> {
        let staging = staging_directory(app, ToolComponent::Aria2)?;
        tokio::fs::create_dir_all(&staging)
            .await
            .map_err(|error| error.to_string())?;
        let archive = staging.join("aria2.zip.download");
        let client = client(settings)?;
        let result = async {
            download_with_fallback(&client, ARIA2_URL, &archive, &token, |received| async move {
                self.set_operation(
                    app,
                    settings,
                    ToolComponent::Aria2,
                    ToolPhase::Downloading,
                    received,
                    ARIA2_DOWNLOAD_BYTES,
                    None,
                )
                .await;
            })
            .await?;
            self.set_operation(
                app,
                settings,
                ToolComponent::Aria2,
                ToolPhase::Verifying,
                ARIA2_DOWNLOAD_BYTES,
                ARIA2_DOWNLOAD_BYTES,
                None,
            )
            .await;
            verify(&archive, ARIA2_HASH).await?;
            check_cancelled(&token)?;
            self.set_operation(
                app,
                settings,
                ToolComponent::Aria2,
                ToolPhase::Extracting,
                ARIA2_DOWNLOAD_BYTES,
                ARIA2_DOWNLOAD_BYTES,
                None,
            )
            .await;
            let archive_copy = archive.clone();
            let staging_copy = staging.clone();
            tokio::task::spawn_blocking(move || extract_aria2(&archive_copy, &staging_copy))
                .await
                .map_err(|error| error.to_string())??;
            check_cancelled(&token)?;
            let directory = tools_directory(app)?;
            tokio::fs::create_dir_all(&directory)
                .await
                .map_err(|error| error.to_string())?;
            // GPLv2 合规三件套：可执行文件 + 许可证文本 + 源码链接（§6）。
            write_aria2_source_link(&directory)?;
            replace_file(staging.join("aria2c.exe"), directory.join("aria2c.exe")).await?;
            replace_file(
                staging.join("aria2-COPYING.txt"),
                directory.join("aria2-COPYING.txt"),
            )
            .await
        }
        .await;
        handle_staging_result(&staging, &result).await;
        result
    }

    async fn install_yt_dlp(
        &self,
        app: &AppHandle,
        settings: &AppSettings,
        token: CancellationToken,
        spec: &YtDlpInstallSpec,
    ) -> Result<(), String> {
        let staging = staging_directory(app, ToolComponent::YtDlp)?;
        tokio::fs::create_dir_all(&staging)
            .await
            .map_err(|error| error.to_string())?;
        let download_path = staging.join(yt_dlp_staging_download_name(&spec.version));
        let client = client(settings)?;
        let result = async {
            download_with_fallback(
                &client,
                &spec.url,
                &download_path,
                &token,
                |received| async move {
                    self.set_operation(
                        app,
                        settings,
                        ToolComponent::YtDlp,
                        ToolPhase::Downloading,
                        received,
                        spec.bytes,
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
                spec.bytes,
                spec.bytes,
                None,
            )
            .await;
            verify(&download_path, &spec.sha256).await?;
            check_cancelled(&token)?;
            let target_file = if !settings.yt_dlp_path.is_empty()
                && existing_file(&settings.yt_dlp_path).is_some()
            {
                PathBuf::from(&settings.yt_dlp_path)
            } else {
                let directory = tools_directory(app)?;
                tokio::fs::create_dir_all(&directory)
                    .await
                    .map_err(|error| error.to_string())?;
                directory.join("yt-dlp.exe")
            };
            replace_file(download_path, target_file.clone()).await?;
            // 记录已安装版本，供状态展示与后续更新对比；失败不影响已完成的
            // 程序替换，但必须明确告知用户（AGENTS.md §7 不吞错）。
            tokio::fs::write(
                target_file.with_file_name(YT_VERSION_MARKER),
                &spec.version,
            )
            .await
            .map_err(|error| {
                format!(
                    "MEDIA_TOOLS_MARKER: yt-dlp 已更新到 {}，但写入版本记录失败：{error}",
                    spec.version
                )
            })
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
            download_with_fallback(&client, FF_URL, &archive, &token, |received| async move {
                self.set_operation(
                    app,
                    settings,
                    ToolComponent::Ffmpeg,
                    ToolPhase::Downloading,
                    received,
                    FF_DOWNLOAD_BYTES,
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
        total: u64,
        error: Option<String>,
    ) {
        let mut status = self.status.write().await;
        refresh_disk_fields(app, settings, &mut status);
        status.active_component = Some(component);
        status.state = phase;
        status.total_bytes = total;
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
        for sub in ["ffmpeg/bin", "ffmpeg", "Programs/ffmpeg/bin", "Programs/ffmpeg"] {
            add_directory(&mut directories, local_app_data.join(sub));
        }
    }
    if let Some(app_data) = std::env::var_os("APPDATA").map(PathBuf::from) {
        add_python_script_directories(&app_data.join("Python"), &mut directories);
        for sub in [
            "bilibili/ffmpeg",
            "bilibili",
            "anythingllm-desktop/storage/engines/ffmpeg/windows-x64",
            "FormatFactory/FFModules/Encoder",
        ] {
            add_directory(&mut directories, app_data.join(sub));
        }
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
        for sub in [
            ".local/bin",
            ".local/ffmpeg/bin",
            ".local/ffmpeg",
            "bin",
            "ffmpeg/bin",
            "ffmpeg",
        ] {
            add_directory(&mut directories, user_profile.join(sub));
        }
    }
    if let Some(program_files) = std::env::var_os("ProgramFiles").map(PathBuf::from) {
        for sub in ["ffmpeg/bin", "ffmpeg", "FormatFactory/FFModules/Encoder"] {
            add_directory(&mut directories, program_files.join(sub));
        }
    }
    if let Some(program_files_x86) = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from) {
        for sub in ["ffmpeg/bin", "ffmpeg", "FormatFactory/FFModules/Encoder"] {
            add_directory(&mut directories, program_files_x86.join(sub));
        }
    }
    let chocolatey = std::env::var_os("ChocolateyInstall")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("ProgramData").map(|root| PathBuf::from(root).join("chocolatey"))
        });
    if let Some(chocolatey) = chocolatey {
        add_directory(&mut directories, chocolatey.join("bin"));
    }

    add_common_drive_directories(&mut directories);

    directories
}

fn add_common_drive_directories(directories: &mut Vec<PathBuf>) {
    for drive_letter in b'A'..=b'Z' {
        let drive = format!("{}:\\", drive_letter as char);
        let drive_path = Path::new(&drive);
        if drive_path.exists() {
            for sub in [
                "ffmpeg\\bin",
                "ffmpeg",
                "tools\\ffmpeg\\bin",
                "tools\\ffmpeg",
                "tools",
                "tools\\bin",
                "software\\ffmpeg\\bin",
                "software\\ffmpeg",
            ] {
                add_directory(directories, drive_path.join(sub));
            }
        }
    }
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
    let mut detected = DetectedMediaTools {
        yt_dlp_path: detected_path_in_directories("yt-dlp.exe", directories),
        ffmpeg_path: detected_path_in_directories("ffmpeg.exe", directories),
        ffprobe_path: detected_path_in_directories("ffprobe.exe", directories),
    };

    let mut related_dirs = Vec::new();
    if let Some(ref yt_path) = detected.yt_dlp_path {
        add_related_directories(Path::new(yt_path), &mut related_dirs);
    }
    if let Some(ref ff_path) = detected.ffmpeg_path {
        add_related_directories(Path::new(ff_path), &mut related_dirs);
    }

    if !related_dirs.is_empty() {
        if detected.yt_dlp_path.is_none() {
            detected.yt_dlp_path = detected_path_in_directories("yt-dlp.exe", &related_dirs);
        }
        if detected.ffmpeg_path.is_none() {
            detected.ffmpeg_path = detected_path_in_directories("ffmpeg.exe", &related_dirs);
        }
        if detected.ffprobe_path.is_none() {
            detected.ffprobe_path = detected_path_in_directories("ffprobe.exe", &related_dirs);
        }
    }

    detected
}

fn add_related_directories(file_path: &Path, directories: &mut Vec<PathBuf>) {
    if let Some(parent) = file_path.parent() {
        add_directory(directories, parent.to_path_buf());
        if let Some(grandparent) = parent.parent() {
            add_directory(directories, grandparent.to_path_buf());
            for sub in [
                "bin",
                "ffmpeg",
                "ffmpeg/bin",
                "tools",
                "tools/ffmpeg",
                "tools/ffmpeg/bin",
                "FFModules/Encoder",
            ] {
                add_directory(directories, grandparent.join(sub));
            }
        }
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
        version: String::new(),
        downloaded_bytes: 0,
        total_bytes: 0,
        installed_bytes: 0,
        error: None,
        yt_dlp_available: false,
        ffmpeg_available: false,
        active_component: None,
        yt_dlp_version: String::new(),
        ffmpeg_version: FF_VERSION.into(),
        yt_dlp_download_bytes: YT_DOWNLOAD_BYTES,
        ffmpeg_download_bytes: FF_DOWNLOAD_BYTES,
        yt_dlp_installed_bytes: 0,
        ffmpeg_installed_bytes: 0,
        yt_dlp_source: "missing".into(),
        ffmpeg_source: "missing".into(),
        yt_dlp_resolved_path: None,
        ffmpeg_resolved_path: None,
        aria2_available: false,
        aria2_version: String::new(),
        aria2_download_bytes: ARIA2_DOWNLOAD_BYTES,
        aria2_installed_bytes: 0,
        aria2_source: "missing".into(),
        aria2_resolved_path: None,
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
    status.yt_dlp_installed_bytes = file_size(yt_dlp.clone().map(|tool| tool.path));
    status.ffmpeg_installed_bytes = ffmpeg
        .map(|tools| file_size(Some(tools.ffmpeg)) + file_size(Some(tools.ffprobe)))
        .unwrap_or(0);
    status.installed_bytes = status
        .yt_dlp_installed_bytes
        .saturating_add(status.ffmpeg_installed_bytes);
    // yt-dlp 版本优先取安装时写入的记录文件；缺失（旧版安装或外部组件）
    // 时回退编译期内置版本，保持与历史行为一致。
    status.yt_dlp_version = yt_dlp
        .as_ref()
        .and_then(|tool| read_version_marker(&tool.path))
        .unwrap_or_else(|| YT_VERSION.into());
    status.version = format!("yt-dlp {} · FFmpeg {}", status.yt_dlp_version, FF_VERSION);
    // aria2 独立解析（仅应用安装的固定版本，见 resolve_aria2 注释）。
    let aria2 = resolve_aria2(app);
    status.aria2_available = aria2.is_some();
    status.aria2_source = if aria2.is_some() { "bundled" } else { "missing" }.into();
    status.aria2_resolved_path = aria2
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    status.aria2_installed_bytes = file_size(aria2);
    status.aria2_version = if status.aria2_available {
        ARIA2_VERSION.into()
    } else {
        String::new()
    };
}

fn file_size(path: Option<PathBuf>) -> u64 {
    path.and_then(|value| std::fs::metadata(value).ok())
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

#[allow(dead_code)]
fn component_available(status: &ToolStatus, component: ToolComponent) -> bool {
    match component {
        ToolComponent::YtDlp => status.yt_dlp_available,
        ToolComponent::Ffmpeg => status.ffmpeg_available,
        ToolComponent::Aria2 => status.aria2_available,
    }
}

fn component_download_bytes(component: ToolComponent) -> u64 {
    match component {
        ToolComponent::YtDlp => YT_DOWNLOAD_BYTES,
        ToolComponent::Ffmpeg => FF_DOWNLOAD_BYTES,
        ToolComponent::Aria2 => ARIA2_DOWNLOAD_BYTES,
    }
}

fn component_files(component: ToolComponent) -> &'static [&'static str] {
    match component {
        ToolComponent::YtDlp => &["yt-dlp.exe"],
        ToolComponent::Ffmpeg => &["ffmpeg.exe", "ffprobe.exe"],
        ToolComponent::Aria2 => &["aria2c.exe", "aria2-COPYING.txt", "aria2-SOURCE.txt"],
    }
}

/// 解析 aria2 可执行文件路径。
///
/// 与 yt-dlp/FFmpeg 不同，aria2 仅使用本应用按需安装的固定版本，
/// 不做系统 PATH / 自定义路径回退：BT 内核行为（分片校验、参数兼容性）
/// 必须与已验证的固定版本绑定（§6 固定版本约束）。
pub fn resolve_aria2(app: &AppHandle) -> Option<PathBuf> {
    bundled_tool_path(app, "aria2c.exe")
}

fn ensure_space(app: &AppHandle, component: ToolComponent) -> Result<(), String> {
    let (download_bytes, install_bytes, label) = match component {
        ToolComponent::YtDlp => (YT_DOWNLOAD_BYTES, YT_INSTALL_BYTES, " yt-dlp "),
        ToolComponent::Ffmpeg => (FF_DOWNLOAD_BYTES, FF_INSTALL_BYTES, " FFmpeg "),
        ToolComponent::Aria2 => (ARIA2_DOWNLOAD_BYTES, ARIA2_INSTALL_BYTES, " aria2 "),
    };
    ensure_space_bytes(app, download_bytes, install_bytes, label)
}

/// 按实际下载/落盘大小做空间检查（在线更新时下载大小来自官方资产）。
fn ensure_space_bytes(
    app: &AppHandle,
    download_bytes: u64,
    install_bytes: u64,
    label: &str,
) -> Result<(), String> {
    let root = tools_root(app)?;
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let required = download_bytes + install_bytes + 32 * 1024 * 1024;
    let available = fs2::available_space(&root).map_err(|error| error.to_string())?;
    if available < required {
        Err(format!(
            "MEDIA_TOOLS_NO_SPACE: 安装{}至少需要 {} MB 可用空间",
            label,
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
        ToolComponent::Aria2 => ".aria2.installing",
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

/// GitHub API 专用客户端：固定应用 UA（GitHub 要求，否则 403）、
/// 连接超时 15s、总超时 20s，并沿用用户的代理设置（GitHub 在部分网络
/// 环境不可直达）。不复用下载客户端，避免长超时阻塞版本检查。
fn api_client(settings: &AppSettings) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .user_agent(API_USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(20));
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

/// 从 GitHub Releases API JSON 解析 `yt-dlp.exe` 资产安装规格（纯函数，便于测试）。
///
/// 要求同时满足，任一不满足返回 `None`（安全默认，不做降级放行）：
/// - `tag_name` 存在且为字符串（剥离前导 `v`/`V`）；
/// - `assets[]` 中存在名为 `yt-dlp.exe` 的资产；
/// - 资产带官方 `digest`（`sha256:<64 位十六进制>`）；
/// - `browser_download_url` 以 yt-dlp 官方仓库下载前缀开头。
fn parse_yt_dlp_release(json: &serde_json::Value) -> Option<YtDlpInstallSpec> {
    let tag = json.get("tag_name")?.as_str()?.trim();
    let version = tag
        .strip_prefix(&['v', 'V'][..])
        .filter(|rest| !rest.is_empty())
        .unwrap_or(tag)
        .to_owned();
    if !is_plausible_version(&version) {
        return None;
    }
    let assets = json.get("assets")?.as_array()?;
    for asset in assets {
        let name = asset.get("name").and_then(|value| value.as_str());
        if name != Some("yt-dlp.exe") {
            continue;
        }
        let url = asset.get("browser_download_url")?.as_str()?;
        if !url.starts_with(YT_ASSET_URL_PREFIX) {
            return None;
        }
        let digest = asset.get("digest")?.as_str()?;
        let sha256 = digest.strip_prefix("sha256:")?;
        if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let bytes = asset.get("size")?.as_u64()?;
        return Some(YtDlpInstallSpec {
            version,
            url: url.to_owned(),
            sha256: sha256.to_ascii_lowercase(),
            bytes,
        });
    }
    None
}

/// 拉取 yt-dlp 官方最新 release 的安装规格。
///
/// 网络/解析错误以中文返回给调用方，不 panic、不 unwrap（AGENTS.md §7）。
async fn fetch_latest_yt_dlp_spec(settings: &AppSettings) -> Result<YtDlpInstallSpec, String> {
    let client = api_client(settings)?;
    let response = client
        .get(YT_RELEASES_LATEST_API)
        .send()
        .await
        .map_err(|error| format!("无法连接 GitHub：{error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            403 => "GitHub 接口请求过于频繁，已触发限流 (403)，请稍后重试或配置代理".into(),
            404 => "未找到 yt-dlp 最新版本信息 (404)".into(),
            code => format!("GitHub 接口返回 HTTP {code}"),
        });
    }
    let body = response
        .text()
        .await
        .map_err(|error| format!("读取版本信息失败：{error}"))?;
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|error| format!("解析版本信息失败：{error}"))?;
    parse_yt_dlp_release(&json)
        .ok_or_else(|| "最新版本缺少官方下载地址或 SHA-256 校验值，为安全起见已取消".into())
}

/// yt-dlp 下载暂存文件名：内置固定版本沿用历史名称以保留断点续传兼容；
/// 其他版本带版本号，避免把不同版本的半成品拼接（AGENTS.md §3 精神）。
fn yt_dlp_staging_download_name(version: &str) -> String {
    if version == YT_VERSION {
        "yt-dlp.exe.download".to_string()
    } else {
        format!("yt-dlp.exe.{version}.download")
    }
}

/// 版本号合理性检查：非空、不超过 32 字符、仅允许数字/字母/点/连字符/加号。
/// 防止把损坏的记录文件或异常 tag 当成版本号展示与比较。
fn is_plausible_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
}

/// 读取 yt-dlp.exe 同目录的版本记录文件，返回可信的版本号。
/// 文件缺失或内容不合理时返回 `None`，由调用方回退编译期内置版本。
fn read_version_marker(executable: &Path) -> Option<String> {
    let content = std::fs::read_to_string(executable.with_file_name(YT_VERSION_MARKER)).ok()?;
    let version = content.trim();
    is_plausible_version(version).then(|| version.to_owned())
}

/// 获取已安装 yt-dlp 的真实版本：优先版本记录文件，缺失时回退内置版本。
fn installed_yt_dlp_version(app: &AppHandle, settings: &AppSettings) -> String {
    resolve_yt_dlp(app, settings)
        .and_then(|path| read_version_marker(&path))
        .unwrap_or_else(|| YT_VERSION.into())
}

fn get_fallback_urls(original_url: &str) -> Vec<String> {
    let mut urls = vec![original_url.to_string()];
    if original_url.contains("github.com") {
        urls.push(format!("https://ghproxy.net/{original_url}"));
        urls.push(format!("https://gh-proxy.com/{original_url}"));
        urls.push(format!("https://ghproxy.cn/{original_url}"));
        urls.push(format!("https://edge.ghproxy.net/{original_url}"));
    }
    urls
}

async fn download_with_fallback<F, Fut>(
    client: &reqwest::Client,
    original_url: &str,
    path: &Path,
    token: &CancellationToken,
    progress: F,
) -> Result<u64, String>
where
    F: Fn(u64) -> Fut + Copy,
    Fut: std::future::Future<Output = ()>,
{
    let candidates = get_fallback_urls(original_url);
    let mut last_error = String::new();

    for (index, candidate_url) in candidates.iter().enumerate() {
        check_cancelled(token)?;
        let download_result = download(client, candidate_url, path, token, progress).await;
        match download_result {
            Ok(received) => return Ok(received),
            Err(err) => {
                if err == "已取消安装" {
                    return Err(err);
                }
                last_error = err;
                if index + 1 < candidates.len() {
                    let _ = tokio::fs::remove_file(path).await;
                }
            }
        }
    }

    Err(last_error)
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

/// aria2 压缩包提取（§6）：只允许 `aria2c.exe` 与 `COPYING`（许可证），
/// COPYING 落盘为 `aria2-COPYING.txt`；阻止绝对路径与 `..` 路径穿越。
fn extract_aria2(archive: &Path, target: &Path) -> Result<(), String> {
    let file = File::open(archive).map_err(|error| error.to_string())?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|error| format!("MEDIA_TOOLS_ARCHIVE: {error}"))?;
    let mut found_exe = false;
    let mut found_license = false;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|error| error.to_string())?;
        let Some(enclosed) = entry.enclosed_name() else {
            return Err("MEDIA_TOOLS_ARCHIVE: 非法压缩路径".into());
        };
        let Some(name) = enclosed.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let output_name = match name {
            "aria2c.exe" => {
                found_exe = true;
                name
            }
            "COPYING" => {
                found_license = true;
                "aria2-COPYING.txt"
            }
            _ => continue,
        };
        let mut output =
            File::create(target.join(output_name)).map_err(|error| error.to_string())?;
        std::io::copy(&mut entry, &mut output).map_err(|error| error.to_string())?;
        output.flush().map_err(|error| error.to_string())?;
    }
    if found_exe && found_license {
        Ok(())
    } else {
        Err("MEDIA_TOOLS_ARCHIVE: aria2 压缩包缺少可执行文件或许可证文本".into())
    }
}

/// 写入 GPLv2 源码获取链接文件（§6 合规要求，随二进制一起分发）。
fn write_aria2_source_link(directory: &Path) -> Result<(), String> {
    let content = format!(
        "aria2 {ARIA2_VERSION}（win-64bit-build1 官方未修改构建）\n\
         许可证：GNU General Public License v2（见 aria2-COPYING.txt）\n\
         源码获取：{ARIA2_SOURCE_URL}\n\
         发布归档：{ARIA2_URL}\n"
    );
    std::fs::write(directory.join("aria2-SOURCE.txt"), content)
        .map_err(|error| format!("MEDIA_TOOLS_MARKER: 写入 aria2 源码链接失败：{error}"))
}

pub async fn remux_flv_to_mp4_if_needed(app: &AppHandle, settings: &AppSettings, file_path: &Path) -> PathBuf {
    if !file_path.exists() {
        return file_path.to_path_buf();
    }
    let ext = file_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    let target_mp4 = file_path.with_extension("mp4");
    let is_flv = ext == "flv" || ext == "ts" || ext == "mkv";
    let is_mp4 = ext == "mp4";

    if !is_flv && !is_mp4 {
        return file_path.to_path_buf();
    }

    let mut needs_remux = is_flv;
    if is_mp4 {
        if let Ok(mut f) = tokio::fs::File::open(file_path).await {
            use tokio::io::AsyncReadExt;
            let mut buf = [0u8; 64];
            if let Ok(n) = f.read(&mut buf).await {
                let slice = &buf[..n];
                // 如果文件前64字节中不包含 ftyp (标准 MP4 必须有 ftyp 标识)，说明是未封装的 FLV/TS 原始流
                needs_remux = !slice.windows(4).any(|w| w == b"ftyp");
            }
        }
    }

    if needs_remux {
        if let Some(ffmpeg) = resolve_ffmpeg(app, settings) {
            let temp_remux = file_path.with_extension("remux_tmp.mp4");
            let mut cmd = create_hidden_tokio_command(&ffmpeg.ffmpeg);
            cmd.args(&[
                "-y",
                "-i",
                file_path.to_str().unwrap_or_default(),
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                "-movflags",
                "+faststart",
                temp_remux.to_str().unwrap_or_default(),
            ]);
            #[cfg(windows)]
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
            if let Ok(status) = cmd.status().await {
                if status.success() && temp_remux.exists() {
                    if file_path != target_mp4 {
                        let _ = tokio::fs::remove_file(file_path).await;
                    }
                    if tokio::fs::rename(&temp_remux, &target_mp4).await.is_ok() {
                        return target_mp4;
                    }
                } else {
                    let _ = tokio::fs::remove_file(&temp_remux).await;
                }
            }
        } else if is_mp4 {
            let flv_path = file_path.with_extension("flv");
            if tokio::fs::rename(file_path, &flv_path).await.is_ok() {
                return flv_path;
            }
        }
    }
    file_path.to_path_buf()
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
    fn generates_github_fallback_urls() {
        let original = "https://github.com/yt-dlp/yt-dlp/releases/download/2026.07.04/yt-dlp.exe";
        let candidates = get_fallback_urls(original);
        assert_eq!(candidates[0], original);
        assert!(candidates.len() > 1);
        assert!(candidates[1].contains("ghproxy.net"));
    }

    #[test]
    fn infers_missing_ffmpeg_from_yt_dlp_related_directory() {
        let root = tempfile::tempdir().unwrap();
        let yt_dir = root.path().join("tools").join("yt-dlp");
        let ff_dir = root.path().join("tools").join("ffmpeg").join("bin");
        std::fs::create_dir_all(&yt_dir).unwrap();
        std::fs::create_dir_all(&ff_dir).unwrap();

        std::fs::write(yt_dir.join("yt-dlp.exe"), b"tool").unwrap();
        std::fs::write(ff_dir.join("ffmpeg.exe"), b"tool").unwrap();
        std::fs::write(ff_dir.join("ffprobe.exe"), b"tool").unwrap();

        // 仅向目录列表提供 yt_dir，未提供 ff_dir
        let detected = detect_tools_in_directories(&[yt_dir]);
        assert!(detected.yt_dlp_path.is_some());
        assert!(detected.ffmpeg_path.is_some());
        assert!(detected.ffprobe_path.is_some());
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
    fn extracts_aria2_exe_and_license_only() {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("aria2.zip");
        let file = File::create(&archive).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("aria2-1.37.0-win-64bit-build1/aria2c.exe", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"aria2c").unwrap();
        writer
            .start_file("aria2-1.37.0-win-64bit-build1/COPYING", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"GPLv2 text").unwrap();
        writer
            .start_file("aria2-1.37.0-win-64bit-build1/README.html", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"ignore").unwrap();
        writer.finish().unwrap();

        extract_aria2(&archive, directory.path()).unwrap();
        assert_eq!(
            std::fs::read(directory.path().join("aria2c.exe")).unwrap(),
            b"aria2c"
        );
        assert_eq!(
            std::fs::read(directory.path().join("aria2-COPYING.txt")).unwrap(),
            b"GPLv2 text"
        );
        assert!(!directory.path().join("README.html").exists());

        write_aria2_source_link(directory.path()).unwrap();
        let source_note = std::fs::read_to_string(directory.path().join("aria2-SOURCE.txt")).unwrap();
        assert!(source_note.contains("GNU General Public License v2"));
        assert!(source_note.contains(ARIA2_SOURCE_URL));
    }

    #[test]
    fn aria2_extraction_requires_both_exe_and_license() {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("aria2.zip");
        let file = File::create(&archive).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("aria2/aria2c.exe", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"exe").unwrap();
        writer.finish().unwrap();
        // 缺少 COPYING → 必须失败（不得落盘为可用版本）。
        assert!(extract_aria2(&archive, directory.path()).is_err());
    }

    #[test]
    fn aria2_component_files_cover_gpl_compliance() {
        let files = component_files(ToolComponent::Aria2);
        assert!(files.contains(&"aria2c.exe"));
        assert!(files.contains(&"aria2-COPYING.txt"));
        assert!(files.contains(&"aria2-SOURCE.txt"));
        // aria2 下载体量应显著小于 FFmpeg（§6 基础包体量约束的旁证）。
        assert!(component_download_bytes(ToolComponent::Aria2) < 5 * 1024 * 1024);
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

    // ---- yt-dlp 在线更新检查 ----

    /// 构造带官方 digest 的 yt-dlp.exe 资产 JSON（模拟 GitHub API 响应）。
    fn yt_dlp_release_json(tag: &str, digest: &str, url: &str) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag,
            "assets": [
                { "name": "yt-dlp", "browser_download_url": "https://github.com/yt-dlp/yt-dlp/releases/download/x/yt-dlp", "size": 100 },
                {
                    "name": "yt-dlp.exe",
                    "browser_download_url": url,
                    "size": 18_500_000,
                    "digest": digest
                }
            ]
        })
    }

    const VALID_DIGEST: &str =
        "sha256:52fe3c26dcf71fbdc85b528589020bb0b8e383155cfa81b64dd447bbe35e24b8";

    #[test]
    fn parse_yt_dlp_release_extracts_official_asset() {
        let json = yt_dlp_release_json(
            "2026.09.06",
            VALID_DIGEST,
            "https://github.com/yt-dlp/yt-dlp/releases/download/2026.09.06/yt-dlp.exe",
        );
        let spec = parse_yt_dlp_release(&json).expect("应解析成功");
        assert_eq!(spec.version, "2026.09.06");
        assert_eq!(spec.bytes, 18_500_000);
        assert_eq!(
            spec.sha256,
            "52fe3c26dcf71fbdc85b528589020bb0b8e383155cfa81b64dd447bbe35e24b8"
        );
        assert!(spec
            .url
            .starts_with("https://github.com/yt-dlp/yt-dlp/releases/download/"));
    }

    #[test]
    fn parse_yt_dlp_release_strips_v_prefix_and_ignores_other_assets() {
        // 非 exe 资产排在前面也必须被跳过；tag 带前导 v 应剥离。
        let json = yt_dlp_release_json(
            "v2026.09.06",
            VALID_DIGEST,
            "https://github.com/yt-dlp/yt-dlp/releases/download/2026.09.06/yt-dlp.exe",
        );
        let spec = parse_yt_dlp_release(&json).expect("应解析成功");
        assert_eq!(spec.version, "2026.09.06");
    }

    #[test]
    fn parse_yt_dlp_release_rejects_missing_or_invalid_digest() {
        let url = "https://github.com/yt-dlp/yt-dlp/releases/download/2026.09.06/yt-dlp.exe";
        // 无 digest 字段：安全默认，拒绝而不是降级放行。
        let mut json = yt_dlp_release_json("2026.09.06", VALID_DIGEST, url);
        json["assets"][1]
            .as_object_mut()
            .unwrap()
            .remove("digest");
        assert!(parse_yt_dlp_release(&json).is_none());
        // 非十六进制 / 长度错误 / 非 sha256 前缀。
        for digest in [
            "sha256:xyz48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a2",
            "sha256:abc123",
            "md5:52fe3c26dcf71fbdc85b528589020bb0b8e383155cfa81b64dd447bbe35e24b8",
        ] {
            let json = yt_dlp_release_json("2026.09.06", digest, url);
            assert!(parse_yt_dlp_release(&json).is_none(), "应拒绝 {digest}");
        }
    }

    #[test]
    fn parse_yt_dlp_release_rejects_foreign_download_url() {
        // 下载地址不在 yt-dlp 官方仓库前缀内（响应被劫持）必须拒绝。
        let json = yt_dlp_release_json(
            "2026.09.06",
            VALID_DIGEST,
            "https://evil.example.com/yt-dlp.exe",
        );
        assert!(parse_yt_dlp_release(&json).is_none());
    }

    #[test]
    fn parse_yt_dlp_release_requires_valid_tag() {
        let url = "https://github.com/yt-dlp/yt-dlp/releases/download/2026.09.06/yt-dlp.exe";
        // 非法 tag（含路径分隔符等非法字符）→ 拒绝。
        let json = yt_dlp_release_json("2026/09/06", VALID_DIGEST, url);
        assert!(parse_yt_dlp_release(&json).is_none());
        // 缺 tag_name → 拒绝。
        let mut json = yt_dlp_release_json("2026.09.06", VALID_DIGEST, url);
        json.as_object_mut().unwrap().remove("tag_name");
        assert!(parse_yt_dlp_release(&json).is_none());
    }

    #[test]
    fn yt_dlp_staging_download_name_keeps_legacy_name_for_pinned() {
        // 内置固定版本沿用历史文件名，保留旧断点续传兼容。
        assert_eq!(
            yt_dlp_staging_download_name(YT_VERSION),
            "yt-dlp.exe.download"
        );
        // 其他版本带版本号，避免不同版本半成品拼接。
        assert_eq!(
            yt_dlp_staging_download_name("2026.09.06"),
            "yt-dlp.exe.2026.09.06.download"
        );
    }

    #[test]
    fn read_version_marker_validates_content() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("yt-dlp.exe");
        std::fs::write(&executable, b"exe").unwrap();
        // 无记录文件 → None（调用方回退内置版本）。
        assert_eq!(read_version_marker(&executable), None);
        // 正常版本（带换行）→ 剥离空白后返回。
        std::fs::write(
            directory.path().join(YT_VERSION_MARKER),
            b"2026.09.06\n",
        )
        .unwrap();
        assert_eq!(
            read_version_marker(&executable),
            Some("2026.09.06".to_string())
        );
        // 损坏内容（含非法字符）→ None。
        std::fs::write(directory.path().join(YT_VERSION_MARKER), b"bad/version\n").unwrap();
        assert_eq!(read_version_marker(&executable), None);
    }

    #[test]
    fn calver_version_compare_orders_yt_dlp_releases() {
        // yt-dlp 使用 CalVer（YYYY.MM.DD），version_compare 的三段数字
        // 比较必须给出正确偏序。
        assert_eq!(
            version_compare("2026.09.06", "2026.07.04"),
            Ordering::Greater
        );
        assert_eq!(
            version_compare("2026.07.04", "2026.07.04"),
            Ordering::Equal
        );
        assert_eq!(version_compare("2026.07.04", "2026.09.06"), Ordering::Less);
        // 跨年。
        assert_eq!(
            version_compare("2027.01.02", "2026.12.31"),
            Ordering::Greater
        );
    }
}
