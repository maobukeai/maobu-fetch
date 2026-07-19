mod bridge;
mod manager;
mod media;
mod media_tools;
mod models;
mod store;

use bridge::PairingService;
use manager::{DownloadManager, SharedManager};
use media_tools::MediaTools;
use models::{
    AppSettings, BatchTaskRequest, DetectedMediaTools, DownloadTask, MediaProbeResult,
    NewTaskRequest, PairingInfo, ToolComponent, ToolStatus,
};
use std::{path::PathBuf, sync::Arc};
use store::Store;
use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, State};

struct TrayMenuItems {
    clipboard: CheckMenuItem<tauri::Wry>,
    low_memory: CheckMenuItem<tauri::Wry>,
    frosted_glass: CheckMenuItem<tauri::Wry>,
}

#[tauri::command]
async fn tasks_list(manager: State<'_, SharedManager>) -> Result<Vec<DownloadTask>, String> {
    manager.list().await
}

#[tauri::command]
async fn task_add(
    request: NewTaskRequest,
    manager: State<'_, SharedManager>,
) -> Result<DownloadTask, String> {
    manager.inner().add(request).await
}

#[tauri::command]
async fn tasks_add_batch(
    request: BatchTaskRequest,
    manager: State<'_, SharedManager>,
) -> Result<Vec<DownloadTask>, String> {
    manager.inner().add_batch(request).await
}

#[tauri::command]
async fn task_action(
    id: String,
    action: String,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    manager.inner().action(&id, &action).await
}

#[tauri::command]
async fn tasks_bulk_action(
    ids: Vec<String>,
    action: String,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    manager.inner().bulk_action(&ids, &action).await
}

#[tauri::command]
async fn task_remove(
    id: String,
    delete_file: bool,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    manager.inner().remove(&id, delete_file).await
}

#[tauri::command]
async fn queue_reorder(ids: Vec<String>, manager: State<'_, SharedManager>) -> Result<(), String> {
    manager.reorder(&ids).await
}

#[tauri::command]
async fn settings_get(manager: State<'_, SharedManager>) -> Result<AppSettings, String> {
    Ok(manager.settings().await)
}

#[tauri::command]
async fn settings_save(
    settings: AppSettings,
    manager: State<'_, SharedManager>,
    tray_items: State<'_, TrayMenuItems>,
) -> Result<(), String> {
    manager.save_settings(settings.clone()).await?;
    let _ = tray_items.clipboard.set_checked(settings.clipboard_monitor);
    let _ = tray_items.low_memory.set_checked(settings.low_memory_mode);
    let _ = tray_items.frosted_glass.set_checked(settings.frosted_glass);
    Ok(())
}

#[tauri::command]
async fn task_verify(id: String, manager: State<'_, SharedManager>) -> Result<String, String> {
    manager.verify_checksum(&id).await
}

#[tauri::command]
async fn task_open_file(id: String, manager: State<'_, SharedManager>) -> Result<(), String> {
    let task = manager.store.get_task(&id).await?.ok_or("任务不存在")?;
    open::that(PathBuf::from(task.destination).join(task.file_name)).map_err(|e| e.to_string())
}

#[tauri::command]
async fn task_open_folder(id: String, manager: State<'_, SharedManager>) -> Result<(), String> {
    let task = manager.store.get_task(&id).await?.ok_or("任务不存在")?;
    open::that(task.destination).map_err(|e| e.to_string())
}

#[tauri::command]
async fn history_clear(
    include_completed: bool,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    manager.store.clear_history(include_completed).await
}

#[tauri::command]
async fn pairing_info(pairing: State<'_, PairingService>) -> Result<PairingInfo, String> {
    pairing.info().await
}

#[tauri::command]
async fn pairing_rotate(pairing: State<'_, PairingService>) -> Result<PairingInfo, String> {
    Ok(pairing.rotate().await)
}

#[tauri::command]
async fn pairing_revoke(manager: State<'_, SharedManager>) -> Result<(), String> {
    manager.store.clear_pairing().await
}

#[tauri::command]
async fn media_probe(
    url: String,
    app: tauri::AppHandle,
    manager: State<'_, SharedManager>,
) -> Result<MediaProbeResult, String> {
    media::probe(&app, &manager.settings().await, &url).await
}

#[tauri::command]
async fn media_tool_status(
    app: tauri::AppHandle,
    tools: State<'_, MediaTools>,
    manager: State<'_, SharedManager>,
) -> Result<ToolStatus, String> {
    Ok(tools.status(&app, &manager.settings().await).await)
}

#[tauri::command]
fn media_tools_detect_system() -> DetectedMediaTools {
    media_tools::detect_system_tools()
}

#[tauri::command]
async fn media_tools_install(
    app: tauri::AppHandle,
    tools: State<'_, MediaTools>,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    let settings = manager.settings().await;
    let status = tools.status(&app, &settings).await;
    let component = if !status.yt_dlp_available {
        ToolComponent::YtDlp
    } else {
        ToolComponent::Ffmpeg
    };
    tools.start_install(app, settings, component).await
}

#[tauri::command]
async fn media_tool_install(
    component: ToolComponent,
    app: tauri::AppHandle,
    tools: State<'_, MediaTools>,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    tools
        .start_install(app, manager.settings().await, component)
        .await
}

#[tauri::command]
async fn media_tools_cancel(tools: State<'_, MediaTools>) -> Result<(), String> {
    tools.cancel().await;
    Ok(())
}

#[tauri::command]
async fn media_tools_remove(
    app: tauri::AppHandle,
    tools: State<'_, MediaTools>,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    let settings = manager.settings().await;
    tools
        .uninstall(&app, &settings, ToolComponent::Ffmpeg)
        .await?;
    tools.uninstall(&app, &settings, ToolComponent::YtDlp).await
}

#[tauri::command]
async fn media_tool_remove(
    component: ToolComponent,
    app: tauri::AppHandle,
    tools: State<'_, MediaTools>,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    tools
        .uninstall(&app, &manager.settings().await, component)
        .await
}

#[tauri::command]
async fn media_tools_check_update(
    app: tauri::AppHandle,
    tools: State<'_, MediaTools>,
    manager: State<'_, SharedManager>,
) -> Result<ToolStatus, String> {
    Ok(tools.status(&app, &manager.settings().await).await)
}

#[tauri::command]
fn app_exit(app: tauri::AppHandle) {
    app.exit(0);
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let data_dir = std::env::var_os("MAOBU_FETCH_DATA_DIR")
                .or_else(|| std::env::var_os("LUMAGET_DATA_DIR"))
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    app.path()
                        .app_data_dir()
                        .unwrap_or_else(|_| PathBuf::from("."))
                });
            let store = Arc::new(Store::open(data_dir)?);
            let manager =
                tauri::async_runtime::block_on(DownloadManager::new(store, app.handle().clone()))?;
            let pairing = PairingService::new(manager.clone());
            let media_tools = MediaTools::new(
                app.handle(),
                &tauri::async_runtime::block_on(manager.settings()),
            );
            app.manage(manager.clone());
            app.manage(pairing.clone());
            app.manage(media_tools);
            let bridge_app = app.handle().clone();
            tauri::async_runtime::spawn(bridge::run(manager.clone(), pairing, bridge_app));

            if let Some(icon) = app.default_window_icon() {
                let show_item = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
                let initial_settings = tauri::async_runtime::block_on(manager.settings());
                let clip_item = CheckMenuItem::with_id(
                    app,
                    "clipboard",
                    "监视剪贴板",
                    true,
                    initial_settings.clipboard_monitor,
                    None::<&str>,
                )?;
                let low_memory_item = CheckMenuItem::with_id(
                    app,
                    "low-memory",
                    "低内存模式",
                    true,
                    initial_settings.low_memory_mode,
                    None::<&str>,
                )?;
                let frosted_glass_item = CheckMenuItem::with_id(
                    app,
                    "frosted-glass",
                    "磨砂玻璃",
                    true,
                    initial_settings.frosted_glass,
                    None::<&str>,
                )?;
                app.manage(TrayMenuItems {
                    clipboard: clip_item.clone(),
                    low_memory: low_memory_item.clone(),
                    frosted_glass: frosted_glass_item.clone(),
                });

                let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
                let menu = Menu::with_items(
                    app,
                    &[
                        &show_item,
                        &clip_item,
                        &low_memory_item,
                        &frosted_glass_item,
                        &quit_item,
                    ],
                )?;

                let _tray = TrayIconBuilder::new()
                    .icon(icon.clone())
                    .menu(&menu)
                    .on_menu_event(|app, event| {
                        if event.id.as_ref() == "show" {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.unminimize();
                                let _ = window.set_focus();
                            }
                        } else if event.id.as_ref() == "clipboard" {
                            let tray_items = app.state::<TrayMenuItems>();
                            if let Ok(checked) = tray_items.clipboard.is_checked() {
                                let manager = app.state::<SharedManager>();
                                let mut settings =
                                    tauri::async_runtime::block_on(manager.settings());
                                settings.clipboard_monitor = checked;
                                let _ = tauri::async_runtime::block_on(
                                    manager.save_settings(settings.clone()),
                                );
                                let _ = app.emit("settings-changed", settings);
                            }
                        } else if event.id.as_ref() == "low-memory" {
                            let tray_items = app.state::<TrayMenuItems>();
                            if let Ok(checked) = tray_items.low_memory.is_checked() {
                                let manager = app.state::<SharedManager>();
                                let mut settings =
                                    tauri::async_runtime::block_on(manager.settings());
                                settings.low_memory_mode = checked;
                                let saved = tauri::async_runtime::block_on(
                                    manager.save_settings(settings.clone()),
                                );
                                if saved.is_ok() {
                                    let _ = app.emit("settings-changed", settings);
                                } else {
                                    let _ = tray_items.low_memory.set_checked(!checked);
                                }
                            }
                        } else if event.id.as_ref() == "frosted-glass" {
                            let tray_items = app.state::<TrayMenuItems>();
                            if let Ok(checked) = tray_items.frosted_glass.is_checked() {
                                let manager = app.state::<SharedManager>();
                                let mut settings =
                                    tauri::async_runtime::block_on(manager.settings());
                                settings.frosted_glass = checked;
                                let saved = tauri::async_runtime::block_on(
                                    manager.save_settings(settings.clone()),
                                );
                                if saved.is_ok() {
                                    let _ = app.emit("settings-changed", settings);
                                } else {
                                    let _ = tray_items.frosted_glass.set_checked(!checked);
                                }
                            }
                        } else if event.id.as_ref() == "quit" {
                            app.exit(0);
                        }
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            let app = tray.app_handle();
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    })
                    .build(app)?;
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            tasks_list,
            task_add,
            tasks_add_batch,
            task_action,
            tasks_bulk_action,
            task_remove,
            queue_reorder,
            settings_get,
            settings_save,
            task_verify,
            task_open_file,
            task_open_folder,
            history_clear,
            pairing_info,
            pairing_rotate,
            pairing_revoke,
            media_probe,
            media_tool_status,
            media_tools_detect_system,
            media_tools_install,
            media_tool_install,
            media_tools_cancel,
            media_tools_remove,
            media_tool_remove,
            media_tools_check_update,
            app_exit
        ])
        .run(tauri::generate_context!())
        .expect("error while running Maobu Fetch");
}
