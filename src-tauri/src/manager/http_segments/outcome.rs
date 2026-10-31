use super::*;

pub(super) async fn settle(
    manager: &DownloadManager,
    task: &mut DownloadTask,
    session: &Session,
    error: Option<String>,
) -> Result<(), String> {
    task.downloaded_bytes = session.progress.load(Ordering::Relaxed);
    task.segments = snapshot_segments(&session.plan.runtimes);
    task.active_connections = 0;
    session.options.apply(task).await;
    if session.disk.paused.load(Ordering::Relaxed) {
        return pause_for_disk(manager, task, session).await;
    }
    if let Some(error) = error {
        if error.starts_with(CLOUD_LINK_DEAD_PREFIX) {
            // Keep the task refreshable; the worker loop owns link refresh and final task status.
            task.error = Some("下载直链已过期，正在尝试自动刷新".into());
        } else if session.token.is_cancelled() {
            task.status = TaskStatus::Paused;
            pause_connections(task);
        }
        manager.store.upsert_task(task).await?;
        manager.emit_task("updated", task);
        manager.emit_task_connections_final(
            &task.id,
            &session.plan.runtimes,
            error.starts_with(CLOUD_LINK_DEAD_PREFIX) || session.token.is_cancelled(),
        );
        return Err(error);
    }
    if session.token.is_cancelled() {
        manager.emit_task_connections_final(&task.id, &session.plan.runtimes, false);
        return Err("任务已暂停".into());
    }
    Ok(())
}

fn pause_connections(task: &mut DownloadTask) {
    task.speed = 0;
    task.eta_seconds = None;
    task.active_connections = 0;
    for segment in &mut task.segments {
        if segment.status == "downloading" {
            segment.status = "paused".into();
        }
    }
}

async fn pause_for_disk(
    manager: &DownloadManager,
    task: &mut DownloadTask,
    session: &Session,
) -> Result<(), String> {
    let available = session.disk.available.load(Ordering::Relaxed);
    let required = session.disk.required.load(Ordering::Relaxed);
    task.status = TaskStatus::PausedByLowDisk;
    pause_connections(task);
    task.error = Some(format!(
        "磁盘空间不足（可用 {} 字节，需要 {} 字节），已暂停",
        available, required
    ));
    manager.store.upsert_task(task).await?;
    manager.emit_task("updated", task);
    manager.emit_task_connections_final(&task.id, &session.plan.runtimes, true);
    let _ = manager.app.emit(
        "task-paused-by-low-disk",
        LowDiskPayload {
            task_id: task.id.clone(),
            available_bytes: available,
            required_bytes: required,
        },
    );
    Err(format!(
        "{LOW_DISK_PREFIX}磁盘空间不足（可用 {} 字节，需要 {} 字节）",
        available, required
    ))
}
