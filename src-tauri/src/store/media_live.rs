//! Actual probe/playlist observations update only an active task's media marker.
use super::*;
use crate::models::{TaskKind, NATIVE_HLS_EXTRACTOR};

impl Store {
    pub async fn pause_recording_after_failure(
        &self,
        id: &str,
        error: &str,
    ) -> Result<Option<DownloadTask>, String> {
        let mut connection = self.connection.lock().await;
        let transaction = connection
            .transaction()
            .map_err(|e| format!("保存录制停止状态失败：{e}"))?;
        let current = transaction
            .query_row("SELECT * FROM tasks WHERE id=?1", [id], task_from_row)
            .optional()
            .map_err(|e| format!("读取录制停止状态失败：{e}"))?;
        let Some(mut current) = current else {
            return Ok(None);
        };
        if current.status != TaskStatus::Downloading || current.task_kind != TaskKind::Http {
            return Ok(None);
        }
        current.status = TaskStatus::Paused;
        current.speed = 0;
        current.active_connections = 0;
        current.eta_seconds = None;
        current.error = Some(crate::manager::redact_sensitive(error));
        for segment in &mut current.segments {
            if segment.status == "downloading" {
                segment.status = "paused".into();
            }
        }
        Self::upsert_with(&transaction, &current)?;
        transaction
            .commit()
            .map_err(|e| format!("提交录制停止状态失败：{e}"))?;
        Ok(Some(current))
    }
    pub async fn set_task_media_live(
        &self,
        id: &str,
        live: bool,
    ) -> Result<Option<DownloadTask>, String> {
        self.write_media_live(id, live, false).await
    }
    pub async fn set_native_hls_live(
        &self,
        id: &str,
        live: bool,
    ) -> Result<Option<DownloadTask>, String> {
        self.write_media_live(id, live, true).await
    }
    async fn write_media_live(
        &self,
        id: &str,
        live: bool,
        native_hls: bool,
    ) -> Result<Option<DownloadTask>, String> {
        let mut connection = self.connection.lock().await;
        let transaction = connection
            .transaction()
            .map_err(|e| format!("保存直播识别状态失败：{e}"))?;
        let current = transaction
            .query_row("SELECT * FROM tasks WHERE id=?1", [id], task_from_row)
            .optional()
            .map_err(|e| format!("读取直播任务失败：{e}"))?;
        let Some(mut current) = current else {
            return Ok(None);
        };
        if current.status != TaskStatus::Downloading || current.task_kind != TaskKind::Http {
            return Ok(None);
        }
        let media = current.media.get_or_insert_with(MediaSelection::default);
        media.is_live = live;
        if native_hls {
            if media.requires_ffmpeg {
                return Err("此任务需要独立音视频合并，不能改用原生单路 HLS".into());
            }
            media.extractor = Some(NATIVE_HLS_EXTRACTOR.into());
        }
        Self::upsert_with(&transaction, &current)?;
        transaction
            .commit()
            .map_err(|e| format!("提交直播识别状态失败：{e}"))?;
        Ok(Some(current))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn recording_failure_pauses_only_active_rows_without_overwriting_user_options() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().to_owned();
        let store = tokio::task::spawn_blocking(move || Store::open(directory))
            .await
            .unwrap()
            .unwrap();
        let mut task = crate::bt::tests::bt_task_for_test();
        task.task_kind = TaskKind::Http;
        task.status = TaskStatus::Downloading;
        task.priority = 7;
        task.downloaded_bytes = 123;
        task.active_connections = 1;
        task.speed = 456;
        store.upsert_task(&task).await.unwrap();
        let paused = store
            .pause_recording_after_failure(&task.id, "请恢复录制后再停止保存")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(paused.status, TaskStatus::Paused);
        assert_eq!(paused.active_connections, 0);
        assert_eq!(paused.speed, 0);
        assert_eq!(paused.priority, 7);
        assert_eq!(paused.downloaded_bytes, 123);
        assert!(paused.error.as_ref().unwrap().contains("恢复录制"));
        for status in [
            TaskStatus::Paused,
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ] {
            task.status = status;
            store.upsert_task(&task).await.unwrap();
            let before = serde_json::to_value(store.get_task(&task.id).await.unwrap()).unwrap();
            assert!(store
                .pause_recording_after_failure(&task.id, "晚到失败")
                .await
                .unwrap()
                .is_none());
            assert_eq!(
                before,
                serde_json::to_value(store.get_task(&task.id).await.unwrap()).unwrap()
            );
        }
        store.remove_task(&task.id).await.unwrap();
        assert!(store
            .pause_recording_after_failure(&task.id, "晚到失败")
            .await
            .unwrap()
            .is_none());
        assert!(store.get_task(&task.id).await.unwrap().is_none());
    }
    #[tokio::test]
    async fn live_marker_preserves_options_and_never_changes_stopped_or_deleted_records() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().to_owned();
        let store = tokio::task::spawn_blocking(move || Store::open(directory))
            .await
            .unwrap()
            .unwrap();
        let mut task = crate::bt::tests::bt_task_for_test();
        task.task_kind = TaskKind::Http;
        task.status = TaskStatus::Downloading;
        task.media = None;
        task.priority = 7;
        task.downloaded_bytes = 123;
        store.upsert_task(&task).await.unwrap();
        let marked = store
            .set_native_hls_live(&task.id, true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(marked.priority, 7);
        assert_eq!(marked.downloaded_bytes, 123);
        assert!(marked.media.as_ref().unwrap().is_live);
        assert_eq!(
            marked.media.as_ref().unwrap().extractor.as_deref(),
            Some(NATIVE_HLS_EXTRACTOR)
        );
        for status in [
            TaskStatus::Paused,
            TaskStatus::Completed,
            TaskStatus::Cancelled,
        ] {
            task.status = status;
            task.media = None;
            store.upsert_task(&task).await.unwrap();
            assert!(store
                .set_task_media_live(&task.id, true)
                .await
                .unwrap()
                .is_none());
            assert!(store
                .get_task(&task.id)
                .await
                .unwrap()
                .unwrap()
                .media
                .is_none());
        }
        store.remove_task(&task.id).await.unwrap();
        assert!(store
            .set_native_hls_live(&task.id, true)
            .await
            .unwrap()
            .is_none());
        assert!(store.get_task(&task.id).await.unwrap().is_none());
    }
    #[test]
    fn old_media_json_defaults_to_non_live() {
        let mut old = serde_json::to_value(MediaSelection::default()).unwrap();
        old.as_object_mut().unwrap().remove("is_live");
        assert!(
            !serde_json::from_value::<MediaSelection>(old)
                .unwrap()
                .is_live
        );
        let old = r#"{"title":"old","thumbnail":null,"extractor":null,"duration":null,"formats":[],"subtitles":[],"drm":false}"#;
        assert!(
            !serde_json::from_str::<crate::models::MediaProbeResult>(old)
                .unwrap()
                .is_live
        );
    }
}
