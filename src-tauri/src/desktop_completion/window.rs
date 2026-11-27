use super::{
    geometry,
    state::{DesktopCompletionState, Snapshot},
    LABEL, UPDATED,
};
use crate::progress_window::{layout::Placement, os};
use std::sync::Arc;
use tauri::{
    AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

pub(super) async fn prepare(
    app: &AppHandle,
    state: &Arc<DesktopCompletionState>,
) -> Result<(), String> {
    if state.snapshot()?.notice.is_none() || app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }
    let app = app.clone();
    let state = state.clone();
    // WebView2 的创建会等待事件线程响应，不得放入 run_on_main_thread 回调。
    tokio::task::spawn_blocking(move || create_hidden(&app, &state))
        .await
        .map_err(|_| "创建桌面完成提醒的后台任务未完成，请重新启动应用".to_string())?
}

fn create_hidden(app: &AppHandle, state: &Arc<DesktopCompletionState>) -> Result<(), String> {
    if state.snapshot()?.notice.is_none() || app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }
    let window = WebviewWindowBuilder::new(
        app,
        LABEL,
        WebviewUrl::App("index.html?view=desktop-completion".into()),
    )
    .title("下载完成 · 猫步下载器")
    .inner_size(352.0, 128.0)
    .decorations(false)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .skip_taskbar(true)
    .always_on_top(true)
    .visible(false)
    .focused(false)
    .transparent(true)
    .background_color(tauri::window::Color(0, 0, 0, 0))
    .shadow(false)
    .build()
    .map_err(|error| format!("无法创建桌面完成提醒，请在主窗口查看任务：{error}"))?;
    let weak = Arc::downgrade(state);
    window.on_window_event(move |event| {
        if let Some(state) = weak.upgrade() {
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    if let Ok(mut cache) = state.lock() {
                        if let Some(head) = cache.snapshot().notice {
                            cache.remove(&head.task_id);
                        }
                    }
                    state.notify.notify_one();
                }
                WindowEvent::ScaleFactorChanged { .. } | WindowEvent::Moved(_) => {
                    state.notify.notify_one();
                }
                _ => {}
            }
        }
    });
    Ok(())
}

pub(super) async fn update(
    app: &AppHandle,
    state: &Arc<DesktopCompletionState>,
) -> Result<(), String> {
    let state = state.clone();
    on_ui(app, move |app| {
        let cache = state.lock()?;
        let snapshot = cache.snapshot();
        if let Some(window) = app.get_webview_window(LABEL) {
            match (&snapshot.notice, cache.presented) {
                (Some(notice), Some((id, height))) if notice.notice_id == id => {
                    position(app, &window, height)?;
                }
                _ => os::show(&window, false)?,
            }
            window
                .emit(UPDATED, &snapshot)
                .map_err(|error| format!("无法更新桌面完成提醒：{error}"))?;
        }
        Ok(())
    })
    .await
}

pub(super) async fn present(
    app: &AppHandle,
    state: &Arc<DesktopCompletionState>,
    task_id: String,
    notice_id: u64,
    height: f64,
) -> Result<Snapshot, String> {
    if !height.is_finite() || !(64.0..=480.0).contains(&height) {
        return Err("完成提醒尺寸无效，请重新打开应用".into());
    }
    let state = state.clone();
    on_ui(app, move |app| {
        let mut cache = state.lock()?;
        cache.check(&task_id, notice_id)?;
        let window = app
            .get_webview_window(LABEL)
            .ok_or("桌面完成提醒窗口尚未准备好，请稍候重试")?;
        position(app, &window, height)?;
        os::pin(&window, true)?;
        os::show(&window, true)?;
        cache.presented = Some((notice_id, height));
        Ok(cache.snapshot())
    })
    .await
}

fn position(app: &AppHandle, window: &WebviewWindow, height: f64) -> Result<(), String> {
    let monitor = app
        .get_webview_window("main")
        .and_then(|main| main.current_monitor().ok().flatten())
        .or_else(|| window.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())
        .ok_or("无法取得屏幕工作区，请在主窗口查看下载记录")?;
    let area = monitor.work_area();
    let work = Placement {
        x: area.position.x,
        y: area.position.y,
        width: area.size.width,
        height: area.size.height,
    };
    let obstacle = app
        .get_webview_window("download-progress")
        .map(|progress| os::visible_bounds(&progress))
        .transpose()?
        .flatten();
    let placement = geometry::placement(&work, monitor.scale_factor(), height, obstacle.as_ref());
    os::place(window, &placement).map_err(|error| format!("无法定位桌面完成提醒：{error}"))
}

async fn on_ui<T: Send + 'static>(
    app: &AppHandle,
    action: impl FnOnce(&AppHandle) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let target = app.clone();
    app.run_on_main_thread(move || {
        let _ = sender.send(action(&target));
    })
    .map_err(|error| format!("无法更新桌面完成提醒窗口：{error}"))?;
    receiver
        .await
        .map_err(|_| "桌面完成提醒窗口已关闭，请重新打开应用".to_string())?
}
