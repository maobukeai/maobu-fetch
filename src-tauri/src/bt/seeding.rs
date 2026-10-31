//! 做种有独立生命周期；下载完成事件和并发名额不等待分享率。
use super::{worker::apply_progress, POLL_INTERVAL};
use crate::manager::{redact_sensitive, SharedManager};
use crate::models::TaskStatus;
use std::sync::Arc;
use tauri::Emitter;
use tokio_util::sync::CancellationToken;

pub(super) async fn start_monitor(manager: &SharedManager, task_id: &str) {
    let generation = uuid::Uuid::new_v4().to_string();
    let token = CancellationToken::new();
    let old = manager
        .bt
        .seed_monitors
        .lock()
        .await
        .insert(task_id.to_owned(), (generation.clone(), token.clone()));
    if let Some((_, old)) = old {
        old.cancel();
    }
    let weak = Arc::downgrade(manager);
    let id = task_id.to_owned();
    tokio::spawn(async move {
        let mut refill = false;
        loop {
            tokio::select! { _ = token.cancelled() => break, _ = tokio::time::sleep(POLL_INTERVAL) => {} }
            let Some(manager) = weak.upgrade() else {
                break;
            };
            let result = update_seeding(&manager, &id).await;
            match result {
                Ok(true) => {}
                Ok(false) => {
                    refill = true;
                    break;
                }
                Err(error) => {
                    tracing::error!(task_id=%id, error=%redact_sensitive(&error), "更新后台做种状态失败");
                    let mut message = redact_sensitive(&error);
                    if let Err(stop_error) = manager.bt.pause_task(&id).await {
                        tracing::error!(task_id=%id, error=%redact_sensitive(&stop_error), "停止后台做种失败");
                        message.push_str(&format!(
                            "；停止做种失败：{}",
                            redact_sensitive(&stop_error)
                        ));
                    }
                    match manager
                        .store
                        .update_bt_seed_request(&id, false, Some(message.clone()))
                        .await
                    {
                        Ok(Some(task)) => manager.emit_task("updated", &task),
                        Ok(None) => {}
                        Err(error) => {
                            message.push_str(&format!(
                                "；保存做种错误失败：{}",
                                redact_sensitive(&error)
                            ));
                        }
                    }
                    let _ = manager.app.emit("notification-error", message);
                    refill = true;
                    break;
                }
            }
        }
        if let Some(manager) = weak.upgrade() {
            let mut monitors = manager.bt.seed_monitors.lock().await;
            let old_or_removed = monitors
                .get(&id)
                .map(|(current, _)| current == &generation)
                .unwrap_or(true);
            if old_or_removed {
                monitors.remove(&id);
            }
            drop(monitors);
            if old_or_removed {
                if let Ok(Some(mut task)) = manager.store.get_task(&id).await {
                    if task.status == TaskStatus::Completed {
                        if let Ok(progress) = manager.bt.status_of(&id, &task.destination).await {
                            apply_progress(&mut task, &progress);
                            match manager.store.update_bt_progress(&task).await {
                                Ok(Some(updated)) => manager.emit_task("updated", &updated),
                                Ok(None) => {}
                                Err(error) => {
                                    tracing::error!(task_id=%id,error=%redact_sensitive(&error),"保存做种结束状态失败")
                                }
                            }
                        }
                    }
                }
            }
            if refill {
                super::seed_policy::schedule_reconcile(manager.clone());
            }
        }
    });
}

async fn update_seeding(manager: &SharedManager, id: &str) -> Result<bool, String> {
    let Some(mut current) = manager.store.get_task(id).await? else {
        return Ok(false);
    };
    // 下载 worker 尚未提交 Completed 时不抢写终态。
    if current.status == TaskStatus::Downloading {
        return Ok(true);
    }
    if current.status != TaskStatus::Completed {
        return Ok(false);
    }
    let progress = manager.bt.status_of(id, &current.destination).await?;
    if progress.state == "error" || progress.state == "removed" {
        return Err(progress
            .error
            .unwrap_or_else(|| "BT 做种连接已停止，请检查任务文件后重新开始做种".into()));
    }
    let settings = manager.settings().await;
    let ratio = if progress.total_bytes == 0 {
        0.0
    } else {
        progress.uploaded_bytes as f64 / progress.total_bytes as f64
    };
    let requested = current
        .bt_meta
        .as_ref()
        .is_some_and(|m| m.seeding_requested);
    let ratio_reached = settings.bt_seed_ratio > 0.0 && ratio >= settings.bt_seed_ratio;
    let continue_seeding = settings.bt_seed_enabled
        && requested
        && (settings.bt_seed_ratio <= 0.0 || ratio < settings.bt_seed_ratio);
    apply_progress(&mut current, &progress);
    if !continue_seeding {
        manager.bt.pause_task(id).await?;
        if ratio_reached {
            manager
                .store
                .update_bt_seed_request(id, false, None)
                .await?;
        }
        if let Some(runtime) = current.bt_runtime.as_mut() {
            runtime.seeding = false;
            runtime.upload_speed = 0;
        }
    }
    if let Some(updated) = manager.store.update_bt_progress(&current).await? {
        manager.emit_task("updated", &updated);
    }
    Ok(continue_seeding)
}
