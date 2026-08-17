//! BT/磁力下载引擎（roadmap BT-02/03/05，2026-08-16 负责人批准）。
//!
//! 架构：aria2 以按需安装的固定版本子进程运行，本模块负责
//! 1. 进程生命周期（随机端口 + `--rpc-secret`，仅 127.0.0.1）；
//! 2. 任务 ↔ aria2 gid 绑定（进程重启后按 infohash 重绑）；
//! 3. 任务运行循环（轮询 aria2 真实状态 → 增量更新任务并 emit）。
//!
//! 强约束（AGENTS.md §3 BT/磁力内核）：做种默认关闭；令牌不落日志；
//! 元数据获取前不伪造文件名/大小；暂停/退出保持可恢复。

pub mod magnet;
pub mod process;
pub mod rpc;
pub mod status;

use crate::manager::SharedManager;
use crate::models::{AppSettings, BtFileEntry, BtRuntimeStatus, DownloadTask, TaskStatus};
use process::{Aria2LaunchConfig, Aria2Process};
use rpc::{Aria2Rpc, SecretToken};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// BT 任务终态错误前缀：worker 收到后直接结束，不进入退避重试
/// （与 `MEDIA_PROBE_ERROR:` 同语义；aria2 未安装、进程死亡等不可重试）。
pub const BT_TERMINAL_PREFIX: &str = "BT_TERMINAL: ";

/// 轮询间隔：aria2 真实状态 → 前端增量更新，1 秒与 HTTP 进度节奏一致
/// （AGENTS.md §8：事件驱动增量更新；非全量轮询任务列表）。
const POLL_INTERVAL: Duration = Duration::from_secs(1);

struct RunningEngine {
    rpc: Arc<Aria2Rpc>,
    process: Aria2Process,
    bindings: HashMap<String, String>,
}

/// BT 引擎：持有 aria2 子进程与任务绑定。所有方法可在多个 worker 间并发调用。
#[derive(Default)]
pub struct BtEngine {
    inner: Mutex<Option<RunningEngine>>,
}

impl BtEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// 确保 aria2 进程已启动并就绪。已启动且存活时直接返回；
    /// 进程死亡时重新拉起（会话文件由 aria2 `--input-file` 自动恢复）。
    pub async fn ensure_started(
        &self,
        app: &tauri::AppHandle,
        settings: &AppSettings,
    ) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        if let Some(running) = guard.as_mut() {
            if running.process.is_running() {
                return Ok(());
            }
        }
        *guard = None;
        let running = start_engine(app, settings).await?;
        *guard = Some(running);
        Ok(())
    }

    pub async fn is_running(&self) -> bool {
        let mut guard = self.inner.lock().await;
        guard
            .as_mut()
            .map(|running| running.process.is_running())
            .unwrap_or(false)
    }

    /// 添加任务到 aria2 并返回 gid。已有绑定（暂停后恢复）或重复下载
    /// （进程重启后按 infohash 重绑）时复用现有 gid 并恢复（unpause）。
    pub async fn add_task(
        &self,
        app: &tauri::AppHandle,
        settings: &AppSettings,
        task: &DownloadTask,
    ) -> Result<String, String> {
        self.ensure_started(app, settings).await?;
        let mut guard = self.inner.lock().await;
        let running = guard
            .as_mut()
            .ok_or_else(|| "aria2 未运行".to_string())?;

        if let Some(gid) = running.bindings.get(&task.id) {
            // 同进程生命周期内的恢复：复用 gid 并解除暂停。
            let _ = running.rpc.unpause(gid).await;
            return Ok(gid.clone());
        }

        let options = build_add_options(settings, task);
        let is_magnet = task.url.trim().to_ascii_lowercase().starts_with("magnet:");
        let add_result = if is_magnet {
            running
                .rpc
                .add_uri(&[task.url.trim().to_string()], &options)
                .await
        } else {
            // 拖放创建的任务优先使用随元数据持久化的种子内容；
            // 路径任务回读磁盘（原文件被移动时提示用户）。
            let torrent_b64 = match task
                .bt_meta
                .as_ref()
                .and_then(|meta| meta.torrent_data_base64.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                Some(inline) => inline.to_string(),
                None => read_torrent_base64(&task.url).await?,
            };
            running.rpc.add_torrent(&torrent_b64, &[], &options).await
        };

        match add_result {
            Ok(gid) => {
                running.bindings.insert(task.id.clone(), gid.clone());
                Ok(gid)
            }
            Err(error) if error.starts_with("BT_DOWNLOAD_DUPLICATE") => {
                // 会话恢复/重复添加：按 infohash 找回已有下载并绑定。
                let info_hash = task
                    .bt_meta
                    .as_ref()
                    .map(|meta| meta.info_hash.as_str())
                    .unwrap_or_default();
                let existing = find_gid_by_infohash(&running.rpc, info_hash).await?;
                let gid = existing
                    .ok_or_else(|| "该磁力/种子已在下载列表中，但找不到对应的 aria2 任务".to_string())?;
                let _ = running.rpc.unpause(&gid).await;
                running.bindings.insert(task.id.clone(), gid.clone());
                Ok(gid)
            }
            Err(error) => Err(error),
        }
    }

    /// 暂停任务（保留分片与控制文件，可恢复）。
    pub async fn pause_task(&self, task_id: &str) -> Result<(), String> {
        self.with_binding(task_id, |rpc, gid| async move { rpc.pause(&gid).await })
            .await
    }

    /// 彻底移除任务（停止下载、清理 aria2 控制文件与结果记录；数据文件不动）。
    pub async fn remove_task(&self, task_id: &str) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        let Some(running) = guard.as_mut() else {
            return Ok(());
        };
        let Some(gid) = running.bindings.remove(task_id) else {
            return Ok(());
        };
        // 进行中的下载需 remove；已完成的 remove 会报错，可忽略。
        let _ = running.rpc.remove(&gid).await;
        let _ = running.rpc.remove_download_result(&gid).await;
        Ok(())
    }

    /// 查询任务进度快照。`base_dir` 为任务下载目录，用于把 aria2 的绝对
    /// 文件路径转换为种子内相对路径（见 status::strip_to_torrent_relative）。
    pub async fn status_of(&self, task_id: &str, base_dir: &str) -> Result<status::BtProgress, String> {
        let gid = self.binding_gid(task_id).await?;
        let rpc = self.rpc_handle().await?;
        let value = rpc.tell_status(&gid, &[]).await?;
        Ok(status::parse_status(&value, base_dir))
    }

    /// 列出种子内文件（元数据就绪后有效）。
    pub async fn files_of(&self, task_id: &str, base_dir: &str) -> Result<Vec<BtFileEntry>, String> {
        let progress = self.status_of(task_id, base_dir).await?;
        if progress.metadata_fetching {
            return Err("BT_METADATA_PENDING: 磁力元数据尚未获取，请稍后再试".into());
        }
        Ok(progress.files)
    }

    /// 勾选文件（aria2 select-file，1 基索引）。
    pub async fn select_files(&self, task_id: &str, indices: &[u32]) -> Result<(), String> {
        let mut selection = String::new();
        for (position, index) in indices.iter().enumerate() {
            if position > 0 {
                selection.push(',');
            }
            selection.push_str(&index.to_string());
        }
        if selection.is_empty() {
            return Err("至少需要勾选一个文件".into());
        }
        self.with_binding(task_id, |rpc, gid| async move {
            rpc.change_option(&gid, &json!({ "select-file": selection }))
                .await
        })
        .await
    }

    /// 设置变更时同步到 aria2：全局限速即时生效；做种策略对已绑定任务
    /// 逐个下发（seed-time/seed-ratio 不在 changeGlobalOption 白名单内）。
    pub async fn apply_settings(&self, settings: &AppSettings) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        let Some(running) = guard.as_mut() else {
            return Ok(()); // 未运行时无需同步，下次启动自然生效。
        };
        let limits = json!({
            "max-overall-download-speed": format_limit(settings.speed_limit_kbps),
            "max-overall-upload-speed": format_limit(settings.bt_upload_limit_kbps),
        });
        running.rpc.change_global_option(&limits).await?;
        // Tracker 列表同步：单独下发且失败仅记录——旧 aria2 会话可能不在此
        // 选项的白名单内，不应因此阻断限速等关键设置的同步。
        let trackers = tracker_list_csv(&settings.bt_extra_trackers);
        if !trackers.is_empty() {
            if let Err(error) = running
                .rpc
                .change_global_option(&json!({ "bt-tracker": trackers }))
                .await
            {
                tracing::warn!(error = %error, "同步 Tracker 列表到 aria2 失败（新任务仍会逐个携带）");
            }
        }
        let seed_option = json!({
            "seed-time": seed_time_hours(settings).to_string(),
            "seed-ratio": format!("{:.2}", settings.bt_seed_ratio.max(0.0)),
        });
        for gid in running.bindings.values() {
            let _ = running.rpc.change_option(gid, &seed_option).await;
        }
        Ok(())
    }

    /// 优雅退出：保存会话 → 终止进程。程序退出路径调用（§3 暂停/退出可恢复）。
    pub async fn shutdown(&self) {
        let mut guard = self.inner.lock().await;
        if let Some(running) = guard.take() {
            running.process.shutdown(&running.rpc).await;
        }
    }

    async fn rpc_handle(&self) -> Result<Arc<Aria2Rpc>, String> {
        let guard = self.inner.lock().await;
        guard
            .as_ref()
            .map(|running| running.rpc.clone())
            .ok_or_else(|| "aria2 未运行，请先重试任务或检查 BT 组件".to_string())
    }

    async fn binding_gid(&self, task_id: &str) -> Result<String, String> {
        let guard = self.inner.lock().await;
        guard
            .as_ref()
            .and_then(|running| running.bindings.get(task_id).cloned())
            .ok_or_else(|| "BT 任务未在 aria2 中注册，请重试任务".to_string())
    }

    async fn with_binding<F, Fut>(&self, task_id: &str, operation: F) -> Result<(), String>
    where
        F: FnOnce(Arc<Aria2Rpc>, String) -> Fut,
        Fut: std::future::Future<Output = Result<(), String>>,
    {
        let rpc = self.rpc_handle().await?;
        let gid = self.binding_gid(task_id).await?;
        operation(rpc, gid).await
    }
}

/// 启动新的 aria2 实例：分配端口 → 拉起进程 → 暂停会话恢复的全部任务
/// （排队控制权归还应用调度器，防止绕过并发槽位）。
async fn start_engine(
    app: &tauri::AppHandle,
    settings: &AppSettings,
) -> Result<RunningEngine, String> {
    let exe_path = crate::media_tools::resolve_aria2(app)
        .ok_or_else(|| "BT 组件未安装，请先在 设置 → BT/磁力 中安装 aria2".to_string())?;
    let session_path = crate::portable::resolve_data_dir(app).join("aria2.session");
    let rpc_port = process::pick_free_port()?;
    let mut bt_listen_port = process::pick_free_port()?;
    if bt_listen_port == rpc_port {
        bt_listen_port = process::pick_free_port()?;
    }
    let secret = SecretToken::generate();
    let rpc = Arc::new(Aria2Rpc::new(rpc_port, secret.clone())?);
    let config = Aria2LaunchConfig {
        exe_path,
        session_path: session_path.clone(),
        rpc_port,
        bt_listen_port,
        secret,
        dir: PathBuf::from(&settings.download_dir),
        download_limit: format_limit(settings.speed_limit_kbps),
        upload_limit: format_limit(settings.bt_upload_limit_kbps),
        seed_time: seed_time_hours(settings),
        max_concurrent: settings.concurrent_downloads as u32 + 4,
        extra_trackers: tracker_list_csv(&settings.bt_extra_trackers),
    };
    tracing::info!(
        port = rpc_port,
        args = ?process::args_for_log(&config).join(" "),
        "启动 aria2 BT 引擎"
    );
    let process = process::launch(&config, &rpc).await?;
    // 会话恢复的任务一律先暂停：由应用调度器决定何时经 worker 恢复，
    // 保证与 HTTP 任务共享同一并发槽位（AGENTS.md §8 队列语义）。
    if let Ok(active) = rpc.tell_active().await {
        for entry in active {
            if let Some(gid) = entry.get("gid").and_then(Value::as_str) {
                let _ = rpc.pause(gid).await;
            }
        }
    }
    Ok(RunningEngine {
        rpc,
        process,
        bindings: HashMap::new(),
    })
}

/// aria2 限速格式：`0` 不限；否则 `NK`（KiB/s，与 HTTP 内核 kbps*1024 一致）。
fn format_limit(kbps: u64) -> String {
    if kbps == 0 {
        "0".into()
    } else {
        format!("{kbps}K")
    }
}

/// 做种时长（小时）。0 = 完成即停（默认）；用户开启做种后用大时长 +
/// seed-ratio 控制停止点。
fn seed_time_hours(settings: &AppSettings) -> u32 {
    if settings.bt_seed_enabled {
        9999
    } else {
        0
    }
}

/// 构建添加任务时的 aria2 下载选项。
fn build_add_options(settings: &AppSettings, task: &DownloadTask) -> Value {
    let mut options = json!({
        "dir": task.destination,
        "seed-time": seed_time_hours(settings).to_string(),
        "seed-ratio": format!("{:.2}", settings.bt_seed_ratio.max(0.0)),
    });
    if let Some(meta) = task.bt_meta.as_ref() {
        if meta.metadata_ready && !meta.selected_files.is_empty() {
            options["select-file"] = Value::String(
                meta.selected_files
                    .iter()
                    .map(|index| index.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        // 边下边看：优先首尾分片（aria2 手册 head[=SIZE],tail[=SIZE]，K/M 后缀）。
        if meta.streaming_priority {
            options["bt-prioritize-piece"] = Value::String("head=16M,tail=16M".into());
        }
    }
    // 每任务追加 Tracker 列表：引擎启动后再修改全局设置也能对新任务生效。
    let trackers = tracker_list_csv(&settings.bt_extra_trackers);
    if !trackers.is_empty() {
        options["bt-tracker"] = Value::String(trackers);
    }
    options
}

/// 把多行 Tracker 文本解析为 aria2 逗号连接列表：按行拆分、去首尾空白、
/// 丢弃空行，保持输入顺序。全部为空时返回空串（不下发该选项）。
pub(crate) fn tracker_list_csv(raw: &str) -> String {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join(",")
}

/// 读取 .torrent 文件并 base64 编码（aria2.addTorrent 参数要求）。
async fn read_torrent_base64(path: &str) -> Result<String, String> {
    let path = std::path::Path::new(path);
    process::validate_torrent_file(path)?;
    let bytes = tokio::task::spawn_blocking({
        let path = path.to_path_buf();
        move || std::fs::read(&path).map_err(|error| format!("读取种子文件失败：{error}"))
    })
    .await
    .map_err(|error| format!("读取种子文件失败：{error}"))??;
    use base64::Engine as _;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// 在 active/waiting/stopped 三个列表中按 infohash 查找已有下载。
async fn find_gid_by_infohash(rpc: &Aria2Rpc, info_hash: &str) -> Result<Option<String>, String> {
    if info_hash.is_empty() {
        return Ok(None);
    }
    let lists = [rpc.tell_active().await, rpc.tell_waiting().await, rpc.tell_stopped().await];
    for list in lists {
        let entries = match list {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries {
            let Some(gid) = entry.get("gid").and_then(Value::as_str) else {
                continue;
            };
            let Ok(full) = rpc.tell_status(gid, &[]).await else {
                continue;
            };
            let found = full
                .get("bittorrent")
                .and_then(|bt| bt.get("infoHash"))
                .and_then(Value::as_str)
                .map(|hash| hash.eq_ignore_ascii_case(info_hash))
                .unwrap_or(false);
            if found {
                return Ok(Some(gid.to_string()));
            }
        }
    }
    Ok(None)
}

// ===== 任务运行循环（由 manager.spawn_worker 调用） =====

/// BT 任务 worker：添加到 aria2 后轮询真实状态，直到完成/失败/取消。
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

/// 轮询 aria2 状态直到终态。取消时暂停 aria2 下载并返回通用错误
/// （worker 检测 token 已取消后静默退出，不覆盖任务状态）。
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
        match progress.aria2_status.as_str() {
            "complete" => return Ok(task.clone()),
            "error" => {
                let message = progress
                    .error
                    .unwrap_or_else(|| "aria2 报告下载失败".into());
                return fail_terminal(manager, task, &message).await;
            }
            "removed" => {
                return fail_terminal(manager, task, "下载已被 aria2 移除，请重试任务").await;
            }
            _ => {}
        }
    }
}

/// 把 aria2 真实进度映射进任务字段（不伪造：元数据阶段不写 total/file_name）。
fn apply_progress(task: &mut DownloadTask, progress: &status::BtProgress) {
    if !progress.metadata_fetching {
        task.total_bytes = progress.total_bytes;
        task.downloaded_bytes = progress.downloaded_bytes;
    } else {
        task.downloaded_bytes = 0;
    }
    task.speed = progress.download_speed;
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

/// 元数据就绪瞬间：回填显示名/文件名/infohash。磁力元数据未就绪前
/// 不写任何名称或大小（§3 BT 约束）。
fn apply_metadata_transition(task: &mut DownloadTask, progress: &status::BtProgress) {
    let Some(meta) = task.bt_meta.as_mut() else {
        return;
    };
    if meta.metadata_ready || progress.metadata_fetching {
        return;
    }
    meta.metadata_ready = true;
    if !progress.info_hash.is_empty() {
        meta.info_hash.clone_from(&progress.info_hash);
        meta.info_hash = meta.info_hash.to_lowercase();
    }
    if let Some(name) = progress.display_name.as_ref() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::BtTaskMeta;

    fn settings_with(seed_enabled: bool) -> AppSettings {
        let mut settings = AppSettings::default();
        settings.bt_seed_enabled = seed_enabled;
        settings
    }

    #[test]
    fn format_limit_maps_zero_and_kbps() {
        assert_eq!(format_limit(0), "0");
        assert_eq!(format_limit(512), "512K");
    }

    #[test]
    fn seed_time_defaults_off() {
        assert_eq!(seed_time_hours(&settings_with(false)), 0);
        assert_eq!(seed_time_hours(&settings_with(true)), 9999);
    }

    #[test]
    fn add_options_respect_seed_policy_and_selection() {
        let mut task = crate::models::DownloadTask {
            id: "t1".into(),
            url: "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567".into(),
            file_name: String::new(),
            destination: "D:/dl".into(),
            total_bytes: 0,
            downloaded_bytes: 0,
            speed: 0,
            eta_seconds: None,
            status: TaskStatus::Queued,
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
                selected_files: vec![1, 3],
                display_name: None,
                metadata_ready: true,
                torrent_data_base64: None,
                streaming_priority: false,
            }),
            bt_runtime: None,
        };
        let settings = settings_with(false);
        let options = build_add_options(&settings, &task);
        assert_eq!(options["seed-time"], "0");
        assert_eq!(options["select-file"], "1,3");
        assert_eq!(options["dir"], "D:/dl");
        assert!(options.get("bt-prioritize-piece").is_none());
        assert!(options.get("bt-tracker").is_none(), "Tracker 列表为空时不下发");

        task.bt_meta = Some(BtTaskMeta {
            metadata_ready: false,
            ..task.bt_meta.clone().unwrap()
        });
        let options = build_add_options(&settings, &task);
        assert!(options.get("select-file").is_none());
    }

    #[test]
    fn add_options_carry_streaming_priority_and_trackers() {
        // 边下边看：meta.streaming_priority 时下发首尾分片优先（aria2 head/tail 语法）。
        let mut task = bt_task_for_test();
        task.bt_meta = Some(BtTaskMeta {
            streaming_priority: true,
            ..task.bt_meta.clone().unwrap()
        });
        let mut settings = settings_with(false);
        settings.bt_extra_trackers = "  https://t1.example.com/announce\n# 注释行\nhttp://t2.example.com/announce\n\n".into();
        let options = build_add_options(&settings, &task);
        assert_eq!(options["bt-prioritize-piece"], "head=16M,tail=16M");
        assert_eq!(
            options["bt-tracker"],
            "https://t1.example.com/announce,http://t2.example.com/announce"
        );
    }

    #[test]
    fn tracker_list_csv_filters_blank_and_comment_lines() {
        assert_eq!(tracker_list_csv(""), "");
        assert_eq!(tracker_list_csv("  \n \n"), "");
        assert_eq!(
            tracker_list_csv(" https://a.example/x \n# comment\nhttp://b.example/y\n"),
            "https://a.example/x,http://b.example/y"
        );
    }

    #[test]
    fn metadata_transition_fills_name_and_hash_only_once() {
        let mut task = bt_task_for_test();
        let progress = status::parse_status(&serde_json::json!({
            "status": "active",
            "totalLength": "1024",
            "completedLength": "512",
            "downloadSpeed": "128",
            "bittorrent": {
                "infoHash": "ABCDEF0123456789ABCDEF0123456789ABCDEF01",
                "info": { "name": "My Torrent" }
            },
            "files": [
                { "index": "1", "path": "D:/dl/My Torrent/video.mp4", "length": "1024", "selected": "true" }
            ]
        }), "D:/dl");
        apply_metadata_transition(&mut task, &progress);
        let meta = task.bt_meta.as_ref().unwrap();
        assert!(meta.metadata_ready);
        assert_eq!(meta.info_hash, "abcdef0123456789abcdef0123456789abcdef01");
        assert_eq!(task.file_name, "video.mp4");

        // 第二次调用不得覆盖用户可能修改过的名称。
        task.file_name = "renamed.mp4".into();
        let again = status::parse_status(&serde_json::json!({
            "status": "active",
            "totalLength": "1024",
            "bittorrent": { "infoHash": "abcdef0123456789abcdef0123456789abcdef01",
                            "info": { "name": "Other" } }
        }), "");
        apply_metadata_transition(&mut task, &again);
        assert_eq!(task.file_name, "renamed.mp4");
    }

    #[test]
    fn progress_mapping_keeps_metadata_phase_empty() {
        let mut task = bt_task_for_test();
        let progress = status::parse_status(&serde_json::json!({
            "status": "active",
            "totalLength": "0",
            "completedLength": "1234",
            "downloadSpeed": "10",
            "files": [ { "index": "1", "path": "[METADATA]x.torrent", "length": "0", "selected": "true" } ]
        }), "");
        apply_progress(&mut task, &progress);
        assert_eq!(task.total_bytes, 0);
        assert_eq!(task.downloaded_bytes, 0);
        let runtime = task.bt_runtime.as_ref().unwrap();
        assert!(runtime.fetching_metadata);
        assert!(task.file_name.is_empty());
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
        }
    }
}
