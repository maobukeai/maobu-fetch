//! 下载完成与后台做种分别管理，进度来自 librqbit 已校验状态。
use super::{status, BT_TERMINAL_PREFIX, POLL_INTERVAL};
use crate::manager::SharedManager;
use crate::models::{BtRuntimeStatus, DownloadTask, TaskStatus};
use tokio_util::sync::CancellationToken;
// ===== 任务运行循环（由 manager.spawn_worker 调用） =====

/// BT 任务 worker：添加到 librqbit 后轮询真实状态，直到完成/失败/取消。
/// 返回语义与 `download_once` 一致：`Ok(task)` = 下载完成；
/// `Err(BT_TERMINAL_PREFIX..)` = 终态失败（任务状态已落库）。
pub async fn run_task(
    manager: &SharedManager,
    mut task: DownloadTask,
    token: CancellationToken,
) -> Result<DownloadTask, String> {
    let settings = manager.settings().await;
    let add_result = tokio::select! {
        biased;
        _ = token.cancelled() => {
            manager.bt.pause_task(&task.id).await?;
            return Err("BT_CANCELLED".into());
        }
        result = manager.bt.add_task(&manager.app, &settings, &mut task) => result,
    };
    let gid = match add_result {
        Ok(gid) => gid,
        Err(error) => return fail_terminal(manager, &mut task, &error).await,
    };
    tracing::info!(task_id = %task.id, gid = %gid, "BT 任务开始");
    poll_until_finished(manager, &mut task, &token).await
}

/// 轮询 BT 状态直到终态。取消时暂停下载并返回通用错误。
async fn poll_until_finished(
    manager: &SharedManager,
    task: &mut DownloadTask,
    token: &CancellationToken,
) -> Result<DownloadTask, String> {
    loop {
        tokio::select! {
            _ = token.cancelled() => {
                manager.bt.pause_task(&task.id).await?;
                return Err("BT_CANCELLED".into());
            }
            _ = tokio::time::sleep(POLL_INTERVAL) => {}
        }
        let progress = match manager.bt.status_of(&task.id, &task.destination).await {
            Ok(progress) => progress,
            Err(error) => return fail_terminal(manager, task, &error).await,
        };
        if token.is_cancelled() {
            manager.bt.pause_task(&task.id).await?;
            return Err("BT_CANCELLED".into());
        }
        apply_progress(task, &progress);
        let Some(current) = manager.store.update_bt_progress(task).await? else {
            manager.bt.pause_task(&task.id).await?;
            return Err("BT_CANCELLED".into());
        };
        *task = current;
        let task_limit = task.per_task_speed_limit;
        manager
            .bt
            .set_task_limits(&task.id, task_limit, task_limit)
            .await?;
        manager.emit_task("updated", task);
        if task.status != TaskStatus::Downloading {
            manager.bt.pause_task(&task.id).await?;
            return Err("BT_CANCELLED".into());
        }
        match progress.state.as_str() {
            "complete" => {
                let settings = manager.settings().await;
                // 原子提交 Completed 之后才交给独立做种调度，避免下载槽和做种混用。
                manager.bt.pause_task(&task.id).await?;
                let paused = manager.bt.status_of(&task.id, &task.destination).await?;
                apply_progress(task, &paused);
                if let Some(meta) = task.bt_meta.as_mut() {
                    meta.seeding_requested = settings.bt_seed_enabled;
                }
                if let Some(runtime) = task.bt_runtime.as_mut() {
                    runtime.seeding = false;
                    runtime.upload_speed = 0;
                }
                // 哈希校验完成就结束下载 worker，后台做种不占用下载槽。
                return Ok(task.clone());
            }
            "error" => {
                let message = progress.error.unwrap_or_else(|| "BT 下载失败".into());
                return fail_terminal(manager, task, &message).await;
            }
            "removed" => {
                return fail_terminal(manager, task, "下载已被移除，请重试任务").await;
            }
            _ => {}
        }
    }
}

/// 把 BT 真实进度映射进任务字段。
pub(super) fn apply_progress(task: &mut DownloadTask, progress: &status::BtProgress) {
    if !progress.metadata_fetching || progress.total_bytes > 0 {
        task.total_bytes = progress.total_bytes;
        task.downloaded_bytes = progress.downloaded_bytes;
    }
    task.speed = progress.download_speed;
    task.active_connections = progress.num_peers.min(u8::MAX as u32) as u8;
    task.eta_seconds = status::estimate_eta(progress);
    task.bt_runtime = Some(BtRuntimeStatus {
        num_seeds: progress.num_seeds,
        seeds_known: progress.seeds_known,
        num_peers: progress.num_peers,
        upload_speed: progress.upload_speed,
        fetching_metadata: progress.metadata_fetching,
        uploaded_bytes: progress.uploaded_bytes,
        seeding: progress.seeder,
    });
    if let Some(meta) = task.bt_meta.as_mut() {
        meta.uploaded_bytes = meta.uploaded_bytes.max(progress.uploaded_bytes);
        if let Some(output) = &progress.output_folder {
            meta.output_folder = Some(output.clone());
        }
        if !progress.metadata_fetching {
            meta.owned_files = progress
                .files
                .iter()
                .map(|file| file.path.clone())
                .collect();
        }
    }
    apply_metadata_transition(task, progress);
}

/// 元数据就绪瞬间：回填显示名/文件名/infohash。
pub(super) fn apply_metadata_transition(task: &mut DownloadTask, progress: &status::BtProgress) {
    let Some(meta) = task.bt_meta.as_mut() else {
        return;
    };
    let had_name = meta.metadata_ready && meta.display_name.is_some();
    if progress.total_bytes > 0 || !progress.metadata_fetching {
        meta.metadata_ready = true;
    }
    if !progress.info_hash.is_empty() {
        meta.info_hash.clone_from(&progress.info_hash);
        meta.info_hash = meta.info_hash.to_lowercase();
    }
    if let Some(name) = progress.display_name.as_ref() {
        if !name.trim().is_empty() {
            meta.display_name = Some(name.clone());
            // 多文件种子显示种子名；单文件种子显示文件名（去掉路径分量）。
            let display = if progress.files.len() <= 1 {
                progress
                    .files
                    .first()
                    .map(|file| last_path_component(&file.path))
                    .unwrap_or_else(|| name.clone())
            } else {
                name.clone()
            };
            let cleaned = crate::manager::safe_name(&display);
            if !had_name && !cleaned.is_empty() {
                task.file_name = cleaned;
                task.category = crate::manager::category(&task.file_name);
            }
        }
    }
}

fn last_path_component(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// 终态失败：落库 Failed + emit + 返回 `BT_TERMINAL` 前缀错误。
async fn fail_terminal(
    manager: &SharedManager,
    task: &mut DownloadTask,
    message: &str,
) -> Result<DownloadTask, String> {
    task.status = TaskStatus::Failed;
    let mut message = crate::manager::redact_sensitive(message);
    task.error = Some(message.clone());
    task.speed = 0;
    task.eta_seconds = None;
    task.active_connections = 0;
    if let Err(error) = manager.bt.pause_task(&task.id).await {
        message = format!(
            "{}；停止任务或保存恢复信息失败：{}",
            message,
            crate::manager::redact_sensitive(&error)
        );
        task.error = Some(message.clone());
    }
    if let Some(current) = manager.store.update_bt_progress(task).await? {
        *task = current;
        manager.emit_task("updated", task);
    }
    Err(format!("{BT_TERMINAL_PREFIX}{message}"))
}
