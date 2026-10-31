//! Verification is a transition on an existing row, never a stale snapshot upsert.
use super::*;
impl Store {
    pub async fn begin_verification(&self, id: &str) -> Result<DownloadTask, String> {
        let mut connection = self.connection.lock().await;
        let transaction = connection
            .transaction()
            .map_err(|e| format!("开始校验失败：{e}"))?;
        let mut task = transaction
            .query_row("SELECT * FROM tasks WHERE id=?1", [id], task_from_row)
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("任务不存在")?;
        if task.task_kind == crate::models::TaskKind::Bt {
            return Err("BT数据由原生分片哈希校验，请在BT详情查看校验状态".into());
        }
        if !matches!(task.status, TaskStatus::Completed | TaskStatus::Failed) {
            return Err("任务尚未完成或正在处理，暂不能校验完成文件".into());
        }
        task.status = TaskStatus::Verifying;
        Self::upsert_with(&transaction, &task)?;
        transaction
            .commit()
            .map_err(|e| format!("保存校验状态失败：{e}"))?;
        Ok(task)
    }
    pub async fn finish_verification(
        &self,
        id: &str,
        hash: Option<String>,
        error: Option<String>,
    ) -> Result<DownloadTask, String> {
        let mut connection = self.connection.lock().await;
        let transaction = connection
            .transaction()
            .map_err(|e| format!("完成校验失败：{e}"))?;
        let mut task = transaction
            .query_row("SELECT * FROM tasks WHERE id=?1", [id], task_from_row)
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("任务已被删除，校验结果未写回")?;
        if task.status != TaskStatus::Verifying {
            return Err("任务状态已变化，未用旧校验结果覆盖新状态".into());
        }
        task.status = if error.is_some() {
            TaskStatus::Failed
        } else {
            TaskStatus::Completed
        };
        task.error = error.map(|value| crate::manager::redact_sensitive(&value));
        if let Some(hash) = hash {
            task.checksum_sha256 = Some(hash);
        }
        task.speed = 0;
        task.active_connections = 0;
        Self::upsert_with(&transaction, &task)?;
        transaction
            .commit()
            .map_err(|e| format!("提交校验结果失败：{e}"))?;
        Ok(task)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn verification_preserves_new_options_and_does_not_resurrect_deleted_or_changed_tasks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_owned();
        let store = tokio::task::spawn_blocking(move || Store::open(path))
            .await
            .unwrap()
            .unwrap();
        let mut task = crate::bt::tests::bt_task_for_test();
        task.task_kind = crate::models::TaskKind::Http;
        task.status = TaskStatus::Completed;
        store.upsert_task(&task).await.unwrap();
        store.begin_verification(&task.id).await.unwrap();
        store
            .update_runtime_options(&task.id, Some(7), Some(12345), None)
            .await
            .unwrap();
        let result = store
            .finish_verification(&task.id, Some("hash".into()), None)
            .await
            .unwrap();
        assert_eq!(result.priority, 7);
        assert_eq!(result.per_task_speed_limit, 12345);
        store.begin_verification(&task.id).await.unwrap();
        store.remove_task(&task.id).await.unwrap();
        assert!(store
            .finish_verification(&task.id, None, None)
            .await
            .is_err());
        assert!(store.get_task(&task.id).await.unwrap().is_none());
        task.status = TaskStatus::Completed;
        store.upsert_task(&task).await.unwrap();
        store.begin_verification(&task.id).await.unwrap();
        task.status = TaskStatus::Queued;
        store.upsert_task(&task).await.unwrap();
        assert!(store
            .finish_verification(&task.id, None, None)
            .await
            .is_err());
        assert_eq!(
            store.get_task(&task.id).await.unwrap().unwrap().status,
            TaskStatus::Queued
        );
    }
}
