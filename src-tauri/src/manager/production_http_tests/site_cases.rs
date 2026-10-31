use super::super::*;
use super::harness::Harness;
use tauri::Manager;

impl DownloadManager {
    pub(crate) async fn settle_public_audit_task(&self, id: &str) -> Result<(), String> {
        self.stop_worker_with_policy(id, false).await?;
        if self.controls.lock().await.contains_key(id) {
            return Err("样本worker控制句柄未退出".into());
        }
        Ok(())
    }
    pub(crate) async fn run_public_audit_task(
        self: &SharedManager,
        mut task: DownloadTask,
        cancel: &CancellationToken,
    ) -> Result<DownloadTask, String> {
        task.status = TaskStatus::Queued;
        self.store.upsert_task(&task).await?;
        self.dispatch_once().await?;
        loop {
            let row = self
                .store
                .get_task(&task.id)
                .await?
                .ok_or("样本任务记录丢失")?;
            if row.status == TaskStatus::Completed
                && !self.controls.lock().await.contains_key(&task.id)
            {
                return Ok(row);
            }
            if matches!(
                row.status,
                TaskStatus::Failed | TaskStatus::RemoteChanged | TaskStatus::Cancelled
            ) && !self.controls.lock().await.contains_key(&task.id)
            {
                return Err(row
                    .error
                    .unwrap_or_else(|| format!("真实管理器终态为{:?}", row.status)));
            }
            if cancel.is_cancelled()
                || row.downloaded_bytes > 256 * 1024 * 1024
                || row.total_bytes > 256 * 1024 * 1024
            {
                self.action(&task.id, "pause").await?;
                return Err("样本已达到时限/体积边界，已暂停真实worker".into());
            }
            if row.status == TaskStatus::Queued {
                self.dispatch_once().await?;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

pub async fn public_samples(app: &AppHandle) {
    let harness = Harness::new(app, Some(240)).await;
    assert!(
        app.manage(harness.manager.clone()),
        "public audit must register its own isolated manager for production media workers"
    );
    let result = crate::media::public_samples::run(app, &harness.manager, &harness.output).await;
    harness.manager.shutdown_bt().await.unwrap();
    let result = result.expect("public sample audit setup must be valid");
    eprintln!("PUBLIC_SAMPLE_BATCH {}", result);
    assert!(result["results"].as_array().unwrap().iter().all(|row|row["passed"]==true),"Some public sites did not pass; inspect each recorded outcome instead of claiming all passed");
}

pub async fn bilibili_sample(app: &AppHandle) {
    assert_eq!(
        std::env::var("MAOBU_TEST_BILIBILI_SAMPLE").as_deref(),
        Ok("1"),
        "public user sample requires explicit opt-in"
    );
    let harness = Harness::new(app, Some(600)).await;
    let result = crate::media::bilibili_sample::run(app, &harness.manager, &harness.output).await;
    harness.manager.shutdown_bt().await.unwrap();
    eprintln!(
        "user-authorized Bilibili sample: {}",
        result.expect("explicit user sample must actually probe, download and validate")
    );
}
