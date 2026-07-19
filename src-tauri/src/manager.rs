use crate::{
    models::{
        AppSettings, BatchTaskRequest, CollisionPolicy, DownloadSegment, DownloadTask,
        NewTaskRequest, TaskProgressEvent, TaskStatus,
    },
    store::Store,
};
use futures_util::StreamExt;
use reqwest::header::{
    ACCEPT_ENCODING, ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, ETAG,
    LAST_MODIFIED, RANGE,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicU8, Ordering},
        Arc,
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
            connection_count: request
                .connection_count
                .unwrap_or(settings.connections_per_download)
                .clamp(1, 32),
            active_connections: 0,
            segments: Vec::new(),
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
                    connection_count: request.connection_count,
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
                task.status = TaskStatus::Queued;
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
            tokio::select! {
                _ = self.dispatcher.notified() => {},
                _ = tokio::time::sleep(Duration::from_millis(500)) => {},
            }
        }
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
        candidates.sort_by_key(|task| (-task.priority, task.queue_position));
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
                        break;
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
            manager.dispatcher.notify_waiters();
        });
    }

    async fn download_once(
        &self,
        mut task: DownloadTask,
        token: CancellationToken,
    ) -> Result<DownloadTask, String> {
        if task.media.is_some() {
            let settings = self.settings().await;
            return crate::media::download(&self.app, &settings, task, token).await;
        }
        let client = self.client.read().await.clone();
        let mut head = client.head(&task.url).header(ACCEPT_ENCODING, "identity");
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
        let settings = self.settings().await;
        let connections = effective_connection_count(&settings, task.connection_count);
        // Accept-Ranges is only advisory and is frequently omitted by CDNs.
        // Verify multi-connection support with an actual one-byte range request.
        let supports_range = if connections > 1 {
            let mut request = client
                .get(&task.url)
                .header(ACCEPT_ENCODING, "identity")
                .header(RANGE, "bytes=0-0");
            for (name, value) in &task.headers {
                request = request.header(name, value);
            }
            match request.send().await {
                Ok(response) if response.status() == reqwest::StatusCode::PARTIAL_CONTENT => {
                    matches!(
                        parse_content_range(&response),
                        Some((0, 0, response_total)) if response_total == total
                    )
                }
                _ => false,
            }
        } else {
            supports_range
        };
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
            let chunk = chunk.map_err(|e| e.to_string())?;
            self.limit(chunk.len() as u64, task.per_task_speed_limit, &task_limiter)
                .await;
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            task.downloaded_bytes += chunk.len() as u64;
            if let Some(segment) = task.segments.first_mut() {
                segment.downloaded_bytes = task.downloaded_bytes;
            }
            if sample.should_emit(task.downloaded_bytes) {
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
        let mut runtimes = Vec::with_capacity(ranges.len());

        for &(index, start, end) in &ranges {
            let part = PathBuf::from(format!("{}.part{index}", temp.to_string_lossy()));
            let expected = end - start + 1;
            let mut existing = fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
            if existing > expected {
                fs::remove_file(&part)
                    .await
                    .map_err(|error| format!("无法清理异常分片 #{}：{error}", index + 1))?;
                existing = 0;
            }
            initial += existing;
            let status = if existing == expected {
                SEGMENT_COMPLETED
            } else {
                SEGMENT_PENDING
            };
            runtimes.push(SegmentRuntime::new(index, start, end, existing, status));
            if existing < expected {
                jobs.push((index, start + existing, end, part));
            }
        }

        let runtimes = Arc::new(runtimes);
        let progress = Arc::new(AtomicU64::new(initial));
        task.downloaded_bytes = initial;
        task.segments = snapshot_segments(&runtimes);
        task.active_connections = 0;
        self.store.upsert_task(&task).await?;
        self.emit_task("updated", &task);

        let reporter_stop = CancellationToken::new();
        let reporter = {
            let stop = reporter_stop.clone();
            let cancel = token.clone();
            let progress = progress.clone();
            let runtimes = runtimes.clone();
            let store = self.store.clone();
            let app = self.app.clone();
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
                    snapshot.active_connections = active_segment_count(&runtimes);
                    sample.apply(&mut snapshot);
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
        let task_limit = task.per_task_speed_limit;
        let task_headers = task.headers.clone();
        let task_url = task.url.clone();
        // Keep _outer handles for use after job_stream completes.
        let token_outer = token.clone();
        let progress_outer = progress.clone();
        let runtimes_outer = runtimes.clone();
        // These are the copies that will be moved into the closure.
        let token_for_stream = token_outer.clone();
        let progress_for_stream = progress_outer.clone();
        let runtimes_for_stream = runtimes_outer.clone();
        let job_stream = futures_util::stream::iter(jobs.into_iter().map(
            move |(index, request_start, end, part)| {
                let client = client.clone();
                let headers = task_headers.clone();
                let url = task_url.clone();
                let token = token_for_stream.clone();
                let progress = progress_for_stream.clone();
                let runtimes = runtimes_for_stream.clone();
                let limiter = task_limiter.clone();
                let global = self.global_limiter.clone();
                let write_buffer_size = write_buffer_size;
                let segment_max_retries = segment_max_retries;
                async move {
                    let runtime = runtimes
                        .iter()
                        .find(|segment| segment.index == index)
                        .ok_or_else(|| format!("找不到分片 #{}", index + 1))?;
                    runtime.status.store(SEGMENT_DOWNLOADING, Ordering::Relaxed);
                    let file = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&part)
                        .await
                        .map_err(|error| error.to_string())?;
                    let mut file = BufWriter::with_capacity(write_buffer_size, file);
                    let mut next_start = request_start;
                    let mut retry_count = 0u32;
                    let result = loop {
                        if token.is_cancelled() {
                            let _ = file.flush().await;
                            break Err("任务已暂停".into());
                        }
                        let current_start = next_start;
                        let transfer = async {
                            let mut request = client
                                .get(&url)
                                .header(ACCEPT_ENCODING, "identity")
                                .header(RANGE, format!("bytes={current_start}-{end}"));
                            for (name, value) in &headers {
                                request = request.header(name, value);
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
                                        && actual_end == end
                                        && actual_total == total => {}
                                _ => return Err("服务器返回了不匹配的 Content-Range".into()),
                            }
                            let mut stream = response.bytes_stream();
                            while let Some(chunk) = stream.next().await {
                                if token.is_cancelled() {
                                    return Err("任务已暂停".into());
                                }
                                let chunk = chunk.map_err(friendly_body_error)?;
                                let chunk_len = chunk.len() as u64;
                                if next_start.saturating_add(chunk_len) > end.saturating_add(1) {
                                    return Err(format!("分片 #{} 返回了过多数据", index + 1));
                                }
                                global.acquire(chunk_len, global_limit).await;
                                limiter.acquire(chunk_len, task_limit).await;
                                file.write_all(&chunk)
                                    .await
                                    .map_err(|error| error.to_string())?;
                                next_start += chunk_len;
                                runtime
                                    .downloaded_bytes
                                    .fetch_add(chunk_len, Ordering::Relaxed);
                                progress.fetch_add(chunk_len, Ordering::Relaxed);
                            }
                            if next_start != end.saturating_add(1) {
                                return Err(format!(
                                    "分片 #{} 提前结束，剩余 {} 字节",
                                    index + 1,
                                    end.saturating_add(1).saturating_sub(next_start)
                                ));
                            }
                            Ok::<(), String>(())
                        }
                        .await;

                        match transfer {
                            Ok(()) => {
                                file.flush().await.map_err(|error| error.to_string())?;
                                break Ok(());
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
                    };
                    runtime.status.store(
                        if result.is_ok() {
                            SEGMENT_COMPLETED
                        } else {
                            SEGMENT_FAILED
                        },
                        Ordering::Relaxed,
                    );
                    result
                }
            },
        ))
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
        if let Some(error) = worker_error {
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
            let part = PathBuf::from(format!("{}.part{index}", temp.to_string_lossy()));
            let expected = end - start + 1;
            let actual = fs::metadata(&part)
                .await
                .map_err(|error| error.to_string())?
                .len();
            if actual != expected {
                return Err(format!(
                    "分片 #{} 大小不完整（应为 {} 字节，实际 {} 字节）",
                    index + 1,
                    expected,
                    actual
                ));
            }
            let mut source = fs::File::open(&part)
                .await
                .map_err(|error| error.to_string())?;
            loop {
                let count = source
                    .read(&mut buffer)
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
            let _ = fs::remove_file(part).await;
        }
        output.flush().await.map_err(|error| error.to_string())?;
        fs::rename(merge, temp)
            .await
            .map_err(|error| error.to_string())?;
        task.downloaded_bytes = total;
        task.segments = snapshot_segments(&runtimes);
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
        for index in 0..128 {
            let _ = fs::remove_file(format!("{}.part{index}", temp.to_string_lossy())).await;
        }
        let _ = fs::remove_file(format!("{}.merge", temp.to_string_lossy())).await;
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

const SEGMENT_PENDING: u8 = 0;
const SEGMENT_DOWNLOADING: u8 = 1;
const SEGMENT_COMPLETED: u8 = 2;
const SEGMENT_FAILED: u8 = 3;

struct SegmentRuntime {
    index: u8,
    start_byte: u64,
    end_byte: u64,
    downloaded_bytes: AtomicU64,
    status: AtomicU8,
}

impl SegmentRuntime {
    fn new(index: u8, start: u64, end: u64, downloaded: u64, status: u8) -> Self {
        Self {
            index,
            start_byte: start,
            end_byte: end,
            downloaded_bytes: AtomicU64::new(downloaded),
            status: AtomicU8::new(status),
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
            status: match segment.status.load(Ordering::Relaxed) {
                SEGMENT_DOWNLOADING => "downloading",
                SEGMENT_COMPLETED => "completed",
                SEGMENT_FAILED => "failed",
                _ => "pending",
            }
            .into(),
        })
        .collect()
}

fn active_segment_count(runtimes: &[SegmentRuntime]) -> u8 {
    runtimes
        .iter()
        .filter(|segment| segment.status.load(Ordering::Relaxed) == SEGMENT_DOWNLOADING)
        .count()
        .min(32) as u8
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
fn validate_settings(s: &AppSettings) -> Result<(), String> {
    if s.concurrent_downloads == 0 || s.concurrent_downloads > 16 {
        return Err("同时下载任务必须为 1–16".into());
    }
    if ![1, 2, 4, 8, 16, 32].contains(&s.connections_per_download) {
        return Err("分段连接数无效".into());
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
        "连接超时".into()
    } else if error.is_connect() {
        "无法连接服务器".into()
    } else {
        error.to_string()
    }
}
fn friendly_body_error(error: reqwest::Error) -> String {
    if error.is_decode() {
        "响应流被代理或服务器提前中断".into()
    } else {
        friendly_reqwest(error)
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
