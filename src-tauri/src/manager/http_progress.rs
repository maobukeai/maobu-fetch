use super::*;
use super::http_transfer::StreamObserver;

pub(super) struct StreamProgress<'a> {
    manager: &'a DownloadManager,
    pub task: DownloadTask,
    limiter: Arc<RateLimiter>,
    token: CancellationToken,
    options: Arc<RuntimeTaskOptions>,
    sample: ProgressSample,
    disk_check_at: Instant,
    disk_bytes: u64,
    persisted_at: Instant,
    persisted_bytes: u64,
    pub disk_blocked: Option<(u64, u64)>,
}

impl<'a> StreamProgress<'a> {
    pub async fn new(manager: &'a DownloadManager, task: DownloadTask,
        limiter: Arc<RateLimiter>, token: CancellationToken) -> Self {
        let options = manager.runtime_task_options(&task).await;
        let initial = task.downloaded_bytes;
        Self { manager, task, limiter, token, options, sample: ProgressSample::new(initial),
            disk_check_at: Instant::now(), disk_bytes: DISK_CHECK_BYTES_INTERVAL,
            persisted_at: Instant::now(), persisted_bytes: initial, disk_blocked: None }
    }

    pub async fn finish(mut self) -> Result<DownloadTask, String> {
        self.task.active_connections = 0;
        if self.task.total_bytes == 0 {
            self.task.total_bytes = self.task.downloaded_bytes;
        }
        if let Some(segment) = self.task.segments.first_mut() {
            segment.end_byte = self.task.total_bytes.saturating_sub(1);
            segment.downloaded_bytes = self.task.downloaded_bytes;
            segment.status = "completed".into();
        }
        self.options.apply(&mut self.task).await;
        Ok(self.task)
    }
}

impl StreamObserver for StreamProgress<'_> {
    async fn before_write(&mut self, bytes: u64) -> Result<(), String> {
        if self.disk_bytes >= DISK_CHECK_BYTES_INTERVAL || self.disk_check_at.elapsed() >= DISK_CHECK_TIME_INTERVAL {
            if let Err((available, required)) = check_disk_space_once(
                &self.task.destination, self.task.total_bytes, self.task.downloaded_bytes,
            ) {
                self.disk_blocked = Some((available, required));
                return Err(format!("{LOW_DISK_PREFIX}磁盘空间不足（可用 {available} 字节，需要 {required} 字节）"));
            }
            self.disk_check_at = Instant::now();
            self.disk_bytes = 0;
        }
        self.manager.limit_with_cancel(&self.task.id, bytes, &self.limiter, &self.token).await;
        Ok(())
    }

    async fn progress(&mut self, total: u64) -> Result<(), String> {
        self.disk_bytes = self.disk_bytes.saturating_add(total.saturating_sub(self.task.downloaded_bytes));
        self.task.downloaded_bytes = total;
        if let Some(segment) = self.task.segments.first_mut() { segment.downloaded_bytes = total; }
        if self.sample.should_emit(total) {
            self.options.apply(&mut self.task).await;
            self.sample.apply(&mut self.task);
            if total != self.persisted_bytes && self.persisted_at.elapsed() >= Duration::from_secs(1) {
                self.task = self.manager.store.update_http_progress(&self.task).await?
                    .ok_or("任务已停止或被删除，已停止传输进度写入")?;
                self.persisted_at = Instant::now();
                self.persisted_bytes = total;
            }
            self.manager.emit_task("updated", &self.task);
        }
        Ok(())
    }
}
