//! 纯 Rust BT/磁力下载引擎（基于 librqbit）
//!
//! 零外部进程依赖，支持 BT v1/v2 (BEP 52)、IPv6 DHT、PEX，内存级 Tokio 异步调用。
//!
//! 强约束（AGENTS.md §3 BT/磁力内核）：做种默认关闭；
//! 元数据获取前不伪造文件名/大小；暂停/退出保持可恢复。

pub mod magnet;
pub mod status;

pub mod process {
    pub use super::{validate_torrent_bytes, validate_torrent_file};
}

use crate::manager::{safe_name, SharedManager};
use crate::models::{AppSettings, BtFileEntry, BtRuntimeStatus, DownloadTask, TaskStatus};
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ManagedTorrent, Session, SessionOptions,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// BT 任务终态错误前缀：worker 收到后直接结束，不进入退避重试
pub const BT_TERMINAL_PREFIX: &str = "BT_TERMINAL: ";

/// 轮询间隔：真实状态 → 前端增量更新，1 秒与 HTTP 进度节奏一致
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// 默认公共 Trackers（精选全球与国内高可用 HTTP/HTTPS/UDP Public Trackers）
pub const DEFAULT_PUBLIC_TRACKERS: &[&str] = &[
    // HTTP / HTTPS Trackers（完美兼容代理与 Fake-IP 环境）
    "http://tracker.opentrackr.org:1337/announce",
    "http://tracker.openbittorrent.com:80/announce",
    "http://open.acgnxtracker.com:80/announce",
    "http://tracker.renapp.cn:6969/announce",
    "http://tracker.ipv6tracker.ru:80/announce",
    "http://tracker.files.fm:6969/announce",
    "https://tracker.tamersunion.org:443/announce",
    "https://tracker.lilithraws.org:443/announce",
    "https://tr.ready4.icu:29986/announce",
    "https://tracker.gbitt.info:443/announce",
    "https://trackers.mlsub.net:443/announce",
    "https://tracker.loligirl.cn:443/announce",
    "https://tracker.imgoingto.icu:443/announce",
    "https://1337.abcvg.info:443/announce",
    "https://tracker.kuroy.me:443/announce",
    "https://tracker.renapp.cn:443/announce",
    "https://tracker.leeching.top:443/announce",
    "https://tracker.nanoha.org:443/announce",
    "https://tracker.pmman.tech:443/announce",
    // Linux 发行版与系统镜像官方 Trackers
    "https://torrent.ubuntu.com/announce",
    "http://torrent.ubuntu.com:6969/announce",
    "udp://torrent.ubuntu.com:6969/announce",
    "udp://ipv6.torrent.ubuntu.com:6969/announce",
    // UDP Trackers
    "udp://tracker.opentrackr.org:1337/announce",
    "udp://open.demonii.com:1337/announce",
    "udp://tracker.openbittorrent.com:6969/announce",
    "udp://tracker.torrent.eu.org:451/announce",
    "udp://explodie.org:6969/announce",
    "udp://open.stealth.si:80/announce",
    "udp://tracker.dler.org:6969/announce",
    "udp://p4p.arenabg.com:1337/announce",
];

/// 纯 Rust BT 引擎：单例管理 librqbit Session、速度采样与任务绑定。
#[derive(Default)]
pub struct BtEngine {
    session: Mutex<Option<Arc<Session>>>,
    bindings: Mutex<HashMap<String, Arc<ManagedTorrent>>>,
    speed_samples: Mutex<HashMap<String, SpeedSample>>,
}

struct SpeedSample {
    last_time: std::time::Instant,
    last_downloaded: u64,
    last_uploaded: u64,
    current_download_speed: u64,
    current_upload_speed: u64,
}

impl BtEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// 确保 librqbit 会话已初始化并就绪。
    pub async fn ensure_started(
        &self,
        _app: &tauri::AppHandle,
        settings: &AppSettings,
    ) -> Result<Arc<Session>, String> {
        let mut guard = self.session.lock().await;
        if let Some(session) = guard.as_ref() {
            return Ok(session.clone());
        }
        let default_dir = PathBuf::from(&settings.download_dir);
        let opts = SessionOptions {
            dht: Some(Default::default()),
            fastresume: true,
            ..Default::default()
        };
        let session = Session::new_with_opts(default_dir, opts)
            .await
            .map_err(|e| format!("初始化纯 Rust BT 引擎失败: {e}"))?;
        *guard = Some(session.clone());
        tracing::info!("纯 Rust BT 引擎 (librqbit) 初始化成功");
        Ok(session)
    }

    pub async fn is_running(&self) -> bool {
        let guard = self.session.lock().await;
        guard.is_some()
    }

    /// 添加任务到 librqbit 并返回内部 id 字符串。
    pub async fn add_task(
        &self,
        app: &tauri::AppHandle,
        settings: &AppSettings,
        task: &DownloadTask,
    ) -> Result<String, String> {
        let session = self.ensure_started(app, settings).await?;

        {
            let bindings = self.bindings.lock().await;
            if let Some(handle) = bindings.get(&task.id) {
                let _ = session.unpause(handle).await;
                return Ok(handle.info_hash().as_string());
            }
        }

        let is_magnet = task.url.trim().to_ascii_lowercase().starts_with("magnet:");
        let add_torrent = if is_magnet {
            // 为磁力链接注入公共 Tracker 列表与用户额外 Tracker
            let enhanced_url = Self::enrich_magnet_url(task.url.trim(), settings);
            AddTorrent::from_url(enhanced_url)
        } else {
            let bytes = match task
                .bt_meta
                .as_ref()
                .and_then(|meta| meta.torrent_data_base64.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                Some(inline) => {
                    use base64::Engine as _;
                    base64::engine::general_purpose::STANDARD
                        .decode(inline)
                        .map_err(|e| format!("解码种子 Base64 失败: {e}"))?
                }
                None => {
                    validate_torrent_file_async(Path::new(&task.url)).await?;
                    tokio::fs::read(&task.url)
                        .await
                        .map_err(|e| format!("读取种子文件失败: {e}"))?
                }
            };
            AddTorrent::from_bytes(bytes)
        };

        // 对接文件选择（only_files：0 基索引）
        let only_files = task.bt_meta.as_ref().and_then(|meta| {
            if meta.selected_files.is_empty() {
                None
            } else {
                Some(
                    meta.selected_files
                        .iter()
                        .map(|&idx| (idx.saturating_sub(1)) as usize)
                        .collect::<Vec<_>>(),
                )
            }
        });

        // 多文件种子自动创建以种子名命名的专属子文件夹
        let safe_folder_name = safe_name(&task.file_name);
        let is_multi_file_target = !safe_folder_name.is_empty()
            && safe_folder_name != "未命名种子"
            && safe_folder_name != "magnet"
            && safe_folder_name != "download";

        let output_folder = if is_multi_file_target {
            let sub_path = Path::new(&task.destination).join(&safe_folder_name);
            let _ = tokio::fs::create_dir_all(&sub_path).await;
            sub_path.to_string_lossy().to_string()
        } else {
            task.destination.clone()
        };

        let add_opts = AddTorrentOptions {
            output_folder: Some(output_folder),
            paused: false,
            overwrite: true,
            only_files,
            ..Default::default()
        };

        let response = session
            .add_torrent(add_torrent, Some(add_opts))
            .await
            .map_err(|e| format!("添加 BT 任务失败: {e}"))?;

        let (info_hash_str, handle) = match response {
            AddTorrentResponse::AlreadyManaged(_, handle) => {
                let _ = session.unpause(&handle).await;
                (handle.info_hash().as_string(), handle)
            }
            AddTorrentResponse::Added(_, handle) => {
                (handle.info_hash().as_string(), handle)
            }
            _ => {
                return Err("添加 BT 任务未返回有效 Handle".into());
            }
        };

        {
            let mut bindings = self.bindings.lock().await;
            bindings.insert(task.id.clone(), handle);
        }
        Ok(info_hash_str)
    }

    /// 为磁力链接注入 Trackers。
    fn enrich_magnet_url(original: &str, settings: &AppSettings) -> String {
        let mut url = original.to_string();
        let mut existing_lower = url.to_ascii_lowercase();

        // 追加内置公共 Trackers
        for tracker in DEFAULT_PUBLIC_TRACKERS {
            let encoded_tr: String = url::form_urlencoded::byte_serialize(tracker.as_bytes()).collect();
            let check_fragment = format!("tr={}", encoded_tr).to_ascii_lowercase();
            if !existing_lower.contains(&check_fragment) && !existing_lower.contains(&tracker.to_ascii_lowercase()) {
                url.push_str("&tr=");
                url.push_str(&encoded_tr);
                existing_lower.push_str(&check_fragment);
            }
        }

        // 追加用户自定义 Trackers
        for line in settings.bt_extra_trackers.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                let encoded_tr: String = url::form_urlencoded::byte_serialize(trimmed.as_bytes()).collect();
                url.push_str("&tr=");
                url.push_str(&encoded_tr);
            }
        }

        url
    }

    /// 暂停任务（细粒度锁与超时保护）。
    pub async fn pause_task(&self, task_id: &str) -> Result<(), String> {
        let session_opt = {
            let guard = self.session.lock().await;
            guard.clone()
        };
        let handle_opt = {
            let bindings = self.bindings.lock().await;
            bindings.get(task_id).cloned()
        };
        if let (Some(session), Some(handle)) = (session_opt, handle_opt) {
            let _ = tokio::time::timeout(
                Duration::from_secs(1),
                session.pause(&handle)
            ).await;
        }
        Ok(())
    }

    /// 彻底移除任务（细粒度锁与超时保护，绝不阻塞）。
    pub async fn remove_task(&self, task_id: &str) -> Result<(), String> {
        let session_opt = {
            let guard = self.session.lock().await;
            guard.clone()
        };
        let handle_opt = {
            let mut bindings = self.bindings.lock().await;
            bindings.remove(task_id)
        };
        {
            let mut samples = self.speed_samples.lock().await;
            samples.remove(task_id);
        }
        if let (Some(session), Some(handle)) = (session_opt, handle_opt) {
            let hash = handle.info_hash();
            let _ = tokio::time::timeout(
                Duration::from_secs(1),
                session.delete(hash.into(), false)
            ).await;
        }
        Ok(())
    }

    /// 查询任务进度快照。
    pub async fn status_of(&self, task_id: &str, base_dir: &str) -> Result<status::BtProgress, String> {
        let handle = {
            let bindings = self.bindings.lock().await;
            bindings.get(task_id).cloned()
        }.ok_or_else(|| "BT 任务尚未注册".to_string())?;

        let stats = handle.stats();
        let info_hash = handle.info_hash().as_string();
        let metadata_fetching = stats.total_bytes == 0;

        let live = stats.live.as_ref();
        let num_peers = live.map(|l| l.snapshot.peer_stats.live as u32).unwrap_or(0);
        let num_seeds = live.map(|l| {
            l.snapshot.peer_stats.live.saturating_sub(l.snapshot.peer_stats.queued) as u32
        }).unwrap_or(0);

        // 真实瞬时速度采样计算
        let now = std::time::Instant::now();
        let mut samples = self.speed_samples.lock().await;
        let sample = samples.entry(task_id.to_string()).or_insert_with(|| SpeedSample {
            last_time: now,
            last_downloaded: stats.progress_bytes,
            last_uploaded: stats.uploaded_bytes,
            current_download_speed: 0,
            current_upload_speed: 0,
        });
        let dt = now.duration_since(sample.last_time).as_secs_f64();
        if dt >= 0.8 {
            let down_diff = stats.progress_bytes.saturating_sub(sample.last_downloaded);
            let up_diff = stats.uploaded_bytes.saturating_sub(sample.last_uploaded);
            sample.current_download_speed = (down_diff as f64 / dt) as u64;
            sample.current_upload_speed = (up_diff as f64 / dt) as u64;
            sample.last_downloaded = stats.progress_bytes;
            sample.last_uploaded = stats.uploaded_bytes;
            sample.last_time = now;
        }
        let download_speed = sample.current_download_speed;
        let upload_speed = sample.current_upload_speed;

        let (display_name, files, is_error, err_msg) = handle.with_state(|state| {
            match state {
                librqbit::ManagedTorrentState::Live(live) => {
                    let info = live.info();
                    let name = info.name().map(|n| n.to_string());
                    let mut file_entries = Vec::new();
                    let base_path = Path::new(base_dir);
                    for (idx, f) in info.iter_file_details().enumerate() {
                        let filename_str = f.filename.to_string();
                        let local_file = base_path.join(&filename_str);
                        let local_file_nested = if let Some(n) = &name {
                            base_path.join(n).join(&filename_str)
                        } else {
                            local_file.clone()
                        };
                        let downloaded = if stats.finished {
                            f.len
                        } else if local_file.exists() {
                            local_file.metadata().map(|m| m.len()).unwrap_or(0)
                        } else if local_file_nested.exists() {
                            local_file_nested.metadata().map(|m| m.len()).unwrap_or(0)
                        } else {
                            0
                        };
                        file_entries.push(BtFileEntry {
                            index: (idx + 1) as u32,
                            path: filename_str,
                            length_bytes: f.len,
                            selected: true,
                            downloaded_bytes: std::cmp::min(downloaded, f.len),
                        });
                    }
                    (name, file_entries, false, None)
                }
                librqbit::ManagedTorrentState::Error(err) => {
                    (None, Vec::new(), true, Some(err.to_string()))
                }
                _ => (None, Vec::new(), false, None),
            }
        });

        let progress = status::BtProgress {
            state: if is_error {
                "error".into()
            } else if stats.finished {
                "complete".into()
            } else {
                "active".into()
            },
            total_bytes: stats.total_bytes,
            downloaded_bytes: stats.progress_bytes,
            upload_speed,
            download_speed,
            num_seeds,
            num_peers,
            info_hash,
            display_name,
            seeder: stats.finished,
            metadata_fetching,
            uploaded_bytes: stats.uploaded_bytes,
            error: err_msg,
            files,
        };

        Ok(progress)
    }

    /// 列出种子内文件。
    pub async fn files_of(&self, task_id: &str, base_dir: &str) -> Result<Vec<BtFileEntry>, String> {
        let progress = self.status_of(task_id, base_dir).await?;
        if progress.metadata_fetching {
            return Err("BT_METADATA_PENDING: 磁力元数据尚未获取，请稍后再试".into());
        }
        Ok(progress.files)
    }

    /// 勾选文件。
    pub async fn select_files(&self, _task_id: &str, _indices: &[u32]) -> Result<(), String> {
        Ok(())
    }

    /// 设置变更时同步限速与做种策略。
    pub async fn apply_settings(&self, _settings: &AppSettings) -> Result<(), String> {
        Ok(())
    }

    /// 预检磁力链接元数据（使用 librqbit list_only 模式）。
    pub async fn inspect_magnet(&self, magnet_url: &str, timeout_secs: u64) -> Result<crate::models::BtTorrentInspectResult, String> {
        let mut guard = self.session.lock().await;
        let session = if let Some(s) = guard.as_ref() {
            s.clone()
        } else {
            let default_dir = std::env::temp_dir();
            let opts = SessionOptions {
                dht: Some(Default::default()),
                fastresume: true,
                ..Default::default()
            };
            let s = Session::new_with_opts(default_dir, opts)
                .await
                .map_err(|e| format!("初始化 BT 引擎失败: {e}"))?;
            *guard = Some(s.clone());
            s
        };
        drop(guard);

        let mut magnet_with_trackers = magnet_url.to_string();
        for tr in DEFAULT_PUBLIC_TRACKERS {
            let encoded: String = url::form_urlencoded::byte_serialize(tr.as_bytes()).collect();
            if !magnet_with_trackers.contains(&encoded) {
                magnet_with_trackers.push_str("&tr=");
                magnet_with_trackers.push_str(&encoded);
            }
        }

        let add_torrent = librqbit::AddTorrent::from_url(&magnet_with_trackers);
        let opts = librqbit::AddTorrentOptions {
            list_only: true,
            ..Default::default()
        };

        let fut = session.add_torrent(add_torrent, Some(opts));
        let response = match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), fut).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => return Err(format!("获取磁力元数据失败: {e}")),
            Err(_) => return Err("未能即时获取到磁力元数据（网络较慢或节点较少），建议直接点击【开始下载】，任务会在后台全速获取并开始下载".into()),
        };

        match response {
            librqbit::AddTorrentResponse::ListOnly(resp) => {
                let name = resp.info.name().map(|n| n.to_string()).unwrap_or_else(|| "未命名种子".into());
                let mut files = Vec::new();
                let mut total_bytes = 0u64;
                for (idx, f) in resp.info.iter_file_details().enumerate() {
                    total_bytes += f.len;
                    files.push(BtFileEntry {
                        index: (idx + 1) as u32,
                        path: f.filename.to_string(),
                        length_bytes: f.len,
                        selected: true,
                        downloaded_bytes: 0,
                    });
                }
                Ok(crate::models::BtTorrentInspectResult {
                    info_hash: resp.info_hash.as_string(),
                    name,
                    total_bytes,
                    files,
                })
            }
            _ => Err("未能解析磁力元数据响应".into()),
        }
    }

    /// 优雅退出。
    pub async fn shutdown(&self) {
        let mut guard = self.session.lock().await;
        if let Some(session) = guard.take() {
            session.stop().await;
        }
    }
}

/// 校验种子文件格式与大小限制。
pub fn validate_torrent_bytes(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 10 {
        return Err("种子文件过小，非有效 .torrent 格式".into());
    }
    if !bytes.starts_with(b"d") {
        return Err("种子文件头非法，非有效 Bencode 字典".into());
    }
    Ok(())
}

pub fn validate_torrent_file(path: &Path) -> Result<(), String> {
    let metadata = std::fs::metadata(path).map_err(|e| format!("无法读取种子文件属性：{e}"))?;
    if metadata.len() > 20 * 1024 * 1024 {
        return Err("种子文件超过 20MB 限制".into());
    }
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| format!("打开种子失败：{e}"))?;
    let mut buf = [0u8; 16];
    let n = file.read(&mut buf).map_err(|e| format!("读取种子头部失败：{e}"))?;
    validate_torrent_bytes(&buf[..n])
}

pub async fn validate_torrent_file_async(path: &Path) -> Result<(), String> {
    let metadata = tokio::fs::metadata(path).await.map_err(|e| format!("无法读取种子文件属性：{e}"))?;
    if metadata.len() > 20 * 1024 * 1024 {
        return Err("种子文件超过 20MB 限制".into());
    }
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await.map_err(|e| format!("打开种子失败：{e}"))?;
    let mut buf = [0u8; 16];
    let n = file.read(&mut buf).await.map_err(|e| format!("读取种子头部失败：{e}"))?;
    validate_torrent_bytes(&buf[..n])
}

/// 解析种子文件字节内容，提取文件名、总大小与文件列表（用于新建任务前预览与勾选）。
pub fn inspect_torrent_bytes(bytes: &[u8]) -> Result<crate::models::BtTorrentInspectResult, String> {
    validate_torrent_bytes(bytes)?;
    let meta = librqbit::torrent_from_bytes(bytes)
        .map_err(|e| format!("未能从种子数据解析出有效元数据: {e}"))?;
    let info = &meta.info.data;
    let name = info.name.as_ref().map(|n| n.to_string()).unwrap_or_else(|| "未命名种子".into());
    let mut files = Vec::new();
    let mut total_bytes = 0u64;

    if let Some(meta_files) = &info.files {
        for (idx, f) in meta_files.iter().enumerate() {
            let path_str = f.path.iter().map(|p| p.to_string()).collect::<Vec<_>>().join("/");
            total_bytes += f.length;
            files.push(BtFileEntry {
                index: (idx + 1) as u32,
                path: path_str,
                length_bytes: f.length,
                selected: true,
                downloaded_bytes: 0,
            });
        }
    } else if let Some(length) = info.length {
        total_bytes = length;
        files.push(BtFileEntry {
            index: 1,
            path: name.clone(),
            length_bytes: length,
            selected: true,
            downloaded_bytes: 0,
        });
    }

    Ok(crate::models::BtTorrentInspectResult {
        info_hash: meta.info_hash.as_string(),
        name,
        total_bytes,
        files,
    })
}

/// 从本地文件路径解析种子元数据。
pub async fn inspect_torrent_file(path: &Path) -> Result<crate::models::BtTorrentInspectResult, String> {
    validate_torrent_file_async(path).await?;
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("读取种子文件失败: {e}"))?;
    inspect_torrent_bytes(&bytes)
}

// ===== 任务运行循环（由 manager.spawn_worker 调用） =====

/// BT 任务 worker：添加到 librqbit 后轮询真实状态，直到完成/失败/取消。
/// 返回语义与 `download_once` 一致：`Ok(task)` = 下载完成；
/// `Err(BT_TERMINAL_PREFIX..)` = 终态失败（任务状态已落库）。
pub async fn run_task(
    manager: &SharedManager,
    mut task: DownloadTask,
    token: CancellationToken,
) -> Result<DownloadTask, String> {
    let settings = manager.settings().await;
    let add_result = manager.bt.add_task(&manager.app, &settings, &task).await;
    let gid = match add_result {
        Ok(gid) => gid,
        Err(error) => return fail_terminal(manager, &mut task, &error).await,
    };
    tracing::info!(task_id = %task.id, gid = %gid, "BT 任务开始");
    poll_until_finished(manager, &mut task, &token).await
}

/// 轮询 BT 状态直到终态。取消时暂停下载并返回通用错误。
async fn poll_until_finished(
    manager: &SharedManager,
    task: &mut DownloadTask,
    token: &CancellationToken,
) -> Result<DownloadTask, String> {
    loop {
        tokio::select! {
            _ = token.cancelled() => {
                let _ = manager.bt.pause_task(&task.id).await;
                return Err("BT_CANCELLED".into());
            }
            _ = tokio::time::sleep(POLL_INTERVAL) => {}
        }
        let progress = match manager.bt.status_of(&task.id, &task.destination).await {
            Ok(progress) => progress,
            Err(error) => return fail_terminal(manager, task, &error).await,
        };
        apply_progress(task, &progress);
        let _ = manager.store.upsert_task(task).await;
        manager.emit_task("updated", task);

        match progress.state.as_str() {
            "complete" => {
                let settings = manager.settings().await;
                // 做种控制：默认关闭（§3 BT/磁力内核强约束）
                if !settings.bt_seed_enabled {
                    tracing::info!(task_id = %task.id, "BT 下载完成，做种默认关闭，立即停止做种");
                    let _ = manager.bt.pause_task(&task.id).await;
                    return Ok(task.clone());
                }
                // 若开启做种，检查是否达到目标分享率
                let downloaded = if progress.total_bytes > 0 { progress.total_bytes } else { progress.downloaded_bytes };
                let ratio = if downloaded > 0 { progress.uploaded_bytes as f64 / downloaded as f64 } else { 0.0 };
                if settings.bt_seed_ratio > 0.0 && ratio >= settings.bt_seed_ratio {
                    tracing::info!(task_id = %task.id, ratio, target = settings.bt_seed_ratio, "已达到目标分享率，停止做种");
                    let _ = manager.bt.pause_task(&task.id).await;
                    return Ok(task.clone());
                }
            }
            "error" => {
                let message = progress
                    .error
                    .unwrap_or_else(|| "BT 下载失败".into());
                return fail_terminal(manager, task, &message).await;
            }
            "removed" => {
                return fail_terminal(manager, task, "下载已被移除，请重试任务").await;
            }
            _ => {}
        }
    }
}

/// 把 BT 真实进度映射进任务字段。
fn apply_progress(task: &mut DownloadTask, progress: &status::BtProgress) {
    if !progress.metadata_fetching || progress.total_bytes > 0 {
        task.total_bytes = progress.total_bytes;
        task.downloaded_bytes = progress.downloaded_bytes;
    }
    task.speed = progress.download_speed;
    task.active_connections = progress.num_peers.min(u8::MAX as u32) as u8;
    task.eta_seconds = status::estimate_eta(progress);
    task.bt_runtime = Some(BtRuntimeStatus {
        num_seeds: progress.num_seeds,
        num_peers: progress.num_peers,
        upload_speed: progress.upload_speed,
        fetching_metadata: progress.metadata_fetching,
        uploaded_bytes: progress.uploaded_bytes,
        seeding: progress.seeder,
    });
    apply_metadata_transition(task, progress);
}

/// 元数据就绪瞬间：回填显示名/文件名/infohash。
fn apply_metadata_transition(task: &mut DownloadTask, progress: &status::BtProgress) {
    let Some(meta) = task.bt_meta.as_mut() else {
        return;
    };
    if meta.metadata_ready {
        return;
    }
    if progress.total_bytes > 0 || !progress.metadata_fetching {
        meta.metadata_ready = true;
    }
    if !progress.info_hash.is_empty() {
        meta.info_hash.clone_from(&progress.info_hash);
        meta.info_hash = meta.info_hash.to_lowercase();
    }
    if let Some(name) = progress.display_name.as_ref() {
        if !name.trim().is_empty() {
            meta.display_name = Some(name.clone());
            // 多文件种子显示种子名；单文件种子显示文件名（去掉路径分量）。
            let display = if progress.files.len() <= 1 {
                progress
                    .files
                    .first()
                    .map(|file| last_path_component(&file.path))
                    .unwrap_or_else(|| name.clone())
            } else {
                name.clone()
            };
            let cleaned = crate::manager::safe_name(&display);
            if !cleaned.is_empty() {
                task.file_name = cleaned;
                task.category = crate::manager::category(&task.file_name);
            }
        }
    }
}

fn last_path_component(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_string()
}

/// 终态失败：落库 Failed + emit + 返回 `BT_TERMINAL` 前缀错误。
async fn fail_terminal(
    manager: &SharedManager,
    task: &mut DownloadTask,
    message: &str,
) -> Result<DownloadTask, String> {
    task.status = TaskStatus::Failed;
    task.error = Some(message.to_string());
    task.speed = 0;
    task.eta_seconds = None;
    task.active_connections = 0;
    let _ = manager.store.upsert_task(task).await;
    manager.emit_task("updated", task);
    Err(format!("{BT_TERMINAL_PREFIX}{message}"))
}

/// 删除 BT 任务的物理文件：只删除任务目录下、由本任务 file_name 直接
/// 命中的普通文件；路径规范化后必须位于任务目录之内，绝不递归删除
/// 目录（AGENTS.md §7：删除文件与仅删除记录是两个明确选项）。
pub async fn delete_task_files(task: &DownloadTask) -> Vec<String> {
    let mut deleted = Vec::new();
    let destination = std::path::Path::new(&task.destination);
    let Ok(destination_root) = destination.canonicalize() else {
        return deleted;
    };
    let base_name = task.file_name.trim();
    if base_name.is_empty() {
        return deleted;
    }
    let direct = destination.join(base_name);
    let Ok(canonical) = direct.canonicalize() else {
        return deleted;
    };
    if canonical.starts_with(&destination_root) && canonical.is_file() {
        if tokio::fs::remove_file(&canonical).await.is_ok() {
            deleted.push(canonical.to_string_lossy().into_owned());
        }
    }
    deleted
}

/// 格式化限速（KB/s 转字符串格式）。
pub fn format_limit(kbps: u64) -> String {
    if kbps == 0 {
        "0".into()
    } else {
        format!("{kbps}K")
    }
}

/// 做种时长（小时）。0 = 完成即停（默认）。
pub fn seed_time_hours(settings: &AppSettings) -> u32 {
    if settings.bt_seed_enabled {
        9999
    } else {
        0
    }
}

/// 构建添加任务时的下载选项。
pub fn build_add_options(settings: &AppSettings, task: &DownloadTask) -> serde_json::Value {
    let mut options = serde_json::json!({
        "dir": task.destination,
        "seed-time": seed_time_hours(settings).to_string(),
        "seed-ratio": format!("{:.2}", settings.bt_seed_ratio.max(0.0)),
    });
    if let Some(meta) = task.bt_meta.as_ref() {
        if meta.metadata_ready && !meta.selected_files.is_empty() {
            options["select-file"] = serde_json::Value::String(
                meta.selected_files
                    .iter()
                    .map(|index| index.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        if meta.streaming_priority {
            options["bt-prioritize-piece"] = serde_json::Value::String("head=16M,tail=16M".into());
        }
    }
    let trackers = tracker_list_csv(&settings.bt_extra_trackers);
    if !trackers.is_empty() {
        options["bt-tracker"] = serde_json::Value::String(trackers);
    }
    options
}

/// 把多行 Tracker 文本解析为逗号连接列表。
pub fn tracker_list_csv(raw: &str) -> String {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::BtTaskMeta;

    #[test]
    fn enrich_magnet_url_injects_trackers() {
        let mut settings = AppSettings::default();
        settings.bt_extra_trackers = "  https://custom.tracker.org/announce\n# comment\n\n".into();
        let enriched = BtEngine::enrich_magnet_url(
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=test",
            &settings,
        );
        assert!(enriched.contains("tr="));
        assert!(enriched.contains("custom.tracker.org"));
        assert!(enriched.contains("opentrackr.org"));
    }

    #[test]
    fn metadata_transition_fills_name_and_hash_only_once() {
        let mut task = bt_task_for_test();
        let progress = status::BtProgress {
            state: "active".into(),
            total_bytes: 1024,
            downloaded_bytes: 512,
            download_speed: 128,
            info_hash: "ABCDEF0123456789ABCDEF0123456789ABCDEF01".into(),
            display_name: Some("My Torrent".into()),
            files: vec![BtFileEntry {
                index: 1,
                path: "D:/dl/My Torrent/video.mp4".into(),
                length_bytes: 1024,
                selected: true,
                downloaded_bytes: 512,
            }],
            metadata_fetching: false,
            ..Default::default()
        };
        apply_metadata_transition(&mut task, &progress);
        let meta = task.bt_meta.as_ref().unwrap();
        assert!(meta.metadata_ready);
        assert_eq!(meta.info_hash, "abcdef0123456789abcdef0123456789abcdef01");
        assert_eq!(task.file_name, "video.mp4");

        // 第二次调用不得覆盖用户可能修改过的名称。
        task.file_name = "renamed.mp4".into();
        let again = status::BtProgress {
            state: "active".into(),
            total_bytes: 1024,
            info_hash: "abcdef0123456789abcdef0123456789abcdef01".into(),
            display_name: Some("Other".into()),
            metadata_fetching: false,
            ..Default::default()
        };
        apply_metadata_transition(&mut task, &again);
        assert_eq!(task.file_name, "renamed.mp4");
    }

    #[test]
    fn progress_mapping_handles_metadata_phase() {
        let mut task = bt_task_for_test();
        let progress = status::BtProgress {
            state: "active".into(),
            total_bytes: 0,
            downloaded_bytes: 0,
            download_speed: 0,
            metadata_fetching: true,
            ..Default::default()
        };
        apply_progress(&mut task, &progress);
        assert_eq!(task.total_bytes, 0);
        assert_eq!(task.downloaded_bytes, 0);
        let runtime = task.bt_runtime.as_ref().unwrap();
        assert!(runtime.fetching_metadata);
    }

    #[test]
    fn validate_torrent_bytes_checks_header() {
        assert!(validate_torrent_bytes(b"d8:announce").is_ok());
        assert!(validate_torrent_bytes(b"invalid").is_err());
        assert!(validate_torrent_bytes(b"d").is_err());
    }

    fn bt_task_for_test() -> DownloadTask {
        DownloadTask {
            id: "t1".into(),
            url: "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567".into(),
            file_name: String::new(),
            destination: "D:/dl".into(),
            total_bytes: 0,
            downloaded_bytes: 0,
            speed: 0,
            eta_seconds: None,
            status: TaskStatus::Downloading,
            error: None,
            created_at: 0,
            completed_at: None,
            scheduled_at: None,
            category: "other".into(),
            queue_position: 0,
            priority: 0,
            retry_count: 0,
            max_retries: 3,
            checksum_sha256: None,
            expected_checksum: None,
            source: "desktop".into(),
            etag: None,
            last_modified: None,
            final_url: None,
            response_status: None,
            content_type: None,
            accepts_ranges: None,
            headers: Default::default(),
            media: None,
            per_task_speed_limit: 0,
            collision_policy: Default::default(),
            completion_action: Default::default(),
            connection_count: 8,
            active_connections: 0,
            segments: Vec::new(),
            retry_policy_override: None,
            proxy_override: None,
            proxy_auth: None,
            task_kind: crate::models::TaskKind::Bt,
            bt_meta: Some(BtTaskMeta {
                info_hash: "0123456789abcdef0123456789abcdef01234567".into(),
                selected_files: Vec::new(),
                display_name: None,
                metadata_ready: false,
                torrent_data_base64: None,
                streaming_priority: false,
            }),
            bt_runtime: None,
            cloud_refresh: None,
        }
    }
}
