//! Status transitions preserve concurrent user settings and never recreate deleted tasks.
use super::*;

impl Store {
    pub async fn update_worker_state(
        &self,
        snapshot: &DownloadTask,
    ) -> Result<Option<DownloadTask>, String> {
        let mut connection = self.connection.lock().await;
        let transaction = connection
            .transaction()
            .map_err(|e| format!("保存任务状态失败：{e}"))?;
        let Some(current) = transaction
            .query_row(
                "SELECT * FROM tasks WHERE id=?1",
                [&snapshot.id],
                task_from_row,
            )
            .optional()
            .map_err(|e| format!("读取任务状态失败：{e}"))?
        else {
            return Ok(None);
        };
        if matches!(
            current.status,
            TaskStatus::Completed | TaskStatus::Cancelled
        ) {
            return Ok(None);
        }
        let mut next = snapshot.clone();
        next.priority = current.priority;
        next.queue_position = current.queue_position;
        next.per_task_speed_limit = current.per_task_speed_limit;
        next.completion_action = current.completion_action;
        next.retry_policy_override = current.retry_policy_override;
        next.proxy_override = current.proxy_override;
        next.proxy_auth = current.proxy_auth;
        Self::upsert_with(&transaction, &next)?;
        transaction
            .commit()
            .map_err(|e| format!("提交任务状态失败：{e}"))?;
        Ok(Some(next))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn failed_worker_preserves_new_options_and_does_not_resurrect_deleted_rows() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_owned();
        let store = tokio::task::spawn_blocking(move || Store::open(path))
            .await
            .unwrap()
            .unwrap();
        let mut stale = crate::bt::tests::bt_task_for_test();
        stale.status = TaskStatus::Downloading;
        store.upsert_task(&stale).await.unwrap();
        let mut other = stale.clone();
        other.id = uuid::Uuid::new_v4().to_string();
        store.upsert_task(&other).await.unwrap();
        store
            .reorder(&[other.id.clone(), stale.id.clone()])
            .await
            .unwrap();
        store
            .update_runtime_options(
                &stale.id,
                Some(8),
                Some(16384),
                Some(CompletionAction::OpenFolder),
            )
            .await
            .unwrap();
        let policy = crate::models::RetryPolicy {
            max_retries: 9,
            ..Default::default()
        };
        store
            .update_retry_policy(&stale.id, Some(policy.clone()))
            .await
            .unwrap();
        store
            .update_proxy(&stale.id, Some(String::new()), None)
            .await
            .unwrap();
        stale.status = TaskStatus::Failed;
        stale.error = Some("network failure".into());
        let saved = store.update_worker_state(&stale).await.unwrap().unwrap();
        assert_eq!(saved.priority, 8);
        assert_eq!(saved.queue_position, 1);
        assert_eq!(saved.per_task_speed_limit, 16384);
        assert_eq!(saved.completion_action, CompletionAction::OpenFolder);
        assert_eq!(saved.retry_policy_override, Some(policy));
        assert_eq!(saved.proxy_override, Some(String::new()));
        assert_eq!(saved.status, TaskStatus::Failed);
        store.remove_task(&stale.id).await.unwrap();
        assert!(store.update_worker_state(&stale).await.unwrap().is_none());
    }
}
