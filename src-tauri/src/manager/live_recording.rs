use super::*;

pub(super) fn is_native_hls(task: &DownloadTask) -> bool {
    task.media.as_ref().is_some_and(|media| {
        !media.requires_ffmpeg
            && media.extractor.as_deref() == Some(crate::models::NATIVE_HLS_EXTRACTOR)
    })
}

impl DownloadManager {
    pub(crate) async fn observe_live_runtime(&self, id: &str, live: bool) {
        if let Some(control) = self.controls.lock().await.get(id) {
            control.observed_live.store(live, Ordering::Release);
        }
    }
    pub(crate) async fn record_media_live(
        &self,
        id: &str,
        live: bool,
        native_hls: bool,
    ) -> Result<Option<DownloadTask>, String> {
        let updated = if native_hls {
            self.store.set_native_hls_live(id, live).await?
        } else {
            self.store.set_task_media_live(id, live).await?
        };
        if updated.is_some() {
            self.observe_live_runtime(id, live).await;
        }
        Ok(updated)
    }
    /// Called with the task operation lease held. Only this explicit action grants publication permission.
    pub(super) async fn finish_recording_locked(&self, id: &str) -> Result<(), String> {
        let task = self.store.get_task(id).await?.ok_or("任务不存在")?;
        let observed = self
            .controls
            .lock()
            .await
            .get(id)
            .is_some_and(|control| control.observed_live.load(Ordering::Acquire));
        if task.status != TaskStatus::Downloading
            || task.task_kind != TaskKind::Http
            || !task.media.as_ref().is_some_and(|media| media.is_live)
            || !observed
        {
            return Err("此任务尚未确认正在录制直播；请先开始或恢复录制，再选择停止并保存".into());
        }
        self.stop_worker_with_policy(id, true).await?;
        let finished = self
            .store
            .get_task(id)
            .await?
            .ok_or("任务已被移除，录制数据已保留")?;
        if finished.status == TaskStatus::Completed {
            return Ok(());
        }
        let message = finished.error.unwrap_or_else(|| {
            "录制尚未产生可保存的完整媒体片段，已暂停并保留恢复数据；请恢复录制后再停止保存".into()
        });
        if let Some(paused) = self
            .store
            .pause_recording_after_failure(id, &message)
            .await?
        {
            self.emit_task("updated", &paused);
        }
        Err(message)
    }
    pub(super) async fn accept_live_stop(
        &self,
        task: &mut DownloadTask,
        temp: &Path,
        token: &CancellationToken,
    ) -> Result<bool, String> {
        let live_http =
            task.url.contains("pull-flv-") || crate::media_platforms::is_douyin_live(&task.url);
        if !live_http
            || !token.is_cancelled()
            || !self
                .live_finalize_permission(&task.id)
                .await
                .load(Ordering::Acquire)
        {
            return Ok(false);
        }
        let metadata = fs::metadata(temp)
            .await
            .map_err(|e| format!("读取停止录制的临时文件失败，已保留恢复数据：{e}"))?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err("录制尚未产生可保存的数据，请重新开始录制".into());
        }
        task.downloaded_bytes = metadata.len();
        task.total_bytes = metadata.len();
        task.active_connections = 0;
        task.segments = vec![DownloadSegment {
            index: 0,
            start_byte: 0,
            end_byte: metadata.len() - 1,
            downloaded_bytes: metadata.len(),
            status: "completed".into(),
        }];
        Ok(true)
    }
}
