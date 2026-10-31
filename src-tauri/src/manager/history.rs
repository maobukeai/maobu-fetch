//! Clearing history stops persisted native sessions while preserving every downloaded file.
use super::*;

impl DownloadManager {
    pub async fn clear_history(
        self: &SharedManager,
        include_completed: bool,
    ) -> Result<(), String> {
        let terminal = |status: &TaskStatus| {
            *status == TaskStatus::Cancelled
                || (include_completed && *status == TaskStatus::Completed)
        };
        let candidates = self.store.list_tasks().await?;
        let mut removed_bt = false;
        for candidate in candidates.into_iter().filter(|task| terminal(&task.status)) {
            let operation = self.task_operation(&candidate.id).await;
            let _guard = operation.lock().await;
            let Some(current) = self.store.get_task(&candidate.id).await? else {
                continue;
            };
            if !terminal(&current.status) {
                continue;
            }
            self.stop_worker(&current.id).await?;
            let Some(current) = self.store.get_task(&candidate.id).await? else {
                continue;
            };
            if !terminal(&current.status) {
                continue;
            }
            let removal = if current.task_kind == TaskKind::Bt {
                Some(self.bt.removal_guard().await)
            } else {
                None
            };
            if let Some(removal) = removal.as_ref() {
                removal.remove_task_for(&current).await?;
                removed_bt = true;
            }
            self.store.remove_task(&current.id).await?;
            drop(removal);
            let _ = self.app.emit("task-removed", current.id);
        }
        if removed_bt {
            self.schedule_seeding_sync();
        }
        Ok(())
    }
}
