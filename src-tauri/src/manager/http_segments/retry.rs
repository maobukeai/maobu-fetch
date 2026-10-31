use super::super::work_stealing::WindowTransferHandle;
use super::*;

pub(super) struct RetryState {
    attempts: u32,
    empty_steps: u32,
    checkpoint_at: Instant,
    checkpoint_bytes: u64,
}
impl RetryState {
    pub fn new(existing: u64) -> Self {
        Self {
            attempts: 0,
            empty_steps: 0,
            checkpoint_at: Instant::now(),
            checkpoint_bytes: existing,
        }
    }
    /// Only validated 206 responses and an in-range 416 count as expired-link evidence.
    pub fn check_progress(
        &mut self,
        session: &Session,
        handle: &WindowTransferHandle,
        runtime: &SegmentRuntime,
        cursor: u64,
        step: &response::Step,
    ) -> Result<(), String> {
        if !step.responded || session.worker_cancel.is_cancelled() {
            return Ok(());
        }
        self.empty_steps = if step.bytes == 0 {
            self.empty_steps.saturating_add(1)
        } else {
            0
        };
        let downloaded = handle.current_downloaded();
        if downloaded.saturating_sub(self.checkpoint_bytes) >= STALL_RECOVERY_BYTES {
            self.checkpoint_at = Instant::now();
            self.checkpoint_bytes = downloaded;
        }
        let remaining = handle
            .current_end()
            .saturating_sub(cursor)
            .saturating_add(1);
        if self.empty_steps >= MAX_EMPTY_STEPS {
            runtime.set_last_error("连续空响应，直链疑似已失效");
            return Err(format!(
                "{CLOUD_LINK_DEAD_PREFIX}分片 #{} 连续 {} 次收到 0 字节响应，直链疑似已失效",
                runtime.index + 1,
                self.empty_steps
            ));
        }
        if self.checkpoint_at.elapsed() >= STALL_TIMEOUT && remaining > STALL_RECOVERY_BYTES {
            runtime.set_last_error("长时间无实质进展，直链疑似已失效");
            return Err(format!(
                "{CLOUD_LINK_DEAD_PREFIX}分片 #{} 超过 {} 秒下载不足 {} 字节，直链疑似已失效",
                runtime.index + 1,
                STALL_TIMEOUT.as_secs(),
                STALL_RECOVERY_BYTES
            ));
        }
        Ok(())
    }
    pub async fn after_step(
        &mut self,
        session: &Session,
        runtime: &SegmentRuntime,
        result: Result<(), String>,
    ) -> Result<(), String> {
        match result {
            Ok(()) => {
                self.attempts = 0;
                Ok(())
            }
            Err(error) if error == ADAPTIVE_YIELD => Ok(()),
            Err(error)
                if session.worker_cancel.is_cancelled()
                    || error.starts_with(REMOTE_CHANGED_PREFIX)
                    || error.starts_with("MEDIA_PROBE_ERROR:")
                    || error.starts_with(INVALID_RANGE_PREFIX) =>
            {
                Err(error)
            }
            Err(error) if self.attempts < session.config.retry.max_retries => {
                self.attempts += 1;
                runtime.retry_count.store(self.attempts, Ordering::Relaxed);
                runtime.set_last_error(&error);
                runtime.retrying.store(true, Ordering::Relaxed);
                let delay = compute_backoff(&session.config.retry, self.attempts)
                    .saturating_add(runtime.index as u64 * 11);
                let result = wait(session, Duration::from_millis(delay)).await;
                runtime.retrying.store(false, Ordering::Relaxed);
                result
            }
            Err(error) => {
                runtime.set_last_error(&error);
                Err(format!(
                    "分片 #{} 连续重试 {} 次后仍失败：{}",
                    runtime.index + 1,
                    self.attempts,
                    error
                ))
            }
        }
    }
}

pub(super) async fn wait(session: &Session, duration: Duration) -> Result<(), String> {
    tokio::select! {
        _ = session.worker_cancel.cancelled() => Err("任务已暂停".into()),
        _ = tokio::time::sleep(duration) => Ok(()),
    }
}
