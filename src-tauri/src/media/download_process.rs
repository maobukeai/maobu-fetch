use super::{
    download_progress::Progress,
    output::{OutputManifest, Workspace},
    process::{self, ManagedChild},
    traffic_proxy::TrafficStats,
};
use crate::{manager::SharedManager, models::DownloadTask};
use std::time::{Duration, Instant};
use tokio::{process::Command, sync::mpsc};
use tokio_util::sync::CancellationToken;

pub(super) async fn run(
    command: &mut Command,
    task: &mut DownloadTask,
    manager: &SharedManager,
    stats: &TrafficStats,
    workspace: &Workspace,
    cancel: &CancellationToken,
    timeout: Option<Duration>,
) -> Result<OutputManifest, String> {
    if cancel.is_cancelled() {
        return Err("任务已暂停".into());
    }
    let mut child = ManagedChild::spawn(command)?;
    let stdout = child.child.stdout.take().ok_or("无法读取媒体工具进度")?;
    let stderr = child
        .child
        .stderr
        .take()
        .ok_or("无法读取媒体工具错误信息")?;
    let (sender, mut receiver) = mpsc::channel(8);
    let _readers = process::lines::Readers(vec![
        tokio::spawn(process::lines::pump(stdout, sender.clone())),
        tokio::spawn(process::lines::pump(stderr, sender.clone())),
    ]);
    drop(sender);
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut progress = Progress::default();
    let mut manifest = OutputManifest::default();
    let mut status = None;
    let mut closed = false;
    let mut errors = String::new();
    let mut last_bytes = stats.received();
    let mut last_tick = Instant::now();
    let operation = async {
        loop {
            if closed && status.is_some() {
                break;
            }
            if cancel.is_cancelled() {
                return Err("任务已暂停".into());
            }
            tokio::select! {
                _ = cancel.cancelled() => return Err("任务已暂停".into()),
                result = child.child.wait(), if status.is_none() => {
                    status = Some(result.map_err(|_| "等待媒体工具退出失败")?);
                    child.close_tree();
                },
                line = receiver.recv(), if !closed => match line {
                    Some(Ok(line)) => {
                        let recorded = manifest.record(&line)?;
                        if line.starts_with("MAOBU_META:") { if let Some(metadata) = &manifest.metadata {
                            workspace.record_metadata(metadata).await?;
                            let current=manager.record_media_live(&task.id,super::live_marker::from_metadata(metadata),false).await?
                                .ok_or("任务已暂停或删除，媒体连接已停止")?;
                            task.media=current.media;
                            manager.emit_task("updated",task);
                        } }
                        if !recorded && !progress.record(&line, task)? {
                            // Keep a bounded tail in memory for classification only, never print raw output.
                            errors.push_str(&line); errors.push('\n');
                            if errors.len() > 32 * 1024 { let keep = errors.char_indices().find(|(index,_)| *index >= errors.len()-16*1024).map(|(index,_)| index).unwrap_or(errors.len()); errors.drain(..keep); }
                        }
                    },
                    Some(Err(error)) => return Err(error),
                    None => closed = true,
                },
                _ = interval.tick() => {
                    let received = stats.received();
                    let elapsed = last_tick.elapsed().as_secs_f64().max(0.001);
                    task.speed = (received.saturating_sub(last_bytes) as f64 / elapsed) as u64;
                    task.active_connections = stats.active().min(u8::MAX as usize) as u8;
                    task.eta_seconds = if task.speed > 0 && task.total_bytes > task.downloaded_bytes { Some((task.total_bytes-task.downloaded_bytes)/task.speed) } else { None };
                    last_bytes = received; last_tick = Instant::now();
                    *task = manager.store.update_http_progress(task).await?
                        .ok_or("任务已停止或删除，媒体连接已停止并保留临时文件")?;
                    manager.emit_task("updated", task);
                }
            }
        }
        if !status.is_some_and(|status| status.success()) {
            let platform = crate::media_platforms::detect_platform(&task.url);
            let safe = crate::manager::redact_sensitive(&errors);
            let diagnosis = crate::manager::diagnose::classify_platform_error(platform, &safe);
            return Err(
                crate::manager::diagnose::platform_error_to_chinese_with_evidence(
                    diagnosis, platform, &safe,
                ),
            );
        }
        Ok(manifest)
    };
    let result = tokio::select! {
        result=operation=>result,
        _=async {match timeout {Some(timeout)=>tokio::time::sleep(timeout).await,None=>std::future::pending::<()>().await}} => Err("媒体下载超过任务时限，已停止全部连接并保留临时文件".into()),
    };
    if result.is_err() {
        child.terminate().await;
    }
    task.active_connections = 0;
    task.speed = 0;
    result
}
