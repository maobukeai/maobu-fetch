use crate::{
    models::{
        AppSettings, BackoffStrategy, BatchTaskRequest, BtNewTaskRequest, BtTaskMeta,
        CollisionPolicy, CompletionAction, ConnectionState, DownloadPreset, DownloadSegment,
        DownloadTask, LowDiskPayload, NewTaskRequest, PowerAction, PowerActionPhase,
        PowerActionState, ProxyAuth, RestorePreview, RestoreStats, RetryPolicy, SegmentStatus,
        SelfcheckReport, TaskConnectionsEvent, TaskKind, TaskProgressEvent, TaskStatus, WaitReason,
        MAX_PRIORITY, MIN_PRIORITY,
    },
    secure_storage::encrypt_password,
    store::Store,
};
use futures_util::StreamExt;
use reqwest::header::{
    ACCEPT_ENCODING, ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE,
    CONTENT_TYPE, ETAG, IF_RANGE, LAST_MODIFIED, RANGE,
};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use md5::Md5;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncReadExt, AsyncWriteExt, BufWriter},
    sync::{Mutex, Notify, RwLock},
};
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

mod bandwidth;
pub mod category_rules;
pub mod completion_action;
pub mod diagnose;
pub mod duplicate;
pub mod filename_cleanup;
pub mod naming_template;
mod precheck;
pub mod task_template;
pub mod work_stealing;

use bandwidth::BandwidthScheduler;
pub use category_rules::{apply_category_rules, normalize_directory, test_category_rule};
pub use diagnose::{classify_error, redact_sensitive, ErrorContext};
pub use filename_cleanup::apply_filename_cleanup;
pub use naming_template::{apply_naming_template, find_template_for_platform, NamingVars};
pub use task_template::{apply_template_to_request, match_template, test_task_template};
pub use work_stealing::{RangeWindow, WindowStatus, WorkStealingCoordinator};

pub type SharedManager = Arc<DownloadManager>;

/// Task 30：`task-notification` 事件载荷。
///
/// 后端在任务进入 Completed / Failed 终态时 emit 此结构，
/// 前端依据 `kind` 决定播放哪种提示音、是否展示"一键重试"按钮。
/// `title` / `body` 与系统通知保持一致，便于在前端 toast 中复用。
#[derive(Clone, serde::Serialize)]
struct TaskNotificationPayload {
    task_id: String,
    /// `"completed"` 或 `"failed"`。
    kind: &'static str,
    title: String,
    body: String,
}

pub(crate) struct RuntimeTaskOptions {
    pub(crate) speed_limit: AtomicU64,
    pub(crate) priority: AtomicI32,
    completion_action: RwLock<CompletionAction>,
}

const POWER_ACTION_COUNTDOWN_MILLIS: u64 = 60_000;

#[derive(Default)]
struct PowerActionRuntime {
    state: PowerActionState,
    target_ids: HashSet<String>,
    countdown_deadline: Option<u64>,
}

impl RuntimeTaskOptions {
    pub(crate) fn new(task: &DownloadTask) -> Self {
        Self {
            speed_limit: AtomicU64::new(task.per_task_speed_limit),
            priority: AtomicI32::new(task.priority),
            completion_action: RwLock::new(task.completion_action.clone()),
        }
    }

    pub(crate) async fn apply(&self, task: &mut DownloadTask) {
        task.per_task_speed_limit = self.speed_limit.load(Ordering::Relaxed);
        task.priority = self.priority.load(Ordering::Relaxed);
        task.completion_action = self.completion_action.read().await.clone();
    }
}

pub struct DownloadManager {
    pub store: Arc<Store>,
    settings: RwLock<AppSettings>,
    client: RwLock<reqwest::Client>,
    controls: Mutex<HashMap<String, CancellationToken>>,
    pub(crate) task_runtime: RwLock<HashMap<String, Arc<RuntimeTaskOptions>>>,
    path_reservation: Mutex<()>,
    power_action: Mutex<PowerActionRuntime>,
    dispatcher: Notify,
    pub(crate) app: AppHandle,
    pub(crate) bandwidth_scheduler: BandwidthScheduler,
    /// BT/磁力引擎（aria2 子进程 + gid 绑定）。2026-08-16 批准纳入。
    pub bt: crate::bt::BtEngine,
}

impl DownloadManager {
    pub async fn new(store: Arc<Store>, app: AppHandle) -> Result<SharedManager, String> {
        let settings = store.get_settings().await?;
        let bandwidth_limit = settings.speed_limit_kbps * 1024;
        let client = build_client(&settings)?;
        let auto_start = settings.auto_start;
        let manager = Arc::new(Self {
            store,
            settings: RwLock::new(settings),
            client: RwLock::new(client),
            controls: Mutex::new(HashMap::new()),
            task_runtime: RwLock::new(HashMap::new()),
            path_reservation: Mutex::new(()),
            power_action: Mutex::new(PowerActionRuntime::default()),
            dispatcher: Notify::new(),
            app,
            bandwidth_scheduler: BandwidthScheduler::new(bandwidth_limit),
            bt: crate::bt::BtEngine::new(),
        });
        // Mark every Downloading task as Interrupted and validate shard files
        // before the scheduler starts. recover_interrupted still handles the
        // Verifying/WaitingNetwork paths so they re-enter the queue.
        let _ = crate::autostart::sync_autostart(auto_start);
        let _ = manager.run_startup_selfcheck().await;
        manager.recover_interrupted().await?;
        let scheduler = manager.clone();
        tauri::async_runtime::spawn(async move { scheduler.scheduler_loop().await });
        // 分时段限速轮询（2026-08-17）：窗口切换时重算生效限速。
        let limiter = manager.clone();
        tauri::async_runtime::spawn(async move { limiter.scheduled_limit_loop().await });
        Ok(manager)
    }

    pub async fn list(&self) -> Result<Vec<DownloadTask>, String> {
        self.store.list_tasks().await
    }

    pub async fn export_tasks(&self, path: &str) -> Result<usize, String> {
        let tasks = self.store.list_tasks().await?;
        crate::task_transfer::export_file(path, &tasks, now()).await
    }

    pub async fn import_tasks(
        self: &SharedManager,
        path: &str,
        destination: &str,
    ) -> Result<Vec<DownloadTask>, String> {
        let requests = crate::task_transfer::import_requests(path, destination).await?;
        let mut imported = Vec::with_capacity(requests.len());
        for (index, request) in requests.into_iter().enumerate() {
            match self.add(request).await {
                Ok(task) => imported.push(task),
                Err(error) => {
                    return Err(format!(
                        "已导入 {} 个任务，第 {} 个任务导入失败：{error}",
                        imported.len(),
                        index + 1
                    ));
                }
            }
        }
        Ok(imported)
    }
    pub async fn settings(&self) -> AppSettings {
        self.settings.read().await.clone()
    }

    pub async fn save_settings(&self, settings: AppSettings) -> Result<(), String> {
        validate_settings(&settings)?;
        let new_client = build_client(&settings)?;
        self.store.save_settings(&settings).await?;
        *self.client.write().await = new_client;
        *self.settings.write().await = settings.clone();
        // 分时段限速（2026-08-17）：立即按当前本地时间应用生效限速，
        // 而不是等下一次窗口轮询；aria2 同步也使用同一生效值。
        // 非 Windows 环境 local_minute_of_day 返回 None，保持基础限速。
        let effective = bandwidth::effective_global_limit_kbps(
            settings.speed_limit_kbps,
            settings.scheduled_limit.as_ref(),
            bandwidth::local_minute_of_day(),
        );
        self.bandwidth_scheduler.set_limit(effective * 1024);
        let _ = crate::autostart::sync_autostart(settings.auto_start);
        // BT 引擎：全局限速/做种策略即时同步（aria2 运行中才生效，未运行时
        // 由下次启动参数承接）。失败仅记录，不阻断设置保存。
        let mut aria2_settings = settings.clone();
        aria2_settings.speed_limit_kbps = effective;
        if let Err(error) = self.bt.apply_settings(&aria2_settings).await {
            tracing::warn!(error = %error, "同步 BT 设置到 aria2 失败");
        }
        self.dispatcher.notify_waiters();
        let _ = self.app.emit("settings-updated", settings);
        Ok(())
    }

    /// 分时段限速轮询：每 30 秒按本地时间重算生效限速，窗口切换时
    /// 同步 HTTP 内核调度器与 aria2（§3 全局限速必须覆盖两个内核）。
    ///
    /// 生效值未变化时不做任何下发，避免无效 RPC。非 Windows 环境下
    /// `local_minute_of_day` 返回 `None`，循环空转不干预限速。
    async fn scheduled_limit_loop(self: Arc<Self>) {
        let mut applied: Option<u64> = None;
        loop {
            let settings = self.settings.read().await.clone();
            let effective = bandwidth::effective_global_limit_kbps(
                settings.speed_limit_kbps,
                settings.scheduled_limit.as_ref(),
                bandwidth::local_minute_of_day(),
            );
            if applied != Some(effective) {
                applied = Some(effective);
                self.bandwidth_scheduler.set_limit(effective * 1024);
                let mut aria2_settings = settings.clone();
                aria2_settings.speed_limit_kbps = effective;
                if let Err(error) = self.bt.apply_settings(&aria2_settings).await {
                    tracing::warn!(error = %error, "同步分时段限速到 aria2 失败");
                }
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }

    pub async fn power_action_state(&self) -> PowerActionState {
        self.power_action.lock().await.state.clone()
    }

    pub async fn arm_power_action(&self, action: PowerAction) -> Result<PowerActionState, String> {
        if action == PowerAction::None {
            return self.cancel_power_action().await;
        }
        let target_ids: HashSet<_> = self
            .store
            .list_tasks()
            .await?
            .into_iter()
            .filter(|task| is_power_action_target(&task.status))
            .map(|task| task.id)
            .collect();
        if target_ids.is_empty() {
            return Err("当前没有等待完成的下载任务".into());
        }
        let state = PowerActionState {
            action,
            phase: PowerActionPhase::Armed,
            remaining_seconds: 0,
            target_count: target_ids.len(),
            message: Some("队列全部成功完成后将开始 60 秒倒计时".into()),
        };
        let mut runtime = self.power_action.lock().await;
        runtime.state = state.clone();
        runtime.target_ids = target_ids;
        runtime.countdown_deadline = None;
        drop(runtime);
        self.emit_power_action_state(&state);
        self.dispatcher.notify_waiters();
        Ok(state)
    }

    pub async fn cancel_power_action(&self) -> Result<PowerActionState, String> {
        let state = PowerActionState::default();
        let mut runtime = self.power_action.lock().await;
        *runtime = PowerActionRuntime::default();
        drop(runtime);
        self.emit_power_action_state(&state);
        Ok(state)
    }

    async fn register_power_action_target(&self, id: &str) {
        let mut runtime = self.power_action.lock().await;
        if runtime.state.phase == PowerActionPhase::Idle {
            return;
        }
        runtime.target_ids.insert(id.to_string());
        runtime.countdown_deadline = None;
        runtime.state.phase = PowerActionPhase::Armed;
        runtime.state.remaining_seconds = 0;
        runtime.state.target_count = runtime.target_ids.len();
        runtime.state.message = Some("检测到新任务，等待整个队列完成".into());
        let state = runtime.state.clone();
        drop(runtime);
        self.emit_power_action_state(&state);
    }

    pub async fn add(
        &self,
        mut request: NewTaskRequest,
    ) -> Result<DownloadTask, String> {
        let parsed = Url::parse(request.url.trim())
            .map_err(|_| "请输入有效的 HTTP/HTTPS 链接".to_string())?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err("仅支持 HTTP/HTTPS 链接".into());
        }
        // Task 36：URL 解析出域名后尝试匹配任务模板，命中则套用未由用户显式设置的字段。
        // 模板查询失败按"无模板命中"处理，不阻断任务创建（与分类规则一致的安全回退）。
        if let Some(host) = parsed.host_str().map(|h| h.to_ascii_lowercase()) {
            if let Ok(templates) = self.store.task_template_list().await {
                if let Some(template) = match_template(&host, &templates) {
                    apply_template_to_request(template, &mut request);
                }
            }
        }
        let settings = self.settings().await;
        let mut file_name = safe_name(request.file_name.as_deref().unwrap_or_else(|| {
            parsed
                .path_segments()
                .and_then(|mut s| s.next_back())
                .filter(|s| !s.is_empty())
                .unwrap_or("download")
        }));
        // Task 20: 用户未手动编辑文件名时应用文件名清理规则。
        // 失败时静默回退到原始文件名（不阻断任务创建）。
        if !request.user_edited_file_name {
            if let Ok(rules) = self.store.filename_cleanup_rule_list().await {
                let cleaned = apply_filename_cleanup(&file_name, &rules);
                if !cleaned.is_empty() {
                    file_name = safe_name(&cleaned);
                }
            }
        }
        let scheduled = request.scheduled_at.filter(|value| *value > now());
        let source = request.source.unwrap_or_else(|| "desktop".into());
        let completion_action = if source == "desktop"
            || matches!(
                request.completion_action,
                CompletionAction::None | CompletionAction::OpenFolder
            ) {
            request.completion_action
        } else {
            CompletionAction::None
        };
        // 目标目录优先级：
        // 1. 用户显式指定的非空目录（不覆盖用户选择）
        // 2. 命中分类规则的目录（仅当用户未指定时自动填充）
        // 3. 全局下载目录
        let destination = self
            .resolve_destination(
                request.destination.as_deref(),
                &settings.download_dir,
                parsed.as_str(),
                &file_name,
            )
            .await;
        let mut task = DownloadTask {
            id: Uuid::new_v4().to_string(),
            url: parsed.to_string(),
            file_name: file_name.clone(),
            destination,
            total_bytes: 0,
            downloaded_bytes: 0,
            speed: 0,
            eta_seconds: None,
            status: if request.start_paused {
                TaskStatus::Paused
            } else if scheduled.is_some() {
                TaskStatus::Scheduled
            } else {
                TaskStatus::Queued
            },
            error: None,
            created_at: now(),
            completed_at: None,
            scheduled_at: scheduled,
            category: category(&file_name),
            queue_position: self.store.next_queue_position().await?,
            priority: request.priority.clamp(MIN_PRIORITY, MAX_PRIORITY),
            retry_count: 0,
            max_retries: settings.max_retries,
            checksum_sha256: None,
            expected_checksum: request
                .expected_checksum
                .map(|x| x.trim().to_ascii_lowercase()),
            source,
            etag: None,
            last_modified: None,
            final_url: None,
            response_status: None,
            content_type: None,
            accepts_ranges: None,
            headers: request.headers,
            media: request.media,
            per_task_speed_limit: request.per_task_speed_limit,
            collision_policy: request.collision_policy,
            completion_action,
            connection_count: request
                .connection_count
                .unwrap_or(settings.connections_per_download)
                .clamp(1, 32),
            active_connections: 0,
            segments: Vec::new(),
            retry_policy_override: None,
            proxy_override: None,
            proxy_auth: None,
            task_kind: TaskKind::Http,
            bt_meta: None,
            bt_runtime: None,
            cloud_refresh: request.cloud_refresh,
        };

        // PikPak 裸直链自动注入 Referer 和元数据
        if task.cloud_refresh.is_none() {
            if let Some(meta) = crate::pikpak::parse_pikpak_direct_link_meta(&task.url) {
                if !task.headers.contains_key("Referer") {
                    task.headers.insert("Referer".to_string(), "https://mypikpak.com/".to_string());
                }
                task.cloud_refresh = Some(crate::models::CloudRefreshMeta {
                    platform: "pikpak-direct".to_string(),
                    share_id: String::new(), // 裸直链无分享 ID
                    file_id: meta.file_id,
                    pass_code_token: None,
                    device_id: None,
                });
            }
        }

        self.reserve_output_path(&mut task).await?;
        self.store.upsert_task(&task).await?;
        self.register_power_action_target(&task.id).await;
        self.emit_task("created", &task);
        self.dispatcher.notify_waiters();
        Ok(task)
    }

    pub async fn add_batch(
        self: &SharedManager,
        request: BatchTaskRequest,
    ) -> Result<Vec<DownloadTask>, String> {
        if request.urls.is_empty() || request.urls.len() > 500 {
            return Err("批量任务数量必须为 1–500".into());
        }
        let mut tasks = Vec::new();
        for url in request.urls {
            let task = self
                .add(NewTaskRequest {
                    url,
                    file_name: None,
                    destination: request.destination.clone(),
                    headers: request.headers.clone(),
                    scheduled_at: request.scheduled_at,
                    priority: request.priority,
                    expected_checksum: None,
                    source: Some("batch".into()),
                    per_task_speed_limit: request.per_task_speed_limit,
                    collision_policy: request.collision_policy.clone(),
                    completion_action: request.completion_action.clone(),
                    media: None,
                    connection_count: request.connection_count,
                    start_paused: false,
                    user_edited_file_name: false,
                    cloud_refresh: None,
                })
                .await?;
            tasks.push(task);
        }
        Ok(tasks)
    }

    /// 解析新任务的目标目录（Task 11）。
    ///
    /// 优先级：
    /// 1. 用户显式指定的非空目录（已规范化）——不覆盖用户选择
    /// 2. 命中分类规则的目录——仅当用户未指定时自动填充
    /// 3. 全局默认下载目录
    ///
    /// `content_type` 在新建任务时未知，固定传 None；
    /// 因此 MIME 规则在新任务流程中不参与匹配，仅 Domain 与 Regex 生效。
    /// MIME 规则可在用户主动“测试规则”时使用。
    async fn resolve_destination(
        &self,
        user_destination: Option<&str>,
        default_download_dir: &str,
        url: &str,
        file_name: &str,
    ) -> String {
        if let Some(dir) = user_destination.map(str::trim).filter(|s| !s.is_empty()) {
            return normalize_directory(dir);
        }
        if let Ok(rules) = self.store.category_rule_list().await {
            if let Some(matched) = apply_category_rules(&rules, url, file_name, None) {
                if !matched.trim().is_empty() {
                    return normalize_directory(&matched);
                }
            }
        }
        normalize_directory(default_download_dir)
    }

    pub async fn action(self: &SharedManager, id: &str, action: &str) -> Result<(), String> {
        let Some(mut task) = self.store.get_task(id).await? else {
            return Err("任务不存在".into());
        };
        match action {
            "pause" => {
                if let Some(token) = self.controls.lock().await.remove(id) {
                    token.cancel();
                }
                task.status = TaskStatus::Paused;
                task.speed = 0;
                task.eta_seconds = None;
                task.active_connections = 0;
                for segment in &mut task.segments {
                    if segment.status == "downloading" {
                        segment.status = "paused".into();
                    }
                }
            }
            "resume" | "retry" => {
                if matches!(task.status, TaskStatus::Completed) && action == "resume" {
                    return Ok(());
                }
                // Task 32.2：用户从 PausedByMetered 手动恢复，标记 user_resumed_after_metered，
                // 阻止定时检查在本次计量网络会话内再次自动暂停。
                // 标记会在网络变为非计量时由 clear_user_resumed_after_metered 清零。
                let was_paused_by_metered = task.status == TaskStatus::PausedByMetered;
                let was_paused_by_low_disk = task.status == TaskStatus::PausedByLowDisk;
                if was_paused_by_low_disk {
                    let available_opt = precheck::check_disk_space(&task.destination);
                    if let Some(available) = available_opt {
                        let remaining = task.total_bytes.saturating_sub(task.downloaded_bytes);
                        let is_multi =
                            task.connection_count > 1 && task.accepts_ranges.unwrap_or(false);
                        let required = if is_multi {
                            remaining
                                .saturating_add(task.total_bytes)
                                .saturating_add(100 * 1024 * 1024)
                        } else {
                            remaining.saturating_add(50 * 1024 * 1024)
                        };
                        if available < required {
                            return Err(format!(
                                "磁盘空间仍不足（可用 {} 字节，需要 {} 字节），无法恢复任务",
                                available, required
                            ));
                        }
                    }
                }
                task.status = if task.scheduled_at.is_some_and(|time| time > now()) {
                    TaskStatus::Scheduled
                } else {
                    TaskStatus::Queued
                };
                task.error = None;
                task.active_connections = 0;
                if action == "retry" {
                    task.retry_count = 0;
                    if task.media.is_some() {
                        task.media = None;
                    }
                }
                if was_paused_by_metered {
                    let mut settings = self.settings().await;
                    if !settings.user_resumed_after_metered {
                        settings.user_resumed_after_metered = true;
                        self.save_settings(settings).await?;
                    }
                }
            }
            "cancel" => {
                if let Some(token) = self.controls.lock().await.remove(id) {
                    token.cancel();
                }
                task.status = TaskStatus::Cancelled;
                task.speed = 0;
                task.eta_seconds = None;
                task.active_connections = 0;
                for segment in &mut task.segments {
                    if segment.status == "downloading" {
                        segment.status = "cancelled".into();
                    }
                }
            }
            "redownload" | "clear-shards" => {
                // User confirmed that the remote resource changed and wants to
                // discard the old shards and start over. Stop any active
                // download, clear shard files, and reset the task so the
                // scheduler picks it up as a fresh download.
                if let Some(token) = self.controls.lock().await.remove(id) {
                    token.cancel();
                }
                if task.task_kind == TaskKind::Bt {
                    // BT：丢弃 aria2 侧下载记录与控制文件；数据文件按重名
                    // 策略由 aria2 自动改名，不删除用户文件（§7）。
                    let _ = self.bt.remove_task(id).await;
                    task.bt_meta = task.bt_meta.take().map(|mut meta| {
                        meta.metadata_ready = false;
                        meta.display_name = None;
                        meta
                    });
                    if task.file_name.is_empty() {
                        task.category = "other".into();
                    }
                } else {
                    self.clear_parts(&task).await;
                }
                task.downloaded_bytes = 0;
                task.total_bytes = 0;
                task.segments.clear();
                task.etag = None;
                task.last_modified = None;
                task.checksum_sha256 = None;
                task.error = None;
                task.speed = 0;
                task.eta_seconds = None;
                task.active_connections = 0;
                task.retry_count = 0;
                task.completed_at = None;
                task.bt_runtime = None;
                task.status = TaskStatus::Queued;
            }
            _ => return Err("未知任务操作".into()),
        }
        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);
        self.dispatcher.notify_waiters();
        Ok(())
    }

    pub async fn bulk_action(
        self: &SharedManager,
        ids: &[String],
        action: &str,
    ) -> Result<(), String> {
        for id in ids {
            self.action(id, action).await?;
        }
        Ok(())
    }

    /// 新建 BT/磁力任务（`bt_task_add` 命令入口，roadmap BT-04/05）。
    ///
    /// 与 HTTP 任务的关键差异：
    /// - 磁力元数据获取前不写文件名/大小（§3 BT 约束，UI 显示"待获取"）；
    /// - 不做 HTTP 语义的输出路径预留：重名由 aria2 `auto-file-renaming=true`
    ///   处理（rename 策略，`allow-overwrite=false` 保证绝不静默覆盖，§7）；
    /// - 分类规则需要 URL 域名，磁力无域名，直接使用全局下载目录。
    pub async fn add_bt(&self, request: BtNewTaskRequest) -> Result<DownloadTask, String> {
        let source = request.source.trim().to_string();
        if source.is_empty() {
            return Err("请输入 magnet: 磁力链接或选择 .torrent 种子文件".into());
        }
        let settings = self.settings().await;
        let is_magnet = source.to_ascii_lowercase().starts_with("magnet:");
        // 拖放 .torrent：内容走 base64（source 仅作显示文件名），校验后随
        // 元数据持久化，暂停任务的后续恢复添加不依赖原文件是否仍在磁盘上。
        let inline_data = request
            .source_data_base64
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let (url, initial_name, meta) = if let Some(data) = inline_data {
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|_| "种子文件内容无效（base64 解码失败）".to_string())?;
            crate::bt::process::validate_torrent_bytes(&bytes)?;
            let stem = Path::new(&source)
                .file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_owned)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "种子任务".into());
            let meta = BtTaskMeta {
                info_hash: String::new(),
                selected_files: request.selected_files.clone(),
                display_name: None,
                metadata_ready: true,
                torrent_data_base64: Some(data.to_string()),
                streaming_priority: request.streaming_priority,
            };
            (source, stem, meta)
        } else if is_magnet {
            let info = crate::bt::magnet::parse_magnet(&source)?;
            let meta = BtTaskMeta {
                info_hash: info.info_hash,
                selected_files: request.selected_files.clone(),
                display_name: info.display_name.clone(),
                metadata_ready: false,
                torrent_data_base64: None,
                streaming_priority: request.streaming_priority,
            };
            // 磁力提示名仅作占位展示（UI 标注"待确认"），不得当作最终文件名。
            (source, info.display_name.unwrap_or_default(), meta)
        } else {
            let path = Path::new(&source);
            crate::bt::process::validate_torrent_file(path)?;
            let stem = path
                .file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_owned)
                .unwrap_or_else(|| "种子任务".into());
            let meta = BtTaskMeta {
                // .torrent 的 infohash 由 aria2 接受添加后回填（无 bencode 依赖）。
                info_hash: String::new(),
                selected_files: request.selected_files.clone(),
                display_name: None,
                metadata_ready: true,
                torrent_data_base64: None,
                streaming_priority: request.streaming_priority,
            };
            (source, stem, meta)
        };
        let destination = match request.destination.as_deref().map(str::trim) {
            Some(dir) if !dir.is_empty() => normalize_directory(dir),
            _ => normalize_directory(&settings.download_dir),
        };
        let display_name_for_file = safe_name(&initial_name);
        let task = DownloadTask {
            id: Uuid::new_v4().to_string(),
            url,
            file_name: display_name_for_file,
            destination,
            total_bytes: 0,
            downloaded_bytes: 0,
            speed: 0,
            eta_seconds: None,
            status: if request.start_paused {
                TaskStatus::Paused
            } else {
                TaskStatus::Queued
            },
            error: None,
            created_at: now(),
            completed_at: None,
            scheduled_at: None,
            category: category(&initial_name),
            queue_position: self.store.next_queue_position().await?,
            priority: 0,
            retry_count: 0,
            max_retries: settings.max_retries,
            checksum_sha256: None,
            expected_checksum: None,
            source: request.source_tag.unwrap_or_else(|| "desktop".into()),
            etag: None,
            last_modified: None,
            final_url: None,
            response_status: None,
            content_type: None,
            accepts_ranges: None,
            headers: HashMap::new(),
            media: None,
            per_task_speed_limit: 0,
            collision_policy: CollisionPolicy::Rename,
            completion_action: CompletionAction::None,
            connection_count: 1,
            active_connections: 0,
            segments: Vec::new(),
            retry_policy_override: None,
            proxy_override: None,
            proxy_auth: None,
            task_kind: TaskKind::Bt,
            bt_meta: Some(meta),
            bt_runtime: None,
            cloud_refresh: None,
        };
        self.store.upsert_task(&task).await?;
        self.register_power_action_target(&task.id).await;
        self.emit_task("created", &task);
        self.dispatcher.notify_waiters();
        Ok(task)
    }

    /// 程序退出路径：优雅关闭 aria2（保存会话，保证重启可恢复，§3 BT）。
    pub async fn shutdown_bt(&self) {
        self.bt.shutdown().await;
    }

    pub async fn remove(self: &SharedManager, id: &str, delete_file: bool) -> Result<(), String> {
        if let Some(token) = self.controls.lock().await.remove(id) {
            token.cancel();
        }

        // 等待正在运行的 worker 退出（最多等待 1 秒）
        let mut retries = 0;
        while self.task_runtime.read().await.contains_key(id) && retries < 20 {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            retries += 1;
        }

        if let Some(task) = self.store.get_task(id).await? {
            let is_completed = task.status == TaskStatus::Completed;
            if task.task_kind == TaskKind::Bt {
                let _ = self.bt.remove_task(id).await;
                if delete_file || !is_completed {
                    let _ = crate::bt::delete_task_files(&task).await;
                }
            } else if delete_file || !is_completed {
                let path = PathBuf::from(&task.destination).join(&task.file_name);
                let _ = fs::remove_file(&path).await;
                let temp_path = PathBuf::from(format!("{}.lumaget", path.to_string_lossy()));
                let _ = fs::remove_file(&temp_path).await;
                self.clear_parts(&task).await;
            }
        }

        self.store.remove_task(id).await?;
        let _ = self.app.emit("task-removed", id.to_string());
        Ok(())
    }

    /// 归档任务至历史记录（保留下载链接、目录与元数据，供日后查阅并一键重新下载）。
    pub async fn archive(self: &SharedManager, id: &str, delete_file: bool) -> Result<(), String> {
        if let Some(token) = self.controls.lock().await.remove(id) {
            token.cancel();
        }

        // 等待正在运行的 worker 退出（最多等待 1 秒）
        let mut retries = 0;
        while self.task_runtime.read().await.contains_key(id) && retries < 20 {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            retries += 1;
        }

        if let Some(mut task) = self.store.get_task(id).await? {
            let is_completed = task.status == TaskStatus::Completed;
            if task.task_kind == TaskKind::Bt {
                let _ = self.bt.remove_task(id).await;
                if delete_file || !is_completed {
                    let _ = crate::bt::delete_task_files(&task).await;
                }
            } else if delete_file || !is_completed {
                let path = PathBuf::from(&task.destination).join(&task.file_name);
                let _ = fs::remove_file(&path).await;
                let temp_path = PathBuf::from(format!("{}.lumaget", path.to_string_lossy()));
                let _ = fs::remove_file(&temp_path).await;
                self.clear_parts(&task).await;
            }

            task.status = TaskStatus::Cancelled;
            if task.completed_at.is_none() {
                task.completed_at = Some(now());
            }
            task.speed = 0;
            task.eta_seconds = None;
            task.active_connections = 0;
            self.store.upsert_task(&task).await?;
            self.emit_task("updated", &task);
            self.dispatcher.notify_waiters();
        }
        Ok(())
    }

    pub async fn reorder(&self, ids: &[String]) -> Result<(), String> {
        self.store.reorder(ids).await?;
        let _ = self.app.emit("queue-updated", ids);
        self.dispatcher.notify_waiters();
        Ok(())
    }

    pub async fn update_task_options(
        &self,
        id: &str,
        priority: Option<i32>,
        per_task_speed_limit: Option<u64>,
        completion_action: Option<CompletionAction>,
    ) -> Result<DownloadTask, String> {
        let Some(mut task) = self.store.get_task(id).await? else {
            return Err("任务不存在".into());
        };
        if let Some(priority) = priority {
            task.priority = priority.clamp(MIN_PRIORITY, MAX_PRIORITY);
        }
        if let Some(limit) = per_task_speed_limit {
            task.per_task_speed_limit = limit;
            if let Some(runtime) = self.task_runtime.read().await.get(id).cloned() {
                runtime.speed_limit.store(limit, Ordering::Relaxed);
            }
        }
        if let Some(action) = completion_action {
            if action == CompletionAction::RunFile && task.source != "desktop" {
                return Err("只有桌面端手动创建的任务可以设置完成后运行文件".into());
            }
            task.completion_action = action;
        }
        if let Some(runtime) = self.task_runtime.read().await.get(id).cloned() {
            runtime.priority.store(task.priority, Ordering::Relaxed);
            *runtime.completion_action.write().await = task.completion_action.clone();
        }
        self.bandwidth_scheduler.set_priority(id, task.priority);
        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);
        self.dispatcher.notify_waiters();
        Ok(task)
    }

    /// 更新任务级重试策略覆盖（Task 14）。
    ///
    /// - `policy = None`：清除覆盖，回退到全局默认 `default_retry_policy`。
    /// - `policy = Some(p)`：将 `p` 写入 `retry_policy_override`。
    ///
    /// 不影响 v1.1 的 ETag/磁盘空间检查（这些检查不参与重试）。
    /// 字段校验：`connection_timeout_secs >= 1`，`max_retries <= 32`，
    /// `initial_backoff_ms`/`max_backoff_ms` 不超过 1 小时。
    pub async fn update_retry_policy(
        &self,
        id: &str,
        policy: Option<RetryPolicy>,
    ) -> Result<DownloadTask, String> {
        if let Some(ref p) = policy {
            if p.connection_timeout_secs == 0 {
                return Err("连接超时必须大于 0 秒".into());
            }
            if p.max_retries > 32 {
                return Err("最大重试次数不能超过 32".into());
            }
            if p.initial_backoff_ms == 0 {
                return Err("初始退避时长必须大于 0 毫秒".into());
            }
            if p.max_backoff_ms < p.initial_backoff_ms {
                return Err("最大退避时长不能小于初始退避时长".into());
            }
            const ONE_HOUR_MS: u64 = 60 * 60 * 1000;
            if p.max_backoff_ms > ONE_HOUR_MS {
                return Err("最大退避时长不能超过 1 小时".into());
            }
            if let Some(timeout) = p.task_timeout_secs {
                if timeout == 0 {
                    return Err("任务总超时为 0 时应使用 null 表示不限制".into());
                }
            }
        }
        let Some(mut task) = self.store.get_task(id).await? else {
            return Err("任务不存在".into());
        };
        task.retry_policy_override = policy;
        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);
        self.dispatcher.notify_waiters();
        Ok(task)
    }

    /// Task 31.5：更新任务级代理覆盖与代理认证。
    ///
    /// - `proxy_override = None`：清除覆盖，回退到全局 `AppSettings.proxy_mode`/`proxy_url`。
    /// - `proxy_override = Some("")`：显式禁用代理（即使全局是 manual）。
    /// - `proxy_override = Some(url)`：使用指定代理 URL，必须通过 `validate_proxy_url` 校验。
    /// - `proxy_auth`：可选认证；密码非空时由 [`encrypt_password`] 加密为 DPAPI 密文后写入 DB。
    ///
    /// 任务必须存在。任务不存在时返回中文错误。更新后 emit `task-updated` 并唤醒调度器。
    pub async fn update_proxy(
        &self,
        id: &str,
        proxy_override: Option<String>,
        proxy_auth: Option<ProxyAuth>,
    ) -> Result<DownloadTask, String> {
        // 校验代理 URL 格式（空字符串允许，表示"显式禁用代理"）。
        if let Some(url) = proxy_override.as_deref() {
            if !url.is_empty() {
                crate::proxy::validate_proxy_url(url)?;
            }
        }
        // 加密代理密码：用户传入的是明文，落库前必须经 DPAPI 加密。
        // 用户名为空时整体视为无认证（与 proxy_auth = None 等价）。
        let encrypted_auth = match proxy_auth {
            Some(mut auth) => {
                if auth.username.trim().is_empty() {
                    None
                } else if auth.password.is_empty() {
                    // 用户名为空但密码非空：保留结构，密码为空字符串。
                    Some(auth)
                } else {
                    match encrypt_password(&auth.password) {
                        Ok(cipher) => {
                            auth.password = cipher;
                            Some(auth)
                        }
                        Err(reason) => return Err(reason),
                    }
                }
            }
            None => None,
        };
        let Some(mut task) = self.store.get_task(id).await? else {
            return Err("任务不存在".into());
        };
        task.proxy_override = proxy_override;
        task.proxy_auth = encrypted_auth;
        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);
        self.dispatcher.notify_waiters();
        Ok(task)
    }

    /// Task 21.2：重命名任务文件名。
    ///
    /// 仅 `Queued`（等待中）状态可重命名。其他状态返回 "任务已开始，无法重命名"。
    /// 校验文件名合法性（非空、无非法字符、无路径分隔符、长度 ≤ 255）后，
    /// 检查目标目录下是否已存在同名文件或 `.lumaget` 临时文件，存在则拒绝。
    /// 不修改磁盘上的文件——`Queued` 状态尚未创建任何分片或目标文件。
    pub async fn rename(&self, id: &str, new_filename: &str) -> Result<DownloadTask, String> {
        let trimmed = new_filename.trim();
        if let Err(reason) = validate_rename_filename(trimmed) {
            return Err(reason);
        }

        let Some(mut task) = self.store.get_task(id).await? else {
            return Err("任务不存在".into());
        };
        if task.status != TaskStatus::Queued {
            return Err("任务已开始，无法重命名".into());
        }
        // 同名检查（不区分大小写，匹配 Windows 文件系统语义）
        let target_path = PathBuf::from(&task.destination).join(trimmed);
        if target_path.exists() {
            return Err(format!("目标目录已存在同名文件：{trimmed}"));
        }
        let temp_path = PathBuf::from(format!("{}.lumaget", target_path.to_string_lossy()));
        if temp_path.exists() {
            return Err(format!("目标目录已存在同名临时文件：{trimmed}"));
        }
        // 同目录其他任务若已使用该文件名，也拒绝（避免两条 Queued 任务争抢同一目标路径）
        for other in self.store.list_tasks().await? {
            if other.id == task.id {
                continue;
            }
            if other.destination == task.destination
                && other.file_name.eq_ignore_ascii_case(trimmed)
            {
                return Err(format!("另一任务已使用该文件名：{trimmed}"));
            }
        }

        task.file_name = trimmed.to_string();
        task.category = category(&task.file_name);
        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);
        self.dispatcher.notify_waiters();
        Ok(task)
    }

    /// 刷新/更新任务的下载链接（支持过期临时直链无缝刷新续传）。
    ///
    /// 保留已下载分片（.lumaget），更新 URL、可选请求头，清空错误，
    /// 重置状态为 `Paused`。
    pub async fn refresh_url(
        &self,
        id: &str,
        new_url: &str,
        headers: Option<HashMap<String, String>>,
    ) -> Result<DownloadTask, String> {
        let trimmed_url = new_url.trim();
        if trimmed_url.is_empty() {
            return Err("下载地址不能为空".into());
        }
        if !trimmed_url.starts_with("http://")
            && !trimmed_url.starts_with("https://")
            && !trimmed_url.starts_with("magnet:?")
        {
            return Err("仅支持 http://、https:// 或 magnet:? 链接".into());
        }

        let Some(mut task) = self.store.get_task(id).await? else {
            return Err("任务不存在".into());
        };

        if matches!(task.status, TaskStatus::Downloading | TaskStatus::Verifying) {
            return Err("任务正在下载或校验中，请先暂停任务后再刷新链接".into());
        }

        task.url = trimmed_url.to_string();
        if let Some(h) = headers {
            task.headers = h;
        }
        // 清理历史错误，重置为暂停状态以待用户恢复
        task.error = None;
        task.status = TaskStatus::Paused;
        task.speed = 0;
        task.active_connections = 0;
        task.eta_seconds = None;

        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);
        self.dispatcher.notify_waiters();
        Ok(task)
    }

    /// 云盘直链失效后的自动刷新（PikPak 等，2026-08-21）。
    ///
    /// 使用任务携带的 `cloud_refresh` 元数据重新解析直链并更新 URL/请求头。
    /// 直链由同一 `file_id` 重新解析，指向同一文件内容（ETag 为内容 MD5，
    /// 续传校验自然通过），已下载分片可安全续接。
    ///
    /// - `Ok(true)`：刷新成功，`task.url` 与请求头已更新。
    /// - `Ok(false)`：任务无刷新元数据或平台暂不支持自动刷新。
    /// - `Err(e)`：刷新尝试失败（分享失效、网络错误等）。
    async fn refresh_cloud_direct_link(
        &self,
        task: &mut DownloadTask,
    ) -> Result<bool, String> {
        let Some(meta) = task.cloud_refresh.clone() else {
            return Ok(false);
        };
        match meta.platform.as_str() {
            "pikpak" => {
                // 沿用首次解析的设备指纹；缺失时生成并固化，后续刷新保持一致
                let device_id = meta
                    .device_id
                    .clone()
                    .unwrap_or_else(|| hex::encode(rand::random::<[u8; 16]>()));
                let direct = crate::pikpak::resolve_pikpak_file(
                    &meta.share_id,
                    &meta.file_id,
                    meta.pass_code_token.as_deref(),
                    &device_id,
                )
                .await
                .map_err(|e| format!("PikPak 直链刷新失败：{e}"))?;
                if direct.url.trim().is_empty() {
                    return Err("PikPak 返回了空的下载直链".into());
                }
                task.url = direct.url;
                for (name, value) in direct.headers {
                    task.headers.insert(name, value);
                }
                task.cloud_refresh = Some(crate::models::CloudRefreshMeta {
                    device_id: Some(device_id),
                    ..meta
                });
                tracing::info!(
                    task_id = %task.id,
                    "云盘直链已自动刷新（pikpak），继续续传"
                );
                Ok(true)
            }
            "pikpak-direct" => {
                // 裸直链无法通过 API 自动刷新——缺少 share_id。
                // 检查直链过期时间戳给出精准诊断。
                let expire = crate::pikpak::parse_pikpak_direct_link_meta(&task.url)
                    .map(|m| m.expire)
                    .unwrap_or(0);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                if expire > 0 && now > expire {
                    Err("PikPak 直链已过期（超过有效期）。请通过 PikPak 分享链接重新创建任务，分享任务支持直链自动刷新续传".into())
                } else {
                    Err("PikPak 裸直链已达到单链接流量配额限制（约 330MB）。请通过 PikPak 分享链接（mypikpak.com/s/xxx）重新创建任务，分享任务支持直链自动刷新续传".into())
                }
            }
            other => Err(format!("平台 {other} 暂不支持直链自动刷新")),
        }
    }

    /// 从订阅源拉取并更新 BT Trackers 列表。
    pub async fn fetch_and_update_trackers(&self, custom_url: Option<&str>) -> Result<usize, String> {
        let settings = self.settings().await;
        let url = custom_url
            .unwrap_or(settings.bt_tracker_subscribe_url.as_str())
            .trim();
        if url.is_empty() {
            return Err("Tracker 订阅地址不能为空".into());
        }

        let client = self.client.read().await.clone();
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("拉取 Tracker 订阅失败：{e}"))?;

        if !response.status().is_success() {
            return Err(format!("订阅服务器返回 HTTP {}", response.status()));
        }

        let text = response
            .text()
            .await
            .map_err(|e| format!("读取 Tracker 列表失败：{e}"))?;

        let mut trackers_set = HashSet::new();
        // 保留现有有效 trackers
        for line in settings.bt_extra_trackers.lines() {
            let t = line.trim();
            if !t.is_empty() && (t.starts_with("http://") || t.starts_with("https://") || t.starts_with("udp://") || t.starts_with("ws://") || t.starts_with("wss://")) {
                trackers_set.insert(t.to_string());
            }
        }

        for line in text.lines() {
            let t = line.trim();
            if !t.is_empty() && (t.starts_with("http://") || t.starts_with("https://") || t.starts_with("udp://") || t.starts_with("ws://") || t.starts_with("wss://")) {
                trackers_set.insert(t.to_string());
            }
        }

        let mut updated_settings = settings;
        let mut sorted_trackers: Vec<String> = trackers_set.into_iter().collect();
        sorted_trackers.sort();
        updated_settings.bt_extra_trackers = sorted_trackers.join("\n");
        let total_count = sorted_trackers.len();

        self.save_settings(updated_settings).await?;
        Ok(total_count)
    }

    pub async fn verify_checksum(&self, id: &str) -> Result<String, String> {
        let Some(mut task) = self.store.get_task(id).await? else {
            return Err("任务不存在".into());
        };
        let path = PathBuf::from(&task.destination).join(&task.file_name);
        task.status = TaskStatus::Verifying;
        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);
        // 支持按长度识别 MD5(32) / SHA-1(40) / SHA-256(64)。无法识别的校验值
        // 直接判失败并显式报错——影响文件完整性判断的错误不得静默跳过（AGENTS.md §7）。
        if let Some(expected) = task.expected_checksum.as_deref() {
            if parse_expected_checksum(expected).is_none() {
                task.status = TaskStatus::Failed;
                task.error = Some(format!(
                    "校验值格式无法识别（支持 MD5/SHA-1/SHA-256 十六进制，长度 32/40/64 位）：{expected}"
                ));
                self.store.upsert_task(&task).await?;
                self.emit_task("updated", &task);
                return Err(task.error.clone().unwrap_or_default());
            }
        }
        let (algorithm, expected_clean) = task
            .expected_checksum
            .as_deref()
            .and_then(parse_expected_checksum)
            .unwrap_or((ChecksumAlgorithm::Sha256, String::new()));
        let hash = digest_file(&path, algorithm).await?;
        task.checksum_sha256 = Some(hash.clone());
        if task.expected_checksum.is_some() {
            if !expected_clean.eq_ignore_ascii_case(&hash) {
                task.status = TaskStatus::Failed;
                task.error = Some(format!("{} 校验不一致", algorithm.label()));
            } else {
                task.status = TaskStatus::Completed;
                task.error = None;
            }
        } else {
            task.status = TaskStatus::Completed;
        }
        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);
        Ok(hash)
    }

    /// Task 32.2：计量网络下自动暂停所有 Downloading 任务。
    ///
    /// 调用方（`lib.rs::setup` 中的定时检查）必须先调用
    /// `crate::network_awareness::should_pause_for_metered` 判定是否应暂停，
    /// 满足条件时再调用本方法。本方法本身不做条件判定，便于测试与复用。
    ///
    /// 行为：
    /// - 遍历全部任务，将 `Downloading` 状态的任务置为 `PausedByMetered`，
    ///   取消活动连接、保留分片、清零速度与活动连接数。
    /// - 不暂停用户手动启动的 `Queued` / `Scheduled` 任务（仅暂停正在下载的）。
    ///   设计理由：用户主动操作应尊重；自动调度在下次进入非计量网络时由调度器恢复。
    /// - 通过 `task-updated` 事件通知前端，并通过返回值告知暂停的任务数，
    ///   调用方据此发 `metered-network-detected` 事件展示 toast。
    /// - 重复调用幂等：已是 `PausedByMetered` 的任务不会被再次处理。
    pub async fn pause_tasks_for_metered_network(self: &SharedManager) -> Result<usize, String> {
        let tasks = self.store.list_tasks().await?;
        let mut paused_count = 0usize;
        for mut task in tasks {
            if task.status != TaskStatus::Downloading {
                continue;
            }
            // 取消活动连接：与 action("pause") 行为一致。
            if let Some(token) = self.controls.lock().await.remove(&task.id) {
                token.cancel();
            }
            task.status = TaskStatus::PausedByMetered;
            task.speed = 0;
            task.eta_seconds = None;
            task.active_connections = 0;
            for segment in &mut task.segments {
                if segment.status == "downloading" {
                    segment.status = "paused".into();
                }
            }
            self.store.upsert_task(&task).await?;
            self.emit_task("updated", &task);
            paused_count += 1;
        }
        if paused_count > 0 {
            self.dispatcher.notify_waiters();
        }
        Ok(paused_count)
    }

    /// Task 32.2：网络从计量变为非计量时清零 `user_resumed_after_metered` 标记。
    ///
    /// 调用方（`lib.rs::setup` 中的定时检查）在网络状态从计量切换为非计量时调用，
    /// 确保下次再进入计量网络时仍能触发自动暂停。
    /// 仅在标记确实为 true 时写入数据库，避免无谓写盘。
    pub async fn clear_user_resumed_after_metered(&self) -> Result<(), String> {
        let mut settings = self.settings().await;
        if !settings.user_resumed_after_metered {
            return Ok(());
        }
        settings.user_resumed_after_metered = false;
        self.save_settings(settings).await?;
        Ok(())
    }

    async fn recover_interrupted(&self) -> Result<(), String> {
        for mut task in self.store.list_tasks().await? {
            if matches!(
                task.status,
                TaskStatus::Downloading | TaskStatus::Verifying | TaskStatus::WaitingNetwork
            ) {
                task.status = TaskStatus::Queued;
                task.speed = 0;
                task.eta_seconds = None;
                task.active_connections = 0;
                for segment in &mut task.segments {
                    if segment.status == "downloading" {
                        segment.status = "paused".into();
                    }
                }
                self.store.upsert_task(&task).await?;
            }
        }
        Ok(())
    }

    /// Runs the startup selfcheck and emits `startup-selfcheck` to the front end.
    ///
    /// The selfcheck is best-effort: any internal failure is logged via the
    /// returned report but never propagates, so a corrupted database or
    /// missing shard file cannot block application startup.
    pub async fn run_startup_selfcheck(&self) -> Result<SelfcheckReport, String> {
        let report = execute_selfcheck(&self.store).await;
        let _ = self.app.emit("startup-selfcheck", report.clone());
        Ok(report)
    }

    /// 队列调度可观察性（Task 15）：解释指定任务为什么还在等待。
    ///
    /// 这是只读操作，不修改任何状态。读取任务状态、并发槽位使用情况、
    /// 队列位置和媒体工具安装状态，返回结构化的等待原因。
    ///
    /// - `Downloading/Completed/Failed/Cancelled/Verifying/WaitingNetwork` → `NotWaiting`
    /// - `Queued` → 依次检查媒体工具、并发槽位、队列前面任务数
    /// - `Paused` → `Paused`
    /// - `PausedByLowDisk` → `PausedByLowDisk`
    /// - `Interrupted` → `Interrupted`
    /// - `RemoteChanged` → `RemoteChanged`
    /// - `Scheduled` → `WaitingScheduledTime { scheduled_at }`
    pub async fn explain_wait_reason(&self, task_id: &str) -> Result<WaitReason, String> {
        let task = self
            .store
            .get_task(task_id)
            .await?
            .ok_or_else(|| "任务不存在".to_string())?;

        // Only Queued tasks need the full picture; for other statuses we can
        // compute the reason from the task alone.
        if !matches!(task.status, TaskStatus::Queued) {
            return Ok(compute_wait_reason(&task, &[], 0, 0, true, true));
        }

        let settings = self.settings().await;
        let max_concurrent = effective_concurrent_downloads(&settings);
        let active_count = self.controls.lock().await.len();
        let all_tasks = self.store.list_tasks().await?;

        let yt_dlp_available = crate::media_tools::resolve_yt_dlp(&self.app, &settings).is_some();
        let ffmpeg_available = crate::media_tools::resolve_ffmpeg(&self.app, &settings).is_some();

        Ok(compute_wait_reason(
            &task,
            &all_tasks,
            active_count,
            max_concurrent,
            yt_dlp_available,
            ffmpeg_available,
        ))
    }

    async fn scheduler_loop(self: SharedManager) {
        loop {
            let _ = self.dispatch_once().await;
            let _ = self.evaluate_power_action().await;
            tokio::select! {
                _ = self.dispatcher.notified() => {},
                _ = tokio::time::sleep(Duration::from_millis(500)) => {},
            }
        }
    }

    async fn evaluate_power_action(&self) -> Result<(), String> {
        let (phase, target_ids) = {
            let runtime = self.power_action.lock().await;
            (runtime.state.phase, runtime.target_ids.clone())
        };
        if phase == PowerActionPhase::Idle || target_ids.is_empty() {
            return Ok(());
        }
        let tasks = self.store.list_tasks().await?;
        let statuses: HashMap<_, _> = tasks
            .into_iter()
            .filter(|task| target_ids.contains(&task.id))
            .map(|task| (task.id, task.status))
            .collect();
        let decision = power_action_decision(&target_ids, &statuses);
        let current = now();
        let mut execute = None;
        let mut runtime = self.power_action.lock().await;
        let previous = runtime.state.clone();
        match decision {
            PowerActionDecision::Waiting => {
                runtime.countdown_deadline = None;
                runtime.state.phase = PowerActionPhase::Armed;
                runtime.state.remaining_seconds = 0;
                runtime.state.message = Some("等待队列中的任务全部完成".into());
            }
            PowerActionDecision::Blocked(message) => {
                runtime.countdown_deadline = None;
                runtime.state.phase = PowerActionPhase::Blocked;
                runtime.state.remaining_seconds = 0;
                runtime.state.message = Some(message);
            }
            PowerActionDecision::Complete => {
                let deadline = *runtime
                    .countdown_deadline
                    .get_or_insert(current.saturating_add(POWER_ACTION_COUNTDOWN_MILLIS));
                if current >= deadline {
                    execute = Some(runtime.state.action);
                    *runtime = PowerActionRuntime::default();
                } else {
                    runtime.state.phase = PowerActionPhase::Countdown;
                    runtime.state.remaining_seconds =
                        power_action_remaining_seconds(deadline, current);
                    runtime.state.message = Some("所有目标任务均已完成，可随时取消".into());
                }
            }
        }
        runtime.state.target_count = runtime.target_ids.len();
        let state = runtime.state.clone();
        drop(runtime);
        if state != previous {
            self.emit_power_action_state(&state);
        }
        if let Some(action) = execute {
            self.emit_power_action_state(&PowerActionState::default());
            if let Err(error) = execute_power_action(action) {
                let failed = PowerActionState {
                    action,
                    phase: PowerActionPhase::Blocked,
                    remaining_seconds: 0,
                    target_count: 0,
                    message: Some(format!("系统操作执行失败：{error}")),
                };
                self.power_action.lock().await.state = failed.clone();
                self.emit_power_action_state(&failed);
            }
        }
        Ok(())
    }

    async fn dispatch_once(self: &SharedManager) -> Result<(), String> {
        let settings = self.settings().await;
        let concurrent_downloads = effective_concurrent_downloads(&settings);
        let active = self.controls.lock().await.len();
        if active >= concurrent_downloads {
            return Ok(());
        }
        let current = now();
        let mut candidates: Vec<_> = self
            .store
            .list_tasks()
            .await?
            .into_iter()
            .filter(|task| {
                task.status == TaskStatus::Queued
                    || (task.status == TaskStatus::Scheduled
                        && task.scheduled_at.is_some_and(|t| t <= current))
            })
            .collect();
        sort_download_candidates(&mut candidates);
        for task in candidates.into_iter().take(concurrent_downloads - active) {
            self.spawn_worker(task).await;
        }
        Ok(())
    }

    async fn spawn_worker(self: &SharedManager, mut task: DownloadTask) {
        if let Err((available, required)) =
            check_disk_space_once(&task.destination, task.total_bytes, task.downloaded_bytes)
        {
            task.status = TaskStatus::PausedByLowDisk;
            task.speed = 0;
            task.eta_seconds = None;
            task.active_connections = 0;
            task.error = Some(format!(
                "磁盘空间不足（可用 {} 字节，需要 {} 字节），已自动暂停",
                available, required
            ));
            let _ = self.store.upsert_task(&task).await;
            self.emit_task("updated", &task);
            let _ = self.app.emit(
                "merge-blocked-by-low-disk",
                LowDiskPayload {
                    task_id: task.id.clone(),
                    available_bytes: available,
                    required_bytes: required,
                },
            );
            return;
        }

        let mut controls = self.controls.lock().await;
        if controls.contains_key(&task.id) {
            return;
        }
        let mut token = CancellationToken::new();
        controls.insert(task.id.clone(), token.clone());
        drop(controls);
        self.task_runtime
            .write()
            .await
            .insert(task.id.clone(), Arc::new(RuntimeTaskOptions::new(&task)));
        task.status = TaskStatus::Downloading;
        task.error = None;
        // 媒体任务标记为"正在解析/下载中"：设置 active_connections = 1 避免前端
        // 在 media::download 设置真实连接数之前因 active_connections=0 误显示"解析中"。
        // 普通下载任务会在 HTTP Range 路径中被覆盖为真实连接数。
        // 注意：用户直接提交媒体 URL（不点"分析媒体"）时 task.media 可能为 None，
        // 需要同时检查 URL 是否属于已知媒体平台（抖音/YouTube/TikTok 等）。
        // BT 任务同理：元数据获取阶段无真实连接数，占用 1 个槽位表示活动。
        if task.media.is_some()
            || crate::media_platforms::detect_platform(&task.url)
                != crate::media_platforms::MediaPlatform::Unknown
            || task.task_kind == TaskKind::Bt
        {
            task.active_connections = 1;
        }
        let _ = self.store.upsert_task(&task).await;
        self.emit_task("updated", &task);
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            let id = task.id.clone();
            let mut attempt = task.retry_count;
            // 云盘直链自动刷新计数：链接失效哨兵触发时刷新直链并续传，
            // 超过 MAX_LINK_REFRESHES 后进入终态失败（防止分享失效时无限刷新）。
            let mut link_refreshes = 0u32;
            // Task 14: 任务总超时起点。任务总超时优先于连接重试，
            // 即使未达 max_retries，超过 task_timeout_secs 也强制失败。
            let worker_start = Instant::now();
            loop {
                // Task 14: 在每次循环开始检查任务总超时。
                // 不影响 v1.1 的 ETag/磁盘空间检查（这些通过专用前缀分支处理，不进入重试）。
                let settings_snapshot = manager.settings().await;
                let policy = effective_retry_policy(&task, &settings_snapshot);
                if let Some(timeout_secs) = policy.task_timeout_secs {
                    if timeout_secs > 0
                        && worker_start.elapsed() >= Duration::from_secs(timeout_secs)
                    {
                        if let Ok(Some(current)) = manager.store.get_task(&id).await {
                            task = current;
                        }
                        task.status = TaskStatus::Failed;
                        task.error =
                            Some(format!("任务总超时（{} 秒）已超过，强制失败", timeout_secs));
                        task.speed = 0;
                        task.eta_seconds = None;
                        task.retry_count = attempt;
                        task.active_connections = 0;
                        for segment in &mut task.segments {
                            if segment.status == "downloading" {
                                segment.status = "failed".into();
                            }
                        }
                        let _ = manager.store.upsert_task(&task).await;
                        manager.emit_task("updated", &task);
                        break;
                    }
                }
                // BT 任务走 aria2 引擎（轮询真实状态），HTTP/媒体任务走下载内核。
                // 返回语义一致：Ok = 完成；Err 前缀决定重试或终态。
                let result = if task.task_kind == TaskKind::Bt {
                    crate::bt::run_task(&manager, task.clone(), token.clone()).await
                } else {
                    manager.download_once(task.clone(), token.clone()).await
                };
                // 云盘直链失效优先于暂停判定：download_segments 收尾时为尽快
                // 停止其他 worker 已取消 token，这里必须先识别哨兵再决定是否 break。
                let cloud_link_dead =
                    matches!(&result, Err(e) if e.starts_with(CLOUD_LINK_DEAD_PREFIX));
                if token.is_cancelled() && !cloud_link_dead {
                    break;
                }
                match result {
                    Ok(mut finished) => {
                        finished.status = TaskStatus::Completed;
                        finished.completed_at = Some(now());
                        finished.speed = 0;
                        finished.eta_seconds = Some(0);
                        finished.active_connections = 0;
                        for segment in &mut finished.segments {
                            segment.status = "completed".into();
                        }
                        // Task 45.4：下载完成后清空 Cookie/Referer/User-Agent 头，
                        // 避免临时登录态被持久化到数据库（AGENTS.md §3、§5）。
                        // 这些头仅在下载过程中通过临时 cookie 文件传递给 yt-dlp，
                        // 完成后必须从 task.headers 中移除。
                        clear_auth_headers(&mut finished.headers);
                        let settings = manager.settings().await;
                        // BT 完成判定即 aria2 分片哈希校验通过（§3 BT），不再
                        // 对多文件种子目录做 HTTP 语义的整文件校验。
                        let needs_file_verify = finished.task_kind == TaskKind::Http
                            && (settings.verify_after_download
                                || finished.expected_checksum.is_some());
                        if needs_file_verify {
                            let _ = manager.store.upsert_task(&finished).await;
                            let _ = manager.verify_checksum(&id).await;
                        } else {
                            let _ = manager.store.upsert_task(&finished).await;
                            manager.emit_task("updated", &finished);
                        }
                        if let Ok(Some(completed)) = manager.store.get_task(&id).await {
                            if completed.status == TaskStatus::Completed {
                                manager.notify_download_completed(&completed).await;
                                manager.perform_completion_action(completed).await;
                            }
                        }
                        break;
                    }
                    Err(error) if error.starts_with("MEDIA_PROBE_ERROR:") => {
                        let clean_err = error.strip_prefix("MEDIA_PROBE_ERROR:").unwrap_or(&error).to_string();
                        if let Ok(Some(current)) = manager.store.get_task(&id).await {
                            task = current;
                        }
                        task.status = TaskStatus::Failed;
                        task.error = Some(clean_err);
                        task.speed = 0;
                        task.eta_seconds = None;
                        task.retry_count = attempt;
                        task.active_connections = 0;
                        for segment in &mut task.segments {
                            if segment.status == "downloading" {
                                segment.status = "failed".into();
                            }
                        }
                        let _ = manager.store.upsert_task(&task).await;
                        manager.emit_task("updated", &task);
                        manager.notify_download_failed(&task).await;
                        break;
                    }
                    Err(error) if error.starts_with(CLOUD_LINK_DEAD_PREFIX) => {
                        // 云盘直链失效（连续空响应/长时间停滞）：用任务携带的
                        // cloud_refresh 元数据自动重新解析直链，刷新成功则
                        // 重建取消令牌并无缝续传（分片保留、进度不回退）。
                        if link_refreshes < MAX_LINK_REFRESHES {
                            match manager.refresh_cloud_direct_link(&mut task).await {
                                Ok(true) => {
                                    link_refreshes += 1;
                                    // 旧 token 已在 download_segments 收尾时取消，
                                    // 必须重建，否则续传会立即以"任务已暂停"失败。
                                    token = CancellationToken::new();
                                    manager.controls.lock().await.insert(id.clone(), token.clone());
                                    task.status = TaskStatus::Downloading;
                                    task.error = Some(format!(
                                        "下载直链已过期，已自动刷新（第 {} 次），正在续传",
                                        link_refreshes
                                    ));
                                    task.speed = 0;
                                    task.active_connections = 0;
                                    let _ = manager.store.upsert_task(&task).await;
                                    manager.emit_task("updated", &task);
                                    // 不消耗 attempt：直链过期不是任务本身失败
                                }
                                Ok(false) => {
                                    // 无刷新元数据或平台不支持：按普通错误进入
                                    // 终态失败，给出可操作提示。PikPak 直链存在
                                    // 单链接流量配额（实测约 330MB，超出后有效
                                    // Range 也会返回 416），直接粘贴的直链无法
                                    // 自动续期，必须通过分享链接重建任务。
                                    if let Ok(Some(current)) = manager.store.get_task(&id).await {
                                        task = current;
                                    }
                                    task.status = TaskStatus::Failed;
                                    task.error = Some(format!(
                                        "{}。该直链已达到单链接流量配额且缺少自动刷新信息，请通过 PikPak 分享链接重新创建任务（分享任务支持直链自动刷新续传）",
                                        error.strip_prefix(CLOUD_LINK_DEAD_PREFIX).unwrap_or(&error)
                                    ));
                                    task.speed = 0;
                                    task.eta_seconds = None;
                                    task.active_connections = 0;
                                    let _ = manager.store.upsert_task(&task).await;
                                    manager.emit_task("updated", &task);
                                    manager.notify_download_failed(&task).await;
                                    break;
                                }
                                Err(refresh_error) => {
                                    // 刷新尝试失败（分享失效/网络错误）：终态失败
                                    if let Ok(Some(current)) = manager.store.get_task(&id).await {
                                        task = current;
                                    }
                                    task.status = TaskStatus::Failed;
                                    task.error = Some(format!(
                                        "直链已失效，自动刷新失败：{refresh_error}"
                                    ));
                                    task.speed = 0;
                                    task.eta_seconds = None;
                                    task.active_connections = 0;
                                    let _ = manager.store.upsert_task(&task).await;
                                    manager.emit_task("updated", &task);
                                    manager.notify_download_failed(&task).await;
                                    break;
                                }
                            }
                        } else {
                            // 刷新次数耗尽：分享本身大概率已失效
                            if let Ok(Some(current)) = manager.store.get_task(&id).await {
                                task = current;
                            }
                            task.status = TaskStatus::Failed;
                            task.error = Some(format!(
                                "直链已失效且自动刷新已达上限（{} 次），请重新解析分享链接",
                                MAX_LINK_REFRESHES
                            ));
                            task.speed = 0;
                            task.eta_seconds = None;
                            task.active_connections = 0;
                            let _ = manager.store.upsert_task(&task).await;
                            manager.emit_task("updated", &task);
                            manager.notify_download_failed(&task).await;
                            break;
                        }
                    }
                    Err(error) if error.starts_with(REMOTE_CHANGED_PREFIX) => {
                        // download_once already marked the task RemoteChanged
                        // and persisted it. Do not retry — the user must
                        // explicitly choose to redownload or cancel.
                        break;
                    }
                    Err(error) if error.starts_with(LOW_DISK_PREFIX) => {
                        // download_once / download_segments / download_stream
                        // already cancelled all active connections, preserved
                        // the downloaded shards, marked the task PausedByLowDisk
                        // and persisted. Do not retry — the user must free
                        // space or change the destination before resuming.
                        // Break without overriding the status.
                        break;
                    }
                    Err(error) if error.starts_with(crate::bt::BT_TERMINAL_PREFIX) => {
                        // BT 终态失败（组件缺失/进程死亡/aria2 error）：
                        // 任务状态已由引擎落库为 Failed，此处不重试、不覆盖状态。
                        break;
                    }
                    Err(error) if is_network_error(&error) => {
                        if let Ok(Some(current)) = manager.store.get_task(&id).await {
                            task = current;
                        }
                        task.status = TaskStatus::WaitingNetwork;
                        task.error = Some("网络不可用，恢复连接后将自动续传".into());
                        task.speed = 0;
                        task.eta_seconds = None;
                        task.active_connections = 0;
                        let _ = manager.store.upsert_task(&task).await;
                        manager.emit_task("updated", &task);
                        if !manager.wait_for_network(&task, token.clone()).await {
                            break;
                        }
                        task.status = TaskStatus::Downloading;
                        task.error = None;
                        if task.media.is_some()
                            || crate::media_platforms::detect_platform(&task.url)
                                != crate::media_platforms::MediaPlatform::Unknown
                        {
                            task.active_connections = 1;
                        }
                        let _ = manager.store.upsert_task(&task).await;
                        manager.emit_task("updated", &task);
                    }
                    Err(error) if attempt < policy.max_retries => {
                        if let Ok(Some(current)) = manager.store.get_task(&id).await {
                            task = current;
                        }
                        attempt += 1;
                        task.retry_count = attempt;
                        task.active_connections = 0;
                        if task.media.is_some() {
                            task.media = None;
                        }
                        task.error = Some(format!("{}，将在稍后重试", error));
                        let _ = manager.store.upsert_task(&task).await;
                        manager.emit_task("updated", &task);
                        // Task 14: 使用 effective_retry_policy 的退避策略。
                        // 退避期间连接停止活动（不占用 server 资源）。
                        let backoff_ms = compute_backoff(&policy, attempt);
                        let wait_secs = backoff_ms / 1000;
                        let capped_secs = wait_secs.min(60);
                        tokio::select! { _ = token.cancelled() => break, _ = tokio::time::sleep(Duration::from_secs(capped_secs)) => {} }
                    }
                    Err(error) => {
                        if let Ok(Some(current)) = manager.store.get_task(&id).await {
                            task = current;
                        }
                        task.status = TaskStatus::Failed;
                        task.error = Some(error);
                        task.speed = 0;
                        task.eta_seconds = None;
                        task.retry_count = attempt;
                        task.active_connections = 0;
                        for segment in &mut task.segments {
                            if segment.status == "downloading" {
                                segment.status = "failed".into();
                            }
                        }
                        let _ = manager.store.upsert_task(&task).await;
                        manager.emit_task("updated", &task);
                        // Task 30.2：进入 Failed 终态时发送失败通知与 `task-notification` 事件。
                        manager.notify_download_failed(&task).await;
                        break;
                    }
                }
            }
            manager.controls.lock().await.remove(&id);
            manager.task_runtime.write().await.remove(&id);
            manager.dispatcher.notify_waiters();
        });
    }

    async fn resolve_cloud_share_task(
        self: &SharedManager,
        task: &mut DownloadTask,
    ) -> Result<(), String> {
        let raw_url = task.url.trim().to_string();
        if raw_url.contains("pan.baidu.com/s/") || raw_url.contains("pan.baidu.com/share/init") {
            let cookie = if let Some(c) = task.headers.get("Cookie").filter(|s| !s.trim().is_empty()) {
                Some(c.clone())
            } else if let Ok(Some(cred)) = self.store.media_credential_get_matching("pan.baidu.com").await {
                Some(cred.cookie)
            } else {
                None
            };

            let pwd = if let Ok(parsed) = url::Url::parse(&raw_url) {
                parsed.query_pairs().find(|(k, _)| k == "pwd").map(|(_, v)| v.to_string())
            } else {
                None
            };

            let share_info = crate::baidupan::inspect_baidu_share(&raw_url, pwd.as_deref(), cookie.as_deref())
                .await
                .map_err(|e| format!("解析百度网盘分享失败：{}", e))?;

            let file_items: Vec<_> = share_info.files.into_iter().filter(|f| f.kind == "drive#file").collect();
            if file_items.is_empty() {
                return Err("百度网盘分享中未找到可下载的文件".into());
            }

            let first_file = &file_items[0];
            let dlink_res = crate::baidupan::resolve_baidu_file(
                &share_info.surl,
                &first_file.id,
                share_info.share_id.as_deref(),
                share_info.uk.as_deref(),
                share_info.sign.as_deref(),
                share_info.timestamp,
                share_info.seckey.as_deref(),
                share_info.randsk.as_deref(),
                cookie.as_deref(),
            )
            .await
            .map_err(|e| format!("获取百度网盘直链失败：{}", e))?;

            task.url = dlink_res.url;
            if !first_file.name.is_empty() {
                task.file_name = first_file.name.clone();
            }
            task.total_bytes = first_file.size;
            task.category = category(&task.file_name);
            for (k, v) in dlink_res.headers {
                task.headers.insert(k, v);
            }
            if let Some(c) = cookie.clone() {
                task.headers.insert("Cookie".to_string(), c);
            }

            // 若包含多个文件，自动把其余文件添加进下载队列
            for file in file_items.iter().skip(1) {
                let file_id = file.id.clone();
                let surl = share_info.surl.clone();
                let sid = share_info.share_id.clone();
                let uk = share_info.uk.clone();
                let sign = share_info.sign.clone();
                let ts = share_info.timestamp;
                let seckey = share_info.seckey.clone();
                let randsk = share_info.randsk.clone();
                let c_opt = cookie.clone();
                let sub_name = file.name.clone();
                let destination = task.destination.clone();
                let collision_policy = task.collision_policy.clone();
                let completion_action = task.completion_action.clone();
                let connection_count = task.connection_count;
                let sub_manager = self.clone();

                tokio::spawn(async move {
                    if let Ok(sub_dlink) = crate::baidupan::resolve_baidu_file(
                        &surl,
                        &file_id,
                        sid.as_deref(),
                        uk.as_deref(),
                        sign.as_deref(),
                        ts,
                        seckey.as_deref(),
                        randsk.as_deref(),
                        c_opt.as_deref(),
                    ).await {
                        let mut sub_headers = sub_dlink.headers;
                        if let Some(c) = c_opt {
                            sub_headers.insert("Cookie".to_string(), c);
                        }
                        let req = crate::models::NewTaskRequest {
                            url: sub_dlink.url,
                            file_name: Some(sub_name),
                            destination: Some(destination),
                            headers: sub_headers,
                            scheduled_at: None,
                            priority: 0,
                            expected_checksum: None,
                            source: Some("baidu".to_string()),
                            per_task_speed_limit: 0,
                            collision_policy,
                            completion_action,
                            media: None,
                            connection_count: Some(connection_count.clamp(1, 16)),
                            start_paused: false,
                            user_edited_file_name: true,
                            cloud_refresh: None,
                        };
                        let _ = sub_manager.add(req).await;
                    }
                });
            }

            let _ = self.store.upsert_task(task).await;
            self.emit_task("updated", task);
        }

        if crate::lanzou::parse_lanzou_url(&raw_url).is_some() {
            let pwd = if let Ok(parsed) = url::Url::parse(&raw_url) {
                parsed.query_pairs().find(|(k, _)| k == "pwd" || k == "p" || k == "passcode").map(|(_, v)| v.to_string())
            } else {
                None
            };

            let share_info = crate::lanzou::inspect_lanzou_share(&raw_url, pwd.as_deref())
                .await
                .map_err(|e| format!("解析蓝奏云分享失败：{}", e))?;

            if share_info.files.is_empty() {
                return Err("蓝奏云分享中未找到可下载的文件".into());
            }

            let first_file = &share_info.files[0];
            let dlink_res = crate::lanzou::resolve_lanzou_file(
                &raw_url,
                &first_file.id,
                pwd.as_deref(),
            )
            .await
            .map_err(|e| format!("获取蓝奏云直链失败：{}", e))?;

            task.url = dlink_res.url;
            if !first_file.name.is_empty() {
                task.file_name = first_file.name.clone();
            }
            if first_file.size > 0 {
                task.total_bytes = first_file.size;
            }
            task.category = category(&task.file_name);
            task.connection_count = task.connection_count.max(16);
            for (k, v) in dlink_res.headers {
                task.headers.insert(k, v);
            }

            for file in share_info.files.iter().skip(1) {
                let f_id = file.id.clone();
                let f_name = file.name.clone();
                let s_url = raw_url.clone();
                let pwd_c = pwd.clone();
                let destination = task.destination.clone();
                let collision_policy = task.collision_policy.clone();
                let completion_action = task.completion_action.clone();
                let connection_count = task.connection_count;
                let sub_manager = self.clone();

                tokio::spawn(async move {
                    if let Ok(sub_dlink) = crate::lanzou::resolve_lanzou_file(&s_url, &f_id, pwd_c.as_deref()).await {
                        let sub_headers = sub_dlink.headers;
                        let req = NewTaskRequest {
                            url: sub_dlink.url,
                            file_name: Some(f_name),
                            destination: Some(destination),
                            headers: sub_headers,
                            scheduled_at: None,
                            priority: 0,
                            expected_checksum: None,
                            source: Some("lanzou".to_string()),
                            per_task_speed_limit: 0,
                            collision_policy,
                            completion_action,
                            media: None,
                            connection_count: Some(connection_count.max(16)),
                            start_paused: false,
                            user_edited_file_name: true,
                            cloud_refresh: None,
                        };
                        let _ = sub_manager.add(req).await;
                    }
                });
            }

            let _ = self.store.upsert_task(task).await;
            self.emit_task("updated", task);
        }

        if let Some(parsed_123) = crate::pan123::parse_pan123_url(&raw_url) {
            let pwd = parsed_123.pass_code.clone();
            let share_info = crate::pan123::inspect_pan123_share(&raw_url, pwd.as_deref())
                .await
                .map_err(|e| format!("解析 123云盘分享失败：{}", e))?;

            let file_items: Vec<_> = share_info.files.into_iter().filter(|f| f.kind == "file").collect();
            if file_items.is_empty() {
                return Err("123云盘分享中未找到可下载的文件".into());
            }

            let first_file = &file_items[0];
            let stored_cred = match self.store.media_credential_get(".123pan.com").await {
                Ok(Some(c)) => Some(c),
                _ => self.store.media_credential_get("123pan.com").await.ok().flatten(),
            };
            let token_str = stored_cred.as_ref().map(|c| c.cookie.as_str());

            let dlink_res = crate::pan123::resolve_pan123_file(
                &parsed_123.share_key,
                first_file.id,
                &first_file.s3_key_flag,
                first_file.size,
                &first_file.etag,
                pwd.as_deref(),
                token_str,
            )
            .await
            .map_err(|e| format!("获取 123云盘直链失败：{}", e))?;

            task.url = dlink_res.url;
            if !first_file.name.is_empty() {
                task.file_name = first_file.name.clone();
            }
            task.total_bytes = first_file.size;
            task.category = category(&task.file_name);
            task.connection_count = task.connection_count.max(16);
            for (k, v) in dlink_res.headers {
                task.headers.insert(k, v);
            }

            for file in file_items.iter().skip(1) {
                let f_id = file.id;
                let f_name = file.name.clone();
                let f_size = file.size;
                let f_s3 = file.s3_key_flag.clone();
                let f_etag = file.etag.clone();
                let s_key = parsed_123.share_key.clone();
                let pwd_c = pwd.clone();
                let destination = task.destination.clone();
                let collision_policy = task.collision_policy.clone();
                let completion_action = task.completion_action.clone();
                let connection_count = task.connection_count;
                let sub_manager = self.clone();
                let cred_str = token_str.map(|s| s.to_string());

                tokio::spawn(async move {
                    if let Ok(sub_dlink) = crate::pan123::resolve_pan123_file(
                        &s_key,
                        f_id,
                        &f_s3,
                        f_size,
                        &f_etag,
                        pwd_c.as_deref(),
                        cred_str.as_deref(),
                    )
                    .await
                    {
                        let sub_headers = sub_dlink.headers;
                        let req = NewTaskRequest {
                            url: sub_dlink.url,
                            file_name: Some(f_name),
                            destination: Some(destination),
                            headers: sub_headers,
                            scheduled_at: None,
                            priority: 0,
                            expected_checksum: None,
                            source: Some("pan123".to_string()),
                            per_task_speed_limit: 0,
                            collision_policy,
                            completion_action,
                            media: None,
                            connection_count: Some(connection_count.max(16)),
                            start_paused: false,
                            user_edited_file_name: true,
                            cloud_refresh: None,
                        };
                        let _ = sub_manager.add(req).await;
                    }
                });
            }

            let _ = self.store.upsert_task(task).await;
            self.emit_task("updated", task);
        }

        Ok(())
    }

    async fn download_once(
        self: &SharedManager,
        mut task: DownloadTask,
        token: CancellationToken,
    ) -> Result<DownloadTask, String> {
        if task.url.contains("pan.baidu.com/s/")
            || task.url.contains("pan.baidu.com/share/init")
            || task.url.contains("pan.quark.cn/s/")
            || task.url.contains("mypikpak.com/s/")
            || task.url.contains("lanzou")
            || task.url.contains("123pan.com/s/")
            || task.url.contains("123684.com/s/")
        {
            self.resolve_cloud_share_task(&mut task).await?;
        }

        let platform = crate::media_platforms::detect_platform(&task.url);
        if platform != crate::media_platforms::MediaPlatform::Unknown && (task.media.is_none() || task.file_name == "download" || task.file_name.is_empty()) {
            let settings = self.settings().await;
            let mut cookie = task.headers.get("Cookie").map(|s| s.as_str());
            let mut referer = task.headers.get("Referer").map(|s| s.as_str());
            let mut user_agent = task.headers.get("User-Agent").map(|s| s.as_str());

            let stored_cred;
            if let Some(domain) = crate::media_cookies::extract_domain(&task.url) {
                tracing::info!(domain = %domain, "开始匹配媒体凭据");
                match self.store.media_credential_get_matching(&domain).await {
                    Ok(Some(cred)) => {
                        tracing::info!(domain = %domain, cookie_len = cred.cookie.len(), "成功匹配到凭据");
                        stored_cred = cred;
                        if cookie.is_none() && !stored_cred.cookie.is_empty() {
                            cookie = Some(&stored_cred.cookie);
                        }
                        if referer.is_none() {
                            referer = stored_cred.referer.as_deref();
                        }
                        if user_agent.is_none() {
                            user_agent = stored_cred.user_agent.as_deref();
                        }
                    }
                    Ok(None) => {
                        tracing::info!(domain = %domain, "未在数据库中找到匹配的凭据");
                        stored_cred = crate::models::MediaCredential {
                            domain: String::new(),
                            cookie: String::new(),
                            referer: None,
                            user_agent: None,
                            updated_at: String::new(),
                        };
                    }
                    Err(e) => {
                        tracing::error!(domain = %domain, error = %e, "获取凭据时发生错误");
                        stored_cred = crate::models::MediaCredential {
                            domain: String::new(),
                            cookie: String::new(),
                            referer: None,
                            user_agent: None,
                            updated_at: String::new(),
                        };
                    }
                }
            } else {
                tracing::warn!(url = %task.url, "无法提取域名，跳过凭据匹配");
                stored_cred = crate::models::MediaCredential {
                    domain: String::new(),
                    cookie: String::new(),
                    referer: None,
                    user_agent: None,
                    updated_at: String::new(),
                };
            }

            match crate::media::probe(&self.app, &settings, &task.url, cookie, referer, user_agent).await {
                Ok(media) => {
                    // 抖音图集自动拆分：用户直接提交抖音图集短链（未先点击"分析媒体"）时，
                    // 自动 probe 识别为 Gallery 后，把每张图作为独立子任务，使用图片直链走 HTTP Range 路径。
                    // 当前任务改为下载第一张图（保留原 task.id，task.media = None 跳过 media::download），
                    // 剩余图片通过 self.add() 创建子任务并触发调度。
                    // 这避免后续 media::download 用 yt-dlp 下载图集（只拿到 1 张图且文件名错误为 .mp4）。
                    if media.media_type == crate::models::MediaType::Gallery {
                        let image_formats: Vec<&crate::models::MediaFormat> = media
                            .formats
                            .iter()
                            .filter(|f| f.image_url.is_some())
                            .collect();
                        if image_formats.is_empty() {
                            return Err(
                                "图集未识别到图片直链，请先点击\"分析媒体\"按钮选择图片后再下载".into(),
                            );
                        }
                        // 文件名 stem：使用 probe 返回的 title（清理 hashtag 后）
                        let raw_title = media.title.clone();
                        let cleaned = regex::Regex::new(r"#[^\s#.]+")
                            .map(|re| re.replace_all(&raw_title, "").to_string())
                            .unwrap_or_else(|_| raw_title.clone());
                        let cleaned = crate::manager::naming_template::sanitize_filename(&cleaned);
                        let stem = if cleaned.trim().is_empty() {
                            "gallery".to_string()
                        } else {
                            cleaned.trim().to_string()
                        };
                        // 第一张图作为当前任务（保留原 task.id），走 HTTP Range 路径
                        let first = image_formats[0];
                        let first_ext = first
                            .extension
                            .as_deref()
                            .map(|e| e.trim_start_matches('.'))
                            .filter(|e| !e.is_empty())
                            .unwrap_or("jpg")
                            .to_string();
                        // image_formats 已按 image_url.is_some() 过滤，let-else 仅作防御，
                        // 避免异常探测数据导致运行时 panic（AGENTS.md §7）。
                        let Some(first_image_url) = first.image_url.clone() else {
                            return Err("图集图片直链缺失，请重试或先点击\"分析媒体\"".into());
                        };
                        task.url = first_image_url;
                        task.file_name = format!("{}_1.{}", stem, first_ext);
                        task.category = category(&task.file_name);
                        task.media = None; // 走 HTTP Range 路径，不调用 yt-dlp
                        task.total_bytes = 0;
                        self.store.upsert_task(&task).await?;
                        self.emit_task("updated", &task);
                        // 剩余图片通过 self.add() 创建子任务，触发调度
                        for (idx, fmt) in image_formats.iter().skip(1).enumerate() {
                            let ext = fmt
                                .extension
                                .as_deref()
                                .map(|e| e.trim_start_matches('.'))
                                .filter(|e| !e.is_empty())
                                .unwrap_or("jpg")
                                .to_string();
                            let Some(item_image_url) = fmt.image_url.clone() else {
                                continue; // 同上，仅防御异常探测数据。
                            };
                            let new_req = NewTaskRequest {
                                url: item_image_url,
                                file_name: Some(format!("{}_{}.{}", stem, idx + 2, ext)),
                                destination: Some(task.destination.clone()),
                                headers: task.headers.clone(),
                                scheduled_at: None,
                                priority: task.priority,
                                expected_checksum: None,
                                source: Some(task.source.clone()),
                                 per_task_speed_limit: task.per_task_speed_limit,
                                collision_policy: task.collision_policy.clone(),
                                // 子任务不触发关机/打开文件夹等完成动作
                                completion_action: CompletionAction::None,
                                media: None,
                                connection_count: Some(task.connection_count),
                                start_paused: false,
                                // 跳过自动文件名清理规则（已显式指定）
                                user_edited_file_name: true,
                                cloud_refresh: None,
                            };
                            if let Err(e) = self.add(new_req).await {
                                tracing::warn!(error = %e, "创建图集子任务失败");
                            }
                        }
                    } else if !media.formats.is_empty() {
                        let direct = media.formats.iter()
                            .filter(|item| item.has_video && !item.requires_ffmpeg && item.url.is_some())
                            .max_by_key(|item| item.height.unwrap_or(0));
                        let has_ffmpeg = crate::media_tools::resolve_ffmpeg(&self.app, &settings).is_some();
                        let merged = if has_ffmpeg {
                            media.formats.iter()
                                .filter(|item| item.has_video && item.has_audio && item.requires_ffmpeg)
                                .max_by_key(|item| item.height.unwrap_or(0))
                        } else {
                            None
                        };
                        let selected_format = direct.or(merged).unwrap_or(&media.formats[0]).clone();
                        task.media = Some(crate::models::MediaSelection {
                            extractor: media.extractor,
                            format_id: Some(selected_format.id.clone()),
                            format_label: Some(selected_format.label.clone()),
                            subtitles: vec![],
                            thumbnail: media.thumbnail,
                            requires_ffmpeg: selected_format.requires_ffmpeg,
                            url: selected_format.url.clone(),
                        });

                        let is_bilibili_or_media_id = task.file_name.starts_with("BV")
                            || task.file_name.starts_with("av")
                            || task.file_name.starts_with("ep")
                            || task.file_name.starts_with("ss");
                        let is_default_name = task.file_name == "download"
                            || task.file_name.starts_with("LHmt")
                            || task.file_name.is_empty()
                            || is_bilibili_or_media_id
                            || task.file_name.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '_')
                            || task.file_name.contains(&task.url);

                        if is_default_name {
                            let ext = selected_format.extension.unwrap_or_else(|| "mp4".to_string()).replace(".", "");
                            let mut name_stem = media.title.clone();
                            if let Ok(rules) = self.store.filename_cleanup_rule_list().await {
                                let after = apply_filename_cleanup(&name_stem, &rules);
                                if !after.is_empty() {
                                    name_stem = after;
                                }
                            }
                            let name = safe_name(&name_stem);
                            task.file_name = format!("{}.{}", name, ext);
                            task.category = category(&task.file_name);
                        }

                        // probe 完成后即将进入 media::download，确保 active_connections >= 1，
                        // 避免前端在 probe 完成到 media::download 设置真实连接数之间
                        // 因 active_connections=0 误显示"解析中"。
                        if task.active_connections == 0 {
                            task.active_connections = 1;
                        }
                        self.store.upsert_task(&task).await?;
                        self.emit_task("updated", &task);
                    } else {
                        return Err("MEDIA_PROBE_ERROR:没有找到可下载的媒体格式".into());
                    }
                }
                Err(err) => {
                    return Err(format!("MEDIA_PROBE_ERROR:{}", err));
                }
            }
        }

        let mut is_resolved_direct_media = false;
        let media_sel_opt = task.media.clone();
        if let Some(media_sel) = media_sel_opt {
            // HLS m3u8 流切片需要特定解析器或 FFmpeg 拼接，其他直接 HTTP FLV/MP4 流统一走原生 download_stream。
            let is_m3u8_live = task.url.contains("pull-hls-") || task.url.contains(".m3u8");
            if !media_sel.requires_ffmpeg && !is_m3u8_live {
                inject_media_credentials(&mut task, &self.store).await;
                let settings = self.settings().await;
                let cookie = task.headers.get("Cookie").map(|s| s.as_str());
                let referer = task.headers.get("Referer").map(|s| s.as_str());
                let user_agent = task.headers.get("User-Agent").map(|s| s.as_str());

                let target_probe_url = if let Some(ref_hdr) = task.headers.get("Referer") {
                    if ref_hdr.contains("douyin.com") || ref_hdr.contains("bilibili.com") || ref_hdr.contains("youtube.com") || ref_hdr.contains("tiktok.com") {
                        ref_hdr.as_str()
                    } else {
                        &task.url
                    }
                } else {
                    &task.url
                };

                let play_url = if let Some(u) = media_sel.url.as_deref().filter(|s| !s.is_empty()) {
                    Some(u.to_string())
                } else if let Ok(probe_res) = crate::media::probe(&self.app, &settings, target_probe_url, cookie, referer, user_agent).await {
                    if !probe_res.title.trim().is_empty() {
                        let raw_title = probe_res.title.clone();
                        let cleaned = crate::manager::naming_template::sanitize_filename(&regex::Regex::new(r"#[^\s#.]+")
                            .map(|re| re.replace_all(&raw_title, "").to_string())
                            .unwrap_or_else(|_| raw_title.clone()));
                        if !cleaned.trim().is_empty() {
                            task.file_name = format!("{}.mp4", cleaned.trim());
                            task.category = category(&task.file_name);
                        }
                    }
                    if let Some(fmt_id) = &media_sel.format_id {
                        probe_res.formats.iter().find(|f| &f.id == fmt_id).and_then(|f| f.url.clone()).or_else(|| probe_res.formats.first().and_then(|f| f.url.clone()))
                    } else {
                        probe_res.formats.first().and_then(|f| f.url.clone())
                    }
                } else {
                    None
                };

                if let Some(purl) = play_url {
                    let temp_client = if task.proxy_override.is_some() {
                        build_task_client(&settings, &task)?
                    } else {
                        self.client.read().await.clone()
                    };

                    let mut req = temp_client.get(&purl).header(ACCEPT_ENCODING, "identity").header(RANGE, "bytes=0-0");
                    if !task.headers.keys().any(|k| k.eq_ignore_ascii_case("User-Agent")) {
                        req = req.header(reqwest::header::USER_AGENT, &settings.user_agent);
                    }
                    if !task.headers.keys().any(|k| k.eq_ignore_ascii_case("Referer")) {
                        if purl.contains("bilibili.com") || purl.contains("bilivideo.com") || task.url.contains("bilibili.com") {
                            req = req.header(reqwest::header::REFERER, "https://www.bilibili.com/");
                            task.headers.insert("Referer".to_string(), "https://www.bilibili.com/".to_string());
                        }
                    }
                    for (name, value) in &task.headers {
                        req = req.header(name, value);
                    }
                    if let Ok(resp) = req.send().await {
                        if resp.status().is_success() || resp.status() == 206 {
                            let total_size = resp.headers().get(CONTENT_RANGE)
                                .and_then(|v| v.to_str().ok())
                                .and_then(parse_content_range_value)
                                .map(|v| v.2)
                                .or_else(|| {
                                    resp.headers().get(CONTENT_LENGTH)
                                        .and_then(|v| v.to_str().ok())
                                        .and_then(|s| s.parse::<u64>().ok())
                                });
                            if let Some(total) = total_size {
                                task.total_bytes = total;
                            }
                            task.url = resp.url().to_string();
                            is_resolved_direct_media = true;
                        }
                    }
                }
            }
        }

        if is_resolved_direct_media {
            let output = self.reserve_output_path(&mut task).await?;
            let settings = self.settings().await;
            let conn_count = if task.total_bytes > 0 && task.total_bytes < 10 * 1024 * 1024 {
                1
            } else {
                if task.connection_count > 1 { task.connection_count } else { settings.connections_per_download.max(8) }
            };
            task.connection_count = conn_count;
            task.active_connections = conn_count;
            task.accepts_ranges = Some(conn_count > 1);
            self.store.upsert_task(&task).await?;
            self.emit_task("updated", &task);

            let temp_client = if task.proxy_override.is_some() {
                let settings = self.settings().await;
                build_task_client(&settings, &task)?
            } else {
                self.client.read().await.clone()
            };

            let _ = ensure_task_temp_dir(&task.destination, &task.id).await;
            let temp = task_temp_path(&task.destination, &task.id, &task.file_name);
            let total = task.total_bytes;
            let task_limiter = Arc::new(crate::manager::RateLimiter::new());

            let task_backup = task.clone();
            let task_id_for_saved = task.id.clone();
            let is_media = task.media.is_some();
            let download_res = if total > 0 && conn_count > 1 {
                self.download_segments(task, &temp_client, &temp, total, conn_count, token.clone(), task_limiter).await
            } else {
                self.download_stream(task, &temp_client, &temp, token.clone(), task_limiter).await
            };

            if token.is_cancelled() || download_res.is_err() {
                let is_cancel = token.is_cancelled();
                if (is_cancel || is_media) && temp.exists() {
                    if let Ok(meta) = fs::metadata(&temp).await {
                        if meta.len() > 0 {
                            let final_output = output;
                            if let Err(_) = fs::rename(&temp, &final_output).await {
                                let _ = fs::copy(&temp, &final_output).await;
                                let _ = fs::remove_file(&temp).await;
                            }
                            crate::media_tools::remux_flv_to_mp4_if_needed(&self.app, &settings, &final_output).await;
                            let mut saved_task = match download_res {
                                Ok(t) => t,
                                Err(_) => self.store.get_task(&task_id_for_saved).await.ok().flatten().unwrap_or(task_backup),
                            };
                            saved_task.downloaded_bytes = meta.len();
                            saved_task.total_bytes = meta.len();
                            saved_task.status = TaskStatus::Completed;
                            let _ = self.store.upsert_task(&saved_task).await;
                            self.emit_task("updated", &saved_task);
                            self.clear_parts(&saved_task).await;
                            return Ok(saved_task);
                        }
                    }
                }
                return Err(download_res.err().unwrap_or_else(|| "任务已暂停".into()));
            }

            task = download_res?;
            let final_output = if output.exists() && task.collision_policy == CollisionPolicy::Rename {
                self.reserve_output_path(&mut task).await?
            } else {
                output
            };
            if final_output.exists() {
                match task.collision_policy {
                    CollisionPolicy::Overwrite => {
                        let _ = fs::remove_file(&final_output).await;
                    }
                    CollisionPolicy::Skip => return Err("目标文件已存在，任务已跳过".into()),
                    CollisionPolicy::Rename => return Err("目标文件在下载完成时发生冲突，请重试任务".into()),
                }
            }
            if let Err(e) = fs::rename(&temp, &final_output).await {
                fs::copy(&temp, &final_output).await.map_err(|err| format!("无法保存完成文件：{err} (原错误: {e})"))?;
                let _ = fs::remove_file(&temp).await;
            }
            self.clear_parts(&task).await;

            if task.media.is_some() {
                let settings = self.settings().await;
                let naming_templates = self.store.platform_naming_template_list().await.unwrap_or_default();
                inject_media_credentials(&mut task, &self.store).await;
                let cookie = task.headers.get("Cookie").cloned();
                let referer = task.headers.get("Referer").cloned();
                let user_agent = task.headers.get("User-Agent").cloned();

                let _ = crate::media::apply_platform_naming_template(
                    &self.app, &settings, &mut task, &final_output,
                    cookie.as_deref(), referer.as_deref(), user_agent.as_deref(), &naming_templates,
                ).await;

                if !task.file_name.contains('.') {
                    let current_disk_path = Path::new(&task.destination).join(&task.file_name);
                    let new_file_name = format!("{}.mp4", task.file_name);
                    let new_disk_path = Path::new(&task.destination).join(&new_file_name);
                    if current_disk_path.exists() && !new_disk_path.exists() {
                        if let Ok(_) = fs::rename(&current_disk_path, &new_disk_path).await {
                            task.file_name = new_file_name;
                            task.category = category(&task.file_name);
                        }
                    }
                }
                let _ = self.store.upsert_task(&task).await;
                self.emit_task("updated", &task);
            }

            return Ok(task);
        }

        if task.media.is_some() {
            self.reserve_output_path(&mut task).await?;
            let settings = self.settings().await;
            let target_conn = if task.connection_count > 1 { task.connection_count } else { settings.connections_per_download.max(8) };
            let conn_count = if task.total_bytes > 0 && task.total_bytes < 10 * 1024 * 1024 { 1 } else { target_conn };
            task.connection_count = conn_count;
            task.active_connections = conn_count;
            self.store.upsert_task(&task).await?;
            self.emit_task("updated", &task);
            // Task 46：媒体任务在调用 yt-dlp 前从数据库按域名补齐缺失的
            // Cookie/Referer/User-Agent。前端通过 task.headers 显式传入的值优先；
            // 仅当对应头不存在时才用数据库存储值填充。
            // 解密失败时安全降级为"无凭证"，不阻塞下载。
            inject_media_credentials(&mut task, &self.store).await;
            let settings = self.settings().await;
            // Task 43：加载平台命名模板列表传给 media::download。
            // 加载失败时降级为空列表（不应用任何模板），不阻塞下载。
            let naming_templates = self
                .store
                .platform_naming_template_list()
                .await
                .unwrap_or_default();
            // 直播任务（抖音直播等）暂停 = 结束录制 + 保存为已完成文件。
            // 参考本项目 B 站直播流程（download_stream 暂停后的 1594-1619 逻辑）：
            // media::download 暂停时只 kill yt-dlp 子进程并返回 Err("任务已暂停")，
            // 这里检测到取消且有输出文件时，重命名 + remux + 标记 Completed。
            // yt-dlp 实际写入的文件扩展名可能与 template 不同（如 xxx.ts 而非 xxx.mp4），
            // 通过 media::find_live_output_file 扫描输出目录找到实际文件。
            let is_live_task = crate::media_platforms::is_douyin_live(&task.url)
                || task.url.contains("pull-hls-")
                || task.url.contains("pull-flv-")
                || task.url.contains(".m3u8");
            let output = std::path::PathBuf::from(&task.destination).join(&task.file_name);
            let output_dir = output
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from(&task.destination));
            let task_backup = task.clone();
            let task_id_for_saved = task.id.clone();
            let download_res = crate::media::download(
                &self.app,
                &settings,
                task,
                token.clone(),
                naming_templates,
            )
            .await;

            if token.is_cancelled() && is_live_task {
                // 手动结束/暂停录制：与 B 站直播 download_stream 暂停保存逻辑一致，
                // 将已录制的片段保存并转封装为标准 MP4，然后将任务标记为已完成。
                let live_file = crate::media::find_live_output_file(&output_dir, &task_backup.file_name).await;
                let saved_file = match &live_file {
                    Some(f) if f.exists() => Some(f.clone()),
                    _ if output.exists() => Some(output.clone()),
                    _ => None,
                };
                if let Some(file_path) = saved_file {
                    if let Ok(meta) = fs::metadata(&file_path).await {
                        if meta.len() > 0 {
                            let clean_file_path = if file_path.to_string_lossy().ends_with(".part") {
                                let non_part = file_path.with_extension("");
                                let _ = fs::rename(&file_path, &non_part).await;
                                non_part
                            } else {
                                file_path
                            };

                            let final_path = crate::media_tools::remux_flv_to_mp4_if_needed(
                                &self.app, &settings, &clean_file_path,
                            )
                            .await;

                            let final_size = fs::metadata(&final_path)
                                .await
                                .map(|m| m.len())
                                .unwrap_or(meta.len());
                            let mut saved_task = match download_res {
                                Ok(t) => t,
                                Err(_) => self
                                    .store
                                    .get_task(&task_id_for_saved)
                                    .await
                                    .ok()
                                    .flatten()
                                    .unwrap_or(task_backup),
                            };
                            if let Some(final_name) = final_path.file_name().and_then(|s| s.to_str()) {
                                saved_task.file_name = final_name.to_string();
                                saved_task.category = category(&saved_task.file_name);
                            }
                            saved_task.downloaded_bytes = final_size;
                            saved_task.total_bytes = final_size;
                            saved_task.speed = 0;
                            saved_task.eta_seconds = None;
                            saved_task.active_connections = 0;
                            saved_task.status = TaskStatus::Completed;
                            saved_task.completed_at = Some(now());
                            let _ = self.store.upsert_task(&saved_task).await;
                            self.emit_task("updated", &saved_task);
                            tracing::info!(
                                task_id = %saved_task.id,
                                file_size = final_size,
                                file_name = %saved_task.file_name,
                                "直播录制已手动停止并保存转码为已完成文件"
                            );
                            return Ok(saved_task);
                        }
                    }
                }
            }
            return download_res;
        }
        // Task 31：任务级 proxy_override 优先于全局；仅在设置了覆盖时重建客户端，
        // 避免无覆盖任务每次都付出 settings 读 + client 构造开销。
        let client = if task.proxy_override.is_some() {
            let settings = self.settings().await;
            build_task_client(&settings, &task)?
        } else {
            self.client.read().await.clone()
        };
        let is_baidu_link = task.url.contains("baidupcs.com") || task.url.contains("pan.baidu.com");
        let probe = if is_baidu_link {
            // 百度 PCS 服务器对 HEAD 请求一律返回 403 Forbidden，直接使用 GET Range: bytes=0-0 探测
            let mut get = client
                .get(&task.url)
                .header(ACCEPT_ENCODING, "identity")
                .header(RANGE, "bytes=0-0");
            for (name, value) in &task.headers {
                get = get.header(name, value);
            }
            get.send().await.map_err(friendly_reqwest)?
        } else {
            let mut head = client.head(&task.url).header(ACCEPT_ENCODING, "identity");
            for (name, value) in &task.headers {
                head = head.header(name, value);
            }
            let initial_probe = head.send().await.map_err(friendly_reqwest)?;
            // 部分服务器/CDN 不支持 HEAD（403/405/501/400）：回退为 GET + bytes=0-0 探针，
            // 避免把可以正常 GET 下载的资源直接判为失败。回退响应可能为 206，
            // 总长度由 probe_total_bytes 从 Content-Range 提取。
            if !initial_probe.status().is_success() {
                let mut get = client
                    .get(&task.url)
                    .header(ACCEPT_ENCODING, "identity")
                    .header(RANGE, "bytes=0-0");
                for (name, value) in &task.headers {
                    get = get.header(name, value);
                }
                match get.send().await {
                    Ok(get_resp) if get_resp.status().is_success() || get_resp.status() == reqwest::StatusCode::PARTIAL_CONTENT => get_resp,
                    _ => initial_probe,
                }
            } else {
                initial_probe
            }
        };
        task.final_url = Some(diagnostic_url(probe.url()));
        task.response_status = Some(probe.status().as_u16());
        task.content_type =
            header_string(&probe, CONTENT_TYPE).map(|value| truncate_text(value, 256));
        task.accepts_ranges = Some(
            probe.status() == reqwest::StatusCode::PARTIAL_CONTENT
                || probe
                    .headers()
                    .get(ACCEPT_RANGES)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.eq_ignore_ascii_case("bytes")),
        );
        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);
        if !probe.status().is_success() && probe.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(format!("服务器返回 HTTP {}", probe.status()));
        }

        let is_m3u8_stream = task.url.contains(".m3u8")
            || task.url.contains("pull-hls-")
            || task.content_type.as_deref().map(|ct| ct.contains("mpegurl") || ct.contains("m3u8")).unwrap_or(false);

        if is_m3u8_stream {
            if task.file_name.ends_with(".m3u8") || task.file_name == "download" || task.file_name.is_empty() {
                let stem = Path::new(&task.file_name).file_stem().and_then(|s| s.to_str()).unwrap_or("video");
                let stem_clean = if stem == "download" || stem == "index" || stem == "playlist" { "video" } else { stem };
                task.file_name = format!("{}.mp4", stem_clean);
                task.category = category(&task.file_name);
            }

            inject_media_credentials(&mut task, &self.store).await;
            let output = self.reserve_output_path(&mut task).await?;
            let _ = ensure_task_temp_dir(&task.destination, &task.id).await;
            let temp = task_temp_path(&task.destination, &task.id, &task.file_name);
            let task_limiter = Arc::new(crate::manager::RateLimiter::new());

            let download_res = crate::m3u8::downloader::download_m3u8_task(
                self,
                task.clone(),
                &client,
                &temp,
                token.clone(),
                task_limiter,
            )
            .await;

            match download_res {
                Ok(mut completed_task) => {
                    if temp.exists() {
                        if let Err(_) = fs::rename(&temp, &output).await {
                            let _ = fs::copy(&temp, &output).await;
                            let _ = fs::remove_file(&temp).await;
                        }
                    }
                    completed_task.status = TaskStatus::Completed;
                    completed_task.completed_at = Some(now());
                    completed_task.speed = 0;
                    completed_task.eta_seconds = None;
                    completed_task.active_connections = 0;
                    self.store.upsert_task(&completed_task).await?;
                    self.emit_task("updated", &completed_task);
                    return Ok(completed_task);
                }
                Err(err) => {
                    return Err(err);
                }
            }
        }

        let total = probe_total_bytes(&probe);
        let etag = header_string(&probe, ETAG);
        let last_modified = header_string(&probe, LAST_MODIFIED);
        // Resume integrity check: if we previously recorded ETag/Last-Modified
        // and already have downloaded bytes or segments, the remote resource
        // must still match. If it changed, we MUST NOT silently stitch old
        // shards onto a new resource — mark the task RemoteChanged and let the
        // user decide whether to redownload or keep the old file.
        let has_progress = task.downloaded_bytes > 0 || !task.segments.is_empty();
        let has_recorded_validator = task.etag.is_some() || task.last_modified.is_some();
        if has_progress
            && has_recorded_validator
            && remote_resource_changed(
                task.etag.as_deref(),
                etag.as_deref(),
                task.last_modified.as_deref(),
                last_modified.as_deref(),
            )
        {
            task.status = TaskStatus::RemoteChanged;
            task.error = Some("远端资源已变化，是否重新下载？".into());
            task.speed = 0;
            task.eta_seconds = None;
            task.active_connections = 0;
            // Keep old etag/last_modified/segments/downloaded_bytes so the user
            // can inspect what was downloaded before deciding.
            self.store.upsert_task(&task).await?;
            self.emit_task("updated", &task);
            return Err(format!(
                "{REMOTE_CHANGED_PREFIX}远端资源已变化，是否重新下载？"
            ));
        }
        task.etag = etag;
        task.last_modified = last_modified;
        task.total_bytes = total;
        if task.file_name == "download" {
            if let Some(name) = disposition_name(&probe) {
                // Task 20: 服务器 Content-Disposition 提供的文件名也属于"未手动编辑"来源，
                // 应用清理规则后再做 safe_name 规范化。失败时静默回退到 disposition 原始名。
                let mut cleaned = name;
                if let Ok(rules) = self.store.filename_cleanup_rule_list().await {
                    let after = apply_filename_cleanup(&cleaned, &rules);
                    if !after.is_empty() {
                        cleaned = safe_name(&after);
                    }
                }
                task.file_name = cleaned;
                task.category = category(&task.file_name);
            }
        }
        let output = if task.downloaded_bytes > 0 || !task.segments.is_empty() {
            PathBuf::from(&task.destination).join(&task.file_name)
        } else {
            self.reserve_output_path(&mut task).await?
        };
        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);
        let _ = ensure_task_temp_dir(&task.destination, &task.id).await;
        let temp = task_temp_path(&task.destination, &task.id, &task.file_name);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);
        let supports_range = probe
            .headers()
            .get(ACCEPT_RANGES)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("bytes"));
        let settings = self.settings().await;
        let mut connections = effective_connection_count(&settings, task.connection_count);
        // CDN cap 前置判定：必须在下方动态连接数调整改写 task.connection_count 之前计算
        // "用户是否显式设置"，否则 suggest_connections 的自动调整结果会被误判为用户
        // 显式设置，导致 CDN 连接数上限几乎从不生效。
        let is_user_explicit = task.connection_count != settings.connections_per_download;
        // Accept-Ranges is only advisory and is frequently omitted by CDNs.
        // Verify multi-connection support with an actual one-byte range request.
        let supports_range = if connections > 1 {
            let mut request = client.get(&task.url);
            for (name, value) in &task.headers {
                request = request.header(name, value);
            }
            request = request
                .header(ACCEPT_ENCODING, "identity")
                .header(RANGE, "bytes=0-0");
            match request.send().await {
                Ok(response) if response.status() == reqwest::StatusCode::PARTIAL_CONTENT => {
                    let valid_range = matches!(
                        parse_content_range(&response),
                        Some((0, 0, response_total)) if response_total == total
                    );
                    valid_range && response.bytes().await.is_ok_and(|body| body.len() == 1)
                }
                _ => false,
            }
        } else {
            supports_range
        };
        task.accepts_ranges = Some(supports_range);

        // Dynamically adjust connections based on file size for un-started tasks
        if task.downloaded_bytes == 0 && task.segments.is_empty() {
            if !supports_range {
                task.connection_count = 1;
                connections = 1;
            } else if settings.connections_per_download > 1
                && task.connection_count == settings.connections_per_download
            {
                let suggested = precheck::suggest_connections(Some(total), supports_range);
                let is_baidu_task = task.url.contains("baidupcs.com") || task.url.contains("pan.baidu.com");
                let is_fast_cloud_task = task.url.contains("quark.cn") || task.url.contains("mypikpak.com") || task.url.contains("pikpak") || task.url.contains("lanzou") || task.url.contains("123pan") || task.url.contains("123684");
                let target_count = if is_baidu_task && total >= 10 * 1024 * 1024 {
                    16
                } else if is_fast_cloud_task && total >= 10 * 1024 * 1024 {
                    32
                } else if total >= 4 * 1024 * 1024 {
                    task.connection_count.max(suggested)
                } else {
                    suggested
                };
                if target_count != task.connection_count {
                    task.connection_count = target_count;
                    connections = target_count;
                }
            }
        }

        // CDN 感知：对已知对并发 Range 请求单独限速的 CDN，自动收紧连接数。
        // 仅在全新任务（无已下载进度）且用户未显式指定连接数时生效，不影响续传。
        // is_user_explicit 已在动态连接数调整之前计算（见上方声明处注释）。
        if !is_user_explicit && task.downloaded_bytes == 0 && task.segments.is_empty() {
            let probe_url = task.final_url.as_deref().unwrap_or(&task.url);
            let cdn_cap = precheck::cdn_connection_cap(probe_url);
            if cdn_cap < connections {
                tracing::info!(
                    task_id = %task.id,
                    capped_connections = cdn_cap,
                    original_connections = connections,
                    "CDN 感知：自动将连接数从 {} 降为 {} 以避免 CDN 限速",
                    connections,
                    cdn_cap
                );
                connections = cdn_cap;
                task.connection_count = cdn_cap;
            }
        }

        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);
        let task_limiter = Arc::new(RateLimiter::new());
        if supports_range && total >= 4 * 1024 * 1024 && connections > 1 {
            task = self
                .download_segments(
                    task,
                    &client,
                    &temp,
                    total,
                    connections,
                    token.clone(),
                    task_limiter,
                )
                .await?;
        } else {
            task.active_connections = 1;
            task.segments = vec![DownloadSegment {
                index: 0,
                start_byte: 0,
                end_byte: total.saturating_sub(1),
                downloaded_bytes: 0,
                status: "downloading".into(),
            }];
            self.store.upsert_task(&task).await?;
            self.emit_task("updated", &task);
            task = self
                .download_stream(task, &client, &temp, token.clone(), task_limiter)
                .await?;
        }
        if token.is_cancelled() {
            return Err("任务已暂停".into());
        }
        let final_output = if output.exists() && task.collision_policy == CollisionPolicy::Rename {
            self.reserve_output_path(&mut task).await?
        } else {
            output
        };
        if final_output.exists() {
            match task.collision_policy {
                CollisionPolicy::Overwrite => fs::remove_file(&final_output)
                    .await
                    .map_err(|error| format!("无法覆盖已有文件：{error}"))?,
                CollisionPolicy::Skip => return Err("目标文件已存在，任务已跳过".into()),
                CollisionPolicy::Rename => {
                    return Err("目标文件在下载完成时发生冲突，请重试任务".into())
                }
            }
        }
        if let Err(e) = fs::rename(&temp, &final_output).await {
            fs::copy(&temp, &final_output)
                .await
                .map_err(|err| format!("无法保存完成文件：{err} (原错误: {e})"))?;
            let _ = fs::remove_file(&temp).await;
        }
        self.clear_parts(&task).await;

        if task.media.is_some() {
            let settings = self.settings().await;
            let naming_templates = self
                .store
                .platform_naming_template_list()
                .await
                .unwrap_or_default();
            inject_media_credentials(&mut task, &self.store).await;
            let cookie = task.headers.get("Cookie").cloned();
            let referer = task.headers.get("Referer").cloned();
            let user_agent = task.headers.get("User-Agent").cloned();

            if let Err(e) = crate::media::apply_platform_naming_template(
                &self.app,
                &settings,
                &mut task,
                &final_output,
                cookie.as_deref(),
                referer.as_deref(),
                user_agent.as_deref(),
                &naming_templates,
            )
            .await
            {
                tracing::warn!(task_id = %task.id, error = %e, "媒体任务平台命名模板重命名失败");
            }

            if !task.file_name.contains('.') {
                let current_disk_path = Path::new(&task.destination).join(&task.file_name);
                let new_file_name = format!("{}.mp4", task.file_name);
                let new_disk_path = Path::new(&task.destination).join(&new_file_name);
                if current_disk_path.exists() && !new_disk_path.exists() {
                    if let Ok(_) = fs::rename(&current_disk_path, &new_disk_path).await {
                        task.file_name = new_file_name;
                        task.category = category(&task.file_name);
                    }
                }
            }

            let _ = self.store.upsert_task(&task).await;
            self.emit_task("updated", &task);
        }

        Ok(task)
    }

    async fn download_stream(
        &self,
        mut task: DownloadTask,
        client: &reqwest::Client,
        temp: &Path,
        token: CancellationToken,
        task_limiter: Arc<RateLimiter>,
    ) -> Result<DownloadTask, String> {
        let runtime_options = self.runtime_task_options(&task).await;
        let existing = fs::metadata(temp).await.map(|m| m.len()).unwrap_or(0);
        let mut request = client.get(&task.url).header(ACCEPT_ENCODING, "identity");
        for (name, value) in &task.headers {
            request = request.header(name, value);
        }
        if existing > 0 {
            request = request.header(RANGE, format!("bytes={existing}-"));
        }
        let response = request.send().await.map_err(friendly_reqwest)?;
        let append = existing > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        if !response.status().is_success() {
            return Err(format!("服务器返回 HTTP {}", response.status()));
        }
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(temp)
            .await
            .map_err(|e| e.to_string())?;
        let write_buffer_size = if self.settings().await.low_memory_mode {
            64 * 1024
        } else {
            1024 * 1024
        };
        let mut file = BufWriter::with_capacity(write_buffer_size, file);
        task.downloaded_bytes = if append { existing } else { 0 };
        if let Some(segment) = task.segments.first_mut() {
            segment.downloaded_bytes = task.downloaded_bytes;
        }
        let mut stream = response.bytes_stream();
        let mut sample = ProgressSample::new(task.downloaded_bytes);
        // 周期性磁盘空间检查状态：写入首字节前及每下载 10MB 或每 5 秒（取先到者）检查一次。
        let mut last_disk_check_at = Instant::now();
        let mut bytes_since_disk_check: u64 = DISK_CHECK_BYTES_INTERVAL;
        // SQLite 持久化节流：UI 事件保持 250ms，DB 写入降至至多 1 次/秒且仅在进度变化时写。
        // 续传以临时文件实际长度（fs::metadata）为准，DB 短暂滞后无正确性影响。
        let mut last_persist_at = Instant::now();
        let mut last_persisted_bytes = task.downloaded_bytes;
        while let Some(chunk) = stream.next().await {
            if token.is_cancelled() {
                file.flush().await.ok();
                return Err("任务已暂停".into());
            };
            let chunk = chunk.map_err(friendly_body_error)?;
            self.limit_with_cancel(&task.id, chunk.len() as u64, &task_limiter, &token)
                .await;
            if token.is_cancelled() {
                file.flush().await.ok();
                return Err("任务已暂停".into());
            }
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            task.downloaded_bytes += chunk.len() as u64;
            if let Some(segment) = task.segments.first_mut() {
                segment.downloaded_bytes = task.downloaded_bytes;
            }
            bytes_since_disk_check = bytes_since_disk_check.saturating_add(chunk.len() as u64);
            if bytes_since_disk_check >= DISK_CHECK_BYTES_INTERVAL
                || last_disk_check_at.elapsed() >= DISK_CHECK_TIME_INTERVAL
            {
                if let Err((available, required)) = check_disk_space_once(
                    &task.destination,
                    task.total_bytes,
                    task.downloaded_bytes,
                ) {
                    // 磁盘空间不足：取消所有活动连接、保留分片、置为 PausedByLowDisk、发事件。
                    token.cancel();
                    file.flush().await.ok();
                    task.status = TaskStatus::PausedByLowDisk;
                    task.speed = 0;
                    task.eta_seconds = None;
                    task.active_connections = 0;
                    if let Some(segment) = task.segments.first_mut() {
                        if segment.status == "downloading" {
                            segment.status = "paused".into();
                        }
                    }
                    task.error = Some(format!(
                        "磁盘空间不足（可用 {} 字节，需要 {} 字节），已暂停",
                        available, required
                    ));
                    self.store.upsert_task(&task).await?;
                    self.emit_task("updated", &task);
                    let _ = self.app.emit(
                        "task-paused-by-low-disk",
                        LowDiskPayload {
                            task_id: task.id.clone(),
                            available_bytes: available,
                            required_bytes: required,
                        },
                    );
                    return Err(format!(
                        "{LOW_DISK_PREFIX}磁盘空间不足（可用 {} 字节，需要 {} 字节）",
                        available, required
                    ));
                }
                last_disk_check_at = Instant::now();
                bytes_since_disk_check = 0;
            }
            if sample.should_emit(task.downloaded_bytes) {
                runtime_options.apply(&mut task).await;
                sample.apply(&mut task);
                if task.downloaded_bytes != last_persisted_bytes
                    && last_persist_at.elapsed() >= Duration::from_secs(1)
                {
                    // 进度持久化失败不中断传输：文件写入不受影响，续传依据是
                    // 临时文件长度而非 DB 进度；持续故障由完成/失败收尾持久化暴露。
                    match self.store.upsert_task(&task).await {
                        Ok(()) => {
                            last_persist_at = Instant::now();
                            last_persisted_bytes = task.downloaded_bytes;
                        }
                        Err(error) => {
                            tracing::warn!(task_id = %task.id, error = %error, "进度持久化失败，下载继续");
                        }
                    }
                }
                self.emit_task("updated", &task);
            }
        }
        file.flush().await.map_err(|e| e.to_string())?;
        // 显式终长校验：已知总长时，服务器提前断流（干净 EOF）不得被当作下载完成，
        // 返回错误进入任务级重试，续传从临时文件当前长度继续；未知长度（total=0）跳过。
        // 分片路径已有合并前逐分片校验，这里是单连接路径的对应防线。
        if task.total_bytes > 0 && task.downloaded_bytes != task.total_bytes {
            return Err(format!(
                "下载提前结束（已接收 {} 字节 / 共 {} 字节），将自动续传",
                task.downloaded_bytes, task.total_bytes
            ));
        }
        task.active_connections = 0;
        if let Some(segment) = task.segments.first_mut() {
            segment.status = "completed".into();
        }
        runtime_options.apply(&mut task).await;
        Ok(task)
    }

    async fn download_segments(
        &self,
        mut task: DownloadTask,
        client: &reqwest::Client,
        temp: &Path,
        total: u64,
        connections: u8,
        token: CancellationToken,
        task_limiter: Arc<RateLimiter>,
    ) -> Result<DownloadTask, String> {
        let connections = connections.clamp(1, 32);
        let ranges = planned_segment_ranges(&task, total, connections);
        let mut initial = 0u64;
        let mut initial_windows = Vec::new();
        let mut window_id_counter = 1u64;
        let mut runtimes = Vec::with_capacity(ranges.len());

        for &(index, start, end) in &ranges {
            let legacy_part = PathBuf::from(format!("{}.part{index}", temp.to_string_lossy()));
            let expected = end - start + 1;
            let mut prefix_bytes = fs::metadata(&legacy_part)
                .await
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            if prefix_bytes > expected {
                fs::remove_file(&legacy_part)
                    .await
                    .map_err(|error| format!("无法清理异常分片 #{}：{error}", index + 1))?;
                prefix_bytes = 0;
            }
            let mut downloaded = prefix_bytes;
            if prefix_bytes < expected {
                let layout =
                    select_window_layout(temp, index, start + prefix_bytes, end, prefix_bytes)
                        .await;
                for &(ordinal, window_start, window_end) in &layout {
                    let path = window_part_path(temp, index, window_start);
                    let expected_window = window_end - window_start + 1;
                    let mut existing_bytes = fs::metadata(&path)
                        .await
                        .map(|metadata| metadata.len())
                        .unwrap_or(0);
                    if existing_bytes > expected_window {
                        fs::remove_file(&path).await.map_err(|error| {
                            format!("无法清理异常续接窗口 #{}：{error}", index + 1)
                        })?;
                        existing_bytes = 0;
                    }
                    downloaded = downloaded.saturating_add(existing_bytes);
                    let is_completed = existing_bytes >= expected_window;
                    initial_windows.push(RangeWindow {
                        id: window_id_counter,
                        segment_index: index,
                        ordinal,
                        start_byte: window_start,
                        end_byte: window_end,
                        existing_bytes,
                        path,
                        status: if is_completed {
                            WindowStatus::Completed
                        } else {
                            WindowStatus::Pending
                        },
                    });
                    window_id_counter += 1;
                }
            }
            initial = initial.saturating_add(downloaded);
            let status = if downloaded == expected {
                SEGMENT_COMPLETED
            } else {
                SEGMENT_PENDING
            };
            runtimes.push(SegmentRuntime::new(index, start, end, downloaded, status));
        }
        initial_windows.sort_by_key(|w| (w.ordinal, w.segment_index));
        let coordinator = Arc::new(WorkStealingCoordinator::new(temp, initial_windows));

        let runtimes = Arc::new(runtimes);
        let progress = Arc::new(AtomicU64::new(initial));
        let adaptive = Arc::new(AdaptiveConnectionGate::new(connections));
        let is_cloud_or_high_conn = connections >= 16
            || task.url.contains("mypikpak.com")
            || task.url.contains("pikpak")
            || task.url.contains("quark.cn")
            || task.url.contains("baidupcs.com")
            || task.url.contains("123pan")
            || task.url.contains("lanzou");
        if is_cloud_or_high_conn {
            adaptive.user_disabled.store(1, Ordering::Relaxed);
        }
        task.downloaded_bytes = initial;
        task.segments = snapshot_segments(&runtimes);
        task.active_connections = 0;
        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);

        let runtime_options = self.runtime_task_options(&task).await;
        let reporter_stop = CancellationToken::new();
        // 低盘暂停的共享状态：disk_checker 检测到空间不足时置位，
        // 主循环据此跳过默认 Paused 处理，改为 PausedByLowDisk 并发事件。
        let low_disk_paused = Arc::new(AtomicBool::new(false));
        let low_disk_available = Arc::new(AtomicU64::new(0));
        let low_disk_required = Arc::new(AtomicU64::new(0));
        let reporter = {
            let stop = reporter_stop.clone();
            let cancel = token.clone();
            let progress = progress.clone();
            let runtimes = runtimes.clone();
            let adaptive = adaptive.clone();
            let store = self.store.clone();
            let app = self.app.clone();
            let runtime_options = runtime_options.clone();
            let mut snapshot = task.clone();
            tokio::spawn(async move {
                let mut sample = ProgressSample::new(initial);
                // Task 18: task-connections 事件节流状态。
                // 频率：每秒一次，不更高（AGENTS.md §8）。
                // 速度计算基于 downloaded_bytes 原子量的真实采样（AGENTS.md §3）。
                let mut last_conn_emit_at = Instant::now();
                let mut last_conn_bytes: Vec<u64> = runtimes
                    .iter()
                    .map(|r| r.downloaded_bytes.load(Ordering::Relaxed))
                    .collect();
                // SQLite 持久化节流：UI 事件保持 250ms 不变，DB 写入降至至多 1 次/秒，
                // 且仅在进度或分片状态变化时写（下载停滞时不再空写）。
                // 续传以分片文件实际长度为准（fs::metadata），DB 进度短暂滞后无正确性影响；
                // 结束状态（暂停/失败/低磁盘/完成）由主循环收尾时统一持久化。
                let mut last_persist_at = Instant::now();
                let mut last_persisted_bytes = initial;
                let mut last_persist_statuses: Vec<u8> = Vec::new();
                loop {
                    tokio::select! {
                        _ = stop.cancelled() => break,
                        _ = cancel.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                    }
                    snapshot.downloaded_bytes = progress.load(Ordering::Relaxed);
                    snapshot.segments = snapshot_segments(&runtimes);
                    runtime_options.apply(&mut snapshot).await;
                    sample.apply(&mut snapshot);
                    adaptive.observe(snapshot.speed);
                    let active_conn_count: u32 = runtimes.iter().map(|r| r.active_windows.load(Ordering::Relaxed) as u32).sum();
                    snapshot.active_connections = active_conn_count.min(connections as u32) as u8;
                    let statuses: Vec<u8> = runtimes
                        .iter()
                        .map(|r| r.status.load(Ordering::Relaxed))
                        .collect();
                    if (snapshot.downloaded_bytes != last_persisted_bytes
                        || statuses != last_persist_statuses)
                        && last_persist_at.elapsed() >= Duration::from_secs(1)
                        && store.upsert_task(&snapshot).await.is_ok()
                    {
                        last_persist_at = Instant::now();
                        last_persisted_bytes = snapshot.downloaded_bytes;
                        last_persist_statuses = statuses;
                    }
                    let _ = app.emit(
                        "task-updated",
                        TaskProgressEvent {
                            task: snapshot.clone(),
                            event: "updated".into(),
                        },
                    );
                    // Task 18: 每秒一次推送 task-connections，仅在该任务处于
                    // Downloading 状态时发出（暂停/完成后由主循环负责最终事件）。
                    if last_conn_emit_at.elapsed() >= Duration::from_secs(1) {
                        let elapsed_secs = last_conn_emit_at.elapsed().as_secs_f64();
                        let segments = snapshot_segment_statuses(
                            &runtimes,
                            &last_conn_bytes,
                            elapsed_secs,
                            false,
                        );
                        last_conn_emit_at = Instant::now();
                        last_conn_bytes = runtimes
                            .iter()
                            .map(|r| r.downloaded_bytes.load(Ordering::Relaxed))
                            .collect();
                        let _ = app.emit(
                            "task-connections",
                            TaskConnectionsEvent {
                                task_id: snapshot.id.clone(),
                                segments,
                                timestamp: now_millis(),
                            },
                        );
                    }
                }
            })
        };

        // 周期性磁盘空间检查任务：每 250ms 评估一次触发条件，
        // 每下载 10MB 或每 5 秒（取先到者）执行一次实际检查。
        // 检测到空间不足时仅设置标志并取消下载 token，DB 与事件由主循环统一处理，
        // 避免与 reporter / 主循环的 DB 写入竞争。
        let disk_checker = {
            let stop = reporter_stop.clone();
            let cancel = token.clone();
            let progress = progress.clone();
            let destination = task.destination.clone();
            let total_bytes = total;
            let initial_bytes = initial;
            let low_disk_paused = low_disk_paused.clone();
            let low_disk_available = low_disk_available.clone();
            let low_disk_required = low_disk_required.clone();
            tokio::spawn(async move {
                let mut last_check_at = Instant::now()
                    .checked_sub(DISK_CHECK_TIME_INTERVAL)
                    .unwrap_or_else(Instant::now);
                let mut last_check_bytes = initial_bytes;
                loop {
                    tokio::select! {
                        _ = stop.cancelled() => break,
                        _ = cancel.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                    }
                    if low_disk_paused.load(Ordering::Relaxed) {
                        break;
                    }
                    let progress_now = progress.load(Ordering::Relaxed);
                    let bytes_since = progress_now.saturating_sub(last_check_bytes);
                    if bytes_since < DISK_CHECK_BYTES_INTERVAL
                        && last_check_at.elapsed() < DISK_CHECK_TIME_INTERVAL
                    {
                        continue;
                    }
                    match check_disk_space_once(&destination, total_bytes, progress_now) {
                        Ok(()) => {
                            last_check_at = Instant::now();
                            last_check_bytes = progress_now;
                        }
                        Err((available, required)) => {
                            // 仅设置标志 + 取消 token，DB 与事件由主循环处理。
                            low_disk_paused.store(true, Ordering::Relaxed);
                            low_disk_available.store(available, Ordering::Relaxed);
                            low_disk_required.store(required, Ordering::Relaxed);
                            cancel.cancel();
                            break;
                        }
                    }
                }
            })
        };

        let runtime_settings = self.settings().await;
        let write_buffer_size = if runtime_settings.low_memory_mode {
            64 * 1024
        } else {
            1024 * 1024
        };
        // Task 14: 连接级重试使用 effective_retry_policy 的 max_retries 和退避策略。
        let segment_retry_policy = effective_retry_policy(&task, &runtime_settings);
        let segment_max_retries = segment_retry_policy.max_retries;
        let task_headers = task.headers.clone();
        let task_url = task.url.clone();
        let task_if_range = if initial > 0 {
            task.etag.clone().or_else(|| task.last_modified.clone())
        } else {
            None
        };
        let token_outer = token.clone();
        let progress_outer = progress.clone();
        let runtimes_outer = runtimes.clone();
        let token_for_workers = token_outer.clone();
        let progress_for_workers = progress_outer.clone();
        let runtimes_for_workers = runtimes_outer.clone();
        let runtime_options_for_workers = runtime_options.clone();
        let adaptive_for_workers = adaptive.clone();
        let bandwidth_for_workers = self.bandwidth_scheduler.clone();
        let task_id_for_workers = task.id.clone();

        let mut worker_handles = Vec::with_capacity(connections as usize);
        for _worker_idx in 0..connections {
            let coordinator = coordinator.clone();
            let client = client.clone();
            let headers = task_headers.clone();
            let url = task_url.clone();
            let if_range = task_if_range.clone();
            let token = token_for_workers.clone();
            let progress = progress_for_workers.clone();
            let runtimes = runtimes_for_workers.clone();
            let limiter = task_limiter.clone();
            let runtime_options = runtime_options_for_workers.clone();
            let adaptive = adaptive_for_workers.clone();
            let bandwidth = bandwidth_for_workers.clone();
            let task_id = task_id_for_workers.clone();
            let write_buffer_size = write_buffer_size;
            let segment_max_retries = segment_max_retries;
            let segment_retry_policy = segment_retry_policy.clone();

            worker_handles.push(tokio::spawn(async move {
                if _worker_idx > 0 {
                    if url.contains("baidupcs.com") || url.contains("pan.baidu.com") {
                        tokio::time::sleep(Duration::from_millis(_worker_idx as u64 * 60)).await;
                    } else if url.contains("mypikpak.com") {
                        tokio::time::sleep(Duration::from_millis(_worker_idx as u64 * 40)).await;
                    }
                }
                loop {
                    if token.is_cancelled() {
                        return Err("任务已暂停".to_string());
                    }

                    let (window, handle) = match coordinator.claim_or_steal_work().await {
                        Some(work) => work,
                        None => {
                            if coordinator.is_all_completed().await {
                                return Ok(());
                            }
                            tokio::select! {
                                _ = token.cancelled() => return Err("任务已暂停".to_string()),
                                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
                            }
                            if coordinator.is_all_completed().await {
                                return Ok(());
                            }
                            continue;
                        }
                    };

                    let index = window.segment_index;
                    let part = window.path.clone();
                    let start_byte = window.start_byte;
                    let existing_bytes = window.existing_bytes;
                    let runtime = match runtimes.iter().find(|segment| segment.index == index) {
                        Some(r) => r,
                        None => {
                            coordinator.finish_window(window.id, false, existing_bytes).await;
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            continue;
                        }
                    };

                    let file = match OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&part)
                        .await
                    {
                        Ok(f) => f,
                        Err(_err) => {
                            coordinator.finish_window(window.id, false, existing_bytes).await;
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            continue;
                        }
                    };
                    let mut file = BufWriter::with_capacity(write_buffer_size, file);

                    let transfer_result = async {
                        let mut next_start = start_byte.saturating_add(existing_bytes);
                        let mut retry_count = 0u32;
                        // 云盘直链失效熔断状态：空响应计数 + 停滞检查点。
                        // PikPak 等云盘直链过期后 CDN 常返回 206 空 body 或
                        // 半途掐断（而非明确 403），若无熔断会陷入
                        // "重连成功但 0 字节" 的无限循环（表现为 0 速度）。
                        let mut empty_step_count = 0u32;
                        let mut stall_check_at = Instant::now();
                        let mut stall_progress_base = existing_bytes;
                        loop {
                            if token.is_cancelled() {
                                let _ = file.flush().await;
                                return Err("任务已暂停".to_string());
                            }
                            let current_end = handle.current_end();
                            if next_start > current_end {
                                let _ = file.flush().await;
                                return Ok(());
                            }

                            let permit = tokio::select! {
                                _ = token.cancelled() => return Err("任务已暂停".to_string()),
                                permit = adaptive.clone().acquire() => permit,
                            };
                            runtime.active_windows.fetch_add(1, Ordering::Relaxed);
                            runtime.status.store(SEGMENT_DOWNLOADING, Ordering::Relaxed);

                            let current_start = next_start;
                            let request_end = handle.current_end();
                            if current_start > request_end {
                                return Ok(());
                            }
                            let mut step_bytes_read = 0u64;
                            // 本步骤是否构成"直链失效证据"：服务器返回过有效 206（空 body/
                            // 中途掐断），或有效 Range 被伪 416 拒绝（PikPak 限速签名）。
                            // 连接级错误（断网、DNS、拒绝连接、403）不构成证据，走既有重试路径。
                            let mut server_responded_206 = false;

                            let step_result = async {
                                let mut request = client.get(&url);
                                for (name, value) in &headers {
                                    request = request.header(name, value);
                                }
                                request = request
                                    .header(ACCEPT_ENCODING, "identity")
                                    .header(RANGE, format!("bytes={current_start}-{request_end}"));
                                if let Some(value) = &if_range {
                                    request = request.header(IF_RANGE, value);
                                }
                                let response = request.send().await.map_err(friendly_reqwest)?;
                                if response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
                                    // 416 必须区分真伪：
                                    // - 真 416（start 已越过文件末尾）：该 Range 确实无法满足，
                                    //   视为分片已完整，正常收尾。
                                    // - 伪 416（start 在文件范围内却被拒）：PikPak 等 CDN 限速签名
                                    //   （实测响应头 Content-Range: bytes */total 正确、X-Xos-Err-Desc: 10）。
                                    //   若误判为"已完成"会陷入 0 进度无限快速循环（20Hz 空转），
                                    //   必须与空 206 一样计入直链失效熔断。
                                    if current_start >= total {
                                        return Ok(());
                                    }
                                    server_responded_206 = true;
                                    return Ok(());
                                }
                                if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                                    return Err(format!(
                                        "服务器返回 HTTP {}，无法安全续传分片 #{}",
                                        response.status(),
                                        index + 1
                                    ));
                                }
                                match parse_content_range(&response) {
                                    Some((actual_start, actual_end, actual_total))
                                        if actual_start == current_start
                                            && actual_end >= actual_start
                                            && actual_end <= request_end
                                            && (actual_total == total || total == 0) => {}
                                    _ => return Err("服务器返回了不匹配的 Content-Range".into()),
                                }
                                // 状态与 Content-Range 均校验通过：后续失败（空 body、
                                // 中途掐断）才可能是直链失效的表现。
                                server_responded_206 = true;
                                let mut stream = response.bytes_stream();
                                let mut idle_seconds = 0u8;
                                loop {
                                    let next = tokio::select! {
                                        _ = token.cancelled() => return Err("任务已暂停".into()),
                                        result = tokio::time::timeout(Duration::from_secs(1), stream.next()) => result,
                                    };
                                    let chunk = match next {
                                        Ok(Some(chunk)) => {
                                            idle_seconds = 0;
                                            chunk
                                        }
                                        Ok(None) => break,
                                        Err(_) if adaptive.should_yield(&permit) => {
                                            return Err(ADAPTIVE_YIELD.into())
                                        }
                                        Err(_) if idle_seconds >= 6 => {
                                            return Err(format!(
                                                "分片 #{} 连续 6 秒未收到数据，自动断点续连",
                                                index + 1
                                            ))
                                        }
                                        Err(_) => {
                                            idle_seconds = idle_seconds.saturating_add(1);
                                            continue;
                                        }
                                    };
                                    if adaptive.should_yield(&permit) {
                                        return Err(ADAPTIVE_YIELD.into());
                                    }
                                    let chunk = chunk.map_err(friendly_body_error)?;
                                    let mut chunk_slice = &chunk[..];
                                    let effective_end = handle.current_end();
                                    let current_cursor = next_start;

                                    if current_cursor > effective_end {
                                        break;
                                    }
                                    let remaining_in_window = effective_end.saturating_sub(current_cursor).saturating_add(1);
                                    if (chunk_slice.len() as u64) > remaining_in_window {
                                        chunk_slice = &chunk_slice[..remaining_in_window as usize];
                                    }
                                    let chunk_len = chunk_slice.len() as u64;
                                    if chunk_len == 0 {
                                        break;
                                    }

                                    bandwidth
                                        .acquire(
                                            &task_id,
                                            chunk_len,
                                            runtime_options.priority.load(Ordering::Relaxed),
                                            &token,
                                        )
                                        .await;
                                    if token.is_cancelled() {
                                        return Err("任务已暂停".into());
                                    }
                                    limiter
                                        .acquire_with_cancel(
                                            chunk_len,
                                            runtime_options.speed_limit.load(Ordering::Relaxed),
                                            &token,
                                        )
                                        .await;
                                    if token.is_cancelled() {
                                        return Err("任务已暂停".into());
                                    }
                                    file.write_all(chunk_slice)
                                        .await
                                        .map_err(|error| error.to_string())?;
                                    next_start += chunk_len;
                                    step_bytes_read += chunk_len;
                                    handle.downloaded_bytes.fetch_add(chunk_len, Ordering::Relaxed);
                                    runtime
                                        .downloaded_bytes
                                        .fetch_add(chunk_len, Ordering::Relaxed);
                                    progress.fetch_add(chunk_len, Ordering::Relaxed);

                                    if next_start > handle.current_end() {
                                        break;
                                    }
                                }
                                Ok::<(), String>(())
                            }.await;

                            drop(permit);
                            let remaining_active = runtime
                                .active_windows
                                .fetch_sub(1, Ordering::Relaxed)
                                .saturating_sub(1);
                            if remaining_active == 0
                                && runtime.status.load(Ordering::Relaxed) != SEGMENT_FAILED
                            {
                                runtime.status.store(SEGMENT_PENDING, Ordering::Relaxed);
                            }

                            // —— 云盘直链失效熔断（AGENTS.md §3：真实状态、不得无限空转）——
                            // 仅统计"服务器返回过有效 206 但 0 字节/近乎无进展"的步骤：
                            // 这是 PikPak 直链过期（206 空 body / 半途掐断）的确切签名。
                            // 连接级错误（断网、DNS、403）不构成直链失效证据，
                            // 走既有重试路径，避免把可重试网络错误误判为终态。
                            if server_responded_206 && !token.is_cancelled() {
                                let downloaded_now = handle.current_downloaded();
                                if step_bytes_read == 0 {
                                    empty_step_count += 1;
                                } else {
                                    empty_step_count = 0;
                                }
                                if downloaded_now.saturating_sub(stall_progress_base)
                                    >= STALL_RECOVERY_BYTES
                                {
                                    // 有实质进展：滚动重置停滞检查点
                                    stall_check_at = Instant::now();
                                    stall_progress_base = downloaded_now;
                                }
                                let window_remaining = handle
                                    .current_end()
                                    .saturating_sub(next_start)
                                    .saturating_add(1);
                                // 剩余不足 1MB 的收尾阶段不做停滞判定（无法再积累 1MB）
                                let stalled = stall_check_at.elapsed() >= STALL_TIMEOUT
                                    && window_remaining > STALL_RECOVERY_BYTES;
                                if empty_step_count >= MAX_EMPTY_STEPS {
                                    let _ = file.flush().await;
                                    runtime.set_last_error("连续空响应，直链疑似已失效");
                                    return Err(format!(
                                        "{CLOUD_LINK_DEAD_PREFIX}分片 #{} 连续 {} 次收到 0 字节响应，直链疑似已失效",
                                        index + 1,
                                        empty_step_count
                                    ));
                                }
                                if stalled {
                                    let _ = file.flush().await;
                                    runtime.set_last_error("长时间无实质进展，直链疑似已失效");
                                    return Err(format!(
                                        "{CLOUD_LINK_DEAD_PREFIX}分片 #{} 超过 {} 秒下载不足 {} 字节，直链疑似已失效",
                                        index + 1,
                                        STALL_TIMEOUT.as_secs(),
                                        STALL_RECOVERY_BYTES
                                    ));
                                }
                            }

                            match step_result {
                                Ok(()) => {
                                    file.flush().await.map_err(|error| error.to_string())?;
                                    retry_count = 0;
                                    let effective_end = handle.current_end();
                                    if next_start <= effective_end {
                                        let reconnect_delay = Duration::from_millis(
                                            8 + (index as u64 % 8).saturating_mul(7),
                                        );
                                        tokio::select! {
                                            _ = token.cancelled() => return Err("任务已暂停".into()),
                                            _ = tokio::time::sleep(reconnect_delay) => {}
                                        }
                                        continue;
                                    }
                                    break Ok(());
                                }
                                Err(error) if error == ADAPTIVE_YIELD => {
                                    file.flush().await.map_err(|flush| flush.to_string())?;
                                    continue;
                                }
                                Err(error) if token.is_cancelled() => {
                                    let _ = file.flush().await;
                                    return Err(error);
                                }
                                Err(error) if error.contains("503") || error.contains("429") => {
                                    file.flush().await.map_err(|flush| flush.to_string())?;
                                    runtime.set_last_error(&error);
                                    runtime.retrying.store(true, Ordering::Relaxed);
                                    let backoff_ms = 400 + (index as u64 % 8).saturating_mul(120);
                                    tokio::select! {
                                        _ = token.cancelled() => {
                                            runtime.retrying.store(false, Ordering::Relaxed);
                                            return Err("任务已暂停".into());
                                        },
                                        _ = tokio::time::sleep(Duration::from_millis(backoff_ms)) => {}
                                    }
                                    runtime.retrying.store(false, Ordering::Relaxed);
                                }
                                Err(error) if retry_count < segment_max_retries || step_bytes_read > 0 => {
                                    file.flush().await.map_err(|flush| flush.to_string())?;
                                    if step_bytes_read > 0 {
                                        retry_count = 0;
                                    } else {
                                        retry_count += 1;
                                    }
                                    runtime.retry_count.store(retry_count, Ordering::Relaxed);
                                    runtime.set_last_error(&error);
                                    runtime.retrying.store(true, Ordering::Relaxed);
                                    let policy_delay_ms = compute_backoff(&segment_retry_policy, retry_count);
                                    let jitter_ms = (index as u64).saturating_mul(11);
                                    let delay_ms = policy_delay_ms.saturating_add(jitter_ms);
                                    tokio::select! {
                                        _ = token.cancelled() => {
                                            runtime.retrying.store(false, Ordering::Relaxed);
                                            return Err("任务已暂停".into());
                                        },
                                        _ = tokio::time::sleep(Duration::from_millis(delay_ms)) => {}
                                    }
                                    runtime.retrying.store(false, Ordering::Relaxed);
                                }
                                Err(error) => {
                                    runtime.set_last_error(&error);
                                    return Err(format!(
                                        "分片 #{} 连续重试 {} 次后仍失败：{}",
                                        index + 1,
                                        retry_count,
                                        error
                                    ));
                                }
                            }
                        }
                    }.await;

                    let actual_downloaded = handle.current_downloaded();
                    let success = transfer_result.is_ok();
                    coordinator.finish_window(window.id, success, actual_downloaded).await;

                    let expected_segment_len = runtime.end_byte - runtime.start_byte + 1;
                    let status = if runtime.downloaded_bytes.load(Ordering::Relaxed) >= expected_segment_len {
                        SEGMENT_COMPLETED
                    } else if runtime.active_windows.load(Ordering::Relaxed) > 0 {
                        SEGMENT_DOWNLOADING
                    } else {
                        SEGMENT_PENDING
                    };
                    runtime.status.store(status, Ordering::Relaxed);

                    if let Err(err) = &transfer_result {
                        // 云盘直链失效哨兵必须穿透窗口级重试循环上抛：
                        // 否则停滞熔断触发后被此处吞掉，worker 会无限重新
                        // 领取同一窗口并再次熔断（表现为永久 0 速度），
                        // spawn_worker 的自动刷新直链逻辑永远无法触发。
                        if err.starts_with(CLOUD_LINK_DEAD_PREFIX) {
                            return Err(err.clone());
                        }
                        if token.is_cancelled() {
                            return Err("任务已暂停".to_string());
                        }
                        tokio::select! {
                            _ = token.cancelled() => return Err("任务已暂停".to_string()),
                            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                        }
                    }
                }
            }));
        }

        let mut worker_error: Option<String> = None;
        for handle in worker_handles {
            match handle.await {
                Ok(Err(error)) => {
                    // 云盘直链失效哨兵优先于其他错误：直链过期时部分
                    // worker 可能先以"重试耗尽"失败，若不优先保留哨兵，
                    // 会掩盖可自动刷新恢复的真实原因。
                    let is_link_dead = error.starts_with(CLOUD_LINK_DEAD_PREFIX);
                    match &worker_error {
                        Some(prev) if !prev.starts_with(CLOUD_LINK_DEAD_PREFIX) && is_link_dead => {
                            worker_error = Some(error);
                            token.cancel();
                        }
                        None => {
                            worker_error = Some(error);
                            token.cancel();
                        }
                        _ => {
                            token.cancel();
                        }
                    }
                }
                Err(join_err) => {
                    if worker_error.is_none() {
                        worker_error = Some(format!("Worker 异常终止: {join_err}"));
                        token.cancel();
                    }
                }
                Ok(Ok(())) => {}
            }
        }
        reporter_stop.cancel();
        let _ = reporter.await;
        let _ = disk_checker.await;

        task.downloaded_bytes = progress.load(Ordering::Relaxed);
        task.segments = snapshot_segments(&runtimes);
        task.active_connections = 0;
        runtime_options.apply(&mut task).await;
        // 磁盘空间不足：disk_checker 已设置标志并取消 token，
        // 主循环统一负责 DB 写入与事件发送，避免与 reporter 竞争。
        if low_disk_paused.load(Ordering::Relaxed) {
            let available = low_disk_available.load(Ordering::Relaxed);
            let required = low_disk_required.load(Ordering::Relaxed);
            task.status = TaskStatus::PausedByLowDisk;
            task.speed = 0;
            task.eta_seconds = None;
            task.active_connections = 0;
            for segment in &mut task.segments {
                if segment.status == "downloading" {
                    segment.status = "paused".into();
                }
            }
            task.error = Some(format!(
                "磁盘空间不足（可用 {} 字节，需要 {} 字节），已暂停",
                available, required
            ));
            self.store.upsert_task(&task).await?;
            self.emit_task("updated", &task);
            // Task 18: 推送最终 task-connections 事件，所有未完成分片 → Paused。
            self.emit_task_connections_final(&task.id, &runtimes, true);
            let _ = self.app.emit(
                "task-paused-by-low-disk",
                LowDiskPayload {
                    task_id: task.id.clone(),
                    available_bytes: available,
                    required_bytes: required,
                },
            );
            return Err(format!(
                "{LOW_DISK_PREFIX}磁盘空间不足（可用 {} 字节，需要 {} 字节）",
                available, required
            ));
        }
        if let Some(error) = worker_error {
            if error.starts_with(CLOUD_LINK_DEAD_PREFIX) {
                // 云盘直链失效：保持 Downloading 状态上抛哨兵错误，
                // 由 spawn_worker 用 cloud_refresh 元数据自动刷新直链后
                // 无缝续传（分片全部保留）。不置 Paused——这不是用户暂停，
                // 也不是终态失败。
                task.error = Some("下载直链已过期，正在尝试自动刷新".into());
                self.store.upsert_task(&task).await?;
                self.emit_task("updated", &task);
                self.emit_task_connections_final(&task.id, &runtimes, true);
                return Err(error);
            }
            if token.is_cancelled() {
                task.status = TaskStatus::Paused;
                task.speed = 0;
                task.eta_seconds = None;
                task.active_connections = 0;
                for segment in &mut task.segments {
                    if segment.status == "downloading" {
                        segment.status = "paused".into();
                    }
                }
            }
            self.store.upsert_task(&task).await?;
            self.emit_task("updated", &task);
            // Task 18: 推送最终 task-connections 事件。
            // - 用户暂停（token cancelled）：所有未完成分片 → Paused。
            // - 分片失败（!cancelled）：保留真实 Failed 状态。
            self.emit_task_connections_final(&task.id, &runtimes, token.is_cancelled());
            return Err(error);
        }
        if token.is_cancelled() {
            // Task 18: 推送最终 task-connections 事件。
            // 此场景下所有分片可能已完成（downloaded == total），保留真实状态。
            self.emit_task_connections_final(&task.id, &runtimes, false);
            return Err("任务已暂停".into());
        }

        // 合并前再校验一次空间：合并需要写入完整文件大小的临时文件。
        // 空间不足时不执行合并，保留已下载分片，任务进入 PausedByLowDisk，
        // 发出 merge-blocked-by-low-disk 事件提示用户清理或更换目录。
        let merge_available = query_available_space_for_destination(&task.destination);
        let merge_required = total;
        if merge_available < merge_required {
            task.status = TaskStatus::PausedByLowDisk;
            task.speed = 0;
            task.eta_seconds = None;
            task.active_connections = 0;
            task.error = Some(format!(
                "合并所需空间不足（可用 {} 字节，需要 {} 字节），请清理或更换目录",
                merge_available, merge_required
            ));
            self.store.upsert_task(&task).await?;
            self.emit_task("updated", &task);
            let _ = self.app.emit(
                "merge-blocked-by-low-disk",
                LowDiskPayload {
                    task_id: task.id.clone(),
                    available_bytes: merge_available,
                    required_bytes: merge_required,
                },
            );
            return Err(format!(
                "{LOW_DISK_PREFIX}合并所需空间不足（可用 {} 字节，需要 {} 字节）",
                merge_available, merge_required
            ));
        }

        let merge = PathBuf::from(format!("{}.merge", temp.to_string_lossy()));
        let mut output = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&merge)
            .await
            .map_err(|error| error.to_string())?;
        let merge_buffer_size = if self.settings().await.low_memory_mode {
            64 * 1024
        } else {
            1024 * 1024
        };
        let mut buffer = vec![0; merge_buffer_size];
        let mut parts_to_cleanup = Vec::new();

        for &(index, start, end) in &ranges {
            let expected = end - start + 1;
            let legacy_part = PathBuf::from(format!("{}.part{index}", temp.to_string_lossy()));
            let prefix_bytes = fs::metadata(&legacy_part)
                .await
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            let mut merged_bytes = 0u64;
            if prefix_bytes > 0 {
                if let Err(err) =
                    append_part(&mut output, &legacy_part, prefix_bytes, &mut buffer).await
                {
                    let _ = fs::remove_file(&merge).await;
                    return Err(err);
                }
                merged_bytes = prefix_bytes;
                parts_to_cleanup.push(legacy_part);
            }
            if prefix_bytes < expected {
                let ordered = coordinator.get_ordered_windows_for_segment(index).await;
                if ordered.is_empty() {
                    let _ = fs::remove_file(&merge).await;
                    return Err(format!("分片 #{} 缺少切片数据", index + 1));
                }
                let mut cursor = start + prefix_bytes;
                for window in ordered {
                    if window.start_byte != cursor {
                        let _ = fs::remove_file(&merge).await;
                        return Err(format!(
                            "分片 #{} 存在切片间隙：期望偏移 {}，实际窗口偏移 {}",
                            index + 1,
                            cursor,
                            window.start_byte
                        ));
                    }
                    let window_bytes = window.end_byte - window.start_byte + 1;
                    if let Err(err) =
                        append_part(&mut output, &window.path, window_bytes, &mut buffer).await
                    {
                        let _ = fs::remove_file(&merge).await;
                        return Err(err);
                    }
                    merged_bytes = merged_bytes.saturating_add(window_bytes);
                    cursor = window.end_byte + 1;
                    parts_to_cleanup.push(window.path);
                }
            }
            if merged_bytes != expected {
                let _ = fs::remove_file(&merge).await;
                return Err(format!(
                    "分片 #{} 大小不完整（应为 {} 字节，实际 {} 字节）",
                    index + 1,
                    expected,
                    merged_bytes
                ));
            }
        }
        if let Err(err) = output.flush().await.map_err(|error| error.to_string()) {
            let _ = fs::remove_file(&merge).await;
            return Err(err);
        }
        if let Err(err) = fs::rename(&merge, temp)
            .await
            .map_err(|error| error.to_string())
        {
            let _ = fs::remove_file(&merge).await;
            return Err(err);
        }

        for part_path in parts_to_cleanup {
            let _ = fs::remove_file(part_path).await;
        }

        task.downloaded_bytes = total;
        task.segments = snapshot_segments(&runtimes);
        Ok(task)
    }

    /// 应用任务级加权全局限速和单任务限速。
    ///
    /// 在限速器 sleep 步进之间检查 `cancel` 信号，保证 50ms 内响应暂停/取消。
    /// 这是 AGENTS.md §3"暂停、取消、重试和程序退出必须停止所有活动连接"的实现。
    async fn limit_with_cancel(
        &self,
        task_id: &str,
        bytes: u64,
        task_limiter: &RateLimiter,
        cancel: &CancellationToken,
    ) {
        let task_limit = self
            .task_runtime
            .read()
            .await
            .get(task_id)
            .map(|runtime| runtime.speed_limit.load(Ordering::Relaxed))
            .unwrap_or(0);
        let priority = self
            .task_runtime
            .read()
            .await
            .get(task_id)
            .map(|runtime| runtime.priority.load(Ordering::Relaxed))
            .unwrap_or(0);
        self.bandwidth_scheduler
            .acquire(task_id, bytes, priority, cancel)
            .await;
        task_limiter
            .acquire_with_cancel(bytes, task_limit, cancel)
            .await;
    }

    async fn runtime_task_options(&self, task: &DownloadTask) -> Arc<RuntimeTaskOptions> {
        self.task_runtime
            .read()
            .await
            .get(&task.id)
            .cloned()
            .unwrap_or_else(|| Arc::new(RuntimeTaskOptions::new(task)))
    }

    async fn reserve_output_path(&self, task: &mut DownloadTask) -> Result<PathBuf, String> {
        let _reservation = self.path_reservation.lock().await;
        let reserved = self
            .store
            .list_tasks()
            .await?
            .into_iter()
            .filter(|other| {
                other.id != task.id
                    && !matches!(other.status, TaskStatus::Completed | TaskStatus::Cancelled)
            })
            .map(|other| path_key(&PathBuf::from(other.destination).join(other.file_name)))
            .collect::<HashSet<_>>();
        let output = resolve_output_path(task, &reserved)?;
        let Some(file_name) = output.file_name().and_then(|value| value.to_str()) else {
            return Err("无法确定目标文件名".into());
        };
        task.file_name = file_name.to_owned();
        task.category = category(&task.file_name);
        self.store.upsert_task(task).await?;
        Ok(output)
    }

    async fn wait_for_network(&self, task: &DownloadTask, token: CancellationToken) -> bool {
        loop {
            tokio::select! {
                _ = token.cancelled() => return false,
                _ = tokio::time::sleep(Duration::from_secs(3)) => {}
            }
            // Task 31：网络探测应与下载使用相同的代理设置，否则会出现
            // "探测通了但下载失败"或"探测失败但下载其实可用"的误判。
            let client = if task.proxy_override.is_some() {
                let settings = self.settings().await;
                match build_task_client(&settings, task) {
                    Ok(c) => c,
                    Err(_) => return false,
                }
            } else {
                self.client.read().await.clone()
            };
            let mut request = client.head(&task.url).header(ACCEPT_ENCODING, "identity");
            for (name, value) in &task.headers {
                request = request.header(name, value);
            }
            let result = tokio::select! {
                _ = token.cancelled() => return false,
                result = tokio::time::timeout(Duration::from_secs(10), request.send()) => result,
            };
            if result.is_ok_and(|response| response.is_ok()) {
                return true;
            }
        }
    }

    async fn perform_completion_action(&self, mut task: DownloadTask) {
        // 统一将错误转换为 String，便于在任务错误字段中展示中文消息。
        // 旧变体（None/OpenFolder/RunFile/Shutdown/Hibernate）直接处理；
        // 新变体（Quit/RunCommand/CopyTo/MoveTo）交给 completion_action::run_extended_action。
        let result: Result<(), String> = match &task.completion_action {
            CompletionAction::None => return,
            CompletionAction::OpenFolder => {
                open::that(&task.destination).map_err(|e| e.to_string())
            }
            CompletionAction::RunFile if task.source == "desktop" => {
                open::that(PathBuf::from(&task.destination).join(&task.file_name))
                    .map_err(|e| e.to_string())
            }
            CompletionAction::RunFile => {
                task.error = Some("已阻止非桌面任务自动运行文件".into());
                let _ = self.store.upsert_task(&task).await;
                self.emit_task("updated", &task);
                return;
            }
            // Shutdown / Hibernate 复用 PowerAction 的 shutdown.exe 调用。
            // 这里直接执行，不进入 PowerAction 倒计时流程；如果用户在 PowerAction
            // 已 Armed/Countdown 时又给任务设置了 Shutdown，倒计时流程会优先生效，
            // 此处直接执行 shutdown.exe 不会与 PowerAction 状态机产生冲突。
            CompletionAction::Shutdown => execute_power_action(PowerAction::Shutdown),
            CompletionAction::Hibernate => execute_power_action(PowerAction::Hibernate),
            // Task 17: 新增完成动作委托给 completion_action 模块。
            // 模板上下文从任务构建；collision_policy 用于 CopyTo/MoveTo 重名处理。
            // 命令失败不破坏下载：返回 Err 时仅写入 task.error，任务仍为 Completed。
            CompletionAction::Quit
            | CompletionAction::RunCommand { .. }
            | CompletionAction::CopyTo { .. }
            | CompletionAction::MoveTo { .. } => {
                let context = completion_action::TemplateContext::from_task(&task);
                completion_action::run_extended_action(
                    &task.completion_action,
                    &context,
                    task.collision_policy.clone(),
                    &self.app,
                )
                .await
            }
        };
        if let Err(error) = result {
            task.error = Some(format!("下载已完成，但完成动作失败：{error}"));
            let _ = self.store.upsert_task(&task).await;
            self.emit_task("updated", &task);
        }
    }

    /// 列出全部下载预设（Task 12）。
    pub async fn preset_list(&self) -> Result<Vec<DownloadPreset>, String> {
        self.store.download_preset_list().await
    }

    /// 新增自定义预设。`connections` 必须是 1/2/4/8/16/32 之一，`is_builtin` 强制为 `false`。
    /// 同 id 已存在时由 SQLite 主键约束返回中文错误。
    pub async fn preset_add(&self, mut preset: DownloadPreset) -> Result<DownloadPreset, String> {
        validate_preset_connections(preset.connections)?;
        if preset.id.trim().is_empty() {
            return Err("预设 ID 不能为空".into());
        }
        if preset.name.trim().is_empty() {
            return Err("预设名称不能为空".into());
        }
        validate_preset_scheduled_at(preset.scheduled_at.as_deref())?;
        // 自定义预设强制 is_builtin = false，避免前端伪造内置预设。
        preset.is_builtin = false;
        match self.store.download_preset_add(preset.clone()).await {
            Ok(saved) => Ok(saved),
            Err(error) if error.contains("UNIQUE") => Err("已存在相同 ID 的预设".into()),
            Err(error) => Err(error),
        }
    }

    /// 更新预设。内置预设可编辑字段，但 `is_builtin` 不可改：以数据库中既有值为准。
    /// 非内置预设同样以数据库中既有值为准（保持 `false`）。
    pub async fn preset_update(&self, mut preset: DownloadPreset) -> Result<(), String> {
        validate_preset_connections(preset.connections)?;
        if preset.name.trim().is_empty() {
            return Err("预设名称不能为空".into());
        }
        validate_preset_scheduled_at(preset.scheduled_at.as_deref())?;
        let existing = self
            .store
            .download_preset_get(&preset.id)
            .await?
            .ok_or_else(|| "预设不存在".to_string())?;
        // is_builtin 以数据库中既有值为准，前端传入的值被忽略。
        preset.is_builtin = existing.is_builtin;
        self.store.download_preset_update(preset).await
    }

    /// 删除预设。仅允许删除 `is_builtin = false` 的自定义预设。
    pub async fn preset_delete(&self, id: &str) -> Result<(), String> {
        let existing = self
            .store
            .download_preset_get(id)
            .await?
            .ok_or_else(|| "预设不存在".to_string())?;
        if existing.is_builtin {
            return Err("内置预设不可删除，可在编辑中调整字段".into());
        }
        self.store.download_preset_delete(id).await
    }

    /// 把预设配置应用到现有任务。
    ///
    /// 应用字段：`connection_count`、`per_task_speed_limit`、`completion_action`、
    /// `expected_checksum`（仅在预设 `verify_checksum = true` 且任务原无校验值时填入占位）、
    /// `scheduled_at`（由 "HH:MM" 转为下一次该时刻的 Unix 毫秒时间戳）。
    /// 仅在任务处于可安全修改的状态（Queued / Paused / Scheduled / Failed / Cancelled）时应用，
    /// 下载中、校验中、网络等待、磁盘不足暂停状态拒绝修改以避免运行时状态混乱。
    pub async fn preset_apply_to_task(
        &self,
        task_id: &str,
        preset_id: &str,
    ) -> Result<DownloadTask, String> {
        let preset = self
            .store
            .download_preset_get(preset_id)
            .await?
            .ok_or_else(|| "预设不存在".to_string())?;
        let mut task = self
            .store
            .get_task(task_id)
            .await?
            .ok_or_else(|| "任务不存在".to_string())?;
        apply_preset_to_task_fields(&mut task, &preset)?;
        // 同步运行时配置（如有活动连接）。
        if let Some(runtime) = self.task_runtime.read().await.get(task_id).cloned() {
            runtime
                .speed_limit
                .store(task.per_task_speed_limit, Ordering::Relaxed);
            *runtime.completion_action.write().await = task.completion_action.clone();
        }
        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);
        self.dispatcher.notify_waiters();
        Ok(task)
    }

    // ===== Task 27: 完整备份与恢复 =====

    /// 导出完整备份到指定路径。
    ///
    /// `include_auth = true` 时必须提供 `password`，备份文件会被 AES-256-GCM 加密；
    /// `include_auth = false` 时备份为明文 JSON，认证字段已被清空。
    /// 路径必须是绝对路径且以 `.json` 结尾（由 `export_bundle` 校验）。
    pub async fn backup_export(
        &self,
        path: &str,
        include_auth: bool,
        password: Option<&str>,
    ) -> Result<(), String> {
        let settings = self.settings().await;
        let tasks = self.store.list_tasks().await?;
        let category_rules = self.store.category_rule_list().await?;
        let filename_cleanup_rules = self.store.filename_cleanup_rule_list().await?;
        let download_presets = self.store.download_preset_list().await?;
        let url_history = self.store.url_history_list().await?;
        let saved_views = self.store.saved_view_list().await?;
        let bundle = crate::task_transfer::build_bundle(
            settings,
            tasks,
            category_rules,
            filename_cleanup_rules,
            download_presets,
            url_history,
            saved_views,
            env!("CARGO_PKG_VERSION"),
            include_auth,
        );
        crate::task_transfer::export_bundle(path, &bundle, password).await
    }

    /// 读取备份文件并计算恢复预览，不修改任何状态。
    ///
    /// 加密文件必须提供密码。返回的 [`RestorePreview`] 列出本次恢复将新增、
    /// 覆盖、跳过的条数，由前端在用户确认前展示。
    pub async fn backup_preview(
        &self,
        path: &str,
        password: Option<&str>,
    ) -> Result<RestorePreview, String> {
        let manifest = crate::task_transfer::read_backup_manifest(path).await?;
        let bundle = crate::task_transfer::read_bundle(path, password).await?;
        let settings = self.settings().await;
        let category_rules = self.store.category_rule_list().await?;
        let filename_cleanup_rules = self.store.filename_cleanup_rule_list().await?;
        let download_presets = self.store.download_preset_list().await?;
        let url_history = self.store.url_history_list().await?;
        let tasks = self.store.list_tasks().await?;
        let task_ids: HashSet<String> = tasks.into_iter().map(|t| t.id).collect();
        let saved_view_ids: HashSet<String> = self
            .store
            .saved_view_list()
            .await?
            .into_iter()
            .map(|view| view.id)
            .collect();
        let current = crate::task_transfer::CurrentState {
            settings: &settings,
            category_rules: &category_rules,
            filename_cleanup_rules: &filename_cleanup_rules,
            download_presets: &download_presets,
            url_history: &url_history,
            saved_view_ids: &saved_view_ids,
            task_ids: &task_ids,
        };
        let mut preview = crate::task_transfer::compute_preview(&bundle, &current);
        preview.encrypted = manifest.encrypted;
        Ok(preview)
    }

    /// 读取备份文件并应用恢复。
    ///
    /// 应用规则：
    /// - **设置**：覆盖当前设置（用户已确认）。
    /// - **分类规则 / 文件名清理规则 / 下载预设**：按 ID upsert（已存在 → 更新，不存在 → 新增）。
    ///   内置预设的 `is_builtin` 以数据库中既有值为准。
    /// - **URL 历史**：按 URL 去重，重复的更新 `last_used`。
    /// - **任务**：按 ID 去重，已存在的跳过（不覆盖用户进度），不存在的直接 upsert（保留原状态）。
    fn sanitize_restored_task(task: &mut DownloadTask) {
        // 强制状态安全：恢复的任务不得以激活状态（Queued / Downloading / Connecting）直接挂载
        if matches!(task.status, TaskStatus::Queued | TaskStatus::Downloading) {
            task.status = TaskStatus::Paused;
        }
        // 强制完成动作安全：恢复的任务禁止自动执行任意命令或关机等危险操作，仅允许 None / OpenFolder
        if !matches!(
            task.completion_action,
            CompletionAction::None | CompletionAction::OpenFolder
        ) {
            task.completion_action = CompletionAction::None;
        }
        // 规范化目标保存路径
        task.destination = normalize_directory(&task.destination);
        task.active_connections = 0;
        task.speed = 0;
        task.eta_seconds = None;
    }

    /// 执行数据库与设置的反序列化恢复（Task 27.6）。
    ///
    /// 在单个 SQLite 事务中完成恢复，确保要么全量提交，要么完全回滚。
    /// 恢复的任务会经过安全净化（强制为 Paused，清除危险 completion_action）。
    pub async fn backup_restore(
        self: &SharedManager,
        path: &str,
        password: Option<&str>,
    ) -> Result<RestoreStats, String> {
        let bundle = crate::task_transfer::read_bundle(path, password).await?;

        // 1. 若包含设置，先预校验设置并尝试构建 HTTP 客户端（校验代理等配置合法性）
        let new_client = if let Some(settings) = &bundle.settings {
            validate_settings(settings)?;
            Some(build_client(settings)?)
        } else {
            None
        };

        // 2. 净化待恢复的任务列表（状态强制转为 Paused，清除危险 completion_action）
        let mut sanitized_tasks = bundle.tasks.clone();
        for task in &mut sanitized_tasks {
            Self::sanitize_restored_task(task);
        }

        // 3. 在单个 SQLite 事务中原子执行持久化
        let (stats, restored_tasks) = self
            .store
            .restore_backup_bundle(&bundle, sanitized_tasks)
            .await?;

        // 4. 事务成功提交后，原子刷新内存中的客户端与设置状态
        if let (Some(settings), Some(client)) = (bundle.settings.clone(), new_client) {
            *self.client.write().await = client;
            *self.settings.write().await = settings.clone();
            let _ = self.app.emit("settings-updated", settings);
        }

        // 5. 广播新增任务事件并通知 Waiters
        for task in &restored_tasks {
            self.emit_task("created", task);
        }
        self.dispatcher.notify_waiters();

        Ok(stats)
    }

    async fn notify_download_completed(&self, task: &DownloadTask) {
        let settings = self.settings().await;
        let Some((title, body)) = completion_notification(&settings, task) else {
            return;
        };
        // Task 30.2：Windows 上用 tauri-winrt-notification 发原生 Toast，
        // 注册 on_activated 回调，点击通知时在同进程内 emit 定位事件。
        // 非 Windows 平台回退到 tauri-plugin-notification 原有实现。
        #[cfg(windows)]
        {
            let app = self.app.clone();
            let task_id = task.id.clone();
            let app_id = self.app.config().identifier.clone();
            let title_c = title.clone();
            let body_c = body.clone();
            let toast_result = notify_win_toast(&app_id, &title_c, &body_c, task_id.clone(), app);
            if let Err(error) = toast_result {
                let _ = self.app.emit(
                    "notification-error",
                    format!("下载已完成，但 Windows 通知发送失败：{error}"),
                );
            }
        }
        #[cfg(not(windows))]
        {
            if let Err(error) = self
                .app
                .notification()
                .builder()
                .title(&title)
                .body(&body)
                .show()
            {
                let _ = self.app.emit(
                    "notification-error",
                    format!("下载已完成，但通知发送失败：{error}"),
                );
            }
        }
        let _ = self.app.emit(
            "task-notification",
            TaskNotificationPayload {
                task_id: task.id.clone(),
                kind: "completed",
                title,
                body,
            },
        );
    }

    /// Task 30.2：下载失败时发送系统通知与 `task-notification` 事件。
    ///
    /// 与 `notify_download_completed` 对称。失败通知的 `task-notification`
    /// 事件 kind = "failed"，前端可据此显示带"一键重试"按钮的 toast。
    async fn notify_download_failed(&self, task: &DownloadTask) {
        let settings = self.settings().await;
        let Some((title, body)) = failure_notification(&settings, task) else {
            return;
        };
        #[cfg(windows)]
        {
            let app = self.app.clone();
            let task_id = task.id.clone();
            let app_id = self.app.config().identifier.clone();
            let title_c = title.clone();
            let body_c = body.clone();
            let toast_result = notify_win_toast(&app_id, &title_c, &body_c, task_id.clone(), app);
            if let Err(error) = toast_result {
                let _ = self.app.emit(
                    "notification-error",
                    format!("下载已失败，但 Windows 通知发送失败：{error}"),
                );
            }
        }
        #[cfg(not(windows))]
        {
            if let Err(error) = self
                .app
                .notification()
                .builder()
                .title(&title)
                .body(&body)
                .show()
            {
                let _ = self.app.emit(
                    "notification-error",
                    format!("下载失败，但通知发送失败：{error}"),
                );
            }
        }
        let _ = self.app.emit(
            "task-notification",
            TaskNotificationPayload {
                task_id: task.id.clone(),
                kind: "failed",
                title,
                body,
            },
        );
    }
    async fn clear_parts(&self, task: &DownloadTask) {
        // 1. 删除任务专属隐藏临时目录 _maobu_tmp/[task_id]/
        let task_dir = task_temp_dir(&task.destination, &task.id);
        let _ = fs::remove_dir_all(&task_dir).await;

        // 2. 若 _maobu_tmp 根目录变为空目录，将其一并删除
        let root_dir = PathBuf::from(&task.destination).join("_maobu_tmp");
        if let Ok(mut entries) = fs::read_dir(&root_dir).await {
            if entries.next_entry().await.ok().flatten().is_none() {
                let _ = fs::remove_dir(&root_dir).await;
            }
        }

        // 3. 兜底清理可能残留在根目录的旧格式 .lumaget 与 .partN 分片
        let output = PathBuf::from(&task.destination).join(&task.file_name);
        let temp = PathBuf::from(format!("{}.lumaget", output.to_string_lossy()));
        for index in 0..128 {
            let _ = fs::remove_file(format!("{}.part{index}", temp.to_string_lossy())).await;
        }
        if let (Some(parent), Some(temp_name)) = (temp.parent(), temp.file_name()) {
            let prefix = format!("{}.part", temp_name.to_string_lossy());
            if let Ok(mut entries) = fs::read_dir(parent).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name();
                    if is_window_part_name(&name.to_string_lossy(), &prefix) {
                        let _ = fs::remove_file(entry.path()).await;
                    }
                }
            }
        }
        let _ = fs::remove_file(format!("{}.merge", temp.to_string_lossy())).await;
        let _ = fs::remove_file(&temp).await;
    }
    pub(crate) fn emit_task(&self, event: &str, task: &DownloadTask) {
        let _ = self.app.emit(
            &format!("task-{event}"),
            TaskProgressEvent {
                task: task.clone(),
                event: event.into(),
            },
        );
    }

    /// Task 18: 发出最后一次 `task-connections` 事件，用于在任务离开 Downloading 状态时
    /// 将最终分片状态（Paused/Failed/Completed）同步给前端。
    ///
    /// - `task_paused = true`：所有未完成分片标记为 `Paused`（用户暂停 / 低盘暂停）。
    /// - `task_paused = false`：保留真实分片状态（Failed / Completed / Downloading）。
    ///
    /// 速度字段在最终事件中始终为 0（无活动连接），不读取模拟数据。
    fn emit_task_connections_final(
        &self,
        task_id: &str,
        runtimes: &[SegmentRuntime],
        task_paused: bool,
    ) {
        let segments = snapshot_segment_statuses(runtimes, &[], 0.0, task_paused);
        let _ = self.app.emit(
            "task-connections",
            TaskConnectionsEvent {
                task_id: task_id.into(),
                segments,
                timestamp: now_millis(),
            },
        );
    }

    fn emit_power_action_state(&self, state: &PowerActionState) {
        let _ = self.app.emit("power-action-state", state.clone());
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PowerActionDecision {
    Waiting,
    Blocked(String),
    Complete,
}

fn is_power_action_target(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Queued
            | TaskStatus::Downloading
            | TaskStatus::Paused
            | TaskStatus::Scheduled
            | TaskStatus::Verifying
            | TaskStatus::WaitingNetwork
    )
}

fn power_action_decision(
    target_ids: &HashSet<String>,
    statuses: &HashMap<String, TaskStatus>,
) -> PowerActionDecision {
    if target_ids.iter().any(|id| !statuses.contains_key(id)) {
        return PowerActionDecision::Blocked("目标任务已被删除，系统操作不会执行".into());
    }
    if statuses
        .values()
        .any(|status| matches!(status, TaskStatus::Failed))
    {
        return PowerActionDecision::Blocked("存在失败任务，系统操作不会执行".into());
    }
    if statuses
        .values()
        .any(|status| matches!(status, TaskStatus::Paused))
    {
        return PowerActionDecision::Blocked("存在暂停任务，恢复后才会继续等待".into());
    }
    if statuses
        .values()
        .any(|status| matches!(status, TaskStatus::Cancelled))
    {
        return PowerActionDecision::Blocked("存在已取消任务，系统操作不会执行".into());
    }
    if statuses
        .values()
        .all(|status| matches!(status, TaskStatus::Completed))
    {
        PowerActionDecision::Complete
    } else {
        PowerActionDecision::Waiting
    }
}

fn power_action_remaining_seconds(deadline: u64, current: u64) -> u64 {
    deadline.saturating_sub(current).div_ceil(1_000)
}

#[cfg(target_os = "windows")]
fn execute_power_action(action: PowerAction) -> Result<(), String> {
    let Some(args) = power_action_command_args(action) else {
        return Ok(());
    };
    let status = Command::new("shutdown.exe")
        .args(args)
        .status()
        .map_err(|error| format!("无法启动 shutdown.exe：{error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("shutdown.exe 返回状态 {status}"))
    }
}

#[cfg(target_os = "windows")]
fn power_action_command_args(action: PowerAction) -> Option<&'static [&'static str]> {
    match action {
        PowerAction::Shutdown => Some(&["/s", "/t", "0"]),
        PowerAction::Hibernate => Some(&["/h"]),
        PowerAction::None => None,
    }
}

#[cfg(not(target_os = "windows"))]
fn execute_power_action(action: PowerAction) -> Result<(), String> {
    let _ = action;
    Err("当前系统不支持该电源操作".into())
}

async fn append_part(
    output: &mut fs::File,
    path: &Path,
    expected: u64,
    buffer: &mut [u8],
) -> Result<(), String> {
    let actual = fs::metadata(path)
        .await
        .map_err(|error| error.to_string())?
        .len();
    if actual != expected {
        return Err(format!(
            "续接窗口大小不完整（应为 {expected} 字节，实际 {actual} 字节）"
        ));
    }
    let mut source = fs::File::open(path)
        .await
        .map_err(|error| error.to_string())?;
    loop {
        let count = source
            .read(buffer)
            .await
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        output
            .write_all(&buffer[..count])
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn is_window_part_name(name: &str, prefix: &str) -> bool {
    let Some(rest) = name.strip_prefix(prefix) else {
        return false;
    };
    let Some((segment, start)) = rest.split_once(".w") else {
        return false;
    };
    !segment.is_empty()
        && segment.bytes().all(|byte| byte.is_ascii_digit())
        && !start.is_empty()
        && start.bytes().all(|byte| byte.is_ascii_digit())
}

/// 返回任务隐藏临时目录路径：`destination/_maobu_tmp/[task_id]/`
pub fn task_temp_dir(destination: &str, task_id: &str) -> PathBuf {
    PathBuf::from(destination).join("_maobu_tmp").join(task_id)
}

/// 返回任务主临时文件路径：`destination/_maobu_tmp/[task_id]/[file_name].lumaget`
pub fn task_temp_path(destination: &str, task_id: &str, file_name: &str) -> PathBuf {
    task_temp_dir(destination, task_id).join(format!("{file_name}.lumaget"))
}

/// 确保任务的隐藏临时目录已创建，并在 Windows 环境下为 `_maobu_tmp` 赋予隐藏属性
pub async fn ensure_task_temp_dir(destination: &str, task_id: &str) -> Result<PathBuf, String> {
    let root_dir = PathBuf::from(destination).join("_maobu_tmp");
    if !root_dir.exists() {
        if let Err(e) = fs::create_dir_all(&root_dir).await {
            return Err(format!("无法创建隐藏临时根目录：{e}"));
        }
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::ffi::OsStrExt;
            let wide: Vec<u16> = root_dir
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            unsafe {
                windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(
                    wide.as_ptr(),
                    windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN,
                );
            }
        }
    }
    let task_dir = root_dir.join(task_id);
    if !task_dir.exists() {
        fs::create_dir_all(&task_dir)
            .await
            .map_err(|e| format!("无法创建任务临时目录：{e}"))?;
    }
    Ok(task_dir)
}

/// Core startup selfcheck logic. Marked as a free function so unit tests can
/// exercise it against a temporary `Store` without constructing an
/// `AppHandle`. `run_startup_selfcheck` wraps this with a Tauri event emit.
async fn execute_selfcheck(store: &Store) -> SelfcheckReport {
    let mut report = SelfcheckReport::default();
    let tasks = match store.list_tasks().await {
        Ok(tasks) => tasks,
        Err(_) => return report,
    };

    for mut task in tasks {
        if task.status != TaskStatus::Downloading {
            continue;
        }

        task.status = TaskStatus::Interrupted;
        task.speed = 0;
        task.eta_seconds = None;
        task.active_connections = 0;

        let output = PathBuf::from(&task.destination).join(&task.file_name);
        let new_temp = task_temp_path(&task.destination, &task.id, &task.file_name);
        let legacy_temp = PathBuf::from(format!("{}.lumaget", output.to_string_lossy()));
        let temp = if new_temp.exists() || task_temp_dir(&task.destination, &task.id).exists() {
            new_temp
        } else {
            legacy_temp
        };
        // download_stream stores the segment data in the .lumaget file itself,
        // while download_segments splits it across .partN[.wM] files. A single
        // segment with index 0 and no .part0 file on disk indicates the
        // single-stream path; everything else uses the multi-connection layout.
        let part0_path = PathBuf::from(format!("{}.part0", temp.to_string_lossy()));
        let part0_exists = fs::metadata(&part0_path).await.is_ok();
        let is_single_stream = task.segments.len() == 1
            && task.segments.first().is_some_and(|s| s.index == 0)
            && !part0_exists;

        for segment in &mut task.segments {
            let mismatched = if is_single_stream {
                let actual = fs::metadata(&temp).await.map(|m| m.len()).unwrap_or(0);
                if actual != segment.downloaded_bytes {
                    let _ = fs::remove_file(&temp).await;
                    true
                } else {
                    false
                }
            } else {
                let actual = measure_segment_bytes(&temp, segment.index).await;
                if actual != segment.downloaded_bytes {
                    drop_segment_files(&temp, segment.index).await;
                    true
                } else {
                    false
                }
            };

            if mismatched {
                segment.downloaded_bytes = 0;
                segment.status = "pending".into();
                report.dropped_shards += 1;
                continue;
            }
            if segment.status == "downloading" {
                segment.status = "pending".into();
            }
        }

        // Recalculate the task-level progress from the surviving shards so
        // the UI does not display a downloaded_bytes total that references
        // bytes we just discarded.
        task.downloaded_bytes = task.segments.iter().map(|s| s.downloaded_bytes).sum();

        if store.upsert_task(&task).await.is_err() {
            // Persisting the recovery failed; keep going so the rest of the
            // task list still gets repaired. The next startup will retry.
            continue;
        }

        report.interrupted_count += 1;
        report.recovered_tasks.push(task.id.clone());
    }

    report
}

/// Sums the on-disk byte count for a multi-connection segment, including the
/// legacy `.partN` prefix file and any `.partN.w<start>` window files.
async fn measure_segment_bytes(temp: &Path, index: u8) -> u64 {
    let mut total = 0u64;
    let legacy = PathBuf::from(format!("{}.part{index}", temp.to_string_lossy()));
    if let Ok(meta) = fs::metadata(&legacy).await {
        total += meta.len();
    }
    if let (Some(parent), Some(temp_name)) = (temp.parent(), temp.file_name()) {
        let prefix = format!("{}.part{index}.w", temp_name.to_string_lossy());
        if let Ok(mut entries) = fs::read_dir(parent).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry.file_name().to_string_lossy().starts_with(&prefix)
                    && entry.metadata().await.map(|m| m.is_file()).unwrap_or(false)
                {
                    total += entry.metadata().await.map(|m| m.len()).unwrap_or(0);
                }
            }
        }
    }
    total
}

/// Removes the legacy `.partN` file and every `.partN.w<start>` window file
/// for a segment that failed the length check.
async fn drop_segment_files(temp: &Path, index: u8) {
    let legacy = PathBuf::from(format!("{}.part{index}", temp.to_string_lossy()));
    let _ = fs::remove_file(&legacy).await;
    if let (Some(parent), Some(temp_name)) = (temp.parent(), temp.file_name()) {
        let prefix = format!("{}.part{index}.w", temp_name.to_string_lossy());
        if let Ok(mut entries) = fs::read_dir(parent).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = fs::remove_file(entry.path()).await;
                }
            }
        }
    }
}



/// 限速器（基于 GCRA / Virtual Scheduling 算法）。
///
/// 同一任务的全部 Range 连接共享一个 `Arc<RateLimiter>`，因此任务级限速
/// 覆盖该任务的全部分段连接。跨任务的全局限速与优先级公平分配由
/// `bandwidth::BandwidthScheduler` 负责。
///
/// 算法：
/// - `next_allowed` 是下一次允许开始传输 `bytes` 字节的最早时间点
/// - 每次 acquire：`wait = max(0, next_allowed - now)`，然后 `next_allowed += bytes / limit`
/// - 如果 `next_allowed` 落后于 `now`（限速器空闲过），先对齐到 `now`（不累积历史额度）
///
/// 与早期令牌桶实现的差异：
/// - 无 `capacity` 上限，能正确处理大 chunk（如 reqwest 8MB 的 bytes_stream chunk）
/// - 早期实现 `tokens` 被 `capacity` 封顶，当 `bytes > capacity` 时陷入无限循环
/// - 移除了 0.15s 静态缓冲（在高并发下导致限速偏差累积）
/// - 单次 acquire 最多 sleep 50ms 后回到调用方，保证 cancel 信号能在 50ms 内响应
/// - 提供 `acquire_with_cancel` 方法，在 sleep 步进之间检查 cancel 信号
pub struct RateLimiter {
    state: Mutex<RateLimiterState>,
}

struct RateLimiterState {
    /// 下一次允许传输的最早时间点。
    next_allowed: Instant,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(RateLimiterState {
                next_allowed: Instant::now(),
            }),
        }
    }

    /// 请求 `bytes` 字节的下载配额。如果当前速率超过 `limit`，会 sleep 等待。
    ///
    /// `limit == 0` 表示不限速，立即返回。
    /// 内部 sleep 以 50ms 为最大步长，确保调用方能在 50ms 内响应 cancel 信号。
    pub async fn acquire(&self, bytes: u64, limit: u64) {
        self.acquire_inner(bytes, limit, None).await
    }

    /// 与 `acquire` 相同，但在 sleep 步进之间检查 `cancel` 信号。
    ///
    /// 当 `cancel` 被触发时，立即返回（不再 sleep）。注意 `next_allowed` 已被推进，
    /// 这意味着被取消的 acquire 不会"补回"等待时间，但也不再多等。
    /// 对于暂停/取消场景，这保证 50ms 内响应，符合 AGENTS.md §3"暂停、取消...
    /// 必须停止所有活动连接"的要求。
    pub async fn acquire_with_cancel(&self, bytes: u64, limit: u64, cancel: &CancellationToken) {
        self.acquire_inner(bytes, limit, Some(cancel)).await
    }

    async fn acquire_inner(&self, bytes: u64, limit: u64, cancel: Option<&CancellationToken>) {
        if limit == 0 || bytes == 0 {
            return;
        }
        // 计算本次请求需要推进 next_allowed 的时间
        let duration_secs = bytes as f64 / limit as f64;
        let needed = Duration::from_secs_f64(duration_secs);
        let wait = {
            let mut state = self.state.lock().await;
            let now = Instant::now();
            // 空闲后对齐到 now，不累积历史额度（避免空闲后突发）
            if state.next_allowed < now {
                state.next_allowed = now;
            }
            let wait = state.next_allowed.saturating_duration_since(now);
            state.next_allowed += needed;
            wait
        };
        // 分段 sleep，确保 cancel 信号能在 50ms 内被调用方 select 捕获
        if !wait.is_zero() {
            let max_step = Duration::from_millis(50);
            let mut remaining = wait;
            while remaining > Duration::ZERO {
                if let Some(cancel) = cancel {
                    if cancel.is_cancelled() {
                        return;
                    }
                }
                let step = remaining.min(max_step);
                tokio::time::sleep(step).await;
                remaining = remaining.saturating_sub(step);
            }
        }
    }
}
struct ProgressSample {
    at: Instant,
    bytes: u64,
    smoothed_speed: f64,
}
impl ProgressSample {
    fn new(bytes: u64) -> Self {
        Self {
            at: Instant::now(),
            bytes,
            smoothed_speed: 0.0,
        }
    }
    fn should_emit(&self, current: u64) -> bool {
        self.at.elapsed() >= Duration::from_millis(250) || current == self.bytes
    }
    fn apply(&mut self, task: &mut DownloadTask) {
        let elapsed = self.at.elapsed().as_secs_f64().max(0.001);
        let instant_speed = (task.downloaded_bytes.saturating_sub(self.bytes)) as f64 / elapsed;
        self.smoothed_speed = smooth_speed(self.smoothed_speed, instant_speed, elapsed);
        task.speed = self.smoothed_speed.round() as u64;
        task.eta_seconds = if task.speed > 0 && task.total_bytes > task.downloaded_bytes {
            Some((task.total_bytes - task.downloaded_bytes) / task.speed)
        } else {
            None
        };
        self.at = Instant::now();
        self.bytes = task.downloaded_bytes
    }
}

fn smooth_speed(previous: f64, current: f64, elapsed: f64) -> f64 {
    if previous <= 0.0 {
        return current.max(0.0);
    }
    // A 1.5 second EWMA removes 250 ms sampling jitter while still reacting
    // quickly to a real throughput change or a stopped connection.
    let alpha = 1.0 - (-elapsed.max(0.001) / 1.5).exp();
    previous + alpha * (current.max(0.0) - previous)
}

const SEGMENT_PENDING: u8 = 0;
const SEGMENT_DOWNLOADING: u8 = 1;
const SEGMENT_COMPLETED: u8 = 2;
const SEGMENT_FAILED: u8 = 3;
const ADAPTIVE_YIELD: &str = "__maobu_adaptive_yield__";
const REMOTE_CHANGED_PREFIX: &str = "REMOTE_CHANGED:";
/// 云盘直链失效哨兵前缀。分段连接判定"链接已死"（连续空响应 / 长时间无
/// 实质进展）时携带此前缀上抛；`spawn_worker` 据此触发自动刷新直链。
const CLOUD_LINK_DEAD_PREFIX: &str = "CLOUD_LINK_DEAD:";
/// 空响应熔断阈值：连接成功返回 206 但 body 为 0 字节，重连后依然为空，
/// 连续达到该次数即判定直链失效（PikPak CDN 死链的典型表现）。
const MAX_EMPTY_STEPS: u32 = 3;
/// 停滞熔断窗口：该时长内窗口下载字节数不足 `STALL_RECOVERY_BYTES`
/// 且剩余仍很多时，判定直链失效（覆盖"慢速滴流"型死链）。
/// 正常慢速单连接（200KB/s）45 秒可下载约 9MB，远超 1MB，不会误熔断。
const STALL_TIMEOUT: Duration = Duration::from_secs(45);
/// 停滞恢复阈值：停滞判定窗口内至少要完成的字节数。
const STALL_RECOVERY_BYTES: u64 = 1024 * 1024;
/// 单任务直链自动刷新上限：防止分享本身失效时无限刷新。
const MAX_LINK_REFRESHES: u32 = 5;
/// 磁盘空间不足错误前缀。`spawn_worker` 据此识别"已由下载循环将任务置为
/// `PausedByLowDisk` 并持久化"，从而不再重试、不进入 Failed。
const LOW_DISK_PREFIX: &str = "LOW_DISK:";
/// 周期性磁盘空间检查的字节间隔：每下载 10MB 触发一次。
const DISK_CHECK_BYTES_INTERVAL: u64 = 10 * 1024 * 1024;
/// 周期性磁盘空间检查的时间间隔：每 5 秒触发一次（与字节间隔取先到者）。
const DISK_CHECK_TIME_INTERVAL: Duration = Duration::from_secs(5);
/// 低盘暂停的安全余量（50MB），覆盖文件系统簇对齐与元数据。
const LOW_DISK_SAFETY_MARGIN_BYTES: u64 = 50 * 1024 * 1024;

/// 计算下载中途周期性检查所需磁盘空间：`remaining + remaining/2 + 50MB`。
///
/// - `remaining`：剩余待下载字节数
/// - `remaining/2`：缓冲与临时文件双写余量
/// - `50MB`：固定安全余量
///
/// 使用 `saturating_add` 防止溢出。
fn compute_low_disk_required_space(total_bytes: u64, downloaded_bytes: u64) -> u64 {
    let remaining = total_bytes.saturating_sub(downloaded_bytes);
    remaining
        .saturating_add(remaining / 2)
        .saturating_add(LOW_DISK_SAFETY_MARGIN_BYTES)
}

/// 查询目标目录所在磁盘的可用空间。
///
/// 目录不存在时向祖先目录回退，直到找到一个存在的目录；全部失败时返回 0
/// （调用方将其视为"空间未知"，按不足处理）。
fn query_available_space_for_destination(destination: &str) -> u64 {
    let path = Path::new(destination);
    if let Some(space) = query_destination_available_space(path) {
        return space;
    }
    let mut current = path;
    while let Some(parent) = current.parent() {
        if parent.as_os_str().is_empty() {
            break;
        }
        if let Some(space) = query_destination_available_space(parent) {
            return space;
        }
        current = parent;
    }
    0
}

/// 查询单个已存在目录的可用空间。
fn query_destination_available_space(path: &Path) -> Option<u64> {
    if !path.exists() {
        return None;
    }
    fs2::available_space(path).ok()
}

/// 下载循环中执行一次磁盘空间检查。
///
/// 返回 `Ok(())` 表示空间充足；返回 `Err((available, required))` 表示空间不足，
/// 调用方应取消所有活动连接、保留分片、将任务置为 `PausedByLowDisk` 并发事件。
fn check_disk_space_once(
    destination: &str,
    total_bytes: u64,
    downloaded_bytes: u64,
) -> Result<(), (u64, u64)> {
    let required = compute_low_disk_required_space(total_bytes, downloaded_bytes);
    let available = query_available_space_for_destination(destination);
    if available < required {
        Err((available, required))
    } else {
        Ok(())
    }
}

/// Compares recorded ETag/Last-Modified against the fresh HEAD response.
///
/// HTTP headers are case-insensitive, so the comparison uses
/// `eq_ignore_ascii_case`. When both sides have an ETag, only the ETag is
/// compared. When either side lacks an ETag, the function falls back to
/// Last-Modified. If neither validator can be compared (missing on one side),
/// it returns `false` so the user can still attempt resume — we never block
/// resumption merely because the server omitted a validator.
fn remote_resource_changed(
    old_etag: Option<&str>,
    new_etag: Option<&str>,
    old_last_modified: Option<&str>,
    new_last_modified: Option<&str>,
) -> bool {
    if let (Some(old), Some(new)) = (old_etag, new_etag) {
        return !old.eq_ignore_ascii_case(new);
    }
    if let (Some(old), Some(new)) = (old_last_modified, new_last_modified) {
        return !old.eq_ignore_ascii_case(new);
    }
    // 若此前已记录 ETag 或 Last-Modified，但新响应缺少对应的校验头无法重新比对，
    // 视为资源无法校验（防止盲目续传拼接坏分片，AGENTS.md §3/P1#6）。
    if old_etag.is_some() || old_last_modified.is_some() {
        return true;
    }
    false
}

struct SegmentRuntime {
    index: u8,
    start_byte: u64,
    end_byte: u64,
    downloaded_bytes: AtomicU64,
    status: AtomicU8,
    active_windows: AtomicU8,
    /// Task 18: 连接级重试次数（独立于 `DownloadTask::retry_count`）。
    retry_count: AtomicU32,
    /// Task 18: 最近一次错误信息（已通过 `redact_sensitive` 脱敏）。
    /// 仅在重试或失败时设置；成功后不清除，便于前端展示"上次错误"。
    last_error: StdMutex<Option<String>>,
    /// Task 18: 是否处于退避重试等待中。`true` 表示连接正在 sleep，
    /// 即将发起下一次 Range 请求。
    retrying: AtomicBool,
}

impl SegmentRuntime {
    fn new(index: u8, start: u64, end: u64, downloaded: u64, status: u8) -> Self {
        Self {
            index,
            start_byte: start,
            end_byte: end,
            downloaded_bytes: AtomicU64::new(downloaded),
            status: AtomicU8::new(status),
            active_windows: AtomicU8::new(0),
            retry_count: AtomicU32::new(0),
            last_error: StdMutex::new(None),
            retrying: AtomicBool::new(false),
        }
    }

    /// Task 18: 设置最近一次错误信息（脱敏后存储）。
    ///
    /// 使用 `redact_sensitive` 处理原始错误字符串，确保不泄露 Cookie、
    /// Authorization、代理密码或 URL token 段（AGENTS.md §3、§7）。
    fn set_last_error(&self, raw_error: &str) {
        let redacted = redact_sensitive(raw_error);
        if let Ok(mut guard) = self.last_error.lock() {
            *guard = Some(redacted);
        }
    }

    /// Task 18: 读取最近一次错误信息（已脱敏）。
    fn last_error(&self) -> Option<String> {
        self.last_error
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or(None)
    }
}

fn snapshot_segments(runtimes: &[SegmentRuntime]) -> Vec<DownloadSegment> {
    runtimes
        .iter()
        .map(|segment| DownloadSegment {
            index: segment.index,
            start_byte: segment.start_byte,
            end_byte: segment.end_byte,
            downloaded_bytes: segment.downloaded_bytes.load(Ordering::Relaxed),
            status: if segment.downloaded_bytes.load(Ordering::Relaxed)
                == segment.end_byte - segment.start_byte + 1
            {
                "completed"
            } else if segment.status.load(Ordering::Relaxed) == SEGMENT_FAILED {
                "failed"
            } else if segment.active_windows.load(Ordering::Relaxed) > 0 {
                "downloading"
            } else {
                "pending"
            }
            .into(),
        })
        .collect()
}

/// Task 18: 把 `SegmentRuntime` 列表汇总为 `Vec<SegmentStatus>` 用于 `task-connections` 事件。
///
/// 速度计算：使用 `prev_bytes` 与 `elapsed_secs` 计算每秒增量字节，
/// 数据来自 `downloaded_bytes` 原子量的真实采样（AGENTS.md §3）。
///
/// 状态映射（与 `snapshot_segments` 一致 + Retrying/Paused）：
/// - `task_paused = true` → `Paused`（任务被取消，所有连接停止）
/// - `downloaded == total` → `Completed`
/// - `status == SEGMENT_FAILED` → `Failed`
/// - `retrying == true` → `Retrying`（退避 sleep 中）
/// - `active_windows > 0` → `Downloading`
/// - 其他 → `Connecting`（已分配但尚未接收数据）
fn snapshot_segment_statuses(
    runtimes: &[SegmentRuntime],
    prev_bytes: &[u64],
    elapsed_secs: f64,
    task_paused: bool,
) -> Vec<SegmentStatus> {
    runtimes
        .iter()
        .enumerate()
        .map(|(i, segment)| {
            let downloaded = segment.downloaded_bytes.load(Ordering::Relaxed);
            let total = segment.end_byte - segment.start_byte + 1;
            let prev = prev_bytes.get(i).copied().unwrap_or(0);
            let speed = if elapsed_secs > 0.001 {
                let delta = downloaded.saturating_sub(prev);
                (delta as f64 / elapsed_secs) as u64
            } else {
                0
            };
            let state = if task_paused {
                ConnectionState::Paused
            } else if downloaded >= total {
                ConnectionState::Completed
            } else if segment.status.load(Ordering::Relaxed) == SEGMENT_FAILED {
                ConnectionState::Failed
            } else if segment.retrying.load(Ordering::Relaxed) {
                ConnectionState::Retrying
            } else if segment.active_windows.load(Ordering::Relaxed) > 0 {
                ConnectionState::Downloading
            } else {
                ConnectionState::Connecting
            };
            SegmentStatus {
                segment_id: segment.index.to_string(),
                start_offset: segment.start_byte,
                downloaded_bytes: downloaded,
                total_bytes: total,
                speed,
                state,
                retry_count: segment.retry_count.load(Ordering::Relaxed),
                error: segment.last_error(),
            }
        })
        .collect()
}

struct RangeWindowJob {
    segment_index: u8,
    ordinal: u32,
    start_byte: u64,
    end_byte: u64,
    existing_bytes: u64,
    path: PathBuf,
}

struct AdaptiveConnectionGate {
    max: u8,
    target: AtomicU8,
    active: AtomicU8,
    epoch: AtomicU64,
    baseline_speed: AtomicU64,
    peak_speed: AtomicU64,
    stable_samples: AtomicU8,
    degraded_samples: AtomicU8,
    probe_samples: AtomicU8,
    gain_samples: AtomicU8,
    weak_samples: AtomicU8,
    probing: AtomicU8,
    disabled: AtomicU8,
    user_disabled: AtomicU8,
    notify: Notify,
}

impl AdaptiveConnectionGate {
    fn new(max: u8) -> Self {
        let max = max.clamp(1, 32);
        Self {
            max,
            target: AtomicU8::new(max),
            active: AtomicU8::new(0),
            epoch: AtomicU64::new(0),
            baseline_speed: AtomicU64::new(0),
            peak_speed: AtomicU64::new(0),
            stable_samples: AtomicU8::new(0),
            degraded_samples: AtomicU8::new(0),
            probe_samples: AtomicU8::new(0),
            gain_samples: AtomicU8::new(0),
            weak_samples: AtomicU8::new(0),
            probing: AtomicU8::new(0),
            disabled: AtomicU8::new(0),
            user_disabled: AtomicU8::new(0),
            notify: Notify::new(),
        }
    }

    async fn acquire(self: Arc<Self>) -> AdaptiveConnectionPermit {
        if self.user_disabled.load(Ordering::Relaxed) > 0 {
            self.active.fetch_add(1, Ordering::Relaxed);
            return AdaptiveConnectionPermit {
                gate: self.clone(),
                epoch: 0,
            };
        }
        loop {
            let notified = self.notify.notified();
            let active = self.active.load(Ordering::Relaxed);
            let target = self.target.load(Ordering::Relaxed);
            if active < target
                && self
                    .active
                    .compare_exchange_weak(
                        active,
                        active.saturating_add(1),
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    )
                    .is_ok()
            {
                return AdaptiveConnectionPermit {
                    gate: self.clone(),
                    epoch: self.epoch.load(Ordering::Relaxed),
                };
            }
            notified.await;
        }
    }

    fn observe(&self, speed: u64) {
        if self.user_disabled.load(Ordering::Relaxed) > 0 || self.max <= 4 {
            return;
        }
        let previous_peak = self.peak_speed.fetch_max(speed, Ordering::Relaxed);
        let peak = previous_peak.max(speed);
        let target = self.target.load(Ordering::Relaxed);
        if self.probing.load(Ordering::Relaxed) == 0
            && target > 4
            && peak >= 8 * 1024 * 1024
            && speed.saturating_mul(100) < peak.saturating_mul(45)
        {
            let degraded = self
                .degraded_samples
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1);
            if degraded >= 8 {
                self.fallback_one_level();
            }
            return;
        }
        self.degraded_samples.store(0, Ordering::Relaxed);
        if self.probing.load(Ordering::Relaxed) > 0 {
            self.observe_probe(speed);
            return;
        }
        if self.disabled.load(Ordering::Relaxed) > 0 || target >= self.max {
            return;
        }
        if speed < 4 * 1024 * 1024 {
            self.stable_samples.store(0, Ordering::Relaxed);
            return;
        }
        let stable = self
            .stable_samples
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        if stable < 4 {
            return;
        }
        self.stable_samples.store(0, Ordering::Relaxed);
        self.baseline_speed.store(speed.max(1), Ordering::Relaxed);
        self.probe_samples.store(0, Ordering::Relaxed);
        self.gain_samples.store(0, Ordering::Relaxed);
        self.weak_samples.store(0, Ordering::Relaxed);
        self.probing.store(1, Ordering::Relaxed);
        let target = self.target.load(Ordering::Relaxed);
        self.target
            .store(target.saturating_mul(2).min(self.max), Ordering::Relaxed);
        self.notify.notify_waiters();
    }

    fn observe_probe(&self, speed: u64) {
        let baseline = self.baseline_speed.load(Ordering::Relaxed).max(1);
        let samples = self
            .probe_samples
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        if speed.saturating_mul(100) >= baseline.saturating_mul(115) {
            self.weak_samples.store(0, Ordering::Relaxed);
            let gains = self
                .gain_samples
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1);
            if gains >= 3 {
                self.accept_probe();
            }
            return;
        }
        self.gain_samples.store(0, Ordering::Relaxed);
        if speed.saturating_mul(100) < baseline.saturating_mul(65) {
            let weak = self
                .weak_samples
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1);
            if weak >= 4 {
                self.reject_probe();
            }
            return;
        }
        self.weak_samples.store(0, Ordering::Relaxed);
        if samples >= 10 {
            self.reject_probe();
        }
    }

    fn accept_probe(&self) {
        self.probing.store(0, Ordering::Relaxed);
        self.stable_samples.store(0, Ordering::Relaxed);
        self.probe_samples.store(0, Ordering::Relaxed);
        self.gain_samples.store(0, Ordering::Relaxed);
        self.weak_samples.store(0, Ordering::Relaxed);
    }

    fn reject_probe(&self) {
        self.fallback_one_level();
    }

    fn fallback_one_level(&self) {
        let target = self.target.load(Ordering::Relaxed);
        self.target.store((target / 2).max(4), Ordering::Relaxed);
        self.probing.store(0, Ordering::Relaxed);
        self.disabled.store(1, Ordering::Relaxed);
        self.degraded_samples.store(0, Ordering::Relaxed);
        self.epoch.fetch_add(1, Ordering::Relaxed);
        self.notify.notify_waiters();
    }

    fn should_yield(&self, permit: &AdaptiveConnectionPermit) -> bool {
        if self.user_disabled.load(Ordering::Relaxed) > 0 {
            return false;
        }
        permit.epoch != self.epoch.load(Ordering::Relaxed)
            || self.active.load(Ordering::Relaxed) > self.target.load(Ordering::Relaxed)
    }

    fn active(&self) -> u8 {
        self.active.load(Ordering::Relaxed)
    }
}

struct AdaptiveConnectionPermit {
    gate: Arc<AdaptiveConnectionGate>,
    epoch: u64,
}

impl Drop for AdaptiveConnectionPermit {
    fn drop(&mut self) {
        self.gate.active.fetch_sub(1, Ordering::Relaxed);
        self.gate.notify.notify_waiters();
    }
}

const RANGE_WINDOW_BASE_BYTES: u64 = 8 * 1024 * 1024;
const RANGE_WINDOW_STEP_BYTES: u64 = 256 * 1024;

fn layout_from_existing_starts(start: u64, end: u64, starts: &[u64]) -> Option<Vec<(u32, u64, u64)>> {
    if starts.is_empty() || starts.first() != Some(&start) {
        return None;
    }
    for window in starts.windows(2) {
        if window[0] >= window[1] || window[1] > end {
            return None;
        }
    }
    if *starts.last().unwrap() > end {
        return None;
    }
    let mut result = Vec::with_capacity(starts.len());
    for (i, &w_start) in starts.iter().enumerate() {
        let w_end = if i + 1 < starts.len() {
            starts[i + 1] - 1
        } else {
            end
        };
        result.push((i as u32, w_start, w_end));
    }
    Some(result)
}

async fn select_window_layout(
    temp: &Path,
    segment_index: u8,
    start: u64,
    end: u64,
    legacy_prefix_bytes: u64,
) -> Vec<(u32, u64, u64)> {
    if legacy_prefix_bytes > 0 {
        return segment_window_ranges(start, end, segment_index);
    }
    let balanced = balanced_window_ranges(start, end, segment_index);
    let existing = existing_window_starts(temp, segment_index).await;
    if existing.is_empty()
        || existing
            .iter()
            .all(|value| balanced.iter().any(|(_, start, _)| start == value))
    {
        balanced
    } else if let Some(layout) = layout_from_existing_starts(start, end, &existing) {
        layout
    } else {
        segment_window_ranges(start, end, segment_index)
    }
}

async fn existing_window_starts(temp: &Path, segment_index: u8) -> Vec<u64> {
    let (Some(parent), Some(temp_name)) = (temp.parent(), temp.file_name()) else {
        return Vec::new();
    };
    let prefix = format!("{}.part{segment_index}.w", temp_name.to_string_lossy());
    let Ok(mut entries) = fs::read_dir(parent).await else {
        return Vec::new();
    };
    let mut starts = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let Some(value) = name
            .to_string_lossy()
            .strip_prefix(&prefix)
            .and_then(|value| {
                if value.bytes().all(|byte| byte.is_ascii_digit()) {
                    value.parse::<u64>().ok()
                } else {
                    None
                }
            })
        else {
            continue;
        };
        starts.push(value);
    }
    starts.sort_unstable();
    starts
}

fn balanced_window_ranges(start: u64, end: u64, index: u8) -> Vec<(u32, u64, u64)> {
    if start > end {
        return Vec::new();
    }
    let length = end - start + 1;
    if length < RANGE_WINDOW_BASE_BYTES.saturating_mul(2) {
        return vec![(0, start, end)];
    }
    let stagger = (index as u64 % 8).saturating_mul(RANGE_WINDOW_STEP_BYTES);
    let first_length = (length / 2)
        .saturating_add(stagger)
        .clamp(RANGE_WINDOW_BASE_BYTES, length - RANGE_WINDOW_BASE_BYTES);
    let tail_start = start + first_length;
    vec![(0, start, tail_start - 1), (1, tail_start, end)]
}

fn range_window_end(start: u64, segment_end: u64, index: u8) -> u64 {
    let window_bytes = RANGE_WINDOW_BASE_BYTES
        .saturating_add((index as u64 % 8).saturating_mul(RANGE_WINDOW_STEP_BYTES));
    start
        .saturating_add(window_bytes.saturating_sub(1))
        .min(segment_end)
}

fn segment_window_ranges(start: u64, end: u64, index: u8) -> Vec<(u32, u64, u64)> {
    if start > end {
        return Vec::new();
    }
    let mut ranges = Vec::new();
    let mut cursor = start;
    let mut ordinal = 0u32;
    while cursor <= end {
        let window_end = range_window_end(cursor, end, index);
        ranges.push((ordinal, cursor, window_end));
        ordinal = ordinal.saturating_add(1);
        cursor = window_end.saturating_add(1);
    }
    ranges
}

fn window_part_path(temp: &Path, segment_index: u8, start: u64) -> PathBuf {
    PathBuf::from(format!(
        "{}.part{segment_index}.w{start}",
        temp.to_string_lossy()
    ))
}

fn requested_segment_count(connections: u8) -> u8 {
    connections.clamp(1, 32)
}

fn planned_segment_ranges(task: &DownloadTask, total: u64, connections: u8) -> Vec<(u8, u64, u64)> {
    let mut saved = task.segments.clone();
    saved.sort_by_key(|segment| segment.index);
    let resumable = saved.len() > 1
        && saved.len() <= 128
        && saved.iter().any(|segment| segment.downloaded_bytes > 0)
        && saved.first().is_some_and(|segment| segment.start_byte == 0)
        && saved
            .windows(2)
            .all(|pair| pair[0].end_byte.checked_add(1) == Some(pair[1].start_byte))
        && saved
            .last()
            .is_some_and(|segment| segment.end_byte == total.saturating_sub(1))
        && saved.iter().all(|segment| {
            segment.start_byte <= segment.end_byte
                && segment.downloaded_bytes <= segment.end_byte - segment.start_byte + 1
        });
    if resumable {
        return saved
            .into_iter()
            .map(|segment| (segment.index, segment.start_byte, segment.end_byte))
            .collect();
    }
    segment_ranges(total, requested_segment_count(connections))
}

fn segment_ranges(total: u64, segments: u8) -> Vec<(u8, u64, u64)> {
    if total == 0 {
        return Vec::new();
    }
    let segments = segments.clamp(1, 128);
    let size = total.div_ceil(segments as u64);
    (0..segments)
        .filter_map(|index| {
            let start = index as u64 * size;
            (start < total).then(|| {
                let end = ((index as u64 + 1) * size - 1).min(total - 1);
                (index, start, end)
            })
        })
        .collect()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

/// 校验下载预设的连接数（Task 12.3）。
///
/// 仅允许 `1 / 2 / 4 / 8 / 16 / 32` 这六个档位，与 §3 下载内核强约束一致。
/// 非法值返回中文错误，便于前端直接展示。
pub fn validate_preset_connections(n: u8) -> Result<(), String> {
    if [1u8, 2, 4, 8, 16, 32].contains(&n) {
        Ok(())
    } else {
        Err("连接数只能是 1 / 2 / 4 / 8 / 16 / 32".into())
    }
}

/// 校验预设的计划时间格式。仅接受 "HH:MM" 24 小时制（HH 00-23，MM 00-59）。
/// `None` 表示立即开始，通过校验。
fn validate_preset_scheduled_at(value: Option<&str>) -> Result<(), String> {
    let Some(raw) = value else {
        return Ok(());
    };
    let bytes = raw.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return Err("计划时间格式必须为 HH:MM".into());
    }
    let parse = |start: usize, end: usize| -> Result<u32, String> {
        let slice = &bytes[start..end];
        let value = std::str::from_utf8(slice)
            .map_err(|_| "计划时间格式必须为 HH:MM".to_string())?
            .parse::<u32>()
            .map_err(|_| "计划时间格式必须为 HH:MM".to_string())?;
        Ok(value)
    };
    let hh = parse(0, 2)?;
    let mm = parse(3, 5)?;
    if hh > 23 || mm > 59 {
        return Err("计划时间格式必须为 HH:MM".into());
    }
    Ok(())
}

/// 将 "HH:MM" 转换为下一次该本地时刻的 Unix 毫秒时间戳。
///
/// 实现使用 `SystemTime` + `Duration` 计算，不引入 chrono 依赖。
/// 因为 `SystemTime` 不携带时区信息，这里以系统本地时区为隐式假设
/// （与前端 `new Date()` 行为一致）。
fn next_scheduled_timestamp(hhmm: &str) -> Option<u64> {
    if validate_preset_scheduled_at(Some(hhmm)).is_err() {
        return None;
    }
    let hh: u64 = hhmm[0..2].parse().ok()?;
    let mm: u64 = hhmm[3..5].parse().ok()?;
    // 当前 Unix 毫秒时间戳
    let now_ms = now();
    // 一天的毫秒数
    const DAY_MS: u64 = 24 * 60 * 60 * 1000;
    // 当前 UTC 时刻的当日毫秒偏移
    let today_ms = now_ms % DAY_MS;
    let target_ms = hh * 60 * 60 * 1000 + mm * 60 * 1000;
    // 简单按 UTC 计算"下一次该时刻"。这与本地时区可能有偏差，但与
    // 现有 task.scheduled_at 的语义保持一致（Unix 毫秒时间戳）。
    // 前端在 UI 上显示时会以本地时区格式化。
    let delta = if target_ms > today_ms {
        target_ms - today_ms
    } else {
        DAY_MS - today_ms + target_ms
    };
    Some(now_ms.saturating_add(delta))
}

/// 把预设字段应用到任务（Task 12.6 集成测试的核心纯函数）。
///
/// 这是 `DownloadManager::preset_apply_to_task` 的纯逻辑部分，提取出来便于测试。
/// 仅修改内存中的 `DownloadTask`，不涉及 store/runtime/event。调用方负责持久化和事件。
///
/// 应用字段：`connection_count`、`per_task_speed_limit`、`completion_action`、
/// `scheduled_at`（由 "HH:MM" 转为下一次该时刻的 Unix 毫秒时间戳）。
///
/// 仅在任务处于可安全修改的状态（Queued / Paused / Scheduled / Failed / Cancelled）时应用；
/// 下载中、校验中、网络等待、磁盘不足暂停状态拒绝修改以避免运行时状态混乱。
pub(crate) fn apply_preset_to_task_fields(
    task: &mut DownloadTask,
    preset: &DownloadPreset,
) -> Result<(), String> {
    if !matches!(
        task.status,
        TaskStatus::Queued
            | TaskStatus::Paused
            | TaskStatus::Scheduled
            | TaskStatus::Failed
            | TaskStatus::Cancelled
    ) {
        return Err("任务正在下载或校验，无法应用预设".into());
    }
    task.connection_count = preset.connections;
    task.per_task_speed_limit = preset.speed_limit.unwrap_or(0);
    task.completion_action = preset.completion_action.clone().unwrap_or_default();
    if let Some(hhmm) = preset.scheduled_at.as_deref() {
        if let Some(timestamp) = next_scheduled_timestamp(hhmm) {
            task.scheduled_at = Some(timestamp);
            if task.status == TaskStatus::Queued {
                task.status = TaskStatus::Scheduled;
            }
        }
    } else {
        // 预设没有计划时间，若任务原本 Scheduled 则改为 Queued。
        if task.status == TaskStatus::Scheduled {
            task.status = TaskStatus::Queued;
        }
        task.scheduled_at = None;
    }
    Ok(())
}

fn validate_settings(s: &AppSettings) -> Result<(), String> {
    if s.concurrent_downloads == 0 || s.concurrent_downloads > 16 {
        return Err("同时下载任务必须为 1–16".into());
    }
    if ![1, 2, 4, 8, 16, 32].contains(&s.connections_per_download) {
        return Err("分段连接数无效".into());
    }
    if s.default_completion_action == CompletionAction::RunFile {
        return Err("全局完成动作不能设置为自动运行文件".into());
    }
    if !["system", "blue", "cyan", "green", "purple", "orange"].contains(&s.accent_color.as_str()) {
        return Err("强调色设置无效".into());
    }
    validate_tool_path(&s.yt_dlp_path, "yt-dlp.exe", "yt-dlp")?;
    if s.ffmpeg_path.is_empty() != s.ffprobe_path.is_empty() {
        return Err("自定义 FFmpeg 必须同时提供 ffmpeg.exe 和 ffprobe.exe".into());
    }
    validate_tool_path(&s.ffmpeg_path, "ffmpeg.exe", "FFmpeg")?;
    validate_tool_path(&s.ffprobe_path, "ffprobe.exe", "FFprobe")?;
    if let Some(schedule) = s.scheduled_limit.as_ref() {
        schedule.validate()?;
    }
    Ok(())
}
fn validate_tool_path(value: &str, expected_name: &str, label: &str) -> Result<(), String> {
    if value.is_empty() {
        return Ok(());
    }
    let path = Path::new(value);
    if !path.is_absolute() || !path.is_file() {
        return Err(format!("{label} 路径不存在或不是有效文件"));
    }
    let valid_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(expected_name));
    if !valid_name {
        return Err(format!("{label} 必须选择 {expected_name}"));
    }
    Ok(())
}
fn effective_concurrent_downloads(settings: &AppSettings) -> usize {
    if settings.low_memory_mode {
        1
    } else {
        settings.concurrent_downloads as usize
    }
}
fn effective_connection_count(settings: &AppSettings, requested: u8) -> u8 {
    let requested = requested.clamp(1, 32);
    if settings.low_memory_mode {
        requested.min(2)
    } else {
        requested
    }
}

/// 计算任务实际生效的重试策略（Task 14）。
///
/// 任务级 `retry_policy_override` 优先于全局 `default_retry_policy`。
/// `None` 覆盖表示使用全局默认。返回值始终非空。
pub fn effective_retry_policy(task: &DownloadTask, settings: &AppSettings) -> RetryPolicy {
    task.retry_policy_override
        .clone()
        .unwrap_or_else(|| settings.default_retry_policy.clone())
}

/// 计算给定尝试次数下的退避时长（毫秒，Task 14）。
///
/// - `attempt` 从 1 开始计数（第 1 次失败后的等待时长）。
/// - `Fixed`：始终返回 `initial_backoff_ms`。
/// - `Exponential`：返回 `min(initial_backoff_ms * 2^(attempt-1), max_backoff_ms)`。
///
/// 退避期间连接应停止活动（不占用 server 资源）。
pub fn compute_backoff(policy: &RetryPolicy, attempt: u32) -> u64 {
    let attempt = attempt.max(1);
    match policy.backoff {
        BackoffStrategy::Fixed => policy.initial_backoff_ms,
        BackoffStrategy::Exponential => {
            let shift = attempt.saturating_sub(1).min(31);
            let raw = policy.initial_backoff_ms.saturating_mul(1u64 << shift);
            raw.min(policy.max_backoff_ms)
        }
    }
}

fn build_client(s: &AppSettings) -> Result<reqwest::Client, String> {
    // Task 14: 连接超时由全局默认 RetryPolicy 决定。
    let connection_timeout_secs = s.default_retry_policy.connection_timeout_secs.max(1);
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent(&s.user_agent)
        .connect_timeout(Duration::from_secs(connection_timeout_secs))
        .pool_max_idle_per_host(if s.low_memory_mode { 1 } else { 32 })
        .tcp_nodelay(true)
        .tcp_keepalive(Some(Duration::from_secs(30)))
        .http2_adaptive_window(true)
        .timeout(Duration::from_secs(24 * 60 * 60));
    if s.proxy_mode == "manual" && !s.proxy_url.is_empty() {
        let mut proxy = reqwest::Proxy::all(&s.proxy_url).map_err(|e| e.to_string())?;
        if !s.proxy_username.is_empty() {
            proxy = proxy.basic_auth(&s.proxy_username, &s.proxy_password)
        }
        builder = builder.proxy(proxy)
    } else if s.proxy_mode == "none" {
        builder = builder.no_proxy()
    }
    builder.build().map_err(|e| e.to_string())
}

/// Task 31：根据任务级 `proxy_override` 构造 reqwest 客户端。
///
/// 优先级：
/// - `task.proxy_override = Some(url)`（非空）：使用任务级代理 URL 与认证。
///   `proxy_auth` 中的密码经 [`crate::proxy::decode_proxy_auth`] 解密为明文后附加。
/// - `task.proxy_override = Some("")`：显式禁用代理（`no_proxy`），覆盖全局 manual。
/// - `task.proxy_override = None`：回退到全局 [`build_client`]（不在此处理）。
///
/// 调用方应仅在 `task.proxy_override.is_some()` 时调用本函数；
/// `None` 情形应直接复用共享 `self.client` 以避免无谓重建。
fn build_task_client(s: &AppSettings, task: &DownloadTask) -> Result<reqwest::Client, String> {
    let connection_timeout_secs = s.default_retry_policy.connection_timeout_secs.max(1);
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent(&s.user_agent)
        .connect_timeout(Duration::from_secs(connection_timeout_secs))
        .pool_max_idle_per_host(if s.low_memory_mode { 1 } else { 32 })
        .tcp_nodelay(true)
        .tcp_keepalive(Some(Duration::from_secs(30)))
        .http2_adaptive_window(true)
        .timeout(Duration::from_secs(24 * 60 * 60));
    match task.proxy_override.as_deref() {
        Some(url) if !url.is_empty() => {
            let mut proxy = reqwest::Proxy::all(url).map_err(|e| e.to_string())?;
            if let Some(auth) = task.proxy_auth.as_ref() {
                if let Some(decoded) = crate::proxy::decode_proxy_auth(auth) {
                    if !decoded.username.is_empty() {
                        proxy = proxy.basic_auth(&decoded.username, &decoded.password);
                    }
                }
            }
            builder = builder.proxy(proxy);
        }
        Some(_) => {
            // Some("")：显式禁用代理。
            builder = builder.no_proxy();
        }
        None => {
            // 理论上不会进入此分支（调用方先检查 is_some）；安全回退到全局 manual。
            if s.proxy_mode == "manual" && !s.proxy_url.is_empty() {
                let mut proxy = reqwest::Proxy::all(&s.proxy_url).map_err(|e| e.to_string())?;
                if !s.proxy_username.is_empty() {
                    proxy = proxy.basic_auth(&s.proxy_username, &s.proxy_password);
                }
                builder = builder.proxy(proxy);
            } else if s.proxy_mode == "none" {
                builder = builder.no_proxy();
            }
        }
    }
    builder.build().map_err(|e| e.to_string())
}
pub(crate) fn safe_name(input: &str) -> String {
    let value: String = input
        .chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let value = value.trim_matches([' ', '.']);
    if value.is_empty() {
        "download".into()
    } else {
        value.chars().take(180).collect()
    }
}

/// Task 21.2：纯校验函数。重命名时文件名必须满足：
/// - 非空（trim 后）
/// - 不含 Windows 非法字符（`<>:"/\|?*`）或控制字符
/// - 不含 `..` 段或以路径分隔符开头（防止路径穿越）
/// - 长度 ≤ 255 字节
///
/// 返回 `Err(String)` 时携带可直接展示的中文错误信息。
fn validate_rename_filename(trimmed: &str) -> Result<(), String> {
    if trimmed.is_empty() {
        return Err("文件名不能为空".into());
    }
    if trimmed
        .chars()
        .any(|c| "<>:\"/\\|?*".contains(c) || c.is_control())
    {
        return Err("文件名包含非法字符（<>:\"/\\|?*）".into());
    }
    if trimmed.contains("..") || trimmed.starts_with('\\') || trimmed.starts_with('/') {
        return Err("文件名不能包含路径分隔符".into());
    }
    if trimmed.len() > 255 {
        return Err("文件名过长（最多 255 字节）".into());
    }
    Ok(())
}
pub(crate) fn category(name: &str) -> String {
    match Path::new(name)
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" | "mkv" | "mov" | "webm" | "m3u8" | "avi" | "flv" | "wmv" | "ts" | "rmvb" | "m4v" | "3gp" => "video",
        "mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" | "wma" | "opus" | "ape" => "audio",
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" | "bmp" | "ico" | "avif" => "images",
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "iso" => "archives",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" | "csv" => "documents",
        "exe" | "msi" | "dmg" | "pkg" | "appimage" | "apk" | "deb" | "rpm" => "apps",
        _ => "other",
    }
    .into()
}
fn header_string(
    response: &reqwest::Response,
    name: reqwest::header::HeaderName,
) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}
fn diagnostic_url(url: &Url) -> String {
    let mut redacted = url.clone();
    let _ = redacted.set_username("");
    let _ = redacted.set_password(None);
    redacted.set_query(None);
    redacted.set_fragment(None);
    redacted.to_string()
}
fn truncate_text(value: String, maximum_chars: usize) -> String {
    value.chars().take(maximum_chars).collect()
}
fn completion_notification(
    settings: &AppSettings,
    task: &DownloadTask,
) -> Option<(String, String)> {
    if !settings.notifications
        || !settings.notify_on_complete
        || task.status != TaskStatus::Completed
    {
        return None;
    }
    Some((
        format!("下载完成：{}", truncate_text(task.file_name.clone(), 80)),
        format!("已保存到 {}", truncate_text(task.destination.clone(), 160)),
    ))
}

/// Task 30.2：下载失败通知文案。
///
/// 与 `completion_notification` 对称：仅当用户启用 `notifications` 且
/// `notify_on_failure` 同时为 true、任务状态为 Failed 时返回标题与正文。
/// `body` 取自 `task.error`（已脱敏），缺失时回退为"未知错误"。
fn failure_notification(settings: &AppSettings, task: &DownloadTask) -> Option<(String, String)> {
    if !settings.notifications || !settings.notify_on_failure || task.status != TaskStatus::Failed {
        return None;
    }
    let body = task
        .error
        .as_deref()
        .map(|e| truncate_text(e.to_string(), 160))
        .unwrap_or_else(|| "未知错误".to_string());
    Some((
        format!("下载失败：{}", truncate_text(task.file_name.clone(), 80)),
        body,
    ))
}
fn parse_content_range(response: &reqwest::Response) -> Option<(u64, u64, u64)> {
    parse_content_range_value(response.headers().get(CONTENT_RANGE)?.to_str().ok()?)
}
fn parse_content_range_value(value: &str) -> Option<(u64, u64, u64)> {
    let value = value.strip_prefix("bytes ")?;
    let (range, total) = value.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start = start.trim().parse().ok()?;
    let end = end.trim().parse().ok()?;
    let total = total.trim().parse().ok()?;
    (start <= end && end < total).then_some((start, end, total))
}

/// 从预检响应中提取资源总长度。
///
/// HEAD/200 响应取 Content-Length；HEAD 被拒绝后回退的 GET + Range 探针返回 206，
/// 此时 Content-Length 是分片长度（如 1 字节），总长度必须取 Content-Range。
fn probe_total_bytes(response: &reqwest::Response) -> u64 {
    if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        parse_content_range(response)
            .map(|(_, _, total)| total)
            .unwrap_or(0)
    } else {
        response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }
}
fn disposition_name(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(CONTENT_DISPOSITION)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|p| p.trim().strip_prefix("filename="))
        .map(|v| safe_name(v.trim_matches(['\"', '\''])))
}
pub(crate) fn friendly_reqwest(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "NETWORK: 连接超时".into()
    } else if error.is_connect() {
        "NETWORK: 无法连接服务器".into()
    } else if error.is_body() || error.is_request() {
        format!("NETWORK: {error}")
    } else {
        error.to_string()
    }
}
pub(crate) fn friendly_body_error(error: reqwest::Error) -> String {
    if error.is_decode() {
        "NETWORK: 响应流因网络或服务器中断而提前结束".into()
    } else {
        friendly_reqwest(error)
    }
}
fn is_network_error(error: &str) -> bool {
    error.contains("NETWORK:")
}
fn path_key(path: &Path) -> String {
    let normalized = path
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
        .and_then(|parent| path.file_name().map(|name| parent.join(name)))
        .unwrap_or_else(|| path.to_path_buf());
    normalized.to_string_lossy().to_ascii_lowercase()
}

fn sort_download_candidates(candidates: &mut [DownloadTask]) {
    // Task 16: 数字越小越优先。先按 priority 升序，同优先级内按 queue_position 升序（创建更早）。
    candidates.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.queue_position.cmp(&right.queue_position))
    });
}

/// 队列调度可观察性（Task 15）：计算任务的等待原因（纯函数，便于测试）。
///
/// 参数：
/// - `task`: 目标任务
/// - `all_tasks`: 所有任务列表（用于统计排在前面的 Queued 任务数）
/// - `active_count`: 当前活动连接数（controls.len()）
/// - `max_concurrent`: 全局并发上限（effective_concurrent_downloads）
/// - `yt_dlp_available`: yt-dlp 是否已安装
/// - `ffmpeg_available`: ffmpeg 是否已安装
///
/// 返回的 `WaitReason` 不会修改任何状态，是只读推断。
fn compute_wait_reason(
    task: &DownloadTask,
    all_tasks: &[DownloadTask],
    active_count: usize,
    max_concurrent: usize,
    yt_dlp_available: bool,
    ffmpeg_available: bool,
) -> WaitReason {
    match task.status {
        // 正在下载、已完成、失败、取消、校验中、等待网络 → 不在等待
        TaskStatus::Downloading
        | TaskStatus::Completed
        | TaskStatus::Failed
        | TaskStatus::Cancelled
        | TaskStatus::Verifying
        | TaskStatus::WaitingNetwork => WaitReason::NotWaiting,

        TaskStatus::Paused => WaitReason::Paused,
        TaskStatus::PausedByLowDisk => WaitReason::PausedByLowDisk,
        TaskStatus::PausedByMetered => WaitReason::PausedByMetered,
        TaskStatus::Interrupted => WaitReason::Interrupted,
        TaskStatus::RemoteChanged => WaitReason::RemoteChanged,

        TaskStatus::Scheduled => {
            let scheduled_at = task
                .scheduled_at
                .map(|ms| ms.to_string())
                .unwrap_or_default();
            WaitReason::WaitingScheduledTime { scheduled_at }
        }

        TaskStatus::Queued => {
            // 1. 媒体任务且工具未安装 → 等待媒体工具
            let (needs_yt_dlp, needs_ffmpeg) = media_task_tool_requirements(task);
            if (needs_yt_dlp && !yt_dlp_available) || (needs_ffmpeg && !ffmpeg_available) {
                return WaitReason::WaitingMediaTools;
            }

            // 2. 并发槽位已满 → 等待并发槽位
            if active_count >= max_concurrent {
                return WaitReason::WaitingConcurrencyLimit {
                    active_count: active_count as u32,
                };
            }

            // 3. 统计排在前面且状态为 Queued 的任务数
            let ahead_count = count_tasks_ahead(task, all_tasks);
            if ahead_count > 0 {
                WaitReason::QueuedBehind { ahead_count }
            } else {
                // 队列中没有更靠前的任务，且有空闲并发槽位 → 即将开始
                WaitReason::NotWaiting
            }
        }
    }
}

/// 判断媒体任务对工具的依赖。
///
/// 返回 `(needs_yt_dlp, needs_ffmpeg)`。非媒体任务返回 `(false, false)`。
/// 当 `format_id` 缺失时，yt-dlp 默认使用 `bestvideo*+bestaudio/best`，
/// 该格式包含 `+`，因此需要 ffmpeg 合并。
fn media_task_tool_requirements(task: &DownloadTask) -> (bool, bool) {
    if let Some(media) = &task.media {
        let format = media
            .format_id
            .as_deref()
            .unwrap_or("bestvideo*+bestaudio/best");
        let needs_ffmpeg = media.requires_ffmpeg || format.contains('+');
        (true, needs_ffmpeg)
    } else {
        (false, false)
    }
}

/// Task 46：从数据库按域名补齐媒体任务缺失的 Cookie/Referer/User-Agent。
///
/// 仅当 `task.headers` 中不存在对应头（大小写不敏感）时才用数据库值填充；
/// 前端显式传入的头始终优先。解密失败（换机器/密文损坏）时安全降级为
/// "无凭证"，不阻塞下载流程。
///
async fn inject_media_credentials(task: &mut DownloadTask, store: &Arc<Store>) {
    let platform = crate::media_platforms::detect_platform(&task.url);
    let is_douyin = platform == crate::media_platforms::MediaPlatform::Douyin || task.url.contains("douyin.com") || task.url.contains("douyinvod.com") || task.url.contains("amemv.com");
    let is_baidu = task.url.contains("baidupcs.com") || task.url.contains("pan.baidu.com");

    let mut has_cookie = task
        .headers
        .keys()
        .any(|k| k.eq_ignore_ascii_case("cookie"));
    let mut has_referer = task
        .headers
        .keys()
        .any(|k| k.eq_ignore_ascii_case("referer") || k.eq_ignore_ascii_case("referrer"));
    let mut has_user_agent = task
        .headers
        .keys()
        .any(|k| k.eq_ignore_ascii_case("user-agent"));

    if let Some(domain) = crate::media_cookies::extract_domain(&task.url) {
        let mut lookup_domains = vec![domain.clone()];
        if is_baidu {
            lookup_domains.push("pan.baidu.com".to_string());
            lookup_domains.push("baidu.com".to_string());
        }
        for d in lookup_domains {
            if let Ok(Some(stored)) = store.media_credential_get_matching(&d).await {
                if !has_cookie && !stored.cookie.is_empty() {
                    task.headers.insert("Cookie".to_string(), stored.cookie);
                    has_cookie = true;
                }
                if !has_referer {
                    if let Some(referer) = stored.referer.filter(|v| !v.trim().is_empty()) {
                        task.headers.insert("Referer".to_string(), referer);
                        has_referer = true;
                    }
                }
                if !has_user_agent {
                    if let Some(ua) = stored.user_agent.filter(|v| !v.trim().is_empty()) {
                        task.headers.insert("User-Agent".to_string(), ua);
                        has_user_agent = true;
                    }
                }
                if has_cookie {
                    break;
                }
            }
        }
    }

    if is_baidu {
        let current_ua = task.headers.get("User-Agent").map(|s| s.as_str()).unwrap_or("");
        if current_ua.is_empty() {
            // 根据直链 URL 中的端点 app_id 智能选择最匹配的 User-Agent 避免 403 签名冲突
            let ua = if task.url.contains("-250528-") || task.url.contains("app_id=250528") {
                "pan.baidu.com"
            } else if task.url.contains("-266719-") || task.url.contains("-498065-") || task.url.contains("-309847-") {
                crate::baidupan::BAIDU_DLINK_USER_AGENT
            } else {
                "pan.baidu.com"
            };
            task.headers.insert("User-Agent".to_string(), ua.to_string());
            has_user_agent = true;
        }
    }

    if !has_user_agent {
        task.headers.insert("User-Agent".to_string(), "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".to_string());
    }
    if !has_referer && is_douyin {
        task.headers.insert("Referer".to_string(), "https://www.douyin.com/".to_string());
    }
}

/// Task 45.4：从 `task.headers` 中移除认证相关头（Cookie/Referer/User-Agent）。
///
/// 用于下载完成后清空临时登录态，避免认证信息持久化到数据库
/// （AGENTS.md §3、§5）。比较使用大小写不敏感的 header name。
/// 移除后 `task.headers` 仍保留其他自定义头（如 X-Custom 等）。
pub(crate) fn clear_auth_headers(headers: &mut std::collections::HashMap<String, String>) {
    headers.retain(|name, _| {
        let lower = name.to_ascii_lowercase();
        !matches!(
            lower.as_str(),
            "cookie" | "referer" | "referrer" | "user-agent"
        )
    });
}

/// Task 45：判断 `task.headers` 是否包含认证相关头（Cookie/Referer/User-Agent）。
///
/// 用于前端展示"包含临时登录态"标记。比较使用大小写不敏感的 header name。
/// 仅测试使用：前端 App.tsx 有自己的 JS 实现（`hasTempAuth`），
/// 此函数为 Rust 侧行为可测试性而保留。
#[cfg(test)]
pub(crate) fn has_auth_headers(headers: &std::collections::HashMap<String, String>) -> bool {
    headers.keys().any(|name| {
        let lower = name.to_ascii_lowercase();
        matches!(
            lower.as_str(),
            "cookie" | "referer" | "referrer" | "user-agent"
        )
    })
}

/// 统计排在目标任务前面且状态为 Queued 的任务数。
///
/// "前面"定义：priority 更小（数字越小越优先），或同优先级但 queue_position 更小（创建更早）。
/// 与 `sort_download_candidates` 的排序逻辑保持一致。
fn count_tasks_ahead(task: &DownloadTask, all_tasks: &[DownloadTask]) -> u32 {
    all_tasks
        .iter()
        .filter(|other| {
            other.id != task.id && other.status == TaskStatus::Queued && is_ahead_of(other, task)
        })
        .count() as u32
}

/// 判断任务 `a` 是否排在任务 `b` 前面。
fn is_ahead_of(a: &DownloadTask, b: &DownloadTask) -> bool {
    if a.priority != b.priority {
        return a.priority < b.priority;
    }
    a.queue_position < b.queue_position
}

fn resolve_output_path(
    task: &DownloadTask,
    reserved_paths: &HashSet<String>,
) -> Result<PathBuf, String> {
    let base = PathBuf::from(&task.destination).join(&task.file_name);
    let reserved = reserved_paths.contains(&path_key(&base));
    if !base.exists() && !reserved {
        return Ok(base);
    }
    match task.collision_policy {
        CollisionPolicy::Overwrite if reserved => {
            Err("另一个未完成任务正在使用同一目标路径".into())
        }
        CollisionPolicy::Overwrite => Ok(base),
        CollisionPolicy::Skip => Err("目标文件已存在，任务已跳过".into()),
        CollisionPolicy::Rename => {
            let stem = base
                .file_stem()
                .and_then(|v| v.to_str())
                .unwrap_or("download");
            let ext = base.extension().and_then(|v| v.to_str());
            for index in 1..10_000 {
                let name = match ext {
                    Some(ext) => format!("{stem} ({index}).{ext}"),
                    None => format!("{stem} ({index})"),
                };
                let candidate = base.with_file_name(name);
                if !candidate.exists() && !reserved_paths.contains(&path_key(&candidate)) {
                    return Ok(candidate);
                }
            }
            Err("无法生成不重复的文件名".into())
        }
    }
}
/// 校验和算法。按预期校验值长度识别：32 位十六进制 = MD5，40 = SHA-1，64 = SHA-256。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ChecksumAlgorithm {
    Md5,
    Sha1,
    Sha256,
}

impl ChecksumAlgorithm {
    fn label(self) -> &'static str {
        match self {
            ChecksumAlgorithm::Md5 => "MD5",
            ChecksumAlgorithm::Sha1 => "SHA-1",
            ChecksumAlgorithm::Sha256 => "SHA-256",
        }
    }
}

/// 解析用户提供的预期校验值：识别 MD5 / SHA-1 / SHA-256（十六进制，长度 32/40/64），
/// 支持可选的 `md5:` / `sha1:` / `sha256:` 前缀（大小写不敏感）。
/// 返回 (算法, 去前缀的小写十六进制)；无法识别时返回 `None`。
fn parse_expected_checksum(expected: &str) -> Option<(ChecksumAlgorithm, String)> {
    let lowered = expected.trim().to_ascii_lowercase();
    let cleaned = lowered
        .strip_prefix("md5:")
        .or_else(|| lowered.strip_prefix("sha1:"))
        .or_else(|| lowered.strip_prefix("sha-1:"))
        .or_else(|| lowered.strip_prefix("sha256:"))
        .or_else(|| lowered.strip_prefix("sha-256:"))
        .unwrap_or(&lowered);
    let algorithm = match cleaned.len() {
        32 => ChecksumAlgorithm::Md5,
        40 => ChecksumAlgorithm::Sha1,
        64 => ChecksumAlgorithm::Sha256,
        _ => return None,
    };
    if !cleaned.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some((algorithm, cleaned.to_string()))
}

/// 流式计算文件摘要（1MB 缓冲），按需只维护所选算法的哈希状态。
async fn digest_file(path: &Path, algorithm: ChecksumAlgorithm) -> Result<String, String> {
    let mut file = fs::File::open(path).await.map_err(|e| e.to_string())?;
    let mut md5 = Md5::new();
    let mut sha1 = Sha1::new();
    let mut sha256 = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let n = file.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        match algorithm {
            ChecksumAlgorithm::Md5 => md5.update(&buffer[..n]),
            ChecksumAlgorithm::Sha1 => sha1.update(&buffer[..n]),
            ChecksumAlgorithm::Sha256 => sha256.update(&buffer[..n]),
        }
    }
    Ok(hex::encode(match algorithm {
        ChecksumAlgorithm::Md5 => md5.finalize().to_vec(),
        ChecksumAlgorithm::Sha1 => sha1.finalize().to_vec(),
        ChecksumAlgorithm::Sha256 => sha256.finalize().to_vec(),
    }))
}

async fn sha256_file(path: &Path) -> Result<String, String> {
    digest_file(path, ChecksumAlgorithm::Sha256).await
}


#[cfg(test)]
mod tests;


/// Task 30.2 Windows 原生 Toast 通知辅助函数。
///
/// 使用 `tauri-winrt-notification` 直接调用 WinRT API 发送带 `on_activated`
/// 回调的 Toast。回调在当前运行进程的线程中同步触发，无需 COM Activator
/// 注册，解决了 `tauri-plugin-notification` 在 Windows 上无法触发点击回调的问题。
///
/// 点击通知时 emit `notification-focus-task` 事件，payload 为 `task_id`，
/// 前端监听该事件后激活主窗口并高亮对应任务。
#[cfg(windows)]
fn notify_win_toast<R: tauri::Runtime>(
    app_id: &str,
    title: &str,
    body: &str,
    task_id: String,
    app: tauri::AppHandle<R>,
) -> Result<(), String> {
    use tauri_winrt_notification::Toast;
    use tauri::Manager;
    // 开发构建（debug_assertions，如 `pnpm tauri dev`）没有注册 AUMID，
    // 回退到 PowerShell AUMID 保证 Toast 能弹出（图标显示为 PowerShell）；
    // 正式构建——无论从安装目录运行还是直接运行 target\release 产物——一律用应用标识。
    // 不再用 exe 路径判断：`pnpm tauri build` 的产物同样位于 target\release，会被误判。
    let effective_app_id = if cfg!(debug_assertions) {
        Toast::POWERSHELL_APP_ID.to_string()
    } else {
        app_id.to_string()
    };

    Toast::new(&effective_app_id)
        .title(title)
        .text1(body)
        .on_activated(move |_action_arg| {
            // 在同一进程内直接触发：show 窗口 + 定位任务
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
            let _ = app.emit("notification-focus-task", task_id.clone());
            Ok(())
        })
        .show()
        .map_err(|e| e.to_string())
}
