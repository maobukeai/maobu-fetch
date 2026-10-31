//! BT 增量写入只作用于已存在任务，不能用旧 worker 快照覆盖选择或复活删除记录。
use super::{task_from_row, Store};
use crate::models::{DownloadTask, TaskKind, TaskStatus};
use rusqlite::{OptionalExtension, TransactionBehavior};

impl Store {
    /// 完成提交与最后一帧合并在同一事务中；删除/暂停之后的旧 worker 没有提交权。
    pub async fn complete_bt_task(
        &self,
        snapshot: &DownloadTask,
    ) -> Result<Option<DownloadTask>, String> {
        let mut connection = self.connection.lock().await;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| format!("保存 BT 完成状态失败：{e}"))?;
        let current = transaction
            .query_row(
                "SELECT * FROM tasks WHERE id=?1",
                [&snapshot.id],
                task_from_row,
            )
            .optional()
            .map_err(|e| format!("读取 BT 任务失败：{e}"))?;
        let Some(mut current) = current else {
            return Ok(None);
        };
        if current.task_kind != TaskKind::Bt || current.status != TaskStatus::Downloading {
            return Ok(None);
        }
        merge_snapshot(&mut current, snapshot)?;
        current.status = TaskStatus::Completed;
        current.completed_at = snapshot.completed_at;
        current.speed = 0;
        current.eta_seconds = Some(0);
        current.active_connections = 0;
        current.error = None;
        if let (Some(meta), Some(incoming)) = (current.bt_meta.as_mut(), snapshot.bt_meta.as_ref())
        {
            meta.seeding_requested = incoming.seeding_requested;
        }
        Self::upsert_with(&transaction, &current)?;
        transaction
            .commit()
            .map_err(|e| format!("提交 BT 完成状态失败：{e}"))?;
        Ok(Some(current))
    }

    /// 只改用户刚选择的文件列表，不覆盖下载线程已提交的新进度。
    pub async fn update_bt_selection(
        &self,
        id: &str,
        indices: Vec<u32>,
    ) -> Result<DownloadTask, String> {
        self.edit_bt_task(id, |task| {
            task.bt_meta
                .as_mut()
                .ok_or("BT 任务缺少元数据")?
                .selected_files = indices;
            Ok(())
        })
        .await?
        .ok_or_else(|| "任务已被删除，未保存文件选择".into())
    }

    pub async fn update_bt_seed_request(
        &self,
        id: &str,
        requested: bool,
        error: Option<String>,
    ) -> Result<Option<DownloadTask>, String> {
        self.edit_bt_task(id, |task| {
            if task.status != TaskStatus::Completed {
                return Err("仅已完成的 BT 任务可以设置做种".into());
            }
            task.bt_meta
                .as_mut()
                .ok_or("BT 任务缺少元数据")?
                .seeding_requested = requested;
            task.error = error;
            Ok(())
        })
        .await
    }

    /// A late preparation error cannot restore an explicit stop, archive, or deleted record.
    pub async fn update_bt_seed_error(
        &self,
        id: &str,
        error: String,
    ) -> Result<Option<DownloadTask>, String> {
        let updated = self
            .edit_bt_task(id, |task| {
                if task.status == TaskStatus::Completed
                    && task
                        .bt_meta
                        .as_ref()
                        .is_some_and(|meta| meta.seeding_requested)
                {
                    task.error = Some(error);
                }
                Ok(())
            })
            .await?;
        Ok(updated.filter(|task| {
            task.status == TaskStatus::Completed
                && task
                    .bt_meta
                    .as_ref()
                    .is_some_and(|meta| meta.seeding_requested)
        }))
    }

    /// 恢复清单时同样保留并发更新的用户字段与进度。
    pub async fn update_bt_ownership(
        &self,
        snapshot: &DownloadTask,
    ) -> Result<Option<DownloadTask>, String> {
        self.edit_bt_task(&snapshot.id, |current| {
            let incoming = snapshot.bt_meta.as_ref().ok_or("BT 任务缺少元数据")?;
            let existing = current.bt_meta.as_mut().ok_or("BT 任务缺少元数据")?;
            if existing.output_folder.is_some() && existing.output_folder != incoming.output_folder
            {
                return Err("BT 输出归属冲突，已保留原记录".into());
            }
            if !existing.info_hash.is_empty()
                && !existing.info_hash.eq_ignore_ascii_case(&incoming.info_hash)
            {
                return Err("BT 种子身份冲突，已保留原记录".into());
            }
            existing.output_folder.clone_from(&incoming.output_folder);
            existing.output_owned = incoming.output_owned;
            existing.owned_files.clone_from(&incoming.owned_files);
            existing.info_hash.clone_from(&incoming.info_hash);
            Ok(())
        })
        .await
    }

    async fn edit_bt_task(
        &self,
        id: &str,
        edit: impl FnOnce(&mut DownloadTask) -> Result<(), String>,
    ) -> Result<Option<DownloadTask>, String> {
        let mut connection = self.connection.lock().await;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| format!("更新 BT 任务失败：{e}"))?;
        let current = transaction
            .query_row("SELECT * FROM tasks WHERE id=?1", [id], task_from_row)
            .optional()
            .map_err(|e| format!("读取 BT 任务失败：{e}"))?;
        let Some(mut current) = current else {
            return Ok(None);
        };
        if current.task_kind != TaskKind::Bt {
            return Err("目标不是 BT 任务".into());
        }
        edit(&mut current)?;
        Self::upsert_with(&transaction, &current)?;
        transaction
            .commit()
            .map_err(|e| format!("提交 BT 任务修改失败：{e}"))?;
        Ok(Some(current))
    }

    pub async fn update_bt_progress(
        &self,
        snapshot: &DownloadTask,
    ) -> Result<Option<DownloadTask>, String> {
        let mut connection = self.connection.lock().await;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| format!("保存 BT 进度失败：{e}"))?;
        let current = transaction
            .query_row(
                "SELECT * FROM tasks WHERE id=?1",
                [&snapshot.id],
                task_from_row,
            )
            .optional()
            .map_err(|e| format!("读取 BT 任务失败：{e}"))?;
        let Some(mut current) = current else {
            return Ok(None);
        };
        if current.task_kind != TaskKind::Bt {
            return Err("进度更新目标不是 BT 任务".into());
        }
        if !matches!(
            current.status,
            TaskStatus::Downloading | TaskStatus::Completed
        ) {
            return Ok(Some(current));
        }
        merge_snapshot(&mut current, snapshot)?;
        Self::upsert_with(&transaction, &current)?;
        transaction
            .commit()
            .map_err(|e| format!("提交 BT 进度失败：{e}"))?;
        Ok(Some(current))
    }
}

fn merge_snapshot(current: &mut DownloadTask, snapshot: &DownloadTask) -> Result<(), String> {
    if snapshot.status == TaskStatus::Failed && current.status == TaskStatus::Downloading {
        current.status = TaskStatus::Failed;
        current.error.clone_from(&snapshot.error);
    }
    current.total_bytes = snapshot.total_bytes;
    current.downloaded_bytes = snapshot.downloaded_bytes;
    current.speed = snapshot.speed;
    current.eta_seconds = snapshot.eta_seconds;
    current.active_connections = snapshot.active_connections;
    current.bt_runtime.clone_from(&snapshot.bt_runtime);
    if let (Some(existing), Some(incoming)) = (current.bt_meta.as_mut(), snapshot.bt_meta.as_ref())
    {
        if let (Some(old), Some(new)) = (&existing.output_folder, &incoming.output_folder) {
            if old != new {
                return Err("BT 输出归属发生冲突，未覆盖原任务记录".into());
            }
        }
        if !existing.info_hash.is_empty()
            && !incoming.info_hash.is_empty()
            && !existing.info_hash.eq_ignore_ascii_case(&incoming.info_hash)
        {
            return Err("BT 种子身份发生变化，未覆盖原任务记录".into());
        }
        let first_name = existing.display_name.is_none() && incoming.display_name.is_some();
        if !incoming.info_hash.is_empty() {
            existing.info_hash.clone_from(&incoming.info_hash);
        }
        existing.metadata_ready |= incoming.metadata_ready;
        existing.uploaded_bytes = existing.uploaded_bytes.max(incoming.uploaded_bytes);
        if incoming.display_name.is_some() {
            existing.display_name.clone_from(&incoming.display_name);
        }
        if existing.output_folder.is_none() {
            existing.output_folder.clone_from(&incoming.output_folder);
            existing.output_owned = incoming.output_owned;
        }
        if !incoming.owned_files.is_empty() {
            existing.owned_files.clone_from(&incoming.owned_files);
        }
        if first_name && current.status != TaskStatus::Completed {
            current.file_name.clone_from(&snapshot.file_name);
            current.category.clone_from(&snapshot.category);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bt::tests::bt_task_for_test;

    #[tokio::test]
    async fn stale_progress_preserves_selection_limits_and_completed_state() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().to_owned();
        let store = tokio::task::spawn_blocking(move || Store::open(path))
            .await
            .unwrap()
            .unwrap();
        let mut old = bt_task_for_test();
        old.bt_meta.as_mut().unwrap().selected_files = vec![1];
        store.upsert_task(&old).await.unwrap();
        let mut edited = old.clone();
        edited.bt_meta.as_mut().unwrap().selected_files = vec![2];
        edited.per_task_speed_limit = 123;
        edited.status = TaskStatus::Completed;
        store.upsert_task(&edited).await.unwrap();
        old.downloaded_bytes = 42;
        let merged = store.update_bt_progress(&old).await.unwrap().unwrap();
        assert_eq!(merged.bt_meta.unwrap().selected_files, vec![2]);
        assert_eq!(merged.per_task_speed_limit, 123);
        assert_eq!(merged.status, TaskStatus::Completed);
    }

    #[tokio::test]
    async fn removed_task_cannot_be_recreated_by_progress() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().to_owned();
        let store = tokio::task::spawn_blocking(move || Store::open(path))
            .await
            .unwrap()
            .unwrap();
        let task = bt_task_for_test();
        store.upsert_task(&task).await.unwrap();
        store
            .connection
            .lock()
            .await
            .execute("DELETE FROM tasks WHERE id=?1", [&task.id])
            .unwrap();
        assert!(store.update_bt_progress(&task).await.unwrap().is_none());
        assert!(store.get_task(&task.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn selection_and_completion_keep_latest_progress_and_user_fields() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().to_owned();
        let store = tokio::task::spawn_blocking(move || Store::open(path))
            .await
            .unwrap()
            .unwrap();
        let mut task = bt_task_for_test();
        task.downloaded_bytes = 999;
        task.per_task_speed_limit = 321;
        store.upsert_task(&task).await.unwrap();
        let edited = store.update_bt_selection(&task.id, vec![2]).await.unwrap();
        assert_eq!(edited.downloaded_bytes, 999);
        let mut stale = task.clone();
        stale.per_task_speed_limit = 0;
        stale.completed_at = Some(42);
        stale.bt_meta.as_mut().unwrap().seeding_requested = true;
        let completed = store.complete_bt_task(&stale).await.unwrap().unwrap();
        assert_eq!(completed.status, TaskStatus::Completed);
        assert_eq!(completed.completed_at, Some(42));
        assert_eq!(completed.per_task_speed_limit, 321);
        assert_eq!(completed.bt_meta.as_ref().unwrap().selected_files, vec![2]);
        assert!(completed.bt_meta.as_ref().unwrap().seeding_requested);
        assert!(store.complete_bt_task(&stale).await.unwrap().is_none());
        store
            .connection
            .lock()
            .await
            .execute("DELETE FROM tasks WHERE id=?1", [&task.id])
            .unwrap();
        assert!(store.complete_bt_task(&stale).await.unwrap().is_none());
        assert!(store.update_bt_selection(&task.id, vec![1]).await.is_err());
    }

    #[tokio::test]
    async fn paused_task_cannot_be_completed_by_old_worker() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().to_owned();
        let store = tokio::task::spawn_blocking(move || Store::open(path))
            .await
            .unwrap()
            .unwrap();
        let mut task = bt_task_for_test();
        let stale = task.clone();
        task.status = TaskStatus::Paused;
        store.upsert_task(&task).await.unwrap();
        assert!(store.complete_bt_task(&stale).await.unwrap().is_none());
        assert_eq!(
            store.get_task(&task.id).await.unwrap().unwrap().status,
            TaskStatus::Paused
        );
    }
}
