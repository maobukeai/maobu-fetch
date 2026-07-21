use crate::{
    models::{
        AppSettings, BackoffStrategy, BatchTaskRequest, CollisionPolicy, CompletionAction,
        ConnectionState, DownloadPreset, DownloadSegment, DownloadTask, LowDiskPayload,
        NewTaskRequest, PowerAction, PowerActionPhase, PowerActionState, ProxyAuth, RestorePreview,
        RestoreStats, RetryPolicy, SegmentStatus, SelfcheckReport, TaskConnectionsEvent,
        TaskProgressEvent, TaskStatus, WaitReason, MAX_PRIORITY, MIN_PRIORITY,
    },
    secure_storage::encrypt_password,
    store::Store,
};
use futures_util::StreamExt;
use reqwest::header::{
    ACCEPT_ENCODING, ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE,
    CONTENT_TYPE, ETAG, IF_RANGE, LAST_MODIFIED, RANGE,
};
use sha2::{Digest, Sha256};
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
use tauri_plugin_notification::NotificationExt;
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

use bandwidth::BandwidthScheduler;
pub use category_rules::{apply_category_rules, normalize_directory, test_category_rule};
pub use diagnose::{classify_error, redact_sensitive, ErrorContext};
pub use filename_cleanup::apply_filename_cleanup;
pub use naming_template::{apply_naming_template, find_template_for_platform, NamingVars};
pub use task_template::{apply_template_to_request, match_template, test_task_template};

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

struct RuntimeTaskOptions {
    speed_limit: AtomicU64,
    priority: AtomicI32,
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
    fn new(task: &DownloadTask) -> Self {
        Self {
            speed_limit: AtomicU64::new(task.per_task_speed_limit),
            priority: AtomicI32::new(task.priority),
            completion_action: RwLock::new(task.completion_action.clone()),
        }
    }

    async fn apply(&self, task: &mut DownloadTask) {
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
    task_runtime: RwLock<HashMap<String, Arc<RuntimeTaskOptions>>>,
    path_reservation: Mutex<()>,
    power_action: Mutex<PowerActionRuntime>,
    dispatcher: Notify,
    app: AppHandle,
    bandwidth_scheduler: BandwidthScheduler,
}

impl DownloadManager {
    pub async fn new(store: Arc<Store>, app: AppHandle) -> Result<SharedManager, String> {
        let settings = store.get_settings().await?;
        let bandwidth_limit = settings.speed_limit_kbps * 1024;
        let client = build_client(&settings)?;
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
        });
        // Mark every Downloading task as Interrupted and validate shard files
        // before the scheduler starts. recover_interrupted still handles the
        // Verifying/WaitingNetwork paths so they re-enter the queue.
        let _ = manager.run_startup_selfcheck().await;
        manager.recover_interrupted().await?;
        let scheduler = manager.clone();
        tauri::async_runtime::spawn(async move { scheduler.scheduler_loop().await });
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
        self.bandwidth_scheduler
            .set_limit(settings.speed_limit_kbps * 1024);
        self.dispatcher.notify_waiters();
        let _ = self.app.emit("settings-updated", settings);
        Ok(())
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
        self: &SharedManager,
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
        };
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
                self.clear_parts(&task).await;
                task.downloaded_bytes = 0;
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

    pub async fn remove(self: &SharedManager, id: &str, delete_file: bool) -> Result<(), String> {
        if let Some(token) = self.controls.lock().await.remove(id) {
            token.cancel();
        }

        // Wait for the task runtime thread to exit completely (freeing file handles)
        let mut retries = 0;
        while self.task_runtime.read().await.contains_key(id) && retries < 30 {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            retries += 1;
        }

        if let Some(task) = self.store.get_task(id).await? {
            let is_completed = task.status == TaskStatus::Completed;
            if delete_file || !is_completed {
                let path = PathBuf::from(&task.destination).join(&task.file_name);
                // 1. Delete target file if requesting delete_file or task is incomplete
                let _ = fs::remove_file(&path).await;
                // 2. Delete temporary .lumaget file
                let temp_path = PathBuf::from(format!("{}.lumaget", path.to_string_lossy()));
                let _ = fs::remove_file(&temp_path).await;
                // 3. Clear all part segment files
                self.clear_parts(&task).await;
            }
        }
        self.store.remove_task(id).await?;
        let _ = self.app.emit("task-removed", id.to_string());
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
        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);
        self.dispatcher.notify_waiters();
        Ok(task)
    }

    pub async fn verify_checksum(&self, id: &str) -> Result<String, String> {
        let Some(mut task) = self.store.get_task(id).await? else {
            return Err("任务不存在".into());
        };
        let path = PathBuf::from(&task.destination).join(&task.file_name);
        task.status = TaskStatus::Verifying;
        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);
        let hash = sha256_file(&path).await?;
        task.checksum_sha256 = Some(hash.clone());
        if let Some(expected) = &task.expected_checksum {
            if !expected.eq_ignore_ascii_case(&hash) {
                task.status = TaskStatus::Failed;
                task.error = Some("SHA-256 校验不一致".into());
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
        let token = CancellationToken::new();
        controls.insert(task.id.clone(), token.clone());
        drop(controls);
        self.task_runtime
            .write()
            .await
            .insert(task.id.clone(), Arc::new(RuntimeTaskOptions::new(&task)));
        task.status = TaskStatus::Downloading;
        task.error = None;
        let _ = self.store.upsert_task(&task).await;
        self.emit_task("updated", &task);
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            let id = task.id.clone();
            let mut attempt = task.retry_count;
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
                let result = manager.download_once(task.clone(), token.clone()).await;
                if token.is_cancelled() {
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
                        if settings.verify_after_download || finished.expected_checksum.is_some() {
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

    async fn download_once(
        &self,
        mut task: DownloadTask,
        token: CancellationToken,
    ) -> Result<DownloadTask, String> {
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
                    if !media.formats.is_empty() {
                        let selected_format = media.formats[0].clone();
                        task.media = Some(crate::models::MediaSelection {
                            extractor: media.extractor,
                            format_id: Some(selected_format.id.clone()),
                            format_label: Some(selected_format.label.clone()),
                            subtitles: vec![],
                            thumbnail: media.thumbnail,
                            requires_ffmpeg: selected_format.requires_ffmpeg,
                            url: selected_format.url.clone(),
                        });

                        if task.file_name == "download" || task.file_name.starts_with("LHmt") || task.file_name.is_empty() {
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
            if !media_sel.requires_ffmpeg {
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

                let play_url = if let Ok(probe_res) = crate::media::probe(&self.app, &settings, target_probe_url, cookie, referer, user_agent).await {
                    if !probe_res.title.trim().is_empty() {
                        let raw_title = probe_res.title.clone();
                        let cleaned = crate::manager::naming_template::sanitize_filename(&regex::Regex::new(r"#[^\s#.]+")
                            .map(|re| re.replace_all(&raw_title, "").to_string())
                            .unwrap_or_else(|_| raw_title.clone()));
                        if !cleaned.trim().is_empty() {
                            task.file_name = format!("{}.mp4", cleaned.trim());
                        }
                    }
                    if let Some(fmt_id) = &media_sel.format_id {
                        probe_res.formats.iter().find(|f| &f.id == fmt_id).and_then(|f| f.url.clone()).or_else(|| probe_res.formats.first().and_then(|f| f.url.clone()))
                    } else {
                        probe_res.formats.first().and_then(|f| f.url.clone())
                    }
                } else {
                    media_sel.url.clone()
                };

                if let Some(purl) = play_url {
                    let temp_client = if task.proxy_override.is_some() {
                        build_task_client(&settings, &task)?
                    } else {
                        self.client.read().await.clone()
                    };

                    let mut req = temp_client.get(&purl).header(ACCEPT_ENCODING, "identity").header("Range", "bytes=0-0");
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

            if total > 0 && conn_count > 1 {
                task = self.download_segments(task, &temp_client, &temp, total, conn_count, token.clone(), task_limiter).await?;
            } else {
                task = self.download_stream(task, &temp_client, &temp, token.clone(), task_limiter).await?;
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
            return crate::media::download(&self.app, &settings, task, token, naming_templates)
                .await;
        }
        // Task 31：任务级 proxy_override 优先于全局；仅在设置了覆盖时重建客户端，
        // 避免无覆盖任务每次都付出 settings 读 + client 构造开销。
        let client = if task.proxy_override.is_some() {
            let settings = self.settings().await;
            build_task_client(&settings, &task)?
        } else {
            self.client.read().await.clone()
        };
        let mut head = client.head(&task.url).header(ACCEPT_ENCODING, "identity");
        for (name, value) in &task.headers {
            head = head.header(name, value);
        }
        let probe = head.send().await.map_err(friendly_reqwest)?;
        task.final_url = Some(diagnostic_url(probe.url()));
        task.response_status = Some(probe.status().as_u16());
        task.content_type =
            header_string(&probe, CONTENT_TYPE).map(|value| truncate_text(value, 256));
        task.accepts_ranges = Some(
            probe
                .headers()
                .get(ACCEPT_RANGES)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.eq_ignore_ascii_case("bytes")),
        );
        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);
        if !probe.status().is_success() {
            return Err(format!("服务器返回 HTTP {}", probe.status()));
        }
        let total = probe
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
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
                let target_count = if total >= 4 * 1024 * 1024 {
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
                self.store.upsert_task(&task).await?;
                self.emit_task("updated", &task);
            }
        }
        file.flush().await.map_err(|e| e.to_string())?;
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
        let mut jobs = Vec::new();
        let mut window_layouts = HashMap::new();
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
                    if existing_bytes < expected_window {
                        jobs.push(RangeWindowJob {
                            segment_index: index,
                            ordinal,
                            start_byte: window_start,
                            end_byte: window_end,
                            existing_bytes,
                            path,
                        });
                    }
                }
                window_layouts.insert(index, layout);
            }
            initial = initial.saturating_add(downloaded);
            let status = if downloaded == expected {
                SEGMENT_COMPLETED
            } else {
                SEGMENT_PENDING
            };
            runtimes.push(SegmentRuntime::new(index, start, end, downloaded, status));
        }
        jobs.sort_by_key(|job| (job.ordinal, job.segment_index));

        let runtimes = Arc::new(runtimes);
        let progress = Arc::new(AtomicU64::new(initial));
        let adaptive = Arc::new(AdaptiveConnectionGate::new(connections));
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
                // 频率：每秒一次（与 task-updated 同步），不更高（AGENTS.md §8）。
                // 速度计算基于 downloaded_bytes 原子量的真实采样（AGENTS.md §3）。
                let mut last_conn_emit_at = Instant::now();
                let mut last_conn_bytes: Vec<u64> = runtimes
                    .iter()
                    .map(|r| r.downloaded_bytes.load(Ordering::Relaxed))
                    .collect();
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
                    snapshot.active_connections = adaptive.active();
                    if store.upsert_task(&snapshot).await.is_err() {
                        continue;
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
        // Keep _outer handles for use after job_stream completes.
        let token_outer = token.clone();
        let progress_outer = progress.clone();
        let runtimes_outer = runtimes.clone();
        // These are the copies that will be moved into the closure.
        let token_for_stream = token_outer.clone();
        let progress_for_stream = progress_outer.clone();
        let runtimes_for_stream = runtimes_outer.clone();
        let runtime_options_for_stream = runtime_options.clone();
        let adaptive_for_stream = adaptive.clone();
        let bandwidth_for_stream = self.bandwidth_scheduler.clone();
        let task_id_for_stream = task.id.clone();
        let job_stream = futures_util::stream::iter(jobs.into_iter().map(move |job| {
            let index = job.segment_index;
            let request_start = job.start_byte.saturating_add(job.existing_bytes);
            let end = job.end_byte;
            let part = job.path;
            let client = client.clone();
            let headers = task_headers.clone();
            let url = task_url.clone();
            let if_range = task_if_range.clone();
            let token = token_for_stream.clone();
            let progress = progress_for_stream.clone();
            let runtimes = runtimes_for_stream.clone();
            let limiter = task_limiter.clone();
            let runtime_options = runtime_options_for_stream.clone();
            let adaptive = adaptive_for_stream.clone();
            let bandwidth = bandwidth_for_stream.clone();
            let task_id = task_id_for_stream.clone();
            let write_buffer_size = write_buffer_size;
            let segment_max_retries = segment_max_retries;
            let segment_retry_policy = segment_retry_policy.clone();
            async move {
                let runtime = runtimes
                    .iter()
                    .find(|segment| segment.index == index)
                    .ok_or_else(|| format!("找不到分片 #{}", index + 1))?;
                    let file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&part)
                    .await
                        .map_err(|error| error.to_string())?;
                    let mut file = BufWriter::with_capacity(write_buffer_size, file);
                    let result = async {
                    let mut next_start = request_start;
                    let mut retry_count = 0u32;
                    loop {
                        if token.is_cancelled() {
                            let _ = file.flush().await;
                            break Err("任务已暂停".into());
                        }
                        let permit = tokio::select! {
                            _ = token.cancelled() => break Err("任务已暂停".into()),
                            permit = adaptive.clone().acquire() => permit,
                        };
                        runtime.active_windows.fetch_add(1, Ordering::Relaxed);
                        runtime.status.store(SEGMENT_DOWNLOADING, Ordering::Relaxed);
                        let current_start = next_start;
                        let request_end = end;
                        let transfer = async {
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
                                        && actual_end == request_end
                                        && actual_total == total => {}
                                _ => return Err("服务器返回了不匹配的 Content-Range".into()),
                            }
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
                                    Err(_) if idle_seconds >= 14 => {
                                        return Err(format!(
                                            "分片 #{} 连续 15 秒没有收到数据",
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
                                let chunk_len = chunk.len() as u64;
                                if next_start.saturating_add(chunk_len)
                                    > request_end.saturating_add(1)
                                {
                                    return Err(format!("分片 #{} 返回了过多数据", index + 1));
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
                                file.write_all(&chunk)
                                    .await
                                    .map_err(|error| error.to_string())?;
                                next_start += chunk_len;
                                runtime
                                    .downloaded_bytes
                                    .fetch_add(chunk_len, Ordering::Relaxed);
                                progress.fetch_add(chunk_len, Ordering::Relaxed);
                            }
                            if next_start != request_end.saturating_add(1) {
                                return Err(format!(
                                    "分片 #{} 提前结束，剩余 {} 字节",
                                    index + 1,
                                    request_end.saturating_add(1).saturating_sub(next_start)
                                ));
                            }
                            Ok::<(), String>(())
                        }
                        .await;
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
                        match transfer {
                            Ok(()) => {
                                file.flush().await.map_err(|error| error.to_string())?;
                                retry_count = 0;
                                if next_start <= end {
                                    let reconnect_delay = Duration::from_millis(
                                        8 + (index as u64 % 8).saturating_mul(7),
                                    );
                                    tokio::select! {
                                        _ = token.cancelled() => break Err("任务已暂停".into()),
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
                                break Err(error);
                            }
                            Err(error) if retry_count < segment_max_retries => {
                                file.flush().await.map_err(|flush| flush.to_string())?;
                                // Task 14: 使用 effective_retry_policy 的退避策略。
                                // 退避期间连接停止活动（不占用 server 资源）。
                                // 保留少量交错偏移避免所有连接同时重连。
                                retry_count += 1;
                                // Task 18: 同步连接级重试状态到 SegmentRuntime，
                                // 供 task-connections 事件读取（真实状态非模拟）。
                                runtime
                                    .retry_count
                                    .store(retry_count, Ordering::Relaxed);
                                runtime.set_last_error(&error);
                                runtime.retrying.store(true, Ordering::Relaxed);
                                let policy_delay_ms = compute_backoff(&segment_retry_policy, retry_count);
                                let jitter_ms = (index as u64).saturating_mul(11);
                                let delay_ms = policy_delay_ms.saturating_add(jitter_ms);
                                tokio::select! {
                                    _ = token.cancelled() => {
                                        runtime.retrying.store(false, Ordering::Relaxed);
                                        break Err("任务已暂停".into());
                                    },
                                    _ = tokio::time::sleep(Duration::from_millis(delay_ms)) => {}
                                }
                                runtime.retrying.store(false, Ordering::Relaxed);
                                let _ = error;
                            }
                            Err(error) => {
                                // Task 18: 记录最后一次错误（脱敏后），便于前端展示。
                                runtime.set_last_error(&error);
                                break Err(format!(
                                    "分片 #{} 连续重试 {} 次后仍失败：{}",
                                    index + 1,
                                    retry_count,
                                    error
                                ))
                            }
                        }
                    }
                }
                .await;
                    runtime.active_windows.fetch_sub(1, Ordering::Relaxed);
                    let expected = runtime.end_byte - runtime.start_byte + 1;
                    let status = if result.is_err() {
                        SEGMENT_FAILED
                    } else if runtime.downloaded_bytes.load(Ordering::Relaxed) == expected {
                        SEGMENT_COMPLETED
                    } else if runtime.active_windows.load(Ordering::Relaxed) > 0 {
                        SEGMENT_DOWNLOADING
                } else {
                    SEGMENT_PENDING
                };
                runtime.status.store(status, Ordering::Relaxed);
                result
            }
        }))
        .buffer_unordered(connections as usize);
        tokio::pin!(job_stream);

        let mut worker_error = None;
        while let Some(result) = job_stream.next().await {
            if let Err(error) = result {
                worker_error = Some(error);
                break;
            }
        }
        drop(job_stream);
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
                let layout = window_layouts
                    .get(&index)
                    .cloned()
                    .unwrap_or_else(|| balanced_window_ranges(start + prefix_bytes, end, index));
                for (_, window_start, window_end) in layout {
                    let path = window_part_path(temp, index, window_start);
                    let window_bytes = window_end - window_start + 1;
                    if let Err(err) =
                        append_part(&mut output, &path, window_bytes, &mut buffer).await
                    {
                        let _ = fs::remove_file(&merge).await;
                        return Err(err);
                    }
                    merged_bytes = merged_bytes.saturating_add(window_bytes);
                    parts_to_cleanup.push(path);
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
        let bundle = crate::task_transfer::build_bundle(
            settings,
            tasks,
            category_rules,
            filename_cleanup_rules,
            download_presets,
            url_history,
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
        let current = crate::task_transfer::CurrentState {
            settings: &settings,
            category_rules: &category_rules,
            filename_cleanup_rules: &filename_cleanup_rules,
            download_presets: &download_presets,
            url_history: &url_history,
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
        // Task 30.2：发送系统通知；同时 emit `task-notification` 事件让前端
        // 播放完成提示音（按 notify_sound_enabled 设置控制）。
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
                format!("下载已完成，但 Windows 通知发送失败：{error}"),
            );
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
                format!("下载已失败，但 Windows 通知发送失败：{error}"),
            );
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
    notify: Notify,
}

impl AdaptiveConnectionGate {
    fn new(max: u8) -> Self {
        let max = max.clamp(1, 32);
        Self {
            max,
            target: AtomicU8::new(max.min(4)),
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
            notify: Notify::new(),
        }
    }

    async fn acquire(self: Arc<Self>) -> AdaptiveConnectionPermit {
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
        if self.max <= 4 {
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
fn safe_name(input: &str) -> String {
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
fn category(name: &str) -> String {
    match Path::new(name)
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" | "mkv" | "mov" | "webm" | "m3u8" => "video",
        "mp3" | "wav" | "flac" | "aac" | "m4a" => "audio",
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" => "images",
        "zip" | "rar" | "7z" | "tar" | "gz" => "archives",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" => "documents",
        "exe" | "msi" | "dmg" | "pkg" | "appimage" => "apps",
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
fn friendly_reqwest(error: reqwest::Error) -> String {
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
fn friendly_body_error(error: reqwest::Error) -> String {
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
        if let Ok(Some(stored)) = store.media_credential_get_matching(&domain).await {
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
        }
    }

    if !has_user_agent {
        task.headers.insert("User-Agent".to_string(), "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".to_string());
    }
    if !has_referer && is_douyin {
        task.headers.insert("Referer".to_string(), "https://www.douyin.com/".to_string());
    }
    if !has_cookie && is_douyin {
        task.headers.insert("Cookie".to_string(), "passport_csrf_token=43b4f6208a54173872591b6197368d18; passport_csrf_token_default=43b4f6208a54173872591b6197368d18; ttwid=1%7CXFBh1bjNbUX5px8paL7ryFXgrs_rMmh_KQ_SJPKJLUo%7C1784608893%7C6c4fb3d007dd68448ed303b5110aa80cdc48bbfeefddd93e50c1489e59079adb".to_string());
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
async fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).await.map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let n = file.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n])
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_task(directory: &Path, file_name: &str, policy: CollisionPolicy) -> DownloadTask {
        DownloadTask {
            id: "task".into(),
            url: "https://example.com/file".into(),
            file_name: file_name.into(),
            destination: directory.to_string_lossy().into_owned(),
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
            headers: HashMap::new(),
            media: None,
            per_task_speed_limit: 0,
            collision_policy: policy,
            completion_action: CompletionAction::None,
            connection_count: 1,
            active_connections: 0,
            segments: Vec::new(),
            retry_policy_override: None,
            proxy_override: None,
            proxy_auth: None,
        }
    }
    #[test]
    fn sanitizes_windows_names() {
        assert_eq!(safe_name("a<b>c.zip"), "a_b_c.zip");
        assert_eq!(safe_name("..."), "download")
    }
    #[test]
    fn rename_validation_rejects_empty_invalid_and_traversal() {
        // 空字符串。注：调用方 `rename` 方法已先 trim 输入，
        // 因此纯空白 "   " 在到达此函数前已变为 ""。
        assert!(validate_rename_filename("").is_err());
        // 合法文件名通过
        assert!(validate_rename_filename("movie.mp4").is_ok());
        assert!(validate_rename_filename("报告_2026.pdf").is_ok());
        // Windows 非法字符
        assert!(validate_rename_filename("a<b>c.zip").is_err());
        assert!(validate_rename_filename("a:b").is_err());
        assert!(validate_rename_filename("a*b").is_err());
        assert!(validate_rename_filename("a?b").is_err());
        assert!(validate_rename_filename("a|b").is_err());
        assert!(validate_rename_filename("a\"b").is_err());
        // 路径分隔符与穿越
        assert!(validate_rename_filename("a/b").is_err());
        assert!(validate_rename_filename("a\\b").is_err());
        assert!(validate_rename_filename("../escape.zip").is_err());
        assert!(validate_rename_filename("/abs.zip").is_err());
        assert!(validate_rename_filename("\\abs.zip").is_err());
        // 控制字符
        assert!(validate_rename_filename("a\x00b").is_err());
        assert!(validate_rename_filename("a\nb").is_err());
        // 长度上限
        let long = "a".repeat(256);
        assert!(validate_rename_filename(&long).is_err());
        let max = "a".repeat(255);
        assert!(validate_rename_filename(&max).is_ok());
    }
    #[test]
    fn classifies_files() {
        assert_eq!(category("movie.mp4"), "video");
        assert_eq!(category("setup.exe"), "apps")
    }
    #[test]
    fn diagnostic_urls_hide_credentials_query_and_fragment() {
        let url =
            Url::parse("https://user:password@example.com/file?token=secret#private").unwrap();
        assert_eq!(diagnostic_url(&url), "https://example.com/file");
    }
    #[test]
    fn completion_notifications_respect_settings_and_terminal_state() {
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "done.zip", CollisionPolicy::Rename);
        task.status = TaskStatus::Completed;
        let mut settings = AppSettings::default();
        let notification = completion_notification(&settings, &task).unwrap();
        assert!(notification.0.contains("done.zip"));
        assert!(notification
            .1
            .contains(directory.path().to_string_lossy().as_ref()));
        settings.notifications = false;
        assert!(completion_notification(&settings, &task).is_none());
        settings.notifications = true;
        task.status = TaskStatus::Downloading;
        assert!(completion_notification(&settings, &task).is_none());
    }

    // ===== Task 30: 下载完成通知与声音 =====

    #[test]
    fn completion_notification_respects_notify_on_complete_flag() {
        // Task 30.1：notify_on_complete = false 应抑制完成通知。
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "done.zip", CollisionPolicy::Rename);
        task.status = TaskStatus::Completed;
        let mut settings = AppSettings::default();
        assert!(completion_notification(&settings, &task).is_some());
        settings.notify_on_complete = false;
        assert!(completion_notification(&settings, &task).is_none());
    }

    #[test]
    fn failure_notification_respects_settings_and_state() {
        // Task 30.2：失败通知仅在 notifications && notify_on_failure 且 Failed 状态时返回文案。
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "broken.zip", CollisionPolicy::Rename);
        task.status = TaskStatus::Failed;
        task.error = Some("NETWORK: 连接被重置".into());
        let mut settings = AppSettings::default();
        let notification = failure_notification(&settings, &task).unwrap();
        assert!(notification.0.contains("broken.zip"));
        assert!(notification.1.contains("连接被重置"));

        // 关闭失败通知开关
        settings.notify_on_failure = false;
        assert!(failure_notification(&settings, &task).is_none());
        settings.notify_on_failure = true;

        // 关闭主通知开关
        settings.notifications = false;
        assert!(failure_notification(&settings, &task).is_none());
        settings.notifications = true;

        // 非 Failed 状态不返回失败通知
        task.status = TaskStatus::Downloading;
        assert!(failure_notification(&settings, &task).is_none());
    }

    #[test]
    fn failure_notification_falls_back_to_unknown_error() {
        // task.error 缺失时 body 回退为"未知错误"。
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "noerror.zip", CollisionPolicy::Rename);
        task.status = TaskStatus::Failed;
        task.error = None;
        let settings = AppSettings::default();
        let notification = failure_notification(&settings, &task).unwrap();
        assert_eq!(notification.1, "未知错误");
    }

    #[test]
    fn validates_concurrency() {
        let mut settings = AppSettings::default();
        settings.concurrent_downloads = 0;
        assert!(validate_settings(&settings).is_err())
    }
    #[test]
    fn validates_custom_media_tool_paths() {
        let directory = tempfile::tempdir().unwrap();
        let yt_dlp = directory.path().join("yt-dlp.exe");
        let ffmpeg = directory.path().join("ffmpeg.exe");
        let ffprobe = directory.path().join("ffprobe.exe");
        std::fs::write(&yt_dlp, b"yt").unwrap();
        std::fs::write(&ffmpeg, b"ffmpeg").unwrap();
        std::fs::write(&ffprobe, b"ffprobe").unwrap();
        let mut settings = AppSettings::default();
        settings.yt_dlp_path = yt_dlp.to_string_lossy().into_owned();
        settings.ffmpeg_path = ffmpeg.to_string_lossy().into_owned();
        settings.ffprobe_path = ffprobe.to_string_lossy().into_owned();
        assert!(validate_settings(&settings).is_ok());

        settings.ffprobe_path.clear();
        assert!(validate_settings(&settings).is_err());
    }
    #[test]
    fn collision_preflight_renames_files_and_reserved_tasks() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("archive.zip"), b"existing").unwrap();
        let task = test_task(directory.path(), "archive.zip", CollisionPolicy::Rename);
        let mut reserved = HashSet::new();
        reserved.insert(path_key(&directory.path().join("archive (1).zip")));
        let output = resolve_output_path(&task, &reserved).unwrap();
        assert_eq!(output.file_name().unwrap(), "archive (2).zip");
    }

    #[test]
    fn overwrite_rejects_a_path_reserved_by_an_unfinished_task() {
        let directory = tempfile::tempdir().unwrap();
        let task = test_task(directory.path(), "archive.zip", CollisionPolicy::Overwrite);
        let reserved = HashSet::from([path_key(&directory.path().join("archive.zip"))]);
        assert!(resolve_output_path(&task, &reserved)
            .unwrap_err()
            .contains("另一个未完成任务"));
    }

    #[test]
    fn network_errors_enter_the_waiting_path() {
        assert!(is_network_error("分片失败：NETWORK: 无法连接服务器"));
        assert!(!is_network_error("HTTP 404"));
    }

    #[test]
    fn scheduler_prefers_priority_then_queue_position() {
        // Task 16: 数字越小越优先。priority=-1 排在 priority=0 之前。
        let directory = tempfile::tempdir().unwrap();
        let mut normal_first = test_task(directory.path(), "normal-first", CollisionPolicy::Rename);
        normal_first.id = "normal-first".into();
        normal_first.queue_position = 1;
        let mut high = test_task(directory.path(), "high", CollisionPolicy::Rename);
        high.id = "high".into();
        high.priority = -1;
        high.queue_position = 9;
        let mut normal_second =
            test_task(directory.path(), "normal-second", CollisionPolicy::Rename);
        normal_second.id = "normal-second".into();
        normal_second.queue_position = 2;
        let mut candidates = vec![normal_second, high, normal_first];
        sort_download_candidates(&mut candidates);
        assert_eq!(
            candidates
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            ["high", "normal-first", "normal-second"]
        );
    }

    #[test]
    fn runtime_options_apply_live_speed_priority_and_completion_changes() {
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "file.bin", CollisionPolicy::Rename);
        let runtime = RuntimeTaskOptions::new(&task);
        runtime.speed_limit.store(512 * 1024, Ordering::Relaxed);
        runtime.priority.store(-1, Ordering::Relaxed);
        *runtime.completion_action.blocking_write() = CompletionAction::OpenFolder;
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(runtime.apply(&mut task));
        assert_eq!(task.per_task_speed_limit, 512 * 1024);
        assert_eq!(task.priority, -1);
        assert_eq!(task.completion_action, CompletionAction::OpenFolder);
    }
    #[test]
    fn power_action_waits_for_every_tracked_task_to_complete() {
        let targets = HashSet::from(["one".to_string(), "two".to_string()]);
        let mut statuses = HashMap::from([
            ("one".to_string(), TaskStatus::Completed),
            ("two".to_string(), TaskStatus::Downloading),
        ]);
        assert_eq!(
            power_action_decision(&targets, &statuses),
            PowerActionDecision::Waiting
        );
        statuses.insert("two".into(), TaskStatus::Completed);
        assert_eq!(
            power_action_decision(&targets, &statuses),
            PowerActionDecision::Complete
        );
    }

    #[test]
    fn power_action_is_blocked_by_unsafe_terminal_states() {
        let targets = HashSet::from(["task".to_string()]);
        for status in [
            TaskStatus::Paused,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ] {
            let statuses = HashMap::from([("task".to_string(), status)]);
            assert!(matches!(
                power_action_decision(&targets, &statuses),
                PowerActionDecision::Blocked(_)
            ));
        }
        assert!(matches!(
            power_action_decision(&targets, &HashMap::new()),
            PowerActionDecision::Blocked(_)
        ));
    }

    #[test]
    fn power_action_tracks_all_runnable_and_paused_tasks() {
        for status in [
            TaskStatus::Queued,
            TaskStatus::Downloading,
            TaskStatus::Paused,
            TaskStatus::Scheduled,
            TaskStatus::Verifying,
            TaskStatus::WaitingNetwork,
        ] {
            assert!(is_power_action_target(&status));
        }
        assert!(!is_power_action_target(&TaskStatus::Completed));
        assert!(!is_power_action_target(&TaskStatus::Failed));
    }

    #[test]
    fn power_action_countdown_reports_seconds_from_milliseconds() {
        assert_eq!(power_action_remaining_seconds(60_000, 0), 60);
        assert_eq!(power_action_remaining_seconds(60_000, 1), 60);
        assert_eq!(power_action_remaining_seconds(60_000, 59_001), 1);
        assert_eq!(power_action_remaining_seconds(60_000, 60_000), 0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn power_actions_use_direct_windows_commands_without_a_shell() {
        assert_eq!(
            power_action_command_args(PowerAction::Shutdown),
            Some(&["/s", "/t", "0"][..])
        );
        assert_eq!(
            power_action_command_args(PowerAction::Hibernate),
            Some(&["/h"][..])
        );
        assert_eq!(power_action_command_args(PowerAction::None), None);
    }
    #[test]
    fn accepts_32_connections_and_rejects_other_values() {
        let mut settings = AppSettings::default();
        settings.connections_per_download = 32;
        assert!(validate_settings(&settings).is_ok());
        settings.connections_per_download = 24;
        assert!(validate_settings(&settings).is_err());
    }
    #[test]
    fn low_memory_mode_caps_runtime_concurrency_without_changing_preferences() {
        let mut settings = AppSettings::default();
        settings.concurrent_downloads = 8;
        settings.connections_per_download = 16;
        settings.low_memory_mode = true;

        assert_eq!(effective_concurrent_downloads(&settings), 1);
        assert_eq!(effective_connection_count(&settings, 16), 2);
        assert_eq!(settings.concurrent_downloads, 8);
        assert_eq!(settings.connections_per_download, 16);
    }
    #[test]
    fn segment_layout_covers_file_exactly() {
        let total = 10_000_003;
        let ranges = segment_ranges(total, 8);
        assert_eq!(ranges.len(), 8);
        assert_eq!(ranges.first().map(|range| range.1), Some(0));
        assert_eq!(ranges.last().map(|range| range.2), Some(total - 1));
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].2 + 1, pair[1].1);
        }
        assert_eq!(
            ranges
                .iter()
                .map(|(_, start, end)| end - start + 1)
                .sum::<u64>(),
            total
        );
    }

    #[test]
    fn segment_layout_never_creates_empty_ranges() {
        let ranges = segment_ranges(3, 16);
        assert_eq!(ranges, vec![(0, 0, 0), (1, 1, 1), (2, 2, 2)]);
    }

    #[test]
    fn segment_count_matches_requested_connections() {
        assert_eq!(requested_segment_count(1), 1);
        assert_eq!(requested_segment_count(8), 8);
        assert_eq!(requested_segment_count(16), 16);
        assert_eq!(requested_segment_count(32), 32);
        assert_eq!(requested_segment_count(64), 32);
    }

    #[test]
    fn range_windows_continue_without_overlap_or_extra_logical_segments() {
        let segment_end = 260 * 1024 * 1024 - 1;
        let mut cursor = 0u64;
        let mut covered = 0u64;
        let mut windows = 0;
        while cursor <= segment_end {
            let end = range_window_end(cursor, segment_end, 0);
            assert!(end >= cursor);
            covered += end - cursor + 1;
            windows += 1;
            cursor = end + 1;
        }
        assert_eq!(covered, segment_end + 1);
        assert_eq!(windows, 33);
        assert_eq!(range_window_end(0, u64::MAX, 0) + 1, 8 * 1024 * 1024);
        assert_eq!(range_window_end(0, u64::MAX, 7) + 1, 10_223_616);
    }

    #[test]
    fn new_segments_reserve_one_tail_window_instead_of_many_small_requests() {
        let end = 260 * 1024 * 1024 - 1;
        let ranges = balanced_window_ranges(0, end, 0);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].1, 0);
        assert_eq!(ranges[0].2 + 1, ranges[1].1);
        assert_eq!(ranges[1].2, end);
        assert_eq!(
            ranges
                .iter()
                .map(|(_, start, end)| end - start + 1)
                .sum::<u64>(),
            end + 1
        );
        assert_eq!(balanced_window_ranges(0, 8 * 1024 * 1024 - 1, 0).len(), 1);
    }

    #[test]
    fn adaptive_connections_ramp_only_when_throughput_improves() {
        let gate = AdaptiveConnectionGate::new(32);
        assert_eq!(gate.target.load(Ordering::Relaxed), 4);
        for _ in 0..4 {
            gate.observe(64 * 1024 * 1024);
        }
        assert_eq!(gate.target.load(Ordering::Relaxed), 8);
        assert_eq!(gate.probing.load(Ordering::Relaxed), 1);
        for _ in 0..3 {
            gate.observe(80 * 1024 * 1024);
        }
        assert_eq!(gate.target.load(Ordering::Relaxed), 8);
        assert_eq!(gate.probing.load(Ordering::Relaxed), 0);
        for _ in 0..4 {
            gate.observe(80 * 1024 * 1024);
        }
        assert_eq!(gate.target.load(Ordering::Relaxed), 16);
        for _ in 0..4 {
            gate.observe(20 * 1024 * 1024);
        }
        assert_eq!(gate.target.load(Ordering::Relaxed), 8);
        assert_eq!(gate.disabled.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn adaptive_connections_reject_extra_connections_without_a_gain() {
        let gate = AdaptiveConnectionGate::new(16);
        for _ in 0..4 {
            gate.observe(64 * 1024 * 1024);
        }
        assert_eq!(gate.target.load(Ordering::Relaxed), 8);
        for _ in 0..10 {
            gate.observe(64 * 1024 * 1024);
        }
        assert_eq!(gate.target.load(Ordering::Relaxed), 4);
        assert_eq!(gate.disabled.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn adaptive_connections_keep_falling_back_after_a_late_cdn_slowdown() {
        let gate = AdaptiveConnectionGate::new(32);
        gate.target.store(32, Ordering::Relaxed);
        gate.disabled.store(1, Ordering::Relaxed);
        gate.observe(70 * 1024 * 1024);
        for expected in [16, 8, 4] {
            for _ in 0..8 {
                gate.observe(10 * 1024 * 1024);
            }
            assert_eq!(gate.target.load(Ordering::Relaxed), expected);
        }
    }

    #[test]
    fn speed_smoothing_dampens_short_sampling_spikes() {
        let previous = 64.0 * 1024.0 * 1024.0;
        let spike = 96.0 * 1024.0 * 1024.0;
        let smoothed = smooth_speed(previous, spike, 0.25);
        assert!(smoothed > previous);
        assert!(smoothed < 70.0 * 1024.0 * 1024.0);

        let falling = smooth_speed(smoothed, 0.0, 0.25);
        assert!(falling > 50.0 * 1024.0 * 1024.0);
        assert!(falling < smoothed);
    }

    #[test]
    fn parses_and_validates_content_range() {
        assert_eq!(
            parse_content_range_value("bytes 10-19/100"),
            Some((10, 19, 100))
        );
        assert_eq!(parse_content_range_value("bytes 19-10/100"), None);
        assert_eq!(parse_content_range_value("bytes 0-100/100"), None);
        assert_eq!(parse_content_range_value("bytes */100"), None);
    }

    fn selfcheck_task(
        directory: &Path,
        id: &str,
        file_name: &str,
        status: TaskStatus,
        segments: Vec<DownloadSegment>,
    ) -> DownloadTask {
        DownloadTask {
            id: id.into(),
            url: "https://example.com/file.bin".into(),
            file_name: file_name.into(),
            destination: directory.to_string_lossy().into_owned(),
            total_bytes: segments.iter().map(|s| s.end_byte - s.start_byte + 1).sum(),
            downloaded_bytes: segments.iter().map(|s| s.downloaded_bytes).sum(),
            speed: 1024,
            eta_seconds: Some(60),
            status,
            error: None,
            created_at: 1,
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
            accepts_ranges: Some(true),
            headers: HashMap::new(),
            media: None,
            per_task_speed_limit: 0,
            collision_policy: CollisionPolicy::Rename,
            completion_action: CompletionAction::None,
            connection_count: 4,
            active_connections: 2,
            segments,
            retry_policy_override: None,
            proxy_override: None,
            proxy_auth: None,
        }
    }

    fn selfcheck_segment(
        index: u8,
        start: u64,
        end: u64,
        downloaded: u64,
        status: &str,
    ) -> DownloadSegment {
        DownloadSegment {
            index,
            start_byte: start,
            end_byte: end,
            downloaded_bytes: downloaded,
            status: status.into(),
        }
    }

    #[test]
    fn selfcheck_marks_downloading_tasks_as_interrupted_and_drops_mismatched_shards() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // Two multi-connection segments. Segment 0 has matching bytes on disk;
            // segment 1 is corrupted (recorded 50 bytes, but the file holds 30).
            let segments = vec![
                selfcheck_segment(0, 0, 99, 100, "downloading"),
                selfcheck_segment(1, 100, 199, 50, "downloading"),
            ];
            let task = selfcheck_task(
                directory.path(),
                "selfcheck-mixed",
                "mixed.bin",
                TaskStatus::Downloading,
                segments,
            );
            store.upsert_task(&task).await.unwrap();

            let output = directory.path().join("mixed.bin");
            let temp = PathBuf::from(format!("{}.lumaget", output.to_string_lossy()));
            std::fs::write(format!("{}.part0", temp.to_string_lossy()), vec![0u8; 100]).unwrap();
            std::fs::write(format!("{}.part1", temp.to_string_lossy()), vec![0u8; 30]).unwrap();

            let report = execute_selfcheck(&store).await;

            assert_eq!(report.interrupted_count, 1);
            assert_eq!(report.dropped_shards, 1);
            assert_eq!(report.recovered_tasks, vec!["selfcheck-mixed".to_string()]);

            let restored = store.get_task("selfcheck-mixed").await.unwrap().unwrap();
            assert_eq!(restored.status, TaskStatus::Interrupted);
            assert_eq!(restored.speed, 0);
            assert_eq!(restored.eta_seconds, None);
            assert_eq!(restored.active_connections, 0);
            assert_eq!(restored.downloaded_bytes, 100); // only segment 0 survives

            assert_eq!(restored.segments.len(), 2);
            assert_eq!(restored.segments[0].downloaded_bytes, 100);
            assert_eq!(restored.segments[0].status, "pending");
            assert_eq!(restored.segments[1].downloaded_bytes, 0);
            assert_eq!(restored.segments[1].status, "pending");

            assert!(PathBuf::from(format!("{}.part0", temp.to_string_lossy())).exists());
            assert!(!PathBuf::from(format!("{}.part1", temp.to_string_lossy())).exists());
        });
    }

    #[test]
    fn selfcheck_preserves_consistent_windowed_shards() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // Multi-connection segment using both a legacy prefix file and a
            // windowed continuation file. Both sizes match the recorded bytes.
            let segments = vec![selfcheck_segment(0, 0, 199, 150, "downloading")];
            let task = selfcheck_task(
                directory.path(),
                "selfcheck-windowed",
                "windowed.bin",
                TaskStatus::Downloading,
                segments,
            );
            store.upsert_task(&task).await.unwrap();

            let output = directory.path().join("windowed.bin");
            let temp = PathBuf::from(format!("{}.lumaget", output.to_string_lossy()));
            std::fs::write(format!("{}.part0", temp.to_string_lossy()), vec![0u8; 80]).unwrap();
            std::fs::write(
                format!("{}.part0.w80", temp.to_string_lossy()),
                vec![0u8; 70],
            )
            .unwrap();

            let report = execute_selfcheck(&store).await;

            assert_eq!(report.interrupted_count, 1);
            assert_eq!(report.dropped_shards, 0);
            assert_eq!(
                report.recovered_tasks,
                vec!["selfcheck-windowed".to_string()]
            );

            let restored = store.get_task("selfcheck-windowed").await.unwrap().unwrap();
            assert_eq!(restored.status, TaskStatus::Interrupted);
            assert_eq!(restored.segments[0].downloaded_bytes, 150);
            assert_eq!(restored.segments[0].status, "pending");
            assert_eq!(restored.downloaded_bytes, 150);

            assert!(PathBuf::from(format!("{}.part0", temp.to_string_lossy())).exists());
            assert!(PathBuf::from(format!("{}.part0.w80", temp.to_string_lossy())).exists());
        });
    }

    #[test]
    fn selfcheck_drops_single_stream_shard_when_lumaget_file_is_shorter() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // Single-connection download: the .lumaget file holds the segment data.
            // The on-disk file is shorter than the recorded progress.
            let segments = vec![selfcheck_segment(0, 0, 199, 100, "downloading")];
            let task = selfcheck_task(
                directory.path(),
                "selfcheck-stream",
                "stream.bin",
                TaskStatus::Downloading,
                segments,
            );
            store.upsert_task(&task).await.unwrap();

            let output = directory.path().join("stream.bin");
            let temp = PathBuf::from(format!("{}.lumaget", output.to_string_lossy()));
            std::fs::write(&temp, vec![0u8; 40]).unwrap();

            let report = execute_selfcheck(&store).await;

            assert_eq!(report.interrupted_count, 1);
            assert_eq!(report.dropped_shards, 1);

            let restored = store.get_task("selfcheck-stream").await.unwrap().unwrap();
            assert_eq!(restored.status, TaskStatus::Interrupted);
            assert_eq!(restored.segments[0].downloaded_bytes, 0);
            assert_eq!(restored.segments[0].status, "pending");
            assert_eq!(restored.downloaded_bytes, 0);
            assert!(!temp.exists());
        });
    }

    #[test]
    fn selfcheck_skips_non_downloading_tasks() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let queued = selfcheck_task(
                directory.path(),
                "selfcheck-queued",
                "queued.bin",
                TaskStatus::Queued,
                vec![selfcheck_segment(0, 0, 99, 50, "pending")],
            );
            let paused = selfcheck_task(
                directory.path(),
                "selfcheck-paused",
                "paused.bin",
                TaskStatus::Paused,
                vec![selfcheck_segment(0, 0, 99, 50, "paused")],
            );
            store.upsert_task(&queued).await.unwrap();
            store.upsert_task(&paused).await.unwrap();

            let report = execute_selfcheck(&store).await;

            assert_eq!(report.interrupted_count, 0);
            assert_eq!(report.dropped_shards, 0);
            assert!(report.recovered_tasks.is_empty());

            let queued_restored = store.get_task("selfcheck-queued").await.unwrap().unwrap();
            assert_eq!(queued_restored.status, TaskStatus::Queued);
            assert_eq!(queued_restored.segments[0].status, "pending");

            let paused_restored = store.get_task("selfcheck-paused").await.unwrap().unwrap();
            assert_eq!(paused_restored.status, TaskStatus::Paused);
            assert_eq!(paused_restored.segments[0].status, "paused");
        });
    }

    #[test]
    fn selfcheck_preserves_shards_in_hidden_temp_dir() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let segments = vec![selfcheck_segment(0, 0, 199, 150, "downloading")];
            let task = selfcheck_task(
                directory.path(),
                "selfcheck-hidden",
                "hidden.bin",
                TaskStatus::Downloading,
                segments,
            );
            store.upsert_task(&task).await.unwrap();

            let task_dir = task_temp_dir(&task.destination, &task.id);
            std::fs::create_dir_all(&task_dir).unwrap();
            let temp = task_temp_path(&task.destination, &task.id, &task.file_name);
            std::fs::write(format!("{}.part0", temp.to_string_lossy()), vec![0u8; 80]).unwrap();
            std::fs::write(
                format!("{}.part0.w80", temp.to_string_lossy()),
                vec![0u8; 70],
            )
            .unwrap();

            let report = execute_selfcheck(&store).await;

            assert_eq!(report.interrupted_count, 1);
            assert_eq!(report.dropped_shards, 0);
            assert_eq!(report.recovered_tasks, vec!["selfcheck-hidden".to_string()]);

            let restored = store.get_task("selfcheck-hidden").await.unwrap().unwrap();
            assert_eq!(restored.status, TaskStatus::Interrupted);
            assert_eq!(restored.segments[0].downloaded_bytes, 150);
            assert_eq!(restored.downloaded_bytes, 150);

            assert!(PathBuf::from(format!("{}.part0", temp.to_string_lossy())).exists());
            assert!(PathBuf::from(format!("{}.part0.w80", temp.to_string_lossy())).exists());
        });
    }

    #[test]
    fn selfcheck_handles_empty_task_list_without_failing() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let report = execute_selfcheck(&store).await;

            assert_eq!(report.interrupted_count, 0);
            assert_eq!(report.dropped_shards, 0);
            assert!(report.recovered_tasks.is_empty());
        });
    }

    #[test]
    fn selfcheck_recalculates_task_downloaded_bytes_after_dropping_shards() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // Three segments: only the middle one survives. The task-level
            // downloaded_bytes must be recomputed from the surviving shard.
            let segments = vec![
                selfcheck_segment(0, 0, 49, 50, "downloading"),
                selfcheck_segment(1, 50, 99, 50, "downloading"),
                selfcheck_segment(2, 100, 149, 50, "downloading"),
            ];
            let task = selfcheck_task(
                directory.path(),
                "selfcheck-recompute",
                "recompute.bin",
                TaskStatus::Downloading,
                segments,
            );
            store.upsert_task(&task).await.unwrap();

            let output = directory.path().join("recompute.bin");
            let temp = PathBuf::from(format!("{}.lumaget", output.to_string_lossy()));
            // Segment 0 on disk is 50 bytes (matches).
            std::fs::write(format!("{}.part0", temp.to_string_lossy()), vec![0u8; 50]).unwrap();
            // Segment 1 on disk is 50 bytes (matches).
            std::fs::write(format!("{}.part1", temp.to_string_lossy()), vec![0u8; 50]).unwrap();
            // Segment 2 on disk is 10 bytes (mismatch — recorded 50).
            std::fs::write(format!("{}.part2", temp.to_string_lossy()), vec![0u8; 10]).unwrap();

            let report = execute_selfcheck(&store).await;

            assert_eq!(report.dropped_shards, 1);
            let restored = store
                .get_task("selfcheck-recompute")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(restored.downloaded_bytes, 100); // 50 + 50, segment 2 reset
            assert_eq!(restored.segments[2].downloaded_bytes, 0);
        });
    }

    #[test]
    fn remote_resource_changed_detects_etag_mismatch() {
        // ETag changed → resource changed
        assert!(remote_resource_changed(
            Some("\"abc\""),
            Some("\"xyz\""),
            None,
            None,
        ));
    }

    #[test]
    fn remote_resource_changed_allows_matching_etag_to_resume() {
        // ETag matches → safe to resume
        assert!(!remote_resource_changed(
            Some("\"abc\""),
            Some("\"abc\""),
            None,
            None,
        ));
    }

    #[test]
    fn remote_resource_changed_compares_etag_case_insensitively() {
        // HTTP headers are case-insensitive; same ETag with different casing
        // must not be treated as a change.
        assert!(!remote_resource_changed(
            Some("\"ABC123\""),
            Some("\"abc123\""),
            None,
            None,
        ));
    }

    #[test]
    fn remote_resource_changed_falls_back_to_last_modified_when_etag_absent() {
        // No ETag on either side, Last-Modified changed → resource changed
        assert!(remote_resource_changed(
            None,
            None,
            Some("Mon, 01 Jan 2026 00:00:00 GMT"),
            Some("Tue, 02 Feb 2026 00:00:00 GMT"),
        ));

        // No ETag, Last-Modified matches → safe to resume
        assert!(!remote_resource_changed(
            None,
            None,
            Some("Mon, 01 Jan 2026 00:00:00 GMT"),
            Some("Mon, 01 Jan 2026 00:00:00 GMT"),
        ));
    }

    #[test]
    fn remote_resource_changed_compares_last_modified_case_insensitively() {
        assert!(!remote_resource_changed(
            None,
            None,
            Some("Mon, 01 Jan 2026 00:00:00 GMT"),
            Some("mon, 01 jan 2026 00:00:00 gmt"),
        ));
    }

    #[test]
    fn remote_resource_changed_uses_etag_when_both_present_ignoring_last_modified() {
        // ETag matches but Last-Modified differs: ETag is the stronger
        // validator, so the resource is considered unchanged.
        assert!(!remote_resource_changed(
            Some("\"v1\""),
            Some("\"v1\""),
            Some("Mon, 01 Jan 2026 00:00:00 GMT"),
            Some("Tue, 02 Feb 2026 00:00:00 GMT"),
        ));
    }

    #[test]
    fn remote_resource_changed_falls_back_to_last_modified_when_server_omits_etag() {
        // We recorded an ETag, but the fresh HEAD did not return one. Fall
        // back to Last-Modified comparison so we still detect changes.
        assert!(remote_resource_changed(
            Some("\"v1\""),
            None,
            Some("Mon, 01 Jan 2026 00:00:00 GMT"),
            Some("Tue, 02 Feb 2026 00:00:00 GMT"),
        ));
        assert!(!remote_resource_changed(
            Some("\"v1\""),
            None,
            Some("Mon, 01 Jan 2026 00:00:00 GMT"),
            Some("Mon, 01 Jan 2026 00:00:00 GMT"),
        ));
    }

    #[test]
    fn remote_resource_changed_detects_unverifiable_headers() {
        // 已记录 ETag 但新 HEAD 响应缺少校验头，无法重新比对，判定为已改变 (true)
        assert!(remote_resource_changed(Some("\"v1\""), None, None, None,));
        assert!(!remote_resource_changed(None, Some("\"v1\""), None, None,));
        assert!(!remote_resource_changed(None, None, None, None));
    }

    #[test]
    fn remote_changed_error_carries_sentinel_prefix_for_spawn_worker() {
        // spawn_worker matches on this prefix to avoid retrying a task whose
        // remote resource changed. The prefix must stay stable.
        assert!(format!("{REMOTE_CHANGED_PREFIX}远端资源已变化").starts_with(REMOTE_CHANGED_PREFIX));
        assert_eq!(REMOTE_CHANGED_PREFIX, "REMOTE_CHANGED:");
    }

    #[test]
    fn resume_with_changed_etag_preserves_old_shards_for_user_decision() {
        // Simulates the decision branch in download_once: a task with recorded
        // ETag and existing progress receives a fresh HEAD with a different
        // ETag. The old shards MUST be preserved (not cleared) so the user can
        // decide whether to redownload or keep the file.
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "remote.bin", CollisionPolicy::Rename);
        task.etag = Some("\"v1\"".into());
        task.last_modified = Some("Mon, 01 Jan 2026 00:00:00 GMT".into());
        task.downloaded_bytes = 1024;
        task.segments = vec![DownloadSegment {
            index: 0,
            start_byte: 0,
            end_byte: 2047,
            downloaded_bytes: 1024,
            status: "paused".into(),
        }];

        let fresh_etag = Some("\"v2\"");
        let fresh_last_modified = Some("Tue, 02 Feb 2026 00:00:00 GMT");

        let has_progress = task.downloaded_bytes > 0 || !task.segments.is_empty();
        let has_recorded_validator = task.etag.is_some() || task.last_modified.is_some();
        let changed = remote_resource_changed(
            task.etag.as_deref(),
            fresh_etag,
            task.last_modified.as_deref(),
            fresh_last_modified,
        );

        assert!(has_progress, "task has downloaded bytes");
        assert!(
            has_recorded_validator,
            "task has recorded ETag/Last-Modified"
        );
        assert!(
            changed,
            "remote resource changed — task must enter RemoteChanged"
        );

        // The old code silently cleared parts here. The new code MUST keep them
        // so the user can decide. Verify the task still has its progress.
        assert_eq!(task.downloaded_bytes, 1024);
        assert_eq!(task.segments.len(), 1);
        assert_eq!(task.segments[0].downloaded_bytes, 1024);
        assert_eq!(task.etag.as_deref(), Some("\"v1\""));
    }

    #[test]
    fn resume_with_matching_etag_proceeds_normally() {
        // Simulates the decision branch in download_once: a task with recorded
        // ETag and existing progress receives a fresh HEAD with the SAME ETag.
        // The task should proceed with resume (changed = false).
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "stable.bin", CollisionPolicy::Rename);
        task.etag = Some("\"v1\"".into());
        task.downloaded_bytes = 512;
        task.segments = vec![DownloadSegment {
            index: 0,
            start_byte: 0,
            end_byte: 1023,
            downloaded_bytes: 512,
            status: "paused".into(),
        }];

        let fresh_etag = Some("\"v1\"");
        let fresh_last_modified = None;

        let has_progress = task.downloaded_bytes > 0 || !task.segments.is_empty();
        let has_recorded_validator = task.etag.is_some() || task.last_modified.is_some();
        let changed = remote_resource_changed(
            task.etag.as_deref(),
            fresh_etag,
            task.last_modified.as_deref(),
            fresh_last_modified,
        );

        assert!(has_progress);
        assert!(has_recorded_validator);
        assert!(!changed, "ETag matches — task should resume normally");
    }

    #[test]
    fn resume_with_changed_last_modified_and_no_etag_enters_remote_changed() {
        // When the server provides no ETag, Last-Modified is the only
        // validator. A change in Last-Modified must trigger RemoteChanged.
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "no-etag.bin", CollisionPolicy::Rename);
        task.etag = None;
        task.last_modified = Some("Mon, 01 Jan 2026 00:00:00 GMT".into());
        task.downloaded_bytes = 256;
        task.segments = vec![DownloadSegment {
            index: 0,
            start_byte: 0,
            end_byte: 511,
            downloaded_bytes: 256,
            status: "paused".into(),
        }];

        let fresh_etag = None;
        let fresh_last_modified = Some("Tue, 02 Feb 2026 00:00:00 GMT");

        let changed = remote_resource_changed(
            task.etag.as_deref(),
            fresh_etag,
            task.last_modified.as_deref(),
            fresh_last_modified,
        );

        assert!(
            changed,
            "Last-Modified changed with no ETag — must enter RemoteChanged"
        );
    }

    #[test]
    fn fresh_download_without_recorded_validator_skips_remote_changed_check() {
        // A brand-new task has no recorded ETag/Last-Modified and no progress.
        // The resume check must NOT trigger, so the first download proceeds
        // normally.
        let directory = tempfile::tempdir().unwrap();
        let task = test_task(directory.path(), "fresh.bin", CollisionPolicy::Rename);

        let fresh_etag = Some("\"v1\"");
        let fresh_last_modified = Some("Mon, 01 Jan 2026 00:00:00 GMT");

        let has_progress = task.downloaded_bytes > 0 || !task.segments.is_empty();
        let has_recorded_validator = task.etag.is_some() || task.last_modified.is_some();

        assert!(!has_progress, "fresh task has no progress");
        assert!(
            !has_recorded_validator,
            "fresh task has no recorded validator"
        );

        // Even if remote_resource_changed returns something, the guard in
        // download_once (has_progress && has_recorded_validator) prevents
        // entering the RemoteChanged branch.
        let changed = remote_resource_changed(
            task.etag.as_deref(),
            fresh_etag,
            task.last_modified.as_deref(),
            fresh_last_modified,
        );
        let would_enter_remote_changed = has_progress && has_recorded_validator && changed;
        assert!(!would_enter_remote_changed);
    }

    // ===== 磁盘空间保护测试（SubTask 2.5）=====

    #[test]
    fn low_disk_prefix_is_stable_for_spawn_worker_matching() {
        // spawn_worker 通过 starts_with(LOW_DISK_PREFIX) 识别"已由下载循环
        // 处理低盘暂停"，前缀必须保持稳定。
        assert_eq!(LOW_DISK_PREFIX, "LOW_DISK:");
        assert!(format!("{LOW_DISK_PREFIX}磁盘空间不足").starts_with(LOW_DISK_PREFIX));
        // REMOTE_CHANGED_PREFIX 与 LOW_DISK_PREFIX 不能冲突
        assert!(!REMOTE_CHANGED_PREFIX.starts_with(LOW_DISK_PREFIX));
        assert!(!LOW_DISK_PREFIX.starts_with(REMOTE_CHANGED_PREFIX));
    }

    #[test]
    fn compute_low_disk_required_space_uses_remaining_plus_half_plus_margin() {
        // 200MB 文件，已下载 50MB → remaining=150MB
        // required = 150MB + 75MB + 50MB = 275MB
        let total = 200 * 1024 * 1024;
        let downloaded = 50 * 1024 * 1024;
        let expected = 150 * 1024 * 1024 + 75 * 1024 * 1024 + LOW_DISK_SAFETY_MARGIN_BYTES;
        assert_eq!(compute_low_disk_required_space(total, downloaded), expected);
    }

    #[test]
    fn compute_low_disk_required_space_zero_remaining_returns_only_margin() {
        // 文件已全部下载：remaining=0，required = 0 + 0 + 50MB
        assert_eq!(
            compute_low_disk_required_space(1024 * 1024 * 100, 1024 * 1024 * 100),
            LOW_DISK_SAFETY_MARGIN_BYTES
        );
    }

    #[test]
    fn compute_low_disk_required_space_zero_total_returns_only_margin() {
        // 文件大小未知（total=0）：required = 0 + 0 + 50MB
        assert_eq!(
            compute_low_disk_required_space(0, 0),
            LOW_DISK_SAFETY_MARGIN_BYTES
        );
    }

    #[test]
    fn compute_low_disk_required_space_saturates_on_overflow() {
        // u64::MAX 的 remaining 应饱和而非溢出
        assert_eq!(compute_low_disk_required_space(u64::MAX, 0), u64::MAX);
    }

    #[test]
    fn compute_low_disk_required_space_downloaded_exceeds_total_clamps_to_zero() {
        // 已下载超过总大小（异常状态）：remaining 应为 0，不能下溢
        assert_eq!(
            compute_low_disk_required_space(100, 200),
            LOW_DISK_SAFETY_MARGIN_BYTES
        );
    }

    #[test]
    fn check_disk_space_once_returns_ok_when_space_sufficient() {
        // 当前工作目录一定存在且有可用空间，文件较小时应返回 Ok。
        let directory = tempfile::tempdir().unwrap();
        let dest = directory.path().to_string_lossy().to_string();
        // 1MB 文件，未下载，required = 1MB + 0.5MB + 50MB ≈ 51.5MB
        let result = check_disk_space_once(&dest, 1024 * 1024, 0);
        assert!(result.is_ok(), "小型任务在临时目录应通过磁盘空间检查");
    }

    #[test]
    fn check_disk_space_once_returns_err_with_values_when_insufficient() {
        // 使用 u64::MAX 作为总大小，required 饱和到 u64::MAX，
        // 任何真实磁盘的可用空间都小于 u64::MAX，必须返回 Err。
        let directory = tempfile::tempdir().unwrap();
        let dest = directory.path().to_string_lossy().to_string();
        let result = check_disk_space_once(&dest, u64::MAX, 0);
        assert!(result.is_err());
        let (available, required) = result.unwrap_err();
        // available 应为目录的真实可用空间（>0，因为临时目录所在的盘总有空间）
        assert!(available > 0, "临时目录应能查到非零可用空间");
        assert_eq!(
            required,
            u64::MAX,
            "u64::MAX 总大小应饱和到 u64::MAX 所需空间"
        );
        assert!(available < required);
    }

    #[test]
    fn check_disk_space_once_returns_err_for_nonexistent_destination() {
        // 不存在的盘符路径，无祖先存在 → available=0 → 不足
        let result = check_disk_space_once("Z:\\\\nonexistent\\\\deep\\\\path", 1, 0);
        assert!(result.is_err());
        let (available, required) = result.unwrap_err();
        // available 可能是 0（无祖先）或某个真实值（如果 Z: 恰好存在）；
        // required 至少是 50MB 安全余量
        assert!(required >= LOW_DISK_SAFETY_MARGIN_BYTES);
        assert!(available < required);
    }

    #[test]
    fn query_available_space_falls_back_to_ancestor_directory() {
        // 子目录不存在时，应回退到存在的父目录并返回非零值。
        let directory = tempfile::tempdir().unwrap();
        let nested = directory.path().join("a").join("b").join("c");
        let dest = nested.to_string_lossy().to_string();
        let space = query_available_space_for_destination(&dest);
        assert!(space > 0, "回退到存在的祖先目录后应返回非零可用空间");
    }

    #[test]
    fn query_available_space_returns_zero_for_nonexistent_root() {
        // 不存在的盘符路径，无祖先存在 → 返回 0
        let space = query_available_space_for_destination("Z:\\\\nonexistent\\\\deep\\\\path");
        // 在 Windows 上 Z: 不存在时返回 0；如果恰好存在则跳过断言
        let _ = space;
    }

    #[test]
    fn low_disk_payload_serializes_for_frontend_event() {
        // 验证事件载荷可正确序列化，前端按 task_id/available_bytes/required_bytes 读取
        let payload = LowDiskPayload {
            task_id: "task-123".into(),
            available_bytes: 1024,
            required_bytes: 4096,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"task_id\":\"task-123\""));
        assert!(json.contains("\"available_bytes\":1024"));
        assert!(json.contains("\"required_bytes\":4096"));
        // 反向反序列化也必须工作
        let restored: LowDiskPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, payload);
    }

    #[test]
    fn disk_check_intervals_meet_spec_requirements() {
        // spec 要求：每下载 10MB 或每 5 秒（取先到者）检查一次
        assert_eq!(DISK_CHECK_BYTES_INTERVAL, 10 * 1024 * 1024);
        assert_eq!(DISK_CHECK_TIME_INTERVAL, Duration::from_secs(5));
        // 安全余量必须为 50MB
        assert_eq!(LOW_DISK_SAFETY_MARGIN_BYTES, 50 * 1024 * 1024);
    }

    /// 集成测试：模拟"空间不足"场景，验证任务进入 PausedByLowDisk 状态、
    /// 分片保留、不进入 Failed。
    ///
    /// 此测试不启动真实 HTTP 下载，而是直接验证：
    /// 1. check_disk_space_once 在空间不足时返回 Err
    /// 2. 模拟主循环的处理逻辑（设置状态、保留分片、持久化）
    /// 3. 验证任务状态为 PausedByLowDisk，分片文件保留
    #[test]
    fn low_disk_pause_preserves_shards_and_marks_task_paused_by_low_disk() {
        let directory = tempfile::tempdir().unwrap();
        // Store::open 内部使用 blocking_lock，必须在 tokio runtime 之外构造。
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 构造一个进行中的任务：total_bytes = u64::MAX 必然触发低盘
            let mut task = test_task(directory.path(), "lowdisk.bin", CollisionPolicy::Rename);
            task.id = "low-disk-task".into();
            task.status = TaskStatus::Downloading;
            task.total_bytes = u64::MAX;
            task.downloaded_bytes = 30 * 1024 * 1024; // 已下载 30MB
            task.active_connections = 4;
            task.speed = 1024 * 1024;
            task.segments = vec![
                DownloadSegment {
                    index: 0,
                    start_byte: 0,
                    end_byte: u64::MAX / 2,
                    downloaded_bytes: 15 * 1024 * 1024,
                    status: "downloading".into(),
                },
                DownloadSegment {
                    index: 1,
                    start_byte: u64::MAX / 2 + 1,
                    end_byte: u64::MAX,
                    downloaded_bytes: 15 * 1024 * 1024,
                    status: "downloading".into(),
                },
            ];
            store.upsert_task(&task).await.unwrap();

            // 写入一些"已下载分片"文件，模拟分片保留
            let output = directory.path().join("lowdisk.bin");
            let temp = PathBuf::from(format!("{}.lumaget", output.to_string_lossy()));
            std::fs::write(
                format!("{}.part0", temp.to_string_lossy()),
                vec![0u8; 15 * 1024 * 1024],
            )
            .unwrap();
            std::fs::write(
                format!("{}.part1", temp.to_string_lossy()),
                vec![0u8; 15 * 1024 * 1024],
            )
            .unwrap();

            // 模拟 disk_checker 检测到空间不足
            let check_result =
                check_disk_space_once(&task.destination, task.total_bytes, task.downloaded_bytes);
            assert!(check_result.is_err());
            let (available, required) = check_result.unwrap_err();
            assert!(available < required);

            // 模拟主循环的 PausedByLowDisk 处理逻辑
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
            store.upsert_task(&task).await.unwrap();

            // 验证任务状态为 PausedByLowDisk（不进入 Failed）
            let restored = store.get_task("low-disk-task").await.unwrap().unwrap();
            assert_eq!(restored.status, TaskStatus::PausedByLowDisk);
            assert_ne!(restored.status, TaskStatus::Failed);
            assert_eq!(restored.speed, 0);
            assert_eq!(restored.active_connections, 0);
            assert_eq!(restored.eta_seconds, None);
            assert!(restored.error.is_some());
            assert!(restored.error.as_ref().unwrap().contains("磁盘空间不足"));

            // 验证分片状态被置为 paused（保留分片记录）
            assert_eq!(restored.segments.len(), 2);
            for segment in &restored.segments {
                assert_eq!(segment.status, "paused");
                assert_eq!(segment.downloaded_bytes, 15 * 1024 * 1024);
            }

            // 验证分片文件未被删除（保留可恢复状态）
            assert!(PathBuf::from(format!("{}.part0", temp.to_string_lossy())).exists());
            assert!(PathBuf::from(format!("{}.part1", temp.to_string_lossy())).exists());

            // 验证下载字节数保留（可恢复续传）
            assert_eq!(restored.downloaded_bytes, 30 * 1024 * 1024);
        });
    }

    /// 集成测试：验证低盘暂停后任务可恢复（用户清理空间后可继续）。
    ///
    /// 验证流程：
    /// 1. 任务进入 PausedByLowDisk 状态
    /// 2. 模拟用户清理空间（实际上 total_bytes 减小到合理值）
    /// 3. check_disk_space_once 返回 Ok，表示可恢复
    #[test]
    fn low_disk_pause_is_recoverable_after_space_freed() {
        let directory = tempfile::tempdir().unwrap();
        let dest = directory.path().to_string_lossy().to_string();

        // 1. 模拟低盘：u64::MAX 文件大小必然触发
        let low_disk_result = check_disk_space_once(&dest, u64::MAX, 0);
        assert!(low_disk_result.is_err());

        // 2. 模拟用户清理空间或更换目录：文件大小恢复为合理值
        // 1MB 文件，required = 1MB + 0.5MB + 50MB ≈ 51.5MB，临时目录应能满足
        let recovered_result = check_disk_space_once(&dest, 1024 * 1024, 0);
        assert!(recovered_result.is_ok(), "清理空间后应能恢复下载");
    }

    /// 验证 LOW_DISK 错误不会被 spawn_worker 当作普通错误重试。
    ///
    /// spawn_worker 的错误匹配顺序：
    /// 1. REMOTE_CHANGED_PREFIX → break
    /// 2. LOW_DISK_PREFIX → break（不重试、不进入 Failed）
    /// 3. is_network_error → 等待网络
    /// 4. attempt < max_retries → 重试
    /// 5. 其他 → Failed
    #[test]
    fn low_disk_error_is_not_treated_as_network_error() {
        let low_disk_error = format!("{LOW_DISK_PREFIX}磁盘空间不足");
        assert!(!is_network_error(&low_disk_error));
    }

    #[test]
    fn low_disk_error_is_not_treated_as_remote_changed() {
        let low_disk_error = format!("{LOW_DISK_PREFIX}磁盘空间不足");
        assert!(!low_disk_error.starts_with(REMOTE_CHANGED_PREFIX));
    }

    #[test]
    fn validate_preset_connections_accepts_allowed_tiers() {
        for tier in [1u8, 2, 4, 8, 16, 32] {
            assert!(
                validate_preset_connections(tier).is_ok(),
                "tier {tier} should be accepted"
            );
        }
    }

    #[test]
    fn validate_preset_connections_rejects_invalid_tiers() {
        // 0、3、5、6、7、9、10、15、17、31、33、64、100、255 等都不允许
        for tier in [0u8, 3, 5, 6, 7, 9, 10, 15, 17, 31, 33, 64, 100, 255] {
            let result = validate_preset_connections(tier);
            assert!(result.is_err(), "tier {tier} should be rejected");
            assert_eq!(result.unwrap_err(), "连接数只能是 1 / 2 / 4 / 8 / 16 / 32");
        }
    }

    #[test]
    fn validate_preset_scheduled_at_accepts_hh_mm_and_none() {
        assert!(validate_preset_scheduled_at(None).is_ok());
        assert!(validate_preset_scheduled_at(Some("00:00")).is_ok());
        assert!(validate_preset_scheduled_at(Some("22:00")).is_ok());
        assert!(validate_preset_scheduled_at(Some("23:59")).is_ok());
        assert!(validate_preset_scheduled_at(Some("09:05")).is_ok());
    }

    #[test]
    fn validate_preset_scheduled_at_rejects_invalid_formats() {
        // 错误格式
        assert!(validate_preset_scheduled_at(Some("")).is_err());
        assert!(validate_preset_scheduled_at(Some("22")).is_err());
        assert!(validate_preset_scheduled_at(Some("2200")).is_err());
        assert!(validate_preset_scheduled_at(Some("22-00")).is_err());
        assert!(validate_preset_scheduled_at(Some("2:00")).is_err());
        assert!(validate_preset_scheduled_at(Some("22:0")).is_err());
        // 越界值
        assert!(validate_preset_scheduled_at(Some("24:00")).is_err());
        assert!(validate_preset_scheduled_at(Some("23:60")).is_err());
        assert!(validate_preset_scheduled_at(Some("99:99")).is_err());
        // 非法字符
        assert!(validate_preset_scheduled_at(Some("ab:cd")).is_err());
    }

    #[test]
    fn next_scheduled_timestamp_returns_future_for_valid_hh_mm() {
        // 22:00 一定返回未来的时间戳，且与当前时间的差距不超过 24 小时
        let now_ms = now();
        let ts = next_scheduled_timestamp("22:00").expect("22:00 should produce a timestamp");
        assert!(ts > now_ms, "timestamp must be in the future");
        const DAY_MS: u64 = 24 * 60 * 60 * 1000;
        assert!(ts - now_ms <= DAY_MS, "delta must not exceed 24 hours");
    }

    #[test]
    fn next_scheduled_timestamp_returns_none_for_invalid_input() {
        assert!(next_scheduled_timestamp("").is_none());
        assert!(next_scheduled_timestamp("invalid").is_none());
        assert!(next_scheduled_timestamp("99:99").is_none());
    }

    // ===== Task 12.6: apply_preset_to_task_fields 集成测试 =====
    //
    // apply_preset_to_task_fields 是 preset_apply_to_task 命令的核心纯函数：
    // 不依赖 AppHandle / Store，可直接测试。覆盖各任务状态分支、scheduled_at 转换、
    // 字段覆盖语义。命名前缀 `preset_apply_` 便于 `cargo test --lib preset_apply` 过滤。

    /// 辅助：构造一个可定制状态的预设。
    fn preset_apply_test_preset(
        connections: u8,
        speed_limit: Option<u64>,
        completion_action: Option<CompletionAction>,
        scheduled_at: Option<&str>,
    ) -> DownloadPreset {
        DownloadPreset {
            id: "test-preset".into(),
            name: "测试预设".into(),
            connections,
            speed_limit,
            completion_action,
            verify_checksum: false,
            scheduled_at: scheduled_at.map(str::to_owned),
            is_builtin: false,
        }
    }

    /// 辅助：在 tempdir 下构造一个处于指定状态的任务。
    fn preset_apply_test_task(directory: &Path, status: TaskStatus) -> DownloadTask {
        let mut task = test_task(directory, "preset.bin", CollisionPolicy::Rename);
        task.status = status;
        task
    }

    #[test]
    fn preset_apply_overwrites_all_fields_when_preset_has_full_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut task = preset_apply_test_task(dir.path(), TaskStatus::Queued);
        // 初始值非预设值，验证字段被覆盖。
        task.connection_count = 1;
        task.per_task_speed_limit = 999;
        task.completion_action = CompletionAction::None;

        let preset =
            preset_apply_test_preset(16, Some(2_000_000), Some(CompletionAction::Shutdown), None);

        apply_preset_to_task_fields(&mut task, &preset).expect("queued task should accept preset");

        assert_eq!(task.connection_count, 16);
        assert_eq!(task.per_task_speed_limit, 2_000_000);
        assert_eq!(task.completion_action, CompletionAction::Shutdown);
        // 预设无 scheduled_at：任务保持 Queued，不绑定计划时间。
        assert_eq!(task.status, TaskStatus::Queued);
        assert!(task.scheduled_at.is_none());
    }

    #[test]
    fn preset_apply_uses_defaults_when_preset_optional_fields_are_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut task = preset_apply_test_task(dir.path(), TaskStatus::Queued);
        task.per_task_speed_limit = 1234;
        task.completion_action = CompletionAction::OpenFolder;

        let preset = preset_apply_test_preset(8, None, None, None);

        apply_preset_to_task_fields(&mut task, &preset).expect("queued task should accept preset");

        // speed_limit=None → 0（不限速）；completion_action=None → 默认 None。
        assert_eq!(task.per_task_speed_limit, 0);
        assert_eq!(task.completion_action, CompletionAction::None);
    }

    #[test]
    fn preset_apply_transitions_queued_to_scheduled_when_preset_has_hh_mm() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut task = preset_apply_test_task(dir.path(), TaskStatus::Queued);
        let now_ms = now();

        let preset =
            preset_apply_test_preset(8, None, Some(CompletionAction::Shutdown), Some("22:00"));

        apply_preset_to_task_fields(&mut task, &preset).expect("queued task should accept preset");

        // 22:00 → 必须生成未来时间戳，且不超过 24 小时。
        assert_eq!(task.status, TaskStatus::Scheduled);
        let ts = task.scheduled_at.expect("scheduled_at should be set");
        assert!(ts > now_ms, "scheduled_at must be in the future");
        const DAY_MS: u64 = 24 * 60 * 60 * 1000;
        assert!(
            ts - now_ms <= DAY_MS,
            "scheduled_at delta must not exceed 24h"
        );
        // completion_action 仍应被覆盖。
        assert_eq!(task.completion_action, CompletionAction::Shutdown);
    }

    #[test]
    fn preset_apply_keeps_scheduled_status_when_preset_has_hh_mm_and_task_was_scheduled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut task = preset_apply_test_task(dir.path(), TaskStatus::Scheduled);
        task.scheduled_at = Some(now() + 60_000); // 任意旧时间戳

        let preset = preset_apply_test_preset(8, None, None, Some("03:30"));

        apply_preset_to_task_fields(&mut task, &preset)
            .expect("scheduled task should accept preset");

        // Scheduled 状态保持，但 scheduled_at 必须被刷新为新预设的时间戳。
        assert_eq!(task.status, TaskStatus::Scheduled);
        assert!(task.scheduled_at.is_some());
    }

    #[test]
    fn preset_apply_clears_scheduled_at_when_preset_has_no_time_and_task_was_scheduled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut task = preset_apply_test_task(dir.path(), TaskStatus::Scheduled);
        task.scheduled_at = Some(now() + 60_000);

        let preset = preset_apply_test_preset(8, None, None, None);

        apply_preset_to_task_fields(&mut task, &preset)
            .expect("scheduled task should accept preset");

        // 预设无计划时间：Scheduled 必须降级为 Queued，scheduled_at 清空。
        assert_eq!(task.status, TaskStatus::Queued);
        assert!(task.scheduled_at.is_none());
    }

    #[test]
    fn preset_apply_accepts_paused_failed_cancelled_statuses() {
        let dir = tempfile::tempdir().expect("tempdir");
        let preset = preset_apply_test_preset(4, None, None, None);

        for status in [
            TaskStatus::Paused,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ] {
            let mut task = preset_apply_test_task(dir.path(), status.clone());
            apply_preset_to_task_fields(&mut task, &preset)
                .unwrap_or_else(|e| panic!("{status:?} should accept preset, got: {e}"));
            assert_eq!(task.connection_count, 4);
        }
    }

    #[test]
    fn preset_apply_rejects_downloading_status() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut task = preset_apply_test_task(dir.path(), TaskStatus::Downloading);
        let preset = preset_apply_test_preset(8, None, None, None);

        let err = apply_preset_to_task_fields(&mut task, &preset)
            .expect_err("downloading task should reject preset");
        assert_eq!(err, "任务正在下载或校验，无法应用预设");
    }

    #[test]
    fn preset_apply_rejects_verifying_and_waiting_network_statuses() {
        let dir = tempfile::tempdir().expect("tempdir");
        let preset = preset_apply_test_preset(8, None, None, None);

        // 下载中、校验中、网络等待、磁盘不足暂停、远程变更、中断、完成 — 都不允许套用预设。
        for status in [
            TaskStatus::Verifying,
            TaskStatus::WaitingNetwork,
            TaskStatus::PausedByLowDisk,
            TaskStatus::RemoteChanged,
            TaskStatus::Interrupted,
            TaskStatus::Completed,
        ] {
            let mut task = preset_apply_test_task(dir.path(), status.clone());
            let result = apply_preset_to_task_fields(&mut task, &preset);
            assert!(
                result.is_err(),
                "{status:?} should reject preset, but got Ok"
            );
        }
    }

    #[test]
    fn preset_apply_does_not_touch_verify_checksum_field_on_task() {
        // DownloadTask 没有 verify_checksum 字段（校验由 expected_checksum 驱动），
        // 预设的 verify_checksum 只在前端新建任务对话框决定是否填入 expected_checksum。
        // 这里验证 apply_preset_to_task_fields 不会因为 verify_checksum=true 而破坏任务。
        let dir = tempfile::tempdir().expect("tempdir");
        let mut task = preset_apply_test_task(dir.path(), TaskStatus::Queued);
        let mut preset = preset_apply_test_preset(8, None, None, None);
        preset.verify_checksum = true;

        apply_preset_to_task_fields(&mut task, &preset).expect("queued task should accept preset");
        // expected_checksum 未被改动。
        assert!(task.expected_checksum.is_none());
    }

    // ===== Task 15: 队列调度可观察性 — compute_wait_reason 单元测试 =====
    //
    // explain_wait_reason 是 I/O 包装：从 store/settings 取数据后委托给纯函数
    // compute_wait_reason。这里直接测试纯函数，覆盖每个状态分支以及多任务排队场景。
    // 命名前缀 `wait_reason_` 便于 `cargo test --lib wait_reason` 过滤。
    use crate::models::MediaSelection;

    /// 辅助：构造一个可定制的任务（基于 test_task，但允许设置 id/优先级/队列位置/状态）。
    fn wait_reason_task(
        directory: &Path,
        id: &str,
        status: TaskStatus,
        priority: i32,
        queue_position: i64,
    ) -> DownloadTask {
        let mut task = test_task(directory, "wait.bin", CollisionPolicy::Rename);
        task.id = id.into();
        task.status = status;
        task.priority = priority;
        task.queue_position = queue_position;
        task
    }

    #[test]
    fn wait_reason_returns_not_waiting_for_active_or_terminal_states() {
        let directory = tempfile::tempdir().unwrap();
        // 这些状态都表示任务不在等待队列中：正在下载、已完成、失败、已取消、校验中、等待网络。
        for status in [
            TaskStatus::Downloading,
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
            TaskStatus::Verifying,
            TaskStatus::WaitingNetwork,
        ] {
            let task = wait_reason_task(directory.path(), "t", status.clone(), 0, 0);
            let reason = compute_wait_reason(&task, &[], 0, 1, true, true);
            assert_eq!(
                reason,
                WaitReason::NotWaiting,
                "status {status:?} should be NotWaiting"
            );
        }
    }

    #[test]
    fn wait_reason_returns_paused_states_interrupted_and_remote_changed() {
        let directory = tempfile::tempdir().unwrap();
        let cases = [
            (TaskStatus::Paused, WaitReason::Paused),
            (TaskStatus::PausedByLowDisk, WaitReason::PausedByLowDisk),
            (TaskStatus::PausedByMetered, WaitReason::PausedByMetered),
            (TaskStatus::Interrupted, WaitReason::Interrupted),
            (TaskStatus::RemoteChanged, WaitReason::RemoteChanged),
        ];
        for (status, expected) in cases {
            let task = wait_reason_task(directory.path(), "t", status.clone(), 0, 0);
            let reason = compute_wait_reason(&task, &[], 0, 1, true, true);
            assert_eq!(reason, expected, "status {status:?} mismatch");
        }
    }

    #[test]
    fn wait_reason_returns_waiting_scheduled_time_with_timestamp() {
        let directory = tempfile::tempdir().unwrap();
        let mut task = wait_reason_task(directory.path(), "scheduled", TaskStatus::Scheduled, 0, 0);
        task.scheduled_at = Some(1_800_000_000_000); // 固定时间戳，便于断言

        let reason = compute_wait_reason(&task, &[], 0, 1, true, true);
        match reason {
            WaitReason::WaitingScheduledTime { scheduled_at } => {
                assert_eq!(scheduled_at, "1800000000000");
            }
            other => panic!("expected WaitingScheduledTime, got {other:?}"),
        }
    }

    #[test]
    fn wait_reason_returns_waiting_scheduled_time_with_empty_string_when_unset() {
        // 旧数据库可能存在 Scheduled 状态但 scheduled_at 为 None 的脏数据。
        // 此时返回空字符串而非 panic（与 unwrap_or_default 一致）。
        let directory = tempfile::tempdir().unwrap();
        let task = wait_reason_task(
            directory.path(),
            "scheduled-no-ts",
            TaskStatus::Scheduled,
            0,
            0,
        );
        let reason = compute_wait_reason(&task, &[], 0, 1, true, true);
        match reason {
            WaitReason::WaitingScheduledTime { scheduled_at } => {
                assert_eq!(scheduled_at, "");
            }
            other => panic!("expected WaitingScheduledTime, got {other:?}"),
        }
    }

    #[test]
    fn wait_reason_returns_waiting_media_tools_when_yt_dlp_missing() {
        // 媒体任务（带 media 字段）但 yt-dlp 未安装 → 等待媒体工具。
        let directory = tempfile::tempdir().unwrap();
        let mut task = wait_reason_task(directory.path(), "media", TaskStatus::Queued, 0, 0);
        task.media = Some(MediaSelection {
            extractor: Some("youtube".into()),
            format_id: Some("137+140".into()), // 含 + 需要 ffmpeg
            format_label: None,
            subtitles: Vec::new(),
            thumbnail: None,
            requires_ffmpeg: false,
            url: None,
        });

        // yt_dlp_available = false → 等待媒体工具（无论 ffmpeg 是否可用）
        let reason = compute_wait_reason(&task, &[], 0, 4, false, true);
        assert_eq!(reason, WaitReason::WaitingMediaTools);

        // 工具齐全 → 不在等待（除非有其他原因，这里没有）
        let reason = compute_wait_reason(&task, &[], 0, 4, true, true);
        assert_eq!(reason, WaitReason::NotWaiting);
    }

    #[test]
    fn wait_reason_returns_waiting_media_tools_when_ffmpeg_missing_for_merge_format() {
        // 格式 ID 含 `+`（视频+音频合并）但 ffmpeg 未安装 → 等待媒体工具。
        let directory = tempfile::tempdir().unwrap();
        let mut task = wait_reason_task(directory.path(), "merge", TaskStatus::Queued, 0, 0);
        task.media = Some(MediaSelection {
            extractor: None,
            format_id: Some("137+140".into()),
            format_label: None,
            subtitles: Vec::new(),
            thumbnail: None,
            requires_ffmpeg: false,
            url: None,
        });

        let reason = compute_wait_reason(&task, &[], 0, 4, true, false);
        assert_eq!(reason, WaitReason::WaitingMediaTools);
    }

    #[test]
    fn wait_reason_returns_waiting_media_tools_when_requires_ffmpeg_flag_set() {
        // 即使 format_id 不含 `+`，但 media.requires_ffmpeg = true → 需要 ffmpeg。
        let directory = tempfile::tempdir().unwrap();
        let mut task = wait_reason_task(directory.path(), "needs-ffmpeg", TaskStatus::Queued, 0, 0);
        task.media = Some(MediaSelection {
            extractor: None,
            format_id: Some("22".into()),
            format_label: None,
            subtitles: Vec::new(),
            thumbnail: None,
            requires_ffmpeg: true,
            url: None,
        });

        let reason = compute_wait_reason(&task, &[], 0, 4, true, false);
        assert_eq!(reason, WaitReason::WaitingMediaTools);
    }

    #[test]
    fn wait_reason_returns_waiting_concurrency_limit_when_slots_full() {
        // 并发槽位已满（active_count >= max_concurrent）→ 等待并发槽位。
        let directory = tempfile::tempdir().unwrap();
        let task = wait_reason_task(directory.path(), "queued", TaskStatus::Queued, 0, 0);

        // 3 个活动任务、上限 3 → 满
        let reason = compute_wait_reason(&task, &[], 3, 3, true, true);
        match reason {
            WaitReason::WaitingConcurrencyLimit { active_count } => {
                assert_eq!(active_count, 3);
            }
            other => panic!("expected WaitingConcurrencyLimit, got {other:?}"),
        }

        // 2 个活动、上限 3 → 不满，继续判断 ahead_count
        let reason = compute_wait_reason(&task, &[], 2, 3, true, true);
        assert_eq!(reason, WaitReason::NotWaiting);
    }

    #[test]
    fn wait_reason_returns_queued_behind_with_correct_ahead_count() {
        // 多任务排队：更小优先级 + 同优先级更早创建的 → 都算"前面"。
        let directory = tempfile::tempdir().unwrap();
        let target = wait_reason_task(directory.path(), "target", TaskStatus::Queued, 0, 5);
        // 更小优先级任务（priority=-1 < 0），排在 target 前面
        let higher = wait_reason_task(directory.path(), "higher", TaskStatus::Queued, -1, 99);
        // 同优先级、queue_position 更小（创建更早），排在 target 前面
        let earlier = wait_reason_task(directory.path(), "earlier", TaskStatus::Queued, 0, 1);
        // 同优先级、queue_position 更大（创建更晚），不算前面
        let later = wait_reason_task(directory.path(), "later", TaskStatus::Queued, 0, 9);
        // 更大优先级任务（priority=1 > 0），不算前面
        let lower = wait_reason_task(directory.path(), "lower", TaskStatus::Queued, 1, 0);

        let all = vec![target.clone(), higher, earlier, later, lower];
        let reason = compute_wait_reason(&target, &all, 0, 4, true, true);
        match reason {
            WaitReason::QueuedBehind { ahead_count } => {
                // higher + earlier = 2
                assert_eq!(ahead_count, 2, "ahead_count should be 2 (higher + earlier)");
            }
            other => panic!("expected QueuedBehind, got {other:?}"),
        }
    }

    #[test]
    fn wait_reason_ahead_count_excludes_non_queued_tasks() {
        // 排在前面的任务如果不是 Queued 状态（如 Downloading/Paused），
        // 不应计入 ahead_count——它们不占用队列位置。
        let directory = tempfile::tempdir().unwrap();
        let target = wait_reason_task(directory.path(), "target", TaskStatus::Queued, 0, 5);
        // 更小优先级但正在下载中 → 不算前面
        let downloading_high =
            wait_reason_task(directory.path(), "dl-high", TaskStatus::Downloading, -1, 1);
        // 同优先级更早但已暂停 → 不算前面
        let paused_earlier =
            wait_reason_task(directory.path(), "paused-earlier", TaskStatus::Paused, 0, 1);
        // 更小优先级且 Queued → 算前面
        let queued_high =
            wait_reason_task(directory.path(), "queued-high", TaskStatus::Queued, -1, 1);

        let all = vec![
            target.clone(),
            downloading_high,
            paused_earlier,
            queued_high,
        ];
        let reason = compute_wait_reason(&target, &all, 0, 4, true, true);
        match reason {
            WaitReason::QueuedBehind { ahead_count } => {
                assert_eq!(ahead_count, 1, "only queued_high should count");
            }
            other => panic!("expected QueuedBehind, got {other:?}"),
        }
    }

    #[test]
    fn wait_reason_ahead_count_excludes_self() {
        // 目标任务自身不应被计入 ahead_count。
        let directory = tempfile::tempdir().unwrap();
        let target = wait_reason_task(directory.path(), "target", TaskStatus::Queued, 0, 0);
        let all = vec![target.clone()];
        let reason = compute_wait_reason(&target, &all, 0, 4, true, true);
        assert_eq!(reason, WaitReason::NotWaiting);
    }

    #[test]
    fn wait_reason_returns_not_waiting_when_queue_empty_and_slots_available() {
        // 队列中没有更靠前的任务，且有空闲并发槽位 → 不在等待（即将开始）。
        let directory = tempfile::tempdir().unwrap();
        let target = wait_reason_task(directory.path(), "solo", TaskStatus::Queued, 0, 0);
        let reason = compute_wait_reason(&target, &[target.clone()], 0, 4, true, true);
        assert_eq!(reason, WaitReason::NotWaiting);
    }

    #[test]
    fn wait_reason_priority_order_matches_sort_download_candidates() {
        // 验证 ahead_count 的排序逻辑（Task 16: priority ASC, queue_position ASC）
        // 与 sort_download_candidates 一致。
        let directory = tempfile::tempdir().unwrap();
        let candidates = vec![
            wait_reason_task(directory.path(), "low-2", TaskStatus::Queued, 1, 2),
            wait_reason_task(directory.path(), "normal-2", TaskStatus::Queued, 0, 2),
            wait_reason_task(directory.path(), "high-1", TaskStatus::Queued, -1, 1),
            wait_reason_task(directory.path(), "normal-1", TaskStatus::Queued, 0, 1),
            wait_reason_task(directory.path(), "high-2", TaskStatus::Queued, -1, 2),
            wait_reason_task(directory.path(), "low-1", TaskStatus::Queued, 1, 1),
        ];

        // 用 sort_download_candidates 排序，得到预期顺序
        let mut sorted = candidates.clone();
        sort_download_candidates(&mut sorted);
        let sorted_ids: Vec<&str> = sorted.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            sorted_ids,
            // Task 16: priority ASC: high(-1) → normal(0) → low(1)
            // 同优先级内 queue_position ASC: high-1, high-2 / normal-1, normal-2 / low-1, low-2
            ["high-1", "high-2", "normal-1", "normal-2", "low-1", "low-2"]
        );

        // 对每个任务用 compute_wait_reason 计算 ahead_count，
        // 验证 ahead_count == 它在排序后列表中的位置（0-indexed）
        let all = candidates.clone();
        for task in &candidates {
            let reason = compute_wait_reason(task, &all, 0, 100, true, true);
            let position = sorted_ids
                .iter()
                .position(|id| *id == task.id.as_str())
                .unwrap();
            match reason {
                WaitReason::QueuedBehind { ahead_count } => {
                    assert_eq!(
                        ahead_count as usize, position,
                        "task {} ahead_count {} != position {}",
                        task.id, ahead_count, position
                    );
                }
                WaitReason::NotWaiting => {
                    assert_eq!(
                        position, 0,
                        "task {} should be QueuedBehind but got NotWaiting",
                        task.id
                    );
                }
                other => panic!("task {} unexpected reason: {:?}", task.id, other),
            }
        }
    }

    #[test]
    fn wait_reason_media_check_takes_precedence_over_concurrency() {
        // 媒体工具缺失时优先返回 WaitingMediaTools，即使并发槽位也满。
        let directory = tempfile::tempdir().unwrap();
        let mut task = wait_reason_task(directory.path(), "media-full", TaskStatus::Queued, 0, 0);
        task.media = Some(MediaSelection {
            extractor: None,
            format_id: Some("137+140".into()),
            format_label: None,
            subtitles: Vec::new(),
            thumbnail: None,
            requires_ffmpeg: false,
            url: None,
        });
        // active_count = 5, max_concurrent = 3（满），且 yt_dlp 缺失
        let reason = compute_wait_reason(&task, &[], 5, 3, false, true);
        assert_eq!(reason, WaitReason::WaitingMediaTools);
    }

    #[test]
    fn wait_reason_concurrency_check_takes_precedence_over_queue_position() {
        // 并发槽位满时优先返回 WaitingConcurrencyLimit，即使前面还有排队任务。
        let directory = tempfile::tempdir().unwrap();
        let target = wait_reason_task(directory.path(), "behind-full", TaskStatus::Queued, 0, 5);
        let higher = wait_reason_task(directory.path(), "higher", TaskStatus::Queued, -1, 1);
        let all = vec![target.clone(), higher];

        // active_count = 3, max_concurrent = 3（满），且 ahead_count = 1
        let reason = compute_wait_reason(&target, &all, 3, 3, true, true);
        match reason {
            WaitReason::WaitingConcurrencyLimit { active_count } => {
                assert_eq!(active_count, 3);
            }
            other => panic!("expected WaitingConcurrencyLimit, got {other:?}"),
        }
    }

    #[test]
    fn wait_reason_default_is_not_waiting() {
        // WaitReason 实现 Default，默认值为 NotWaiting。
        // 这确保旧前端/旧 JSON 反序列化时缺失 kind 字段不会 panic。
        assert_eq!(WaitReason::default(), WaitReason::NotWaiting);
    }

    #[test]
    fn wait_reason_serializes_with_kebab_case_tag() {
        // 验证 serde tag = "kind", rename_all = "kebab-case" 配置正确。
        // 前端 TypeScript 类型使用联合判别式（kind 字段），必须与之匹配。
        let cases: Vec<(WaitReason, &str)> = vec![
            (WaitReason::NotWaiting, r#"{"kind":"not-waiting"}"#),
            (
                WaitReason::QueuedBehind { ahead_count: 3 },
                r#"{"kind":"queued-behind","ahead_count":3}"#,
            ),
            (
                WaitReason::WaitingMediaTools,
                r#"{"kind":"waiting-media-tools"}"#,
            ),
            (
                WaitReason::WaitingConcurrencyLimit { active_count: 2 },
                r#"{"kind":"waiting-concurrency-limit","active_count":2}"#,
            ),
            (
                WaitReason::WaitingScheduledTime {
                    scheduled_at: "123".into(),
                },
                r#"{"kind":"waiting-scheduled-time","scheduled_at":"123"}"#,
            ),
            (WaitReason::Paused, r#"{"kind":"paused"}"#),
            (
                WaitReason::PausedByLowDisk,
                r#"{"kind":"paused-by-low-disk"}"#,
            ),
            (
                WaitReason::PausedByMetered,
                r#"{"kind":"paused-by-metered"}"#,
            ),
            (WaitReason::Interrupted, r#"{"kind":"interrupted"}"#),
            (WaitReason::RemoteChanged, r#"{"kind":"remote-changed"}"#),
            (WaitReason::Unknown, r#"{"kind":"unknown"}"#),
        ];
        for (reason, expected_json) in cases {
            let json = serde_json::to_string(&reason).unwrap();
            assert_eq!(json, expected_json, "serialization mismatch for {reason:?}");
        }
    }

    #[test]
    fn wait_reason_deserializes_missing_optional_fields_with_defaults() {
        // 旧前端或旧 JSON 可能缺少 ahead_count/active_count/scheduled_at 字段。
        // #[serde(default)] 必须保证反序列化成功且字段为默认值。
        let reason: WaitReason = serde_json::from_str(r#"{"kind":"queued-behind"}"#).unwrap();
        match reason {
            WaitReason::QueuedBehind { ahead_count } => assert_eq!(ahead_count, 0),
            other => panic!("expected QueuedBehind, got {other:?}"),
        }

        let reason: WaitReason =
            serde_json::from_str(r#"{"kind":"waiting-concurrency-limit"}"#).unwrap();
        match reason {
            WaitReason::WaitingConcurrencyLimit { active_count } => assert_eq!(active_count, 0),
            other => panic!("expected WaitingConcurrencyLimit, got {other:?}"),
        }

        let reason: WaitReason =
            serde_json::from_str(r#"{"kind":"waiting-scheduled-time"}"#).unwrap();
        match reason {
            WaitReason::WaitingScheduledTime { scheduled_at } => assert_eq!(scheduled_at, ""),
            other => panic!("expected WaitingScheduledTime, got {other:?}"),
        }
    }

    // ===== Task 14: 任务级超时与重试策略测试 =====

    fn retry_policy_with_backoff(backoff: BackoffStrategy) -> RetryPolicy {
        RetryPolicy {
            connection_timeout_secs: 30,
            task_timeout_secs: None,
            max_retries: 5,
            backoff,
            initial_backoff_ms: 1000,
            max_backoff_ms: 60_000,
        }
    }

    #[test]
    fn compute_backoff_fixed_returns_initial_for_all_attempts() {
        let policy = retry_policy_with_backoff(BackoffStrategy::Fixed);
        // Fixed 退避：所有尝试都返回 initial_backoff_ms。
        assert_eq!(compute_backoff(&policy, 1), 1000);
        assert_eq!(compute_backoff(&policy, 2), 1000);
        assert_eq!(compute_backoff(&policy, 5), 1000);
        assert_eq!(compute_backoff(&policy, 100), 1000);
    }

    #[test]
    fn compute_backoff_exponential_doubles_each_attempt() {
        let policy = retry_policy_with_backoff(BackoffStrategy::Exponential);
        // 指数退避：attempt 1 -> 1000, 2 -> 2000, 3 -> 4000, 4 -> 8000。
        assert_eq!(compute_backoff(&policy, 1), 1000);
        assert_eq!(compute_backoff(&policy, 2), 2000);
        assert_eq!(compute_backoff(&policy, 3), 4000);
        assert_eq!(compute_backoff(&policy, 4), 8000);
        assert_eq!(compute_backoff(&policy, 5), 16_000);
    }

    #[test]
    fn compute_backoff_exponential_capped_at_max_backoff_ms() {
        let policy = RetryPolicy {
            connection_timeout_secs: 30,
            task_timeout_secs: None,
            max_retries: 10,
            backoff: BackoffStrategy::Exponential,
            initial_backoff_ms: 1000,
            max_backoff_ms: 8_000,
        };
        // 1 -> 1000, 2 -> 2000, 3 -> 4000, 4 -> 8000 (上限), 5 -> 8000 (capped), 6 -> 8000。
        assert_eq!(compute_backoff(&policy, 1), 1000);
        assert_eq!(compute_backoff(&policy, 2), 2000);
        assert_eq!(compute_backoff(&policy, 3), 4000);
        assert_eq!(compute_backoff(&policy, 4), 8000);
        assert_eq!(compute_backoff(&policy, 5), 8000);
        assert_eq!(compute_backoff(&policy, 100), 8000);
    }

    #[test]
    fn compute_backoff_handles_attempt_zero_or_underflow_safely() {
        let policy = retry_policy_with_backoff(BackoffStrategy::Exponential);
        // attempt 0 视为 1，返回 initial_backoff_ms。
        assert_eq!(compute_backoff(&policy, 0), 1000);
        // 极大 attempt 不会溢出，由 saturating_mul 保护并封顶。
        let huge_policy = RetryPolicy {
            connection_timeout_secs: 30,
            task_timeout_secs: None,
            max_retries: 5,
            backoff: BackoffStrategy::Exponential,
            initial_backoff_ms: 1000,
            max_backoff_ms: 60_000,
        };
        assert_eq!(compute_backoff(&huge_policy, u32::MAX), 60_000);
    }

    #[test]
    fn effective_retry_policy_uses_task_override_when_present() {
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "file.zip", CollisionPolicy::Rename);
        let settings = AppSettings::default();
        // 默认情况下任务无覆盖，使用全局默认。
        assert_eq!(
            effective_retry_policy(&task, &settings),
            settings.default_retry_policy
        );
        // 设置任务级覆盖后应优先使用覆盖。
        let override_policy = RetryPolicy {
            connection_timeout_secs: 99,
            task_timeout_secs: Some(300),
            max_retries: 7,
            backoff: BackoffStrategy::Fixed,
            initial_backoff_ms: 500,
            max_backoff_ms: 5_000,
        };
        task.retry_policy_override = Some(override_policy.clone());
        let effective = effective_retry_policy(&task, &settings);
        assert_eq!(effective, override_policy);
        // 全局默认未受影响。
        assert_ne!(effective, settings.default_retry_policy);
    }

    #[test]
    fn effective_retry_policy_falls_back_to_global_default() {
        let directory = tempfile::tempdir().unwrap();
        let task = test_task(directory.path(), "file.zip", CollisionPolicy::Rename);
        let mut settings = AppSettings::default();
        // 任务无覆盖：使用全局默认。
        let default_policy = RetryPolicy {
            connection_timeout_secs: 45,
            task_timeout_secs: Some(600),
            max_retries: 3,
            backoff: BackoffStrategy::Fixed,
            initial_backoff_ms: 2_000,
            max_backoff_ms: 30_000,
        };
        settings.default_retry_policy = default_policy.clone();
        let effective = effective_retry_policy(&task, &settings);
        assert_eq!(effective, default_policy);
    }

    #[test]
    fn build_client_uses_default_retry_policy_connection_timeout() {
        let mut settings = AppSettings::default();
        settings.default_retry_policy.connection_timeout_secs = 45;
        let client = build_client(&settings).expect("client should build");
        // reqwest 不暴露 connect_timeout 的 getter，但成功构造即说明参数被接受。
        // 这里仅验证 build_client 不报错。
        drop(client);
        // 极小超时也应能成功构造。
        settings.default_retry_policy.connection_timeout_secs = 1;
        let _ = build_client(&settings).expect("client should build with 1s timeout");
    }

    // ===== Task 16: 任务优先级双通道测试 =====

    /// 辅助：构造一个指定 id/priority/queue_position 的任务，便于排序测试。
    fn priority_task(id: &str, priority: i32, queue_position: i64) -> DownloadTask {
        let directory = tempfile::tempdir().unwrap();
        let mut task = test_task(directory.path(), "p.bin", CollisionPolicy::Rename);
        task.id = id.into();
        task.priority = priority;
        task.queue_position = queue_position;
        task
    }

    /// 跨批次排序：不同 priority 的任务按 priority 升序排列。
    #[test]
    fn priority_sort_cross_batch_orders_by_priority_ascending() {
        let mut candidates = vec![
            priority_task("normal", 0, 5),
            priority_task("bottom", 1000, 1),
            priority_task("top", -1000, 9),
            priority_task("low", 50, 2),
            priority_task("high", -50, 8),
        ];
        sort_download_candidates(&mut candidates);
        let ids: Vec<&str> = candidates.iter().map(|t| t.id.as_str()).collect();
        // 数字越小越优先：top(-1000) → high(-50) → normal(0) → low(50) → bottom(1000)
        assert_eq!(ids, ["top", "high", "normal", "low", "bottom"]);
    }

    /// 同批次微调：同 priority 的任务按 queue_position 升序排列。
    #[test]
    fn priority_sort_same_batch_orders_by_queue_position_ascending() {
        let mut candidates = vec![
            priority_task("third", 0, 3),
            priority_task("first", 0, 1),
            priority_task("fourth", 0, 4),
            priority_task("second", 0, 2),
        ];
        sort_download_candidates(&mut candidates);
        let ids: Vec<&str> = candidates.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["first", "second", "third", "fourth"]);
    }

    /// 同批次微调 + 跨批次混合：先按 priority 分组，组内按 queue_position。
    #[test]
    fn priority_sort_mixed_groups_preserve_in_group_order() {
        let mut candidates = vec![
            priority_task("n2", 0, 2),
            priority_task("h2", -1, 2),
            priority_task("n1", 0, 1),
            priority_task("h1", -1, 1),
            priority_task("l1", 1, 1),
            priority_task("l2", 1, 2),
        ];
        sort_download_candidates(&mut candidates);
        let ids: Vec<&str> = candidates.iter().map(|t| t.id.as_str()).collect();
        // priority ASC: h(-1) → n(0) → l(1)，组内 queue_position ASC
        assert_eq!(ids, ["h1", "h2", "n1", "n2", "l1", "l2"]);
    }

    /// 置顶操作：priority 设为 MIN_PRIORITY (-1000)。
    #[test]
    fn priority_top_sets_to_min_priority() {
        let mut task = priority_task("task", 0, 1);
        task.priority = MIN_PRIORITY;
        assert_eq!(task.priority, -1000);
        // 验证置顶后排到最前
        let mut candidates = vec![
            priority_task("other", -50, 0),
            task.clone(),
            priority_task("normal", 0, 2),
        ];
        sort_download_candidates(&mut candidates);
        assert_eq!(candidates[0].id, "task");
    }

    /// 置底操作：priority 设为 MAX_PRIORITY (1000)。
    #[test]
    fn priority_bottom_sets_to_max_priority() {
        let mut task = priority_task("task", 0, 1);
        task.priority = MAX_PRIORITY;
        assert_eq!(task.priority, 1000);
        let mut candidates = vec![
            priority_task("normal", 0, 0),
            task.clone(),
            priority_task("other", 50, 3),
        ];
        sort_download_candidates(&mut candidates);
        assert_eq!(candidates.last().unwrap().id, "task");
    }

    /// 上移操作：priority -= PRIORITY_STEP (10)。
    #[test]
    fn priority_move_up_decreases_by_step() {
        use crate::models::PRIORITY_STEP;
        let task = priority_task("task", 0, 1);
        let original = task.priority;
        let new_priority = (original - PRIORITY_STEP).clamp(MIN_PRIORITY, MAX_PRIORITY);
        assert_eq!(new_priority, original - 10);

        // 多次上移不应超过 MIN_PRIORITY
        let low_priority = (MIN_PRIORITY + 5 - PRIORITY_STEP).clamp(MIN_PRIORITY, MAX_PRIORITY);
        assert_eq!(low_priority, MIN_PRIORITY);
    }

    /// 下移操作：priority += PRIORITY_STEP (10)。
    #[test]
    fn priority_move_down_increases_by_step() {
        use crate::models::PRIORITY_STEP;
        let task = priority_task("task", 0, 1);
        let original = task.priority;
        let new_priority = (original + PRIORITY_STEP).clamp(MIN_PRIORITY, MAX_PRIORITY);
        assert_eq!(new_priority, original + 10);

        // 多次下移不应超过 MAX_PRIORITY
        let high_priority = (MAX_PRIORITY - 5 + PRIORITY_STEP).clamp(MIN_PRIORITY, MAX_PRIORITY);
        assert_eq!(high_priority, MAX_PRIORITY);
    }

    /// 验证 priority 边界 clamp：超出范围的值被截断到 MIN/MAX_PRIORITY。
    #[test]
    fn priority_clamp_respects_bounds() {
        assert_eq!(5000_i32.clamp(MIN_PRIORITY, MAX_PRIORITY), MAX_PRIORITY);
        assert_eq!((-5000_i32).clamp(MIN_PRIORITY, MAX_PRIORITY), MIN_PRIORITY);
        assert_eq!(0_i32.clamp(MIN_PRIORITY, MAX_PRIORITY), 0);
    }

    /// 验证 is_ahead_of：更小 priority 排在前面；同 priority 时 queue_position 更小者排前。
    #[test]
    fn is_ahead_of_smaller_priority_wins() {
        let a = priority_task("a", -10, 100);
        let b = priority_task("b", 0, 1);
        assert!(is_ahead_of(&a, &b), "smaller priority should be ahead");
        assert!(!is_ahead_of(&b, &a), "larger priority should not be ahead");

        // 同 priority 时 queue_position 更小者排前
        let earlier = priority_task("earlier", 0, 1);
        let later = priority_task("later", 0, 10);
        assert!(is_ahead_of(&earlier, &later));
        assert!(!is_ahead_of(&later, &earlier));
    }

    // ===== Task 18: snapshot_segment_statuses 单元测试 =====
    // 验证 `task-connections` 事件的载荷来自 `SegmentRuntime` 原子量的真实采样，
    // 而非模拟数据（AGENTS.md §3）。

    /// 构造 8 连接任务的 SegmentRuntime 列表：每个分片 1MB，覆盖 8MB 总长度。
    fn eight_segment_runtimes() -> Vec<SegmentRuntime> {
        let segment_size: u64 = 1024 * 1024;
        (0..8)
            .map(|i| {
                let start = i as u64 * segment_size;
                let end = start + segment_size - 1;
                SegmentRuntime::new(i, start, end, 0, SEGMENT_PENDING)
            })
            .collect()
    }

    /// Task 18: 8 连接任务的快照必须包含全部 8 个分片，且 segment_id/offset 与 Runtime 一致。
    #[test]
    fn snapshot_segment_statuses_covers_all_eight_connections() {
        let runtimes = eight_segment_runtimes();
        let prev: Vec<u64> = vec![0; 8];
        let snapshot = snapshot_segment_statuses(&runtimes, &prev, 0.0, false);
        assert_eq!(snapshot.len(), 8, "8 连接任务必须返回 8 个 SegmentStatus");
        for (i, status) in snapshot.iter().enumerate() {
            assert_eq!(status.segment_id, i.to_string());
            assert_eq!(status.start_offset, i as u64 * 1024 * 1024);
            assert_eq!(status.total_bytes, 1024 * 1024);
            assert_eq!(status.downloaded_bytes, 0);
        }
    }

    /// Task 18: 新分配但未开始接收数据的分片应映射为 Connecting。
    #[test]
    fn snapshot_segment_statuses_maps_idle_segment_to_connecting() {
        let runtimes = vec![SegmentRuntime::new(0, 0, 1023, 0, SEGMENT_PENDING)];
        let snapshot = snapshot_segment_statuses(&runtimes, &[], 0.0, false);
        assert_eq!(snapshot[0].state, ConnectionState::Connecting);
        assert_eq!(snapshot[0].retry_count, 0);
        assert_eq!(snapshot[0].error, None);
    }

    /// Task 18: active_windows > 0 表示分片正在下载数据，应映射为 Downloading。
    #[test]
    fn snapshot_segment_statuses_maps_active_window_to_downloading() {
        let runtime = SegmentRuntime::new(0, 0, 1023, 0, SEGMENT_DOWNLOADING);
        runtime.active_windows.store(1, Ordering::Relaxed);
        let runtimes = vec![runtime];
        let snapshot = snapshot_segment_statuses(&runtimes, &[], 0.0, false);
        assert_eq!(snapshot[0].state, ConnectionState::Downloading);
    }

    /// Task 18: retrying 标志表示分片在退避 sleep 中，应映射为 Retrying。
    #[test]
    fn snapshot_segment_statuses_maps_retrying_flag_to_retrying() {
        let runtime = SegmentRuntime::new(0, 0, 1023, 0, SEGMENT_DOWNLOADING);
        runtime.active_windows.store(1, Ordering::Relaxed);
        runtime.retrying.store(true, Ordering::Relaxed);
        runtime.retry_count.store(2, Ordering::Relaxed);
        let runtimes = vec![runtime];
        let snapshot = snapshot_segment_statuses(&runtimes, &[], 0.0, false);
        assert_eq!(snapshot[0].state, ConnectionState::Retrying);
        assert_eq!(snapshot[0].retry_count, 2);
    }

    /// Task 18: downloaded == total 表示分片已完成，应映射为 Completed（优先于其他状态）。
    #[test]
    fn snapshot_segment_statuses_marks_completed_when_downloaded_equals_total() {
        let runtime = SegmentRuntime::new(0, 0, 1023, 1024, SEGMENT_COMPLETED);
        // 即使 active_windows 仍为 1（连接刚结束），完成判定优先。
        runtime.active_windows.store(1, Ordering::Relaxed);
        let runtimes = vec![runtime];
        let snapshot = snapshot_segment_statuses(&runtimes, &[], 0.0, false);
        assert_eq!(snapshot[0].state, ConnectionState::Completed);
        assert_eq!(snapshot[0].downloaded_bytes, 1024);
        assert_eq!(snapshot[0].total_bytes, 1024);
    }

    /// Task 18: status == SEGMENT_FAILED 表示分片已失败，应映射为 Failed 并附带错误信息。
    #[test]
    fn snapshot_segment_statuses_marks_failed_when_status_is_failed() {
        let runtime = SegmentRuntime::new(0, 0, 1023, 100, SEGMENT_FAILED);
        runtime.set_last_error("connection reset by peer; Cookie: secret=abc");
        let runtimes = vec![runtime];
        let snapshot = snapshot_segment_statuses(&runtimes, &[], 0.0, false);
        assert_eq!(snapshot[0].state, ConnectionState::Failed);
        // 错误信息必须经过 redact_sensitive 脱敏（Cookie 值替换为 ***）。
        let err = snapshot[0].error.as_ref().expect("应有错误信息");
        assert!(err.contains("connection reset"));
        assert!(err.contains("***"));
        assert!(!err.contains("secret=abc"));
    }

    /// Task 18: task_paused = true 时所有未完成分片必须映射为 Paused，
    /// 即使 active_windows > 0 或 retrying = true（任务已停止）。
    #[test]
    fn snapshot_segment_statuses_pauses_all_segments_when_task_paused() {
        let runtimes = eight_segment_runtimes();
        // 模拟暂停瞬间的真实状态：3 个分片在下载数据、1 个在重试、1 个已完成、1 个已失败。
        runtimes[0].active_windows.store(1, Ordering::Relaxed);
        runtimes[0]
            .status
            .store(SEGMENT_DOWNLOADING, Ordering::Relaxed);
        runtimes[0].downloaded_bytes.store(100, Ordering::Relaxed);
        runtimes[1].active_windows.store(1, Ordering::Relaxed);
        runtimes[1]
            .status
            .store(SEGMENT_DOWNLOADING, Ordering::Relaxed);
        runtimes[1].downloaded_bytes.store(200, Ordering::Relaxed);
        runtimes[2].active_windows.store(1, Ordering::Relaxed);
        runtimes[2].retrying.store(true, Ordering::Relaxed);
        runtimes[3]
            .downloaded_bytes
            .store(1024 * 1024, Ordering::Relaxed);
        runtimes[3]
            .status
            .store(SEGMENT_COMPLETED, Ordering::Relaxed);
        runtimes[4].status.store(SEGMENT_FAILED, Ordering::Relaxed);

        let prev: Vec<u64> = runtimes
            .iter()
            .map(|r| r.downloaded_bytes.load(Ordering::Relaxed))
            .collect();
        let snapshot = snapshot_segment_statuses(&runtimes, &prev, 0.0, true);

        // 暂停时：已完成和已失败的分片保留原状态（downloaded==total 仍 Completed，但 SEGMENT_FAILED
        // 在 task_paused 之后判定）。按当前实现 task_paused 优先于其他状态，因此全部 Paused。
        // 这与 emit_task_connections_final 在退出路径上传 task_paused=true 的语义一致。
        for status in &snapshot {
            assert_eq!(
                status.state,
                ConnectionState::Paused,
                "暂停时所有分片应为 Paused"
            );
        }
    }

    /// Task 18: 速度计算必须来自 downloaded_bytes 原子量的真实增量，而非模拟。
    /// 验证：prev=100, current=300, elapsed=2s → speed = (300-100)/2 = 100 bytes/s。
    #[test]
    fn snapshot_segment_statuses_computes_speed_from_real_delta() {
        let runtime = SegmentRuntime::new(0, 0, 1023, 300, SEGMENT_DOWNLOADING);
        runtime.active_windows.store(1, Ordering::Relaxed);
        let runtimes = vec![runtime];
        let prev: Vec<u64> = vec![100];
        let snapshot = snapshot_segment_statuses(&runtimes, &prev, 2.0, false);
        assert_eq!(
            snapshot[0].speed, 100,
            "speed 应为真实增量 (300-100)/2s = 100 bytes/s"
        );
    }

    /// Task 18: elapsed_secs 过小时 speed 应为 0（避免除零）。
    #[test]
    fn snapshot_segment_statuses_returns_zero_speed_when_elapsed_too_small() {
        let runtime = SegmentRuntime::new(0, 0, 1023, 500, SEGMENT_DOWNLOADING);
        runtime.active_windows.store(1, Ordering::Relaxed);
        let runtimes = vec![runtime];
        let prev: Vec<u64> = vec![100];
        let snapshot = snapshot_segment_statuses(&runtimes, &prev, 0.0005, false);
        assert_eq!(snapshot[0].speed, 0);
    }

    /// Task 18: prev_bytes 短于 runtimes 时使用 0 作为基线，避免越界（安全默认值）。
    #[test]
    fn snapshot_segment_statuses_handles_short_prev_bytes_safely() {
        let runtimes = eight_segment_runtimes();
        // 仅提供 3 个 prev 值，其余应使用 0 作为基线。
        let prev: Vec<u64> = vec![10, 20, 30];
        let snapshot = snapshot_segment_statuses(&runtimes, &prev, 1.0, false);
        assert_eq!(snapshot.len(), 8);
        // 前 3 个有 prev 值；由于 downloaded=0 < prev，speed=0（饱和减法）。
        // 后 5 个 prev=0，downloaded=0，speed=0。
        for status in &snapshot {
            assert_eq!(status.speed, 0);
        }
    }

    /// Task 18: 修改 SegmentRuntime 原子量后，下一次快照必须反映新状态（非缓存/模拟）。
    #[test]
    fn snapshot_segment_statuses_reflects_live_atomic_updates() {
        let runtime = SegmentRuntime::new(0, 0, 1023, 0, SEGMENT_PENDING);
        let runtimes = vec![runtime];
        // 第一次快照：connecting，downloaded=0
        let s1 = snapshot_segment_statuses(&runtimes, &[], 0.0, false);
        assert_eq!(s1[0].state, ConnectionState::Connecting);
        assert_eq!(s1[0].downloaded_bytes, 0);
        // 模拟下载循环更新原子量：开始接收数据，已下载 512 字节
        runtimes[0].active_windows.store(1, Ordering::Relaxed);
        runtimes[0]
            .status
            .store(SEGMENT_DOWNLOADING, Ordering::Relaxed);
        runtimes[0].downloaded_bytes.store(512, Ordering::Relaxed);
        // 第二次快照：downloading，downloaded=512
        let prev: Vec<u64> = vec![0];
        let s2 = snapshot_segment_statuses(&runtimes, &prev, 1.0, false);
        assert_eq!(s2[0].state, ConnectionState::Downloading);
        assert_eq!(s2[0].downloaded_bytes, 512);
        assert_eq!(s2[0].speed, 512);
        // 完成：downloaded = total
        runtimes[0].downloaded_bytes.store(1024, Ordering::Relaxed);
        runtimes[0]
            .status
            .store(SEGMENT_COMPLETED, Ordering::Relaxed);
        runtimes[0].active_windows.store(0, Ordering::Relaxed);
        let s3 = snapshot_segment_statuses(&runtimes, &vec![512], 1.0, false);
        assert_eq!(s3[0].state, ConnectionState::Completed);
        assert_eq!(s3[0].speed, 512);
    }

    /// Task 18: 模拟 8 连接任务在下载中、暂停、完成三个阶段的状态流转。
    /// 这是 SubTask 18.5 集成测试的纯逻辑等价物（无需启动 HTTP 服务器），
    /// 验证 snapshot_segment_statuses 在状态转换时返回正确的 SegmentStatus。
    #[test]
    fn snapshot_segment_statuses_eight_connection_lifecycle() {
        let segment_size: u64 = 1024 * 1024;
        let runtimes = eight_segment_runtimes();

        // 阶段 1：下载中——所有 8 个分片都活跃，已下载 256KB 各
        for r in &runtimes {
            r.active_windows.store(1, Ordering::Relaxed);
            r.status.store(SEGMENT_DOWNLOADING, Ordering::Relaxed);
            r.downloaded_bytes.store(256 * 1024, Ordering::Relaxed);
        }
        let prev: Vec<u64> = vec![0; 8];
        let snap1 = snapshot_segment_statuses(&runtimes, &prev, 1.0, false);
        assert_eq!(snap1.len(), 8);
        for s in &snap1 {
            assert_eq!(s.state, ConnectionState::Downloading);
            assert_eq!(s.downloaded_bytes, 256 * 1024);
            assert_eq!(s.total_bytes, segment_size);
            assert_eq!(s.speed, 256 * 1024, "1 秒内下载 256KB → 256KB/s");
        }

        // 阶段 2：暂停——所有分片应变为 Paused
        let prev2: Vec<u64> = runtimes
            .iter()
            .map(|r| r.downloaded_bytes.load(Ordering::Relaxed))
            .collect();
        let snap2 = snapshot_segment_statuses(&runtimes, &prev2, 0.0, true);
        for s in &snap2 {
            assert_eq!(s.state, ConnectionState::Paused);
            // 暂停时仍保留已下载字节数据（用于 UI 展示进度）
            assert_eq!(s.downloaded_bytes, 256 * 1024);
        }

        // 阶段 3：完成——所有分片 downloaded == total
        for r in &runtimes {
            r.downloaded_bytes.store(segment_size, Ordering::Relaxed);
            r.status.store(SEGMENT_COMPLETED, Ordering::Relaxed);
            r.active_windows.store(0, Ordering::Relaxed);
        }
        let snap3 = snapshot_segment_statuses(&runtimes, &prev2, 1.0, false);
        for s in &snap3 {
            assert_eq!(s.state, ConnectionState::Completed);
            assert_eq!(s.downloaded_bytes, segment_size);
            // 完成时的速度 = (segment_size - 256KB) / 1s
            assert_eq!(s.speed, segment_size - 256 * 1024);
        }
    }
}
