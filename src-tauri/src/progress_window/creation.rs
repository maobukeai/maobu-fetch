//! WebView2 creation must run outside the Windows event thread (Tauri 2.11.5).
use super::{state::ProgressWindowState, LABEL};
use std::sync::Arc;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

/// Called only by the single, serial event worker, before dispatching OS updates.
pub(super) async fn prepare(
    app: &AppHandle,
    state: &Arc<ProgressWindowState>,
) -> Result<(), String> {
    let should_show = state.lock()?.should_show();
    if !should_show || app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }
    let app = app.clone();
    let state = state.clone();
    create_on_worker(move || {
        // Recheck after the blocking pool starts: a new UI request may have hidden
        // the batch while this operation was waiting to be scheduled.
        let pinned = {
            let mut cache = state.lock()?;
            if !cache.should_show() || app.get_webview_window(LABEL).is_some() {
                return Ok(());
            }
            cache.pinned
        };
        create_hidden(&app, state, pinned)
    })
    .await
}

async fn create_on_worker(
    create: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    tokio::task::spawn_blocking(create)
        .await
        .map_err(|error| format!("创建下载进度窗口的后台任务未能完成：{error}"))?
}

fn create_hidden(
    app: &AppHandle,
    state: Arc<ProgressWindowState>,
    pinned: bool,
) -> Result<(), String> {
    // Do not move this builder into an event handler or run_on_main_thread.
    // The UI/event thread must remain free to process WebView2 creation messages.
    let window = WebviewWindowBuilder::new(
        app,
        LABEL,
        WebviewUrl::App("index.html?view=download-progress".into()),
    )
    .title("下载进度 · 猫步下载器")
    .inner_size(224.0, 28.0)
    .decorations(false)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .skip_taskbar(true)
    .always_on_top(pinned)
    .visible(false)
    .focused(false)
    .transparent(true)
    .background_color(tauri::window::Color(0, 0, 0, 0))
    .shadow(false)
    .build()
    .map_err(|error| format!("无法创建下载进度悬浮窗：{error}"))?;
    let weak = Arc::downgrade(&state);
    window.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            if let Some(state) = weak.upgrade() {
                match state.lock() {
                    Ok(mut cache) => cache.dismiss(),
                    Err(error) => tracing::warn!(reason=%error, "关闭进度悬浮窗失败"),
                }
                state.notify.notify_one();
            }
        }
        WindowEvent::ScaleFactorChanged { .. } | WindowEvent::Moved(_) => {
            if let Some(state) = weak.upgrade() {
                state.notify.notify_one();
            }
        }
        _ => {}
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::create_on_worker;

    #[tokio::test(flavor = "current_thread")]
    async fn creation_runs_off_the_calling_thread_and_preserves_builder_failure() {
        let caller = std::thread::current().id();
        create_on_worker(move || {
            assert_ne!(std::thread::current().id(), caller);
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(
            create_on_worker(|| Err("synthetic WebView2 creation failure".into())).await,
            Err("synthetic WebView2 creation failure".into())
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failed_creation_worker_returns_an_error_without_panicking_the_caller() {
        let result = create_on_worker(|| panic!("synthetic worker failure")).await;
        assert!(result.unwrap_err().contains("后台任务未能完成"));
    }
}
