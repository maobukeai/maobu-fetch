use super::{state::DesktopCompletionState, window, LABEL};
use crate::models::{AppSettings, TaskStatus};
use serde::Deserialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Listener, Manager};

#[derive(Deserialize)]
struct TaskUpdate {
    task: TaskReference,
}
#[derive(Deserialize)]
struct TaskReference {
    id: String,
    status: TaskStatus,
}

/// 必须在创建 DownloadManager（启动调度器）之前注册，不读取用户历史任务。
pub(crate) fn register(app: &AppHandle) {
    let state = DesktopCompletionState::new();
    if !app.manage(state.clone()) {
        return;
    }
    let settings_state = state.clone();
    app.listen_any("settings-changed", move |event| {
        if let Ok(settings) = serde_json::from_str::<AppSettings>(event.payload()) {
            if let Ok(mut cache) = settings_state.lock() {
                cache.preferences(&settings);
            }
            settings_state.notify.notify_one();
        }
    });
    let removed = state.clone();
    app.listen_any("task-removed", move |event| {
        if let Ok(id) = serde_json::from_str::<String>(event.payload()) {
            if let Ok(mut cache) = removed.lock() {
                cache.invalidate(&id);
            }
            removed.notify.notify_one();
        }
    });
    let updated = state.clone();
    app.listen_any("task-updated", move |event| {
        // 不订阅逐字节进度；无提醒历史时直接跳过，状态事件也仅提取 id/status。
        if !updated.lock().is_ok_and(|cache| cache.has_recent_tasks()) {
            return;
        }
        if let Ok(payload) = serde_json::from_str::<TaskUpdate>(event.payload()) {
            if payload.task.status != TaskStatus::Completed {
                if updated
                    .lock()
                    .is_ok_and(|mut cache| cache.invalidate(&payload.task.id))
                {
                    updated.notify.notify_one();
                }
            }
        }
    });
    let progress = state.clone();
    app.listen_any("download-progress-updated", move |_| {
        progress.notify.notify_one();
    });
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            state.notify.notified().await;
            let _operation = state.operation.lock().await;
            if let Err(error) = window::prepare(&app, &state).await {
                report(&app, &error);
                continue;
            }
            if let Err(error) = window::update(&app, &state).await {
                report(&app, &error);
            }
        }
    });
}

pub(crate) fn enqueue(
    app: &AppHandle,
    task_id: String,
    title: String,
    body: String,
    settings: &AppSettings,
) -> Result<(), String> {
    let state = app
        .try_state::<Arc<DesktopCompletionState>>()
        .ok_or("桌面完成提醒尚未初始化，请重启应用后重试")?;
    if state.lock()?.enqueue(task_id, title, body, settings)? {
        state.notify.notify_one();
    }
    Ok(())
}

pub(super) fn report(app: &AppHandle, error: &str) {
    let message = crate::manager::redact_sensitive(error);
    tracing::warn!(reason = %message, "桌面完成提醒暂不可用");
    let _ = app.emit_to(
        "main",
        "notification-error",
        format!("桌面完成提醒暂不可用，请在主窗口查看任务：{message}"),
    );
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.emit("desktop-completion-error", message);
    }
}
