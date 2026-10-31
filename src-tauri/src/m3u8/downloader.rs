//! Tauri task adapter for the independently tested HLS engine.
use super::{
    engine::{self, HlsOptions},
    request::{HlsClient, LimitHook, TransferProgress},
    resume,
};
use crate::{
    manager::{DownloadManager, RateLimiter, RuntimeTaskOptions},
    models::{DownloadTask, TaskStatus},
};
use std::{
    path::Path,
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

pub use super::resume::segment_part_path;

pub async fn download_m3u8_task(
    manager: &DownloadManager,
    task: DownloadTask,
    client: &reqwest::Client,
    temp_path: &Path,
    token: CancellationToken,
    task_limiter: Arc<RateLimiter>,
) -> Result<DownloadTask, String> {
    let existing_runtime = manager.task_runtime.read().await.get(&task.id).cloned();
    let runtime = match existing_runtime {
        Some(runtime) => runtime,
        None => {
            let runtime = Arc::new(RuntimeTaskOptions::new(&task));
            manager
                .task_runtime
                .write()
                .await
                .insert(task.id.clone(), runtime.clone());
            runtime
        }
    };
    let progress = Arc::new(TransferProgress::default());
    let work_cancel = token.child_token();
    let finalize_live = manager.live_finalize_permission(&task.id).await;
    let options = HlsOptions {
        http: HlsClient {
            client: client.clone(),
            source_url: task.url.clone(),
            headers: task.headers.clone(),
            cancel: work_cancel.clone(),
            progress: progress.clone(),
            retries: task.max_retries,
            limit: bandwidth_hook(manager, &task, task_limiter, runtime.clone()),
        },
        user_cancel: token.clone(),
        finalize_live,
        concurrency: usize::from(task.connection_count),
        live_idle_timeout: Duration::from_secs(90),
    };
    let mut download = Box::pin(engine::download(&options, temp_path));
    let mut ticker = tokio::time::interval(Duration::from_millis(500));
    let mut previous = (Instant::now(), 0u64);
    let outcome = loop {
        tokio::select! {
            biased;
            result = &mut download => break result,
            _ = ticker.tick() => {
                if token.is_cancelled() { continue; }
                if let Err(error) = publish_progress(manager, &task.id, &progress, &runtime, &mut previous).await {
                    work_cancel.cancel();
                    let _ = (&mut download).await;
                    return Err(error);
                }
            }
        }
    };
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            if error.starts_with("REMOTE_CHANGED:") {
                if let Some(mut current) = manager.store.get_task(&task.id).await? {
                    current.status = TaskStatus::RemoteChanged;
                    current.error = Some(error.trim_start_matches("REMOTE_CHANGED:").to_owned());
                    current.speed = 0;
                    current.active_connections = 0;
                    manager.store.upsert_task(&current).await?;
                    manager.emit_task("updated", &current);
                }
            }
            return Err(error);
        }
    };
    let mut finished = manager
        .store
        .get_task(&task.id)
        .await?
        .ok_or("M3U8 任务记录已移除，停止保存完成状态")?;
    finished.downloaded_bytes = outcome.bytes;
    finished.total_bytes = outcome.bytes;
    finished.speed = 0;
    finished.eta_seconds = None;
    finished.active_connections = 0;
    if outcome.stopped_live {
        finished.status = TaskStatus::Completed;
    }
    Ok(finished)
}

fn bandwidth_hook(
    manager: &DownloadManager,
    task: &DownloadTask,
    limiter: Arc<RateLimiter>,
    runtime: Arc<RuntimeTaskOptions>,
) -> LimitHook {
    let bandwidth = manager.bandwidth_scheduler.clone();
    let task_id = task.id.clone();
    Arc::new(move |bytes, cancel| {
        let (bandwidth, task_id, runtime, limiter) = (
            bandwidth.clone(),
            task_id.clone(),
            runtime.clone(),
            limiter.clone(),
        );
        Box::pin(async move {
            bandwidth
                .acquire(
                    &task_id,
                    bytes,
                    runtime.priority.load(Ordering::Relaxed),
                    &cancel,
                )
                .await;
            limiter
                .acquire_with_cancel(bytes, runtime.speed_limit.load(Ordering::Relaxed), &cancel)
                .await;
        })
    })
}

async fn publish_progress(
    manager: &DownloadManager,
    id: &str,
    progress: &TransferProgress,
    runtime: &RuntimeTaskOptions,
    previous: &mut (Instant, u64),
) -> Result<(), String> {
    let Some(mut current) = manager
        .record_media_live(id, progress.is_live.load(Ordering::Acquire), true)
        .await?
    else {
        return Err("M3U8 任务记录已移除，停止活动连接".into());
    };
    if current.status != TaskStatus::Downloading {
        return Ok(());
    }
    let now = Instant::now();
    let network_bytes = progress.network_bytes.load(Ordering::Relaxed);
    let elapsed = now.duration_since(previous.0).as_secs_f64();
    current.speed = if elapsed > 0.0 {
        (network_bytes.saturating_sub(previous.1) as f64 / elapsed) as u64
    } else {
        0
    };
    *previous = (now, network_bytes);
    current.downloaded_bytes = progress.saved_bytes.load(Ordering::Relaxed);
    current.total_bytes = 0;
    current.active_connections = progress.active_connections.load(Ordering::Relaxed).min(32) as u8;
    current.eta_seconds = None;
    runtime.apply(&mut current).await;
    let Some(current) = manager.store.update_http_progress(&current).await? else {
        return Err("M3U8 任务已暂停或移除，停止活动连接".into());
    };
    manager.emit_task("updated", &current);
    Ok(())
}

/// Only after the manager has atomically published and verified the final target.
pub async fn cleanup_completed_parts(temp: &Path) -> Result<(), String> {
    let path = resume::journal_path(temp);
    let data = match tokio::fs::read(&path).await {
        Ok(data) => data,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("读取 M3U8 清理清单失败：{error}")),
    };
    let journal: resume::ResumeJournal =
        serde_json::from_slice(&data).map_err(|_| "M3U8 清理清单损坏，已保留恢复文件")?;
    for entry in journal.entries {
        let part = segment_part_path(temp, entry.index);
        match tokio::fs::remove_file(part).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("清理 M3U8 分片失败：{error}")),
        }
    }
    tokio::fs::remove_file(path)
        .await
        .map_err(|error| format!("清理 M3U8 恢复清单失败：{error}"))
}
