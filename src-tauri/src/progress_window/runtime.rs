use super::{creation, native, state::ProgressWindowState};
use crate::{
    manager::SharedManager,
    models::{AppSettings, TaskProgressEvent},
};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Listener, Manager};

/// Register after SharedManager has been managed. Initial DB snapshot is read once;
/// all later progress is applied incrementally from the existing task events.
pub(crate) fn register(app: &AppHandle, manager: SharedManager) {
    let state = ProgressWindowState::new();
    if !app.manage(state.clone()) {
        return;
    }
    for event in ["task-created", "task-updated"] {
        let state = state.clone();
        app.listen_any(event, move |event| {
            match serde_json::from_str::<TaskProgressEvent>(event.payload()) {
                Ok(payload) => {
                    if let Ok(mut cache) = state.lock() {
                        cache.update(&payload.task);
                    }
                }
                Err(_) => tracing::warn!("未能读取下载进度事件，保留上次有效状态"),
            }
            state.notify.notify_one();
        });
    }
    let removed = state.clone();
    app.listen_any("task-removed", move |event| {
        if let Ok(id) = serde_json::from_str::<String>(event.payload()) {
            if let Ok(mut cache) = removed.lock() {
                cache.update_id(id, None);
            }
            removed.notify.notify_one();
        }
    });
    let settings_state = state.clone();
    app.listen_any("settings-changed", move |event| {
        if let Ok(settings) = serde_json::from_str::<AppSettings>(event.payload()) {
            if let Ok(mut cache) = settings_state.lock() {
                cache.preferences(&settings);
            }
            settings_state.notify.notify_one();
        }
    });
    let initial_state = state.clone();
    let initial_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let settings = manager.settings().await;
        match manager.list().await {
            Ok(tasks) => {
                if let Ok(mut cache) = initial_state.lock() {
                    cache.initialize(tasks, &settings);
                }
            }
            Err(error) => report(
                &initial_app,
                &initial_state,
                format!("无法读取下载任务，请在主窗口重试：{error}"),
            ),
        }
        initial_state.notify.notify_one();
    });
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            state.notify.notified().await;
            // One event-driven, coalesced update. No permanent polling timer and
            // no per-progress-event scan of the user's SQLite task history.
            tokio::time::sleep(Duration::from_millis(250)).await;
            if let Err(error) = creation::prepare(&app, &state).await {
                report(&app, &state, error);
                continue;
            }
            let ui_app = app.clone();
            let ui_state = state.clone();
            if let Err(error) =
                app.run_on_main_thread(move || match native::apply(&ui_app, &ui_state) {
                    Err(error) => report(&ui_app, &ui_state, error),
                    Ok(()) => {
                        if let Ok(mut cache) = ui_state.lock() {
                            if !cache.snapshot().loading && cache.error.take().is_some() {
                                cache.revision = cache.revision.saturating_add(1);
                                ui_state.notify.notify_one();
                            }
                        }
                    }
                })
            {
                report(&app, &state, format!("无法更新下载进度窗口：{error}"));
            }
        }
    });
}

fn report(app: &AppHandle, state: &ProgressWindowState, error: String) {
    let safe = crate::manager::redact_sensitive(&error);
    let changed = if let Ok(mut cache) = state.lock() {
        let changed = cache.error.as_deref() != Some(&safe);
        cache.error = Some(safe.clone());
        cache.revision = cache.revision.saturating_add(1);
        changed
    } else {
        true
    };
    if changed {
        tracing::warn!(reason=%safe, "下载进度悬浮窗暂不可用");
        let _ = app.emit_to("main", "progress-window-error", safe);
    }
}
