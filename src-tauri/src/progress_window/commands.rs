use super::{
    authorized, native,
    state::{ProgressWindowState, Snapshot},
};
use crate::manager::SharedManager;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

type Progress<'a> = State<'a, Arc<ProgressWindowState>>;

#[tauri::command]
pub(crate) fn progress_window_snapshot(
    window: WebviewWindow,
    state: Progress<'_>,
) -> Result<Snapshot, String> {
    authorized(window.label())?;
    state.snapshot()
}

#[tauri::command]
pub(crate) async fn progress_window_set_pinned(
    window: WebviewWindow,
    app: AppHandle,
    state: Progress<'_>,
    manager: State<'_, SharedManager>,
    pinned: bool,
) -> Result<Snapshot, String> {
    authorized(window.label())?;
    let _operation = state.operation.lock().await;
    let previous = state.lock()?.pinned;
    state.lock()?.pin_inflight = true;
    if let Err(error) = native::set_pinned_on_main(&app, pinned).await {
        state.lock()?.pin_inflight = false;
        return Err(error);
    }
    let settings = match manager.set_progress_window_pinned(pinned).await {
        Ok(settings) => settings,
        Err(error) => {
            let rollback = native::set_pinned_on_main(&app, previous).await;
            state.lock()?.pin_inflight = false;
            state.notify.notify_one();
            if let Err(rollback) = rollback {
                return Err(format!("{error}；窗口置顶状态恢复失败：{rollback}"));
            }
            return Err(error);
        }
    };
    state.lock()?.preferences(&settings);
    state.lock()?.pin_inflight = false;
    state.notify.notify_one();
    state.snapshot()
}

#[tauri::command]
pub(crate) fn progress_window_set_expanded(
    window: WebviewWindow,
    state: Progress<'_>,
    expanded: bool,
) -> Result<Snapshot, String> {
    authorized(window.label())?;
    {
        let mut cache = state.lock()?;
        cache.expanded = expanded;
        cache.revision = cache.revision.saturating_add(1);
    }
    state.notify.notify_one();
    state.snapshot()
}

#[tauri::command]
pub(crate) fn progress_window_set_revealed(
    window: WebviewWindow,
    state: Progress<'_>,
    revealed: bool,
) -> Result<Snapshot, String> {
    authorized(window.label())?;
    if state.lock()?.set_revealed(revealed) {
        state.notify.notify_one();
    }
    state.snapshot()
}

#[tauri::command]
pub(crate) fn progress_window_dismiss(
    window: WebviewWindow,
    state: Progress<'_>,
) -> Result<Snapshot, String> {
    authorized(window.label())?;
    state.lock()?.dismiss();
    state.notify.notify_one();
    state.snapshot()
}

#[tauri::command]
pub(crate) async fn progress_window_restore(
    window: WebviewWindow,
    state: Progress<'_>,
    manager: State<'_, SharedManager>,
) -> Result<Snapshot, String> {
    authorized(window.label())?;
    if state.snapshot()?.loading {
        let settings = manager.settings().await;
        let tasks = manager.list().await.map_err(|error| {
            format!(
                "无法读取下载任务：{}",
                crate::manager::redact_sensitive(&error)
            )
        })?;
        state.lock()?.initialize(tasks, &settings);
    }
    {
        let mut cache = state.lock()?;
        cache.dismissed = None;
        cache.revision = cache.revision.saturating_add(1);
    }
    state.notify.notify_one();
    state.snapshot()
}

#[tauri::command]
pub(crate) async fn progress_window_task_action(
    window: WebviewWindow,
    state: Progress<'_>,
    manager: State<'_, SharedManager>,
    task_id: String,
    action: String,
) -> Result<Snapshot, String> {
    authorized(window.label())?;
    if action != "pause" {
        return Err("请在主窗口继续已暂停的任务；悬浮窗仅支持暂停进行中的下载".into());
    }
    let _operation = state.operation.lock().await;
    let task = manager
        .store
        .get_task(&task_id)
        .await?
        .ok_or_else(|| "任务已被移除，请刷新主窗口".to_string())?;
    let can_pause =
        super::state::CachedTask::from_task(&task).is_some_and(|task| task.view.can_pause);
    if !can_pause {
        return Err("任务状态已改变，当前不能暂停，请在主窗口查看".into());
    }
    manager.action(&task_id, "pause").await?;
    if let Some(task) = manager.store.get_task(&task_id).await? {
        state.lock()?.update(&task);
    } else {
        state.lock()?.update_id(task_id, None);
    }
    state.notify.notify_one();
    state.snapshot()
}

#[tauri::command]
pub(crate) async fn progress_window_show_main(
    window: WebviewWindow,
    app: AppHandle,
    state: Progress<'_>,
    manager: State<'_, SharedManager>,
    task_id: Option<String>,
) -> Result<Snapshot, String> {
    authorized(window.label())?;
    if let Some(id) = task_id.as_deref() {
        if manager.store.get_task(id).await?.is_none() {
            return Err("任务已被移除，请打开任务列表查看".into());
        }
    }
    let main = app
        .get_webview_window("main")
        .ok_or_else(|| "主窗口尚未准备好，请稍后重试".to_string())?;
    crate::bring_window_to_front(&main);
    if let Some(id) = task_id {
        app.emit_to("main", "notification-focus-task", id)
            .map_err(|error| format!("主窗口已打开，但未能定位任务：{error}"))?;
    }
    state.snapshot()
}
