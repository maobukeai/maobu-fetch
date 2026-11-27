use super::{commands, register, state::DesktopCompletionState, LABEL};
use crate::{
    manager::DownloadManager,
    models::{AppSettings, NewTaskRequest},
    store::Store,
};
use std::{sync::Arc, time::Duration};
use tauri::{AppHandle, Listener, Manager, RunEvent};

#[test]
#[ignore = "Windows 实机：启动本地 Vite 后，依次操作三条桌面完成提醒"]
fn desktop_completion_interactive_smoke() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_owned();
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = format!(
        "app.maobu.completion-test.{}",
        uuid::Uuid::new_v4().simple()
    );
    context.config_mut().app.windows[0].title = "猫步完成提醒实机验收".into();
    context.config_mut().app.windows[0].url =
        tauri::WebviewUrl::External("about:blank".parse().unwrap());
    let app = tauri::Builder::default()
        .any_thread()
        .invoke_handler(tauri::generate_handler![
            commands::desktop_completion_snapshot,
            commands::desktop_completion_present,
            commands::desktop_completion_act,
        ])
        .setup(|app| {
            register(app.handle());
            Ok(())
        })
        .build(context)
        .expect("构建隔离验收窗口");
    let code = app.run_return(move |app, event| {
        if !matches!(event, RunEvent::Ready) {
            return;
        }
        let app = app.clone();
        let root = root.clone();
        tauri::async_runtime::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(210), run(&app, root)).await;
            match result {
                Ok(Ok(())) => app.exit(0),
                other => {
                    eprintln!("桌面完成提醒验收未完成：{other:?}");
                    app.exit(1);
                }
            }
        });
    });
    assert_eq!(code, 0);
}

async fn run(app: &AppHandle, root: std::path::PathBuf) -> Result<(), String> {
    let main = app.get_webview_window("main").ok_or("测试主窗口未创建")?;
    main.minimize().map_err(|error| error.to_string())?;
    let data = root.join("data");
    let downloads = root.join("downloads");
    tokio::fs::create_dir_all(&downloads)
        .await
        .map_err(|error| error.to_string())?;
    let store = Arc::new(
        tokio::task::spawn_blocking(move || Store::open(data))
            .await
            .map_err(|error| error.to_string())??,
    );
    let settings = AppSettings {
        download_dir: downloads.to_string_lossy().into_owned(),
        completion_notice_seconds: 0,
        concurrent_downloads: 1,
        proxy_mode: "none".into(),
        notify_sound_enabled: false,
        ..Default::default()
    };
    store.save_settings(&settings).await?;
    let manager = DownloadManager::new(store, app.clone()).await?;
    app.manage(manager.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| error.to_string())?;
    let origin = format!(
        "http://{}",
        listener.local_addr().map_err(|error| error.to_string())?
    );
    let server = tokio::spawn(async move {
        let router = axum::Router::new().route(
            "/{name}",
            axum::routing::get(|| async { "Maobu Fetch local completion test" }),
        );
        axum::serve(listener, router).await
    });
    let mut ids = Vec::new();
    let state = app.state::<Arc<DesktopCompletionState>>();
    for (step, action) in ["打开文件", "打开文件夹", "查看详情"]
        .into_iter()
        .enumerate()
    {
        let name = format!("{}-{}.txt", step + 1, action);
        let request: NewTaskRequest = serde_json::from_value(serde_json::json!({
            "url": format!("{origin}/{}.txt", step + 1), "file_name": name,
            "user_edited_file_name": true, "connection_count": 1,
        }))
        .map_err(|error| error.to_string())?;
        ids.push(manager.add(request).await?.id);
        wait_for_popup(app, state.inner(), step).await?;
    }
    if !main.is_minimized().map_err(|error| error.to_string())? {
        return Err("提醒弹出时主窗口没有保持最小化".into());
    }
    eprintln!("三条本地下载提醒就绪，请依次点击打开文件、打开文件夹和查看详情");
    let (done, focused) = tokio::sync::oneshot::channel();
    let done = std::sync::Mutex::new(Some(done));
    let last = ids.pop().ok_or("未创建测试任务")?;
    app.listen_any("notification-focus-task", move |event| {
        if serde_json::from_str::<String>(event.payload()).is_ok_and(|id| id == last) {
            if let Ok(mut done) = done.lock() {
                if let Some(done) = done.take() {
                    let _ = done.send(());
                }
            }
        }
    });
    focused.await.map_err(|_| "未收到查看详情事件".to_owned())?;
    server.abort();
    manager.shutdown_bt().await?;
    Ok(())
}

async fn wait_for_popup(
    app: &AppHandle,
    state: &DesktopCompletionState,
    pending: usize,
) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            if state.snapshot()?.pending_count == pending {
                if let Some(window) = app.get_webview_window(LABEL) {
                    #[cfg(windows)]
                    if crate::progress_window::os::visible_bounds(&window)?.is_some() {
                        return Ok(());
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(75)).await;
        }
    })
    .await
    .map_err(|_| "等待本地下载完成并显示桌面提醒超时".to_string())?
}
