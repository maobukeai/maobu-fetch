use super::*;
use futures_util::FutureExt;
use tokio::sync::watch;

pub(super) fn remaining_budget(policy: &RetryPolicy, start: Instant) -> Option<Duration> {
    policy
        .task_timeout_secs
        .filter(|seconds| *seconds > 0)
        .map(|seconds| Duration::from_secs(seconds).saturating_sub(start.elapsed()))
}

/// 网络等待和退避也属于任务总时限，不能在没有活动传输时无限等待。
pub(super) async fn wait_with_budget<F: std::future::Future>(
    future: F,
    token: &CancellationToken,
    remaining: Option<Duration>,
) -> Result<F::Output, String> {
    let deadline = async {
        match remaining {
            Some(remaining) => tokio::time::sleep(remaining).await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        biased;
        _ = token.cancelled() => Err("任务已暂停".into()),
        _ = deadline => Err("TASK_TIMEOUT:任务总超时，已停止等待，分片已保留".into()),
        result = future => Ok(result),
    }
}

pub(super) async fn run_attempt<F>(
    future: F,
    attempt: &CancellationToken,
    timeout: Option<Duration>,
) -> Result<DownloadTask, String>
where
    F: std::future::Future<Output = Result<DownloadTask, String>>,
{
    let Some(timeout) = timeout else {
        return future.await;
    };
    tokio::pin!(future);
    tokio::select! {
        biased;
        result = &mut future => result,
        _ = tokio::time::sleep(timeout) => {
            attempt.cancel();
            // 必须等连接、写入器和分片 worker 退出；不能 drop 后假报已停止。
            let _ = future.await;
            Err("TASK_TIMEOUT:任务总超时，活动连接已停止，分片已保留".into())
        }
    }
}

/// 句柄一直保留到该代 worker 退出。停止动作等待落盘后才改变可见状态。
pub(super) struct WorkerControl {
    pub token: CancellationToken,
    pub finalize_live: Arc<AtomicBool>,
    pub observed_live: AtomicBool,
    pub children: tokio_util::task::TaskTracker,
    pub kind: TaskKind,
    pub started_at: Instant,
    completed: watch::Sender<bool>,
}

impl WorkerControl {
    pub fn new() -> Arc<Self> {
        let (completed, _) = watch::channel(false);
        Arc::new(Self {
            token: CancellationToken::new(),
            finalize_live: Arc::new(AtomicBool::new(false)),
            observed_live: AtomicBool::new(false),
            children: tokio_util::task::TaskTracker::new(),
            kind: TaskKind::Http,
            started_at: Instant::now(),
            completed,
        })
    }
    pub fn cancel(&self) {
        self.token.cancel();
    }
    pub(super) fn finish(&self) {
        self.completed.send_replace(true);
    }
    async fn wait(&self) -> Result<(), String> {
        let mut receiver = self.completed.subscribe();
        while !*receiver.borrow_and_update() {
            receiver
                .changed()
                .await
                .map_err(|_| "下载停止确认通道已关闭".to_string())?;
        }
        Ok(())
    }
}

impl DownloadManager {
    pub(super) fn schedule_seeding_sync(self: &SharedManager) {
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = crate::bt::sync_seeding_policy(&manager).await {
                let message = format!("BT 做种策略未能全部生效：{}", redact_sensitive(&error));
                tracing::error!(error = %message, "同步做种策略失败");
                let _ = manager.app.emit("notification-error", message);
            }
        });
    }
    pub(super) async fn child_tasks(&self, id: &str) -> tokio_util::task::TaskTracker {
        self.controls
            .lock()
            .await
            .get(id)
            .map(|control| control.children.clone())
            .unwrap_or_default()
    }
    pub(crate) async fn live_finalize_permission(&self, id: &str) -> Arc<AtomicBool> {
        self.controls
            .lock()
            .await
            .get(id)
            .map(|control| control.finalize_live.clone())
            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)))
    }
    pub(super) async fn fail_worker_task(
        &self,
        id: &str,
        error: &str,
        attempt: u32,
    ) -> Result<(), String> {
        let Some(mut task) = self.store.get_task(id).await? else {
            return Ok(());
        };
        task.status = TaskStatus::Failed;
        task.error = Some(redact_sensitive(error));
        task.retry_count = attempt;
        task.speed = 0;
        task.eta_seconds = None;
        task.active_connections = 0;
        for segment in &mut task.segments {
            if segment.status == "downloading" {
                segment.status = "failed".into();
            }
        }
        if let Some(task) = self.store.update_worker_state(&task).await? {
            self.emit_task("updated", &task);
            self.notify_download_failed(&task).await;
        }
        Ok(())
    }
    pub(super) async fn task_operation(&self, id: &str) -> Arc<Mutex<()>> {
        self.task_operations
            .lock()
            .await
            .entry(id.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub(super) async fn stop_worker(&self, id: &str) -> Result<(), String> {
        self.stop_worker_with_policy(id, false).await
    }

    pub(super) async fn stop_worker_with_policy(
        &self,
        id: &str,
        finalize_live: bool,
    ) -> Result<(), String> {
        let control = self.controls.lock().await.get(id).cloned();
        if let Some(control) = control {
            control
                .finalize_live
                .store(finalize_live, Ordering::Release);
            control.cancel();
            tokio::time::timeout(Duration::from_secs(30), control.wait())
                .await
                .map_err(|_| "下载仍在停止和保存分片，请稍后重试；任务记录已保留".to_string())??;
        }
        Ok(())
    }

    pub(super) fn spawn_guarded_worker(
        self: &SharedManager,
        task: DownloadTask,
        control: Arc<WorkerControl>,
    ) {
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            let id = task.id.clone();
            let result = std::panic::AssertUnwindSafe(Self::run_worker_loop(
                manager.clone(),
                task,
                control.token.clone(),
            ))
            .catch_unwind()
            .await;
            if result.is_err() {
                control.cancel();
            }
            control.children.close();
            control.children.wait().await;
            if result.is_err() {
                if let Ok(Some(mut task)) = manager.store.get_task(&id).await {
                    task.status = TaskStatus::Failed;
                    task.error = Some("下载工作线程异常结束，已保留分片，请重试或检查日志".into());
                    task.speed = 0;
                    task.active_connections = 0;
                    match manager.store.update_worker_state(&task).await {
                        Ok(Some(current)) => manager.emit_task("updated", &current),
                        Ok(None) => {}
                        Err(error) => {
                            tracing::error!(task_id=%id,error=%redact_sensitive(&error),"工作线程异常状态保存失败，恢复文件已保留");
                            let _ = manager.app.emit(
                                "notification-error",
                                "工作线程异常状态保存失败，请检查磁盘或数据库权限",
                            );
                        }
                    }
                }
            }
            let mut controls = manager.controls.lock().await;
            // 即使将来允许新一代接管，旧代也不能删除新代句柄。
            if controls
                .get(&id)
                .is_some_and(|active| Arc::ptr_eq(active, &control))
            {
                controls.remove(&id);
                manager.task_runtime.write().await.remove(&id);
            }
            drop(controls);
            if let Err(error) = manager.rebalance_bandwidth().await {
                tracing::error!(error = %redact_sensitive(&error), "重新分配全局限速失败");
            }
            control.finish();
            manager.dispatcher.notify_waiters();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stop_signal_is_not_completion_and_multiple_waiters_observe_finished_generation() {
        let worker = WorkerControl::new();
        worker.cancel();
        assert!(worker.token.is_cancelled());
        assert!(
            tokio::time::timeout(Duration::from_millis(10), worker.wait())
                .await
                .is_err()
        );
        let first = worker.clone();
        let second = worker.clone();
        let one = tokio::spawn(async move { first.wait().await });
        let two = tokio::spawn(async move { second.wait().await });
        worker.finish();
        one.await.unwrap().unwrap();
        two.await.unwrap().unwrap();
        worker.wait().await.unwrap();
        assert!(!Arc::ptr_eq(&worker, &WorkerControl::new()));
    }

    #[tokio::test]
    async fn task_deadline_cancels_and_awaits_transfer_cleanup() {
        let token = CancellationToken::new();
        let cleaned = Arc::new(AtomicBool::new(false));
        let finished = cleaned.clone();
        let transfer = async {
            token.cancelled().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
            finished.store(true, Ordering::Release);
            Err("连接已关闭".to_owned())
        };
        let result = run_attempt(transfer, &token, Some(Duration::from_millis(5))).await;
        assert!(result.unwrap_err().starts_with("TASK_TIMEOUT:"));
        assert!(cleaned.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn deadline_also_bounds_network_wait_and_retry_backoff() {
        let token = CancellationToken::new();
        let wait = std::future::pending::<()>();
        let result = wait_with_budget(wait, &token, Some(Duration::from_millis(5))).await;
        assert!(result.unwrap_err().starts_with("TASK_TIMEOUT:"));
        token.cancel();
        assert_eq!(
            wait_with_budget(std::future::pending::<()>(), &token, None)
                .await
                .unwrap_err(),
            "任务已暂停"
        );
    }
}

impl DownloadManager {
    pub(super) async fn save_worker_state(&self, task: &mut DownloadTask) -> bool {
        match self.store.update_worker_state(task).await {
            Ok(Some(current)) => {
                *task = current;
                true
            }
            Ok(None) => false,
            Err(error) => {
                tracing::error!(task_id=%task.id,error=%redact_sensitive(&error),"任务状态保存失败，已停止后续操作并保留数据");
                let _ = self.app.emit(
                    "notification-error",
                    "任务状态保存失败，已停止后续操作，请检查磁盘或数据库权限",
                );
                false
            }
        }
    }
}
