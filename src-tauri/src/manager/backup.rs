use crate::{
    manager::category_rules::normalize_directory,
    models::{AppSettings, CompletionAction, DownloadTask, RestorePreview, TaskStatus},
    store::Store,
    task_transfer::{build_bundle, compute_preview, export_bundle, read_backup_manifest, read_bundle, CurrentState},
};
use std::collections::HashSet;

/// 强制净化待恢复的任务。
pub fn sanitize_restored_task(task: &mut DownloadTask) {
    if matches!(task.status, TaskStatus::Queued | TaskStatus::Downloading) {
        task.status = TaskStatus::Paused;
    }
    if !matches!(
        task.completion_action,
        CompletionAction::None | CompletionAction::OpenFolder
    ) {
        task.completion_action = CompletionAction::None;
    }
    task.destination = normalize_directory(&task.destination);
    task.active_connections = 0;
    task.speed = 0;
    task.eta_seconds = None;
}

pub async fn build_and_export_backup(
    store: &Store,
    settings: AppSettings,
    path: &str,
    include_auth: bool,
    password: Option<&str>,
) -> Result<(), String> {
    let tasks = store.list_tasks().await?;
    let category_rules = store.category_rule_list().await?;
    let filename_cleanup_rules = store.filename_cleanup_rule_list().await?;
    let download_presets = store.download_preset_list().await?;
    let url_history = store.url_history_list().await?;
    let saved_views = store.saved_view_list().await?;
    let bundle = build_bundle(
        settings,
        tasks,
        category_rules,
        filename_cleanup_rules,
        download_presets,
        url_history,
        saved_views,
        env!("CARGO_PKG_VERSION"),
        include_auth,
    );
    export_bundle(path, &bundle, password).await
}

pub async fn compute_backup_preview(
    store: &Store,
    settings: &AppSettings,
    path: &str,
    password: Option<&str>,
) -> Result<RestorePreview, String> {
    let manifest = read_backup_manifest(path).await?;
    let bundle = read_bundle(path, password).await?;
    let category_rules = store.category_rule_list().await?;
    let filename_cleanup_rules = store.filename_cleanup_rule_list().await?;
    let download_presets = store.download_preset_list().await?;
    let url_history = store.url_history_list().await?;
    let tasks = store.list_tasks().await?;
    let task_ids: HashSet<String> = tasks.into_iter().map(|t| t.id).collect();
    let saved_view_ids: HashSet<String> = store
        .saved_view_list()
        .await?
        .into_iter()
        .map(|view| view.id)
        .collect();
    let current = CurrentState {
        settings,
        category_rules: &category_rules,
        filename_cleanup_rules: &filename_cleanup_rules,
        download_presets: &download_presets,
        url_history: &url_history,
        saved_view_ids: &saved_view_ids,
        task_ids: &task_ids,
    };
    let mut preview = compute_preview(&bundle, &current);
    preview.encrypted = manifest.encrypted;
    Ok(preview)
}
