//! 原生HLS/录制的容器校验与最终发布，封装转换不改变恢复文件的归属。
use super::*;

struct PublicationScope {
    cancel: CancellationToken,
    stop: CancellationToken,
    permission: Option<Arc<AtomicBool>>,
    deadline: Option<Instant>,
}
impl Drop for PublicationScope {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
impl PublicationScope {
    fn allowed(&self) -> bool {
        !self.cancel.is_cancelled()
            && self
                .permission
                .as_ref()
                .is_none_or(|permission| permission.load(Ordering::Acquire))
            && self
                .deadline
                .is_none_or(|deadline| Instant::now() < deadline)
    }
}

impl DownloadManager {
    async fn publication_scope(
        &self,
        task: &DownloadTask,
        parent: &CancellationToken,
        settings: &AppSettings,
    ) -> Result<PublicationScope, String> {
        if !parent.is_cancelled() {
            return Ok(PublicationScope {
                cancel: parent.clone(),
                stop: CancellationToken::new(),
                permission: None,
                deadline: None,
            });
        }
        let permission = self.live_finalize_permission(&task.id).await;
        if !permission.load(Ordering::Acquire) {
            return Err("任务已停止，恢复数据已保留".into());
        }
        let started = self
            .controls
            .lock()
            .await
            .get(&task.id)
            .map(|control| control.started_at)
            .unwrap_or_else(Instant::now);
        let deadline = effective_retry_policy(task, settings)
            .task_timeout_secs
            .filter(|seconds| *seconds > 0)
            .and_then(|seconds| started.checked_add(Duration::from_secs(seconds)));
        let scope = PublicationScope {
            cancel: CancellationToken::new(),
            stop: CancellationToken::new(),
            permission: Some(permission.clone()),
            deadline,
        };
        let stop = scope.stop.clone();
        let cancel = scope.cancel.clone();
        self.child_tasks(&task.id).await.spawn(async move {
            loop {
                if !permission.load(Ordering::Acquire) || deadline.is_some_and(|deadline|Instant::now()>=deadline) {
                    cancel.cancel(); break;
                }
                tokio::select! { _=stop.cancelled()=>break, _=tokio::time::sleep(Duration::from_millis(25))=>{} }
            }
        });
        Ok(scope)
    }

    pub(super) async fn publish_native_media(
        &self,
        mut task: DownloadTask,
        source: &Path,
        parent: &CancellationToken,
    ) -> Result<DownloadTask, String> {
        file_integrity::validate_completed_file(source, task.downloaded_bytes).await?;
        let settings = self.settings().await;
        let scope = self.publication_scope(&task, parent, &settings).await?;
        let prepared = crate::media::prepare_native_media_file(
            &self.app,
            &settings,
            &mut task,
            source,
            &scope.cancel,
        )
        .await?;
        if !scope.allowed() {
            return Err("录制保存已停止或超过任务总时限，源文件与分片已保留".into());
        }
        let length = fs::metadata(&prepared)
            .await
            .map_err(|e| format!("读取媒体输出失败：{e}"))?
            .len();
        file_integrity::validate_completed_file(&prepared, length).await?;
        task.total_bytes = length;
        task.downloaded_bytes = length;
        let output = self.reserve_output_path(&mut task).await?;
        if !scope.allowed() {
            return Err("任务已停止，媒体暂存文件已保留".into());
        }
        record_task_publication(&task, &prepared, &output, &scope.cancel).await?;
        publish_file_atomically(
            &prepared,
            &output,
            task.collision_policy == CollisionPolicy::Overwrite,
        )
        .await?;
        file_integrity::validate_completed_file(&output, length).await?;
        task.status = TaskStatus::Completed;
        task.completed_at = Some(now());
        task.speed = 0;
        task.eta_seconds = Some(0);
        task.active_connections = 0;
        Ok(task)
    }
}
