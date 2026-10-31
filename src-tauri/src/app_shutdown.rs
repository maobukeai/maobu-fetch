use crate::manager::{redact_sensitive, SharedManager};
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use tauri::{AppHandle, Emitter, Manager, RunEvent};

pub(crate) fn handler() -> impl FnMut(&AppHandle, RunEvent) {
    let started = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(AtomicBool::new(false));
    move |app, event| {
        let RunEvent::ExitRequested { api, code, .. } = event else { return; };
        if ready.load(Ordering::Acquire) { return; }
        let Some(manager) = app.try_state::<SharedManager>() else { return; };
        api.prevent_exit();
        if started.swap(true, Ordering::AcqRel) { return; }
        tauri::async_runtime::spawn(shutdown(
            app.clone(), manager.inner().clone(), started.clone(), ready.clone(), code.unwrap_or(0),
        ));
    }
}

async fn shutdown(app: AppHandle, manager: SharedManager,
    started: Arc<AtomicBool>, ready: Arc<AtomicBool>, code: i32) {
    match manager.shutdown_bt().await {
        Ok(()) => {
            ready.store(true, Ordering::Release);
            app.exit(code);
        }
        Err(error) => {
            started.store(false, Ordering::Release);
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
            }
            let _ = app.emit("shutdown-error", format!("退出前保存下载状态失败，程序保持运行：{}", redact_sensitive(&error)));
        }
    }
}
