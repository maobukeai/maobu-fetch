use super::*;
use super::{
    http_progress::StreamProgress,
    http_response::{if_range_validator, response_plan},
    http_transfer::receive_response,
};

impl DownloadManager {
    pub(super) async fn download_stream(
        &self,
        task: DownloadTask,
        client: &reqwest::Client,
        temp: &Path,
        token: CancellationToken,
        limiter: Arc<RateLimiter>,
    ) -> Result<DownloadTask, String> {
        self.download_stream_with_initial_response(task, client, temp, token, limiter, None)
            .await
    }

    pub(super) async fn download_stream_with_initial_response(
        &self,
        mut task: DownloadTask,
        client: &reqwest::Client,
        temp: &Path,
        token: CancellationToken,
        limiter: Arc<RateLimiter>,
        initial: Option<reqwest::Response>,
    ) -> Result<DownloadTask, String> {
        let existing = match fs::metadata(temp).await {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(format!("无法读取下载临时文件：{error}")),
        };
        if existing > 0 && if_range_validator(&task).is_none() {
            return Err(
                "REMOTE_CHANGED:旧数据缺少可核验的资源标识，已保留原文件，请确认后重新下载".into(),
            );
        }
        let requested_start = existing;
        let response = match initial.filter(|_| existing == 0) {
            Some(response) => response,
            None => {
                self.stream_request(&task, client, requested_start, &token)
                    .await?
            }
        };
        let plan = response_plan(&task, &response, requested_start)?;
        task.downloaded_bytes = plan.initial_bytes;
        task.total_bytes = plan.total_bytes;
        task.active_connections = 1;
        let buffer = if self.settings().await.low_memory_mode {
            64 * 1024
        } else {
            1024 * 1024
        };
        let mut progress = StreamProgress::new(self, task, limiter, token.clone()).await;
        let result = receive_response(response, &plan, temp, &token, buffer, &mut progress).await;
        if let Some((available, required)) = progress.disk_blocked {
            // receive_response 已完成 flush 后才宣告暂停。
            progress.task.status = TaskStatus::PausedByLowDisk;
            progress.task.speed = 0;
            progress.task.eta_seconds = None;
            progress.task.active_connections = 0;
            progress.task.error = Some(format!(
                "磁盘空间不足（可用 {available} 字节，需要 {required} 字节），已暂停"
            ));
            token.cancel();
            self.store.update_http_progress(&progress.task).await?;
            self.emit_task("updated", &progress.task);
            let _ = self.app.emit(
                "task-paused-by-low-disk",
                LowDiskPayload {
                    task_id: progress.task.id.clone(),
                    available_bytes: available,
                    required_bytes: required,
                },
            );
        }
        if let Err(error) = result {
            self.store.update_http_progress(&progress.task).await?;
            return Err(error);
        }
        progress.finish().await
    }

    async fn stream_request(
        &self,
        task: &DownloadTask,
        client: &reqwest::Client,
        start: u64,
        token: &CancellationToken,
    ) -> Result<reqwest::Response, String> {
        let is_post = task
            .method
            .as_deref()
            .is_some_and(|method| method.eq_ignore_ascii_case("POST"));
        let mut request = if is_post {
            client.post(&task.url)
        } else {
            client.get(&task.url)
        };
        request = request.header(ACCEPT_ENCODING, "identity");
        if is_post {
            if let Some(body) = &task.body {
                request = request.body(body.clone());
            }
        }
        if start > 0 {
            request = request.header(RANGE, format!("bytes={start}-"));
            if let Some(validator) = if_range_validator(task) {
                request = request.header(IF_RANGE, validator);
            }
        }
        super::http_redirect::send(client, request, &task.headers, &task.url, token).await
    }
}
