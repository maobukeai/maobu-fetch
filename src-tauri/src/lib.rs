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
    AppSettings, BatchTaskRequest, DownloadTask, MediaProbeResult, NewTaskRequest, PairingInfo,
    ToolStatus,
};
use std::{path::PathBuf, sync::Arc};
use store::Store;
use tauri::{Manager, State};

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
) -> Result<(), String> {
    manager.save_settings(settings).await
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
async fn media_probe(url: String, app: tauri::AppHandle) -> Result<MediaProbeResult, String> {
    media::probe(&app, &url).await
}

#[tauri::command]
async fn media_tool_status(tools: State<'_, MediaTools>) -> Result<ToolStatus, String> {
    Ok(tools.status().await)
}

#[tauri::command]
async fn media_tools_install(
    app: tauri::AppHandle,
    tools: State<'_, MediaTools>,
    manager: State<'_, SharedManager>,
) -> Result<(), String> {
    tools.start_install(app, manager.settings().await).await
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
) -> Result<(), String> {
    tools.uninstall(&app).await
}

#[tauri::command]
async fn media_tools_check_update(tools: State<'_, MediaTools>) -> Result<ToolStatus, String> {
    Ok(tools.status().await)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let data_dir = std::env::var_os("LUMAGET_DATA_DIR")
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
            let media_tools = MediaTools::new(app.handle());
            app.manage(manager.clone());
            app.manage(pairing.clone());
            app.manage(media_tools);
            let bridge_app = app.handle().clone();
            tauri::async_runtime::spawn(bridge::run(manager, pairing, bridge_app));
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
            media_tools_install,
            media_tools_cancel,
            media_tools_remove,
            media_tools_check_update
        ])
        .run(tauri::generate_context!())
        .expect("error while running LumaGet");
}
