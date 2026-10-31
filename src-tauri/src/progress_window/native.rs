use super::{layout, os, state::ProgressWindowState, LABEL, UPDATED};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

pub(super) fn apply(app: &AppHandle, state: &Arc<ProgressWindowState>) -> Result<(), String> {
    let (show, snapshot, pin_inflight) = {
        let mut cache = state.lock()?;
        (cache.should_show(), cache.snapshot(), cache.pin_inflight)
    };
    if !show {
        if let Some(window) = app.get_webview_window(LABEL) {
            os::show(&window, false)?;
            window
                .emit(UPDATED, snapshot)
                .map_err(|error| format!("无法更新下载进度：{error}"))?;
        }
        return Ok(());
    }
    let window = match app.get_webview_window(LABEL) {
        Some(window) => window,
        None => {
            // A task can arrive between the async preparation and this UI callback.
            // Queue another preparation instead of building on the Windows event thread.
            state.notify.notify_one();
            return Ok(());
        }
    };
    if !pin_inflight {
        os::pin(&window, snapshot.always_on_top)?;
    }
    position(
        app,
        &window,
        snapshot.total_count,
        snapshot.expanded,
        snapshot.revealed,
    )?;
    window
        .emit(UPDATED, snapshot)
        .map_err(|error| format!("无法更新下载进度：{error}"))?;
    os::show(&window, true)?;
    Ok(())
}

fn position(
    app: &AppHandle,
    window: &WebviewWindow,
    count: usize,
    expanded: bool,
    revealed: bool,
) -> Result<(), String> {
    let monitor = window
        .current_monitor()
        .map_err(|error| format!("无法读取进度窗口屏幕：{error}"))?
        .or_else(|| {
            app.get_webview_window("main")
                .and_then(|main| main.current_monitor().ok().flatten())
        })
        .or_else(|| app.primary_monitor().ok().flatten())
        .ok_or_else(|| "无法取得可用屏幕工作区，请在主窗口查看下载进度".to_string())?;
    let work = monitor.work_area();
    let bounds = layout::placement(
        work.position.x,
        work.position.y,
        work.size.width,
        work.size.height,
        monitor.scale_factor(),
        count,
        expanded,
        revealed,
    );
    os::place(window, &bounds)
}

pub(super) async fn set_pinned_on_main(app: &AppHandle, pinned: bool) -> Result<(), String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let target = app.clone();
    app.run_on_main_thread(move || {
        let result = target
            .get_webview_window(LABEL)
            .map_or(Ok(()), |window| os::pin(&window, pinned));
        let _ = sender.send(result);
    })
    .map_err(|error| format!("无法设置下载进度窗口置顶状态：{error}"))?;
    receiver
        .await
        .map_err(|_| "进度窗口已关闭，无法完成置顶操作".to_string())?
}
