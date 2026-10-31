//! HTTP 进度只更新已有行的传输字段，保留并发修改的用户选项。
use super::*;
use crate::models::TaskKind;

impl Store {
    pub async fn update_http_progress(&self, snapshot: &DownloadTask) -> Result<Option<DownloadTask>, String> {
        self.write_http_snapshot(snapshot, false).await
    }
    pub async fn complete_http_task(&self, snapshot: &DownloadTask) -> Result<Option<DownloadTask>, String> {
        self.write_http_snapshot(snapshot, true).await
    }
    async fn write_http_snapshot(&self, snapshot: &DownloadTask, completed: bool) -> Result<Option<DownloadTask>, String> {
        let mut connection = self.connection.lock().await;
        let transaction = connection.transaction().map_err(|e| format!("保存下载进度失败：{e}"))?;
        let current = transaction.query_row("SELECT * FROM tasks WHERE id=?1", [&snapshot.id], task_from_row)
            .optional().map_err(|e| format!("读取下载进度失败：{e}"))?;
        let Some(mut current) = current else { return Ok(None); };
        if current.task_kind != TaskKind::Http { return Err("HTTP 进度目标不是普通下载任务".into()); }
        if current.status != TaskStatus::Downloading && !(completed && current.status == TaskStatus::Completed) {
            return Ok(None);
        }
        current.total_bytes = snapshot.total_bytes;
        current.downloaded_bytes = snapshot.downloaded_bytes;
        current.speed = snapshot.speed;
        current.eta_seconds = snapshot.eta_seconds;
        current.active_connections = snapshot.active_connections;
        current.segments.clone_from(&snapshot.segments);
        if snapshot.status == TaskStatus::PausedByLowDisk {
            current.status = TaskStatus::PausedByLowDisk;
            current.error.clone_from(&snapshot.error);
        }
        if completed {
            current.file_name.clone_from(&snapshot.file_name);
            current.category.clone_from(&snapshot.category);
            current.status = TaskStatus::Completed;
            current.completed_at = snapshot.completed_at;
            current.error.clone_from(&snapshot.error);
            current.speed = 0;
            current.active_connections = 0;
            crate::manager::clear_auth_headers(&mut current.headers);
        }
        Self::upsert_with(&transaction, &current)?;
        transaction.commit().map_err(|e| format!("提交下载进度失败：{e}"))?;
        Ok(Some(current))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn stale_transfer_preserves_options_and_cannot_resurrect_a_removed_record() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_owned();
        let store = tokio::task::spawn_blocking(move || Store::open(path)).await.unwrap().unwrap();
        let mut transfer = crate::bt::tests::bt_task_for_test();
        transfer.task_kind = TaskKind::Http;
        transfer.status = TaskStatus::Downloading;
        store.upsert_task(&transfer).await.unwrap();
        store.update_runtime_options(&transfer.id, Some(9), Some(64), Some(CompletionAction::OpenFolder)).await.unwrap();
        transfer.downloaded_bytes = 100;
        let progress = store.update_http_progress(&transfer).await.unwrap().unwrap();
        assert_eq!(progress.downloaded_bytes, 100);
        assert_eq!(progress.priority, 9);
        assert_eq!(progress.per_task_speed_limit, 64);
        assert_eq!(progress.completion_action, CompletionAction::OpenFolder);
        transfer.status = TaskStatus::Completed;
        let completed = store.complete_http_task(&transfer).await.unwrap().unwrap();
        assert_eq!(completed.priority, 9);
        store.remove_task(&transfer.id).await.unwrap();
        assert!(store.update_http_progress(&transfer).await.unwrap().is_none());
        assert!(store.complete_http_task(&transfer).await.unwrap().is_none());
    }
}
