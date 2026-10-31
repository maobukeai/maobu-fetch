//! 只恢复用户请求的做种任务，容量独立于下载槽；启动前只读重检。
use super::{worker::apply_progress, BtEngine};
use crate::manager::{redact_sensitive, SharedManager};
use crate::models::{AppSettings, DownloadTask, TaskKind, TaskStatus};

impl BtEngine {
    /// 返回前只做哈希校验；损坏或缺失数据不会触发重新下载。
    pub(super) async fn prepare_seeding(
        &self,
        settings: &AppSettings,
        task: &mut DownloadTask,
    ) -> Result<(), String> {
        if task.task_kind != TaskKind::Bt || task.status != TaskStatus::Completed {
            return Err("仅已完成的 BT 任务可以开始做种".into());
        }
        self.prepare_task(settings, task, false).await?;
        let session = self.session.lock().await.clone().ok_or("BT 会话已停止")?;
        let mut handle = self
            .bindings
            .lock()
            .await
            .get(&task.id)
            .cloned()
            .ok_or("BT 任务尚未注册")?;
        if matches!(
            handle.stats().state,
            librqbit::TorrentStatsState::Initializing { paused: true }
        ) {
            // A cancelled native recheck stays Initializing+paused until a download resume.
            // Rebind its metadata with delete_files=false instead: seed-only preparation must
            // not open the peer stream that Session::unpause would attach on incomplete data.
            self.remove_task_for_unlocked(task).await?;
            self.prepare_task(settings, task, false).await?;
            handle = self
                .bindings
                .lock()
                .await
                .get(&task.id)
                .cloned()
                .ok_or("BT 任务重检未返回句柄")?;
        }
        // 即使 clean fastresume 的抽样通过，也完整检查一次，防止上传被外部修改的数据。
        wait_for_check(&session, &handle).await?;
        self.pause_task_for_preparation(&task.id).await?;
        handle
            .recheck_paused()
            .map_err(|e| format!("启动 BT 数据重检失败：{e}"))?;
        wait_for_check(&session, &handle).await?;
        if !handle.stats().finished {
            return Err("已有 BT 文件缺失或未通过哈希校验，已保留完成记录并停止做种；请检查文件，或明确选择重新下载".into());
        }
        Ok(())
    }

    async fn start_verified_seed(
        &self,
        settings: &AppSettings,
        task: &DownloadTask,
    ) -> Result<(), String> {
        let _registration = self.registration.lock().await;
        let session = self.get_or_init_session(Some(settings)).await?;
        let handle = self
            .bindings
            .lock()
            .await
            .get(&task.id)
            .cloned()
            .ok_or("BT 任务尚未注册")?;
        if !handle.stats().finished {
            return Err("BT 数据尚未完整校验，未开始做种".into());
        }
        self.configure_handle(&session, &handle, task, true).await
    }
}

pub(super) async fn wait_for_check(
    session: &librqbit::Session,
    handle: &librqbit::ManagedTorrent,
) -> Result<(), String> {
    let cancellation = session.cancellation_token();
    tokio::select! {
        _ = cancellation.cancelled() => Err("BT 会话已停止，已取消做种检查".into()),
        result = handle.wait_until_initialized() => result.map_err(|e| format!("检查 BT 数据失败：{e}")),
    }
}

/// 显式开始/停止历史任务的做种意图；全局开关仍具有优先权。
pub async fn request_seeding(
    manager: &SharedManager,
    id: &str,
    requested: bool,
) -> Result<(), String> {
    let Some(task) = manager
        .store
        .update_bt_seed_request(id, requested, None)
        .await?
    else {
        return Err("任务已被删除".into());
    };
    manager.emit_task("updated", &task);
    if !requested {
        manager.bt.pause_task(id).await?;
    }
    sync_seeding_policy(manager).await
}

/// 由启动、设置改变和 Completed 提交事件触发，不轮询全量任务。
pub async fn sync_seeding_policy(manager: &SharedManager) -> Result<(), String> {
    let _guard = manager.bt.seed_sync.lock().await;
    let settings = manager.settings().await;
    let tasks = manager.store.list_tasks().await?;
    let eligible: Vec<_> = tasks
        .into_iter()
        .filter(|t| {
            t.task_kind == TaskKind::Bt
                && t.status == TaskStatus::Completed
                && t.bt_meta.as_ref().is_some_and(|m| m.seeding_requested)
        })
        .collect();
    let limit = usize::from(settings.concurrent_downloads.clamp(1, 32));
    let running: Vec<_> = manager
        .bt
        .seed_monitors
        .lock()
        .await
        .keys()
        .cloned()
        .collect();
    let mut retained = 0;
    for id in running {
        let keep =
            settings.bt_seed_enabled && retained < limit && eligible.iter().any(|t| t.id == id);
        if keep {
            retained += 1;
        } else {
            manager.bt.pause_task(&id).await?;
        }
    }
    if !settings.bt_seed_enabled {
        return Ok(());
    }
    let mut errors = Vec::new();
    for mut task in eligible {
        if manager.bt.seed_monitors.lock().await.contains_key(&task.id) {
            continue;
        }
        if retained >= limit {
            break;
        }
        if ratio_reached(&task, settings.bt_seed_ratio) {
            if let Some(updated) = manager
                .store
                .update_bt_seed_request(&task.id, false, None)
                .await?
            {
                manager.emit_task("updated", &updated);
            }
            continue;
        }
        let preparation = manager.bt.begin_seed_preparation(&task.id);
        let result = preparation
            .run(manager.bt.prepare_seeding(&settings, &mut task))
            .await;
        drop(preparation);
        let Some(result) = result else {
            manager.bt.pause_task_for_preparation(&task.id).await?;
            return Ok(());
        };
        if let Err(error) = result {
            if !manager.settings().await.bt_seed_enabled {
                continue;
            }
            let error = redact_sensitive(&error);
            if let Some(updated) = manager
                .store
                .update_bt_seed_error(&task.id, error.clone())
                .await?
            {
                manager.emit_task("updated", &updated);
                errors.push(error);
            }
            continue;
        }
        // 重检可能耗时，必须重新确认删除/用户停止/全局开关，不能复活旧意图。
        let current_settings = manager.settings().await;
        let Some(current) = manager.store.get_task(&task.id).await? else {
            // A legacy caller may have removed its record during the full recheck.
            // We already hold seed_sync here; remove a newly prepared paused binding without relocking it.
            manager.bt.remove_task_for_unlocked(&task).await?;
            continue;
        };
        if !current_settings.bt_seed_enabled
            || current.status != TaskStatus::Completed
            || !current
                .bt_meta
                .as_ref()
                .is_some_and(|m| m.seeding_requested)
        {
            continue;
        }
        manager.store.update_bt_ownership(&task).await?;
        if let Err(error) = manager
            .bt
            .start_verified_seed(&current_settings, &current)
            .await
        {
            let mut error = redact_sensitive(&error);
            if let Err(stop) = manager.bt.pause_task(&task.id).await {
                error.push_str(&format!(
                    "；停止做种或保存恢复信息失败：{}",
                    redact_sensitive(&stop)
                ));
            }
            if !manager.settings().await.bt_seed_enabled {
                continue;
            }
            if let Some(updated) = manager
                .store
                .update_bt_seed_error(&task.id, error.clone())
                .await?
            {
                manager.emit_task("updated", &updated);
                errors.push(error);
            }
            continue;
        }
        // 一旦原生开始运行就安排 monitor；后续 SQLite 报错也不能留下无管理上传。
        super::seeding::start_monitor(manager, &task.id).await;
        let progress = manager.bt.status_of(&task.id, &task.destination).await?;
        apply_progress(&mut task, &progress);
        if let Some(updated) = manager.store.update_bt_progress(&task).await? {
            manager.emit_task("updated", &updated);
            retained += 1;
        } else {
            manager.bt.pause_task(&task.id).await?;
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

pub(super) fn ratio_reached(task: &DownloadTask, target: f64) -> bool {
    target > 0.0
        && task.total_bytes > 0
        && task.bt_meta.as_ref().map(|m| m.uploaded_bytes).unwrap_or(0) as f64
            / task.total_bytes as f64
            >= target
}

/// Boxed future breaks the monitor -> reconciliation -> monitor type cycle.
pub(super) fn schedule_reconcile(manager: SharedManager) {
    let work: futures_util::future::BoxFuture<'static, ()> = Box::pin(async move {
        if let Err(error) = sync_seeding_policy(&manager).await {
            tracing::error!(error=%redact_sensitive(&error), "恢复 BT 后台做种失败");
        }
    });
    tokio::spawn(work);
}
