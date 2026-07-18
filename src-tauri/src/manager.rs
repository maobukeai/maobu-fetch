use crate::{
    models::{
        AppSettings, BatchTaskRequest, CollisionPolicy, DownloadTask, NewTaskRequest,
        TaskProgressEvent, TaskStatus,
    },
    store::Store,
};
use futures_util::StreamExt;
use reqwest::header::{
    ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, ETAG, LAST_MODIFIED, RANGE,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Mutex, Notify, RwLock},
};
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

pub type SharedManager = Arc<DownloadManager>;

pub struct DownloadManager {
    pub store: Arc<Store>,
    settings: RwLock<AppSettings>,
    client: RwLock<reqwest::Client>,
    controls: Mutex<HashMap<String, CancellationToken>>,
    dispatcher: Notify,
    app: AppHandle,
    global_limiter: Arc<RateLimiter>,
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
            dispatcher: Notify::new(),
            app,
            global_limiter: Arc::new(RateLimiter::new()),
        });
        manager.recover_interrupted().await?;
        let scheduler = manager.clone();
        tauri::async_runtime::spawn(async move { scheduler.scheduler_loop().await });
        Ok(manager)
    }

    pub async fn list(&self) -> Result<Vec<DownloadTask>, String> {
        self.store.list_tasks().await
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
        let task = DownloadTask {
            id: Uuid::new_v4().to_string(),
            url: parsed.to_string(),
            file_name: file_name.clone(),
            destination: request.destination.unwrap_or(settings.download_dir.clone()),
            total_bytes: 0,
            downloaded_bytes: 0,
            speed: 0,
            eta_seconds: None,
            status: if scheduled.is_some() {
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
            source: request.source.unwrap_or_else(|| "desktop".into()),
            etag: None,
            last_modified: None,
            headers: request.headers,
            media: request.media,
            per_task_speed_limit: request.per_task_speed_limit,
            collision_policy: request.collision_policy,
        };
        self.store.upsert_task(&task).await?;
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
                    per_task_speed_limit: 0,
                    collision_policy: request.collision_policy.clone(),
                    media: None,
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
            }
            "resume" | "retry" => {
                if matches!(task.status, TaskStatus::Completed) && action == "resume" {
                    return Ok(());
                }
                task.status = TaskStatus::Queued;
                task.error = None;
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
        if let Some(task) = self.store.get_task(id).await? {
            if delete_file {
                let path = PathBuf::from(&task.destination).join(&task.file_name);
                let _ = fs::remove_file(path).await;
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
            if matches!(task.status, TaskStatus::Downloading | TaskStatus::Verifying) {
                task.status = TaskStatus::Queued;
                task.speed = 0;
                task.eta_seconds = None;
                self.store.upsert_task(&task).await?;
            }
        }
        Ok(())
    }

    async fn scheduler_loop(self: SharedManager) {
        loop {
            let _ = self.dispatch_once().await;
            tokio::select! {
                _ = self.dispatcher.notified() => {},
                _ = tokio::time::sleep(Duration::from_millis(500)) => {},
            }
        }
    }

    async fn dispatch_once(self: &SharedManager) -> Result<(), String> {
        let settings = self.settings().await;
        let active = self.controls.lock().await.len();
        if active >= settings.concurrent_downloads as usize {
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
        candidates.sort_by_key(|task| (-task.priority, task.queue_position));
        for task in candidates
            .into_iter()
            .take(settings.concurrent_downloads as usize - active)
        {
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
                        let settings = manager.settings().await;
                        if settings.verify_after_download || finished.expected_checksum.is_some() {
                            let _ = manager.store.upsert_task(&finished).await;
                            let _ = manager.verify_checksum(&id).await;
                        } else {
                            let _ = manager.store.upsert_task(&finished).await;
                            manager.emit_task("updated", &finished);
                        }
                        break;
                    }
                    Err(error) if attempt < task.max_retries => {
                        attempt += 1;
                        task.retry_count = attempt;
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
                        task.status = TaskStatus::Failed;
                        task.error = Some(error);
                        task.speed = 0;
                        task.eta_seconds = None;
                        task.retry_count = attempt;
                        let _ = manager.store.upsert_task(&task).await;
                        manager.emit_task("updated", &task);
                        break;
                    }
                }
            }
            manager.controls.lock().await.remove(&id);
            manager.dispatcher.notify_waiters();
        });
    }

    async fn download_once(
        &self,
        mut task: DownloadTask,
        token: CancellationToken,
    ) -> Result<DownloadTask, String> {
        if task.media.is_some() {
            return crate::media::download(&self.app, task, token).await;
        }
        let settings = self.settings().await;
        let client = self.client.read().await.clone();
        let mut head = client.head(&task.url);
        for (name, value) in &task.headers {
            head = head.header(name, value);
        }
        let probe = head.send().await.map_err(friendly_reqwest)?;
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
        let output = resolve_output_path(&task).await?;
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
        let connections = settings.connections_per_download.clamp(1, 16);
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
            task = self
                .download_stream(task, &client, &temp, token.clone(), task_limiter)
                .await?;
        }
        if token.is_cancelled() {
            return Err("任务已暂停".into());
        }
        if output.exists() && task.collision_policy == CollisionPolicy::Overwrite {
            let _ = fs::remove_file(&output).await;
        }
        fs::rename(&temp, &output)
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
        let existing = fs::metadata(temp).await.map(|m| m.len()).unwrap_or(0);
        let mut request = client.get(&task.url);
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
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(temp)
            .await
            .map_err(|e| e.to_string())?;
        task.downloaded_bytes = if append { existing } else { 0 };
        let mut stream = response.bytes_stream();
        let mut sample = ProgressSample::new(task.downloaded_bytes);
        while let Some(chunk) = stream.next().await {
            if token.is_cancelled() {
                file.flush().await.ok();
                return Err("任务已暂停".into());
            };
            let chunk = chunk.map_err(|e| e.to_string())?;
            self.limit(chunk.len() as u64, task.per_task_speed_limit, &task_limiter)
                .await;
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            task.downloaded_bytes += chunk.len() as u64;
            if sample.should_emit(task.downloaded_bytes) {
                sample.apply(&mut task);
                self.store.upsert_task(&task).await?;
                self.emit_task("updated", &task);
            }
        }
        file.flush().await.map_err(|e| e.to_string())?;
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
        let size = total.div_ceil(connections as u64);
        let progress = Arc::new(Mutex::new(task.downloaded_bytes));
        let sample = Arc::new(Mutex::new(ProgressSample::new(task.downloaded_bytes)));
        let mut jobs = futures_util::stream::FuturesUnordered::new();
        let mut initial = 0;
        for index in 0..connections {
            let start = index as u64 * size;
            if start >= total {
                continue;
            };
            let end = ((index as u64 + 1) * size - 1).min(total - 1);
            let part = PathBuf::from(format!("{}.part{index}", temp.to_string_lossy()));
            let existing = fs::metadata(&part)
                .await
                .map(|m| m.len())
                .unwrap_or(0)
                .min(end - start + 1);
            initial += existing;
            let request_start = start + existing;
            if request_start > end {
                continue;
            };
            let client = client.clone();
            let headers = task.headers.clone();
            let url = task.url.clone();
            let token = token.clone();
            let progress = progress.clone();
            let sample = sample.clone();
            let limiter = task_limiter.clone();
            let global = self.global_limiter.clone();
            let global_limit = self.settings().await.speed_limit_kbps * 1024;
            let task_limit = task.per_task_speed_limit;
            let store = self.store.clone();
            let app = self.app.clone();
            let mut snapshot = task.clone();
            jobs.push(async move {
                let mut req = client
                    .get(url)
                    .header(RANGE, format!("bytes={request_start}-{end}"));
                for (name, value) in headers {
                    req = req.header(name, value)
                }
                let response = req.send().await.map_err(friendly_reqwest)?;
                if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                    return Err("服务器不再支持分段续传".into());
                }
                let content_range = response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if !content_range.starts_with(&format!("bytes {request_start}-")) {
                    return Err("服务器返回了无效的 Content-Range".into());
                }
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(part)
                    .await
                    .map_err(|e| e.to_string())?;
                let mut stream = response.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    if token.is_cancelled() {
                        return Err("任务已暂停".into());
                    }
                    let chunk = chunk.map_err(|e| e.to_string())?;
                    global.acquire(chunk.len() as u64, global_limit).await;
                    limiter.acquire(chunk.len() as u64, task_limit).await;
                    file.write_all(&chunk).await.map_err(|e| e.to_string())?;
                    let mut value = progress.lock().await;
                    *value += chunk.len() as u64;
                    let current = *value;
                    drop(value);
                    let mut s = sample.lock().await;
                    if s.should_emit(current) {
                        snapshot.downloaded_bytes = current;
                        s.apply(&mut snapshot);
                        store.upsert_task(&snapshot).await?;
                        let _ = app.emit(
                            "task-updated",
                            TaskProgressEvent {
                                task: snapshot.clone(),
                                event: "updated".into(),
                            },
                        );
                    }
                }
                file.flush().await.map_err(|e| e.to_string())?;
                Ok::<(), String>(())
            });
        }
        task.downloaded_bytes = initial;
        *progress.lock().await = initial;
        while let Some(result) = jobs.next().await {
            result?
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
            .map_err(|e| e.to_string())?;
        let mut buffer = vec![0; 1024 * 1024];
        for index in 0..connections {
            let part = PathBuf::from(format!("{}.part{index}", temp.to_string_lossy()));
            let mut source = fs::File::open(&part).await.map_err(|e| e.to_string())?;
            loop {
                let n = source.read(&mut buffer).await.map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                output
                    .write_all(&buffer[..n])
                    .await
                    .map_err(|e| e.to_string())?
            }
            let _ = fs::remove_file(part).await;
        }
        output.flush().await.map_err(|e| e.to_string())?;
        fs::rename(merge, temp).await.map_err(|e| e.to_string())?;
        task.downloaded_bytes = total;
        Ok(task)
    }

    async fn limit(&self, bytes: u64, task_limit: u64, task_limiter: &RateLimiter) {
        let global = self.settings().await.speed_limit_kbps * 1024;
        self.global_limiter.acquire(bytes, global).await;
        task_limiter.acquire(bytes, task_limit).await
    }
    async fn clear_parts(&self, task: &DownloadTask) {
        let output = PathBuf::from(&task.destination).join(&task.file_name);
        let temp = PathBuf::from(format!("{}.lumaget", output.to_string_lossy()));
        for index in 0..16 {
            let _ = fs::remove_file(format!("{}.part{index}", temp.to_string_lossy())).await;
        }
        let _ = fs::remove_file(&temp).await;
    }
    fn emit_task(&self, event: &str, task: &DownloadTask) {
        let _ = self.app.emit(
            &format!("task-{event}"),
            TaskProgressEvent {
                task: task.clone(),
                event: event.into(),
            },
        );
    }
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
}
impl ProgressSample {
    fn new(bytes: u64) -> Self {
        Self {
            at: Instant::now(),
            bytes,
        }
    }
    fn should_emit(&self, current: u64) -> bool {
        self.at.elapsed() >= Duration::from_millis(250) || current == self.bytes
    }
    fn apply(&mut self, task: &mut DownloadTask) {
        let elapsed = self.at.elapsed().as_secs_f64().max(0.001);
        task.speed = ((task.downloaded_bytes - self.bytes) as f64 / elapsed) as u64;
        task.eta_seconds = if task.speed > 0 && task.total_bytes > task.downloaded_bytes {
            Some((task.total_bytes - task.downloaded_bytes) / task.speed)
        } else {
            None
        };
        self.at = Instant::now();
        self.bytes = task.downloaded_bytes
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn validate_settings(s: &AppSettings) -> Result<(), String> {
    if s.concurrent_downloads == 0 || s.concurrent_downloads > 16 {
        return Err("同时下载任务必须为 1–16".into());
    }
    if ![1, 2, 4, 8, 16].contains(&s.connections_per_download) {
        return Err("分段连接数无效".into());
    }
    Ok(())
}
fn build_client(s: &AppSettings) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent(&s.user_agent)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(24 * 60 * 60));
    if s.proxy_mode == "manual" && !s.proxy_url.is_empty() {
        let mut proxy = reqwest::Proxy::all(&s.proxy_url).map_err(|e| e.to_string())?;
        if !s.proxy_username.is_empty() {
            proxy = proxy.basic_auth(&s.proxy_username, &s.proxy_password)
        }
        builder = builder.proxy(proxy)
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
        "连接超时".into()
    } else if error.is_connect() {
        "无法连接服务器".into()
    } else {
        error.to_string()
    }
}
async fn resolve_output_path(task: &DownloadTask) -> Result<PathBuf, String> {
    let base = PathBuf::from(&task.destination).join(&task.file_name);
    if !base.exists() {
        return Ok(base);
    }
    match task.collision_policy {
        CollisionPolicy::Overwrite => Ok(base),
        CollisionPolicy::Skip => Err("目标文件已存在".into()),
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
                if !candidate.exists() {
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
    fn validates_concurrency() {
        let mut settings = AppSettings::default();
        settings.concurrent_downloads = 0;
        assert!(validate_settings(&settings).is_err())
    }
}
