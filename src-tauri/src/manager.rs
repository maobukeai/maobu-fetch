use crate::{
    models::{
        AppSettings, BatchTaskRequest, CollisionPolicy, CompletionAction, DownloadSegment,
        DownloadTask, NewTaskRequest, PowerAction, PowerActionPhase, PowerActionState,
        TaskProgressEvent, TaskStatus,
    },
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
        atomic::{AtomicI32, AtomicU64, AtomicU8, Ordering},
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

pub type SharedManager = Arc<DownloadManager>;

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

#[derive(Clone)]
struct TrayTaskSnapshot {
    status: TaskStatus,
    downloaded_bytes: u64,
    total_bytes: u64,
    speed: u64,
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
    global_limiter: Arc<RateLimiter>,
    tray_tasks: StdMutex<HashMap<String, TrayTaskSnapshot>>,
    last_tray_update: AtomicU64,
}

impl DownloadManager {
    pub async fn new(store: Arc<Store>, app: AppHandle) -> Result<SharedManager, String> {
        let settings = store.get_settings().await?;
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
            global_limiter: Arc::new(RateLimiter::new()),
            tray_tasks: StdMutex::new(HashMap::new()),
            last_tray_update: AtomicU64::new(0),
        });
        manager.recover_interrupted().await?;
        manager.reload_tray_snapshots().await?;
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
        *self.client.write().await = build_client(&settings)?;
        *self.settings.write().await = settings.clone();
        self.store.save_settings(&settings).await?;
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
        request: NewTaskRequest,
    ) -> Result<DownloadTask, String> {
        let parsed = Url::parse(request.url.trim())
            .map_err(|_| "请输入有效的 HTTP/HTTPS 链接".to_string())?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err("仅支持 HTTP/HTTPS 链接".into());
        }
        let settings = self.settings().await;
        let file_name = safe_name(request.file_name.as_deref().unwrap_or_else(|| {
            parsed
                .path_segments()
                .and_then(|mut s| s.next_back())
                .filter(|s| !s.is_empty())
                .unwrap_or("download")
        }));
        let scheduled = request.scheduled_at.filter(|value| *value > now());
        let source = request.source.unwrap_or_else(|| "desktop".into());
        let completion_action =
            if source == "desktop" || request.completion_action != CompletionAction::RunFile {
                request.completion_action
            } else {
                CompletionAction::None
            };
        let mut task = DownloadTask {
            id: Uuid::new_v4().to_string(),
            url: parsed.to_string(),
            file_name: file_name.clone(),
            destination: request.destination.unwrap_or(settings.download_dir.clone()),
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
            priority: request.priority.clamp(-10, 10),
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
                })
                .await?;
            tasks.push(task);
        }
        Ok(tasks)
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
            if delete_file {
                let path = PathBuf::from(&task.destination).join(&task.file_name);
                // 1. Delete final completed file
                let _ = fs::remove_file(&path).await;
                // 2. Delete temporary .lumaget file
                let temp_path = PathBuf::from(format!("{}.lumaget", path.to_string_lossy()));
                let _ = fs::remove_file(&temp_path).await;
                // 3. Clear all part segment files
                self.clear_parts(&task).await;
            }
        }
        self.store.remove_task(id).await?;
        if let Ok(mut tasks) = self.tray_tasks.lock() {
            tasks.remove(id);
        }
        self.update_tray_tooltip(true);
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
            task.priority = priority.clamp(-10, 10);
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
            loop {
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
                    Err(error) if attempt < task.max_retries => {
                        if let Ok(Some(current)) = manager.store.get_task(&id).await {
                            task = current;
                        }
                        attempt += 1;
                        task.retry_count = attempt;
                        task.active_connections = 0;
                        task.error = Some(format!("{}，将在稍后重试", error));
                        let _ = manager.store.upsert_task(&task).await;
                        manager.emit_task("updated", &task);
                        let wait = manager
                            .settings()
                            .await
                            .retry_base_seconds
                            .saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)));
                        tokio::select! { _ = token.cancelled() => break, _ = tokio::time::sleep(Duration::from_secs(wait.min(60))) => {} }
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
        if task.media.is_some() {
            self.reserve_output_path(&mut task).await?;
            self.emit_task("updated", &task);
            let settings = self.settings().await;
            return crate::media::download(&self.app, &settings, task, token).await;
        }
        let client = self.client.read().await.clone();
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
        if task.etag.is_some() && task.etag != etag {
            self.clear_parts(&task).await;
            task.downloaded_bytes = 0;
            task.segments.clear();
        }
        task.etag = etag;
        task.last_modified = last_modified;
        task.total_bytes = total;
        if task.file_name == "download" {
            if let Some(name) = disposition_name(&probe) {
                task.file_name = name;
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
        let temp = PathBuf::from(format!("{}.lumaget", output.to_string_lossy()));
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
        let connections = effective_connection_count(&settings, task.connection_count);
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
        fs::rename(&temp, &final_output)
            .await
            .map_err(|e| format!("无法保存完成文件：{e}"))?;
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
        while let Some(chunk) = stream.next().await {
            if token.is_cancelled() {
                file.flush().await.ok();
                return Err("任务已暂停".into());
            };
            let chunk = chunk.map_err(friendly_body_error)?;
            self.limit(&task.id, chunk.len() as u64, &task_limiter)
                .await;
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            task.downloaded_bytes += chunk.len() as u64;
            if let Some(segment) = task.segments.first_mut() {
                segment.downloaded_bytes = task.downloaded_bytes;
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
                }
            })
        };

        let runtime_settings = self.settings().await;
        let global_limit = runtime_settings.speed_limit_kbps * 1024;
        let write_buffer_size = if runtime_settings.low_memory_mode {
            64 * 1024
        } else {
            1024 * 1024
        };
        let segment_max_retries = task.max_retries;
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
            let global = self.global_limiter.clone();
            let write_buffer_size = write_buffer_size;
            let segment_max_retries = segment_max_retries;
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
                                global.acquire(chunk_len, global_limit).await;
                                limiter
                                    .acquire(
                                        chunk_len,
                                        runtime_options.speed_limit.load(Ordering::Relaxed),
                                    )
                                    .await;
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
                                let delay_ms = 250u64
                                    .saturating_mul(1u64 << retry_count.min(3))
                                    .saturating_add(index as u64 * 11);
                                retry_count += 1;
                                tokio::select! {
                                    _ = token.cancelled() => break Err("任务已暂停".into()),
                                    _ = tokio::time::sleep(Duration::from_millis(delay_ms)) => {}
                                }
                                let _ = error;
                            }
                            Err(error) => {
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

        task.downloaded_bytes = progress.load(Ordering::Relaxed);
        task.segments = snapshot_segments(&runtimes);
        task.active_connections = 0;
        runtime_options.apply(&mut task).await;
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
            return Err(error);
        }
        if token.is_cancelled() {
            return Err("任务已暂停".into());
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
        for &(index, start, end) in &ranges {
            let expected = end - start + 1;
            let legacy_part = PathBuf::from(format!("{}.part{index}", temp.to_string_lossy()));
            let prefix_bytes = fs::metadata(&legacy_part)
                .await
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            let mut merged_bytes = 0u64;
            if prefix_bytes > 0 {
                append_part(&mut output, &legacy_part, prefix_bytes, &mut buffer).await?;
                merged_bytes = prefix_bytes;
                let _ = fs::remove_file(&legacy_part).await;
            }
            if prefix_bytes < expected {
                let layout = window_layouts
                    .get(&index)
                    .cloned()
                    .unwrap_or_else(|| balanced_window_ranges(start + prefix_bytes, end, index));
                for (_, window_start, window_end) in layout {
                    let path = window_part_path(temp, index, window_start);
                    let window_bytes = window_end - window_start + 1;
                    append_part(&mut output, &path, window_bytes, &mut buffer).await?;
                    merged_bytes = merged_bytes.saturating_add(window_bytes);
                    let _ = fs::remove_file(path).await;
                }
            }
            if merged_bytes != expected {
                return Err(format!(
                    "分片 #{} 大小不完整（应为 {} 字节，实际 {} 字节）",
                    index + 1,
                    expected,
                    merged_bytes
                ));
            }
        }
        output.flush().await.map_err(|error| error.to_string())?;
        fs::rename(merge, temp)
            .await
            .map_err(|error| error.to_string())?;
        task.downloaded_bytes = total;
        task.segments = snapshot_segments(&runtimes);
        Ok(task)
    }

    async fn limit(&self, task_id: &str, bytes: u64, task_limiter: &RateLimiter) {
        let global = self.settings().await.speed_limit_kbps * 1024;
        let task_limit = self
            .task_runtime
            .read()
            .await
            .get(task_id)
            .map(|runtime| runtime.speed_limit.load(Ordering::Relaxed))
            .unwrap_or(0);
        self.global_limiter.acquire(bytes, global).await;
        task_limiter.acquire(bytes, task_limit).await
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
            let client = self.client.read().await.clone();
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
        let result = match task.completion_action {
            CompletionAction::None => return,
            CompletionAction::OpenFolder => open::that(&task.destination),
            CompletionAction::RunFile if task.source == "desktop" => {
                open::that(PathBuf::from(&task.destination).join(&task.file_name))
            }
            CompletionAction::RunFile => {
                task.error = Some("已阻止非桌面任务自动运行文件".into());
                let _ = self.store.upsert_task(&task).await;
                self.emit_task("updated", &task);
                return;
            }
        };
        if let Err(error) = result {
            task.error = Some(format!("下载已完成，但完成动作失败：{error}"));
            let _ = self.store.upsert_task(&task).await;
            self.emit_task("updated", &task);
        }
    }

    async fn notify_download_completed(&self, task: &DownloadTask) {
        let settings = self.settings().await;
        let Some((title, body)) = completion_notification(&settings, task) else {
            return;
        };
        if let Err(error) = self
            .app
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
        {
            let _ = self.app.emit(
                "notification-error",
                format!("下载已完成，但 Windows 通知发送失败：{error}"),
            );
        }
    }
    async fn clear_parts(&self, task: &DownloadTask) {
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
    fn emit_task(&self, event: &str, task: &DownloadTask) {
        if let Ok(mut tasks) = self.tray_tasks.lock() {
            tasks.insert(
                task.id.clone(),
                TrayTaskSnapshot {
                    status: task.status.clone(),
                    downloaded_bytes: task.downloaded_bytes,
                    total_bytes: task.total_bytes,
                    speed: task.speed,
                },
            );
        }
        self.update_tray_tooltip(false);
        let _ = self.app.emit(
            &format!("task-{event}"),
            TaskProgressEvent {
                task: task.clone(),
                event: event.into(),
            },
        );
    }

    async fn reload_tray_snapshots(&self) -> Result<(), String> {
        let snapshots = self
            .store
            .list_tasks()
            .await?
            .into_iter()
            .map(|task| {
                (
                    task.id,
                    TrayTaskSnapshot {
                        status: task.status,
                        downloaded_bytes: task.downloaded_bytes,
                        total_bytes: task.total_bytes,
                        speed: task.speed,
                    },
                )
            })
            .collect();
        if let Ok(mut tasks) = self.tray_tasks.lock() {
            *tasks = snapshots;
        }
        self.update_tray_tooltip(true);
        Ok(())
    }

    fn update_tray_tooltip(&self, force: bool) {
        let current = now_millis();
        let previous = self.last_tray_update.load(Ordering::Relaxed);
        if !force && current.saturating_sub(previous) < 1_000 {
            return;
        }
        self.last_tray_update.store(current, Ordering::Relaxed);
        let tooltip = self
            .tray_tasks
            .lock()
            .map(|tasks| tray_tooltip(tasks.values()))
            .unwrap_or_else(|_| "猫步下载器".into());
        if let Some(tray) = self.app.tray_by_id("main-tray") {
            let _ = tray.set_tooltip(Some(tooltip));
        }
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

struct RateLimiter {
    state: Mutex<(Instant, f64)>,
}
impl RateLimiter {
    fn new() -> Self {
        Self {
            state: Mutex::new((Instant::now(), 0.0)),
        }
    }
    async fn acquire(&self, bytes: u64, limit: u64) {
        if limit == 0 {
            return;
        }
        let mut state = self.state.lock().await;
        let elapsed = state.0.elapsed().as_secs_f64();
        state.1 = (state.1 - elapsed * limit as f64).max(0.0) + bytes as f64;
        state.0 = Instant::now();
        let wait = (state.1 / limit as f64 - 0.15).max(0.0);
        drop(state);
        if wait > 0.0 {
            tokio::time::sleep(Duration::from_secs_f64(wait.min(1.0))).await
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

struct SegmentRuntime {
    index: u8,
    start_byte: u64,
    end_byte: u64,
    downloaded_bytes: AtomicU64,
    status: AtomicU8,
    active_windows: AtomicU8,
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
        }
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

fn tray_tooltip<'a>(tasks: impl Iterator<Item = &'a TrayTaskSnapshot>) -> String {
    let relevant: Vec<_> = tasks
        .filter(|task| {
            matches!(
                task.status,
                TaskStatus::Queued
                    | TaskStatus::Downloading
                    | TaskStatus::Paused
                    | TaskStatus::Scheduled
                    | TaskStatus::Verifying
                    | TaskStatus::WaitingNetwork
            )
        })
        .collect();
    if relevant.is_empty() {
        return "猫步下载器 · 无活动任务".into();
    }
    let speed: u64 = relevant.iter().map(|task| task.speed).sum();
    let (downloaded, total) = relevant.iter().filter(|task| task.total_bytes > 0).fold(
        (0_u64, 0_u64),
        |(downloaded, total), task| {
            (
                downloaded.saturating_add(task.downloaded_bytes.min(task.total_bytes)),
                total.saturating_add(task.total_bytes),
            )
        },
    );
    let mut result = format!("猫步下载器 · {} 个任务", relevant.len());
    if speed > 0 {
        result.push_str(&format!(" · {}/s", compact_bytes(speed)));
    }
    if total > 0 {
        result.push_str(&format!(" · {}%", downloaded.saturating_mul(100) / total));
    }
    result
}

fn compact_bytes(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1_024 {
        format!("{:.1} KB", bytes as f64 / 1_024.0)
    } else {
        format!("{bytes} B")
    }
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
fn build_client(s: &AppSettings) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent(&s.user_agent)
        .connect_timeout(Duration::from_secs(20))
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
    if !settings.notifications || task.status != TaskStatus::Completed {
        return None;
    }
    Some((
        format!("下载完成：{}", truncate_text(task.file_name.clone(), 80)),
        format!("已保存到 {}", truncate_text(task.destination.clone(), 160)),
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
    candidates.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.queue_position.cmp(&right.queue_position))
    });
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
        }
    }
    #[test]
    fn sanitizes_windows_names() {
        assert_eq!(safe_name("a<b>c.zip"), "a_b_c.zip");
        assert_eq!(safe_name("..."), "download")
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
    #[test]
    fn tray_tooltip_aggregates_known_progress_without_runtime_polling() {
        let tasks = [
            TrayTaskSnapshot {
                status: TaskStatus::Downloading,
                downloaded_bytes: 25,
                total_bytes: 100,
                speed: 2_048,
            },
            TrayTaskSnapshot {
                status: TaskStatus::Paused,
                downloaded_bytes: 25,
                total_bytes: 100,
                speed: 0,
            },
        ];
        assert_eq!(
            tray_tooltip(tasks.iter()),
            "猫步下载器 · 2 个任务 · 2.0 KB/s · 25%"
        );
        assert_eq!(tray_tooltip(std::iter::empty()), "猫步下载器 · 无活动任务");
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
        let directory = tempfile::tempdir().unwrap();
        let mut normal_first = test_task(directory.path(), "normal-first", CollisionPolicy::Rename);
        normal_first.id = "normal-first".into();
        normal_first.queue_position = 1;
        let mut high = test_task(directory.path(), "high", CollisionPolicy::Rename);
        high.id = "high".into();
        high.priority = 1;
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
        runtime.priority.store(1, Ordering::Relaxed);
        *runtime.completion_action.blocking_write() = CompletionAction::OpenFolder;
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(runtime.apply(&mut task));
        assert_eq!(task.per_task_speed_limit, 512 * 1024);
        assert_eq!(task.priority, 1);
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
}
