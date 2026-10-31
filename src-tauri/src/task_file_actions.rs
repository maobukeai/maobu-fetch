//! 文件动作只指向当前任务的真实归属，不扫描全局下载目录猜测目标。
use crate::{manager::{is_safe_path_component, SharedManager}, models::{DownloadTask, TaskKind}};
use std::path::{Path, PathBuf};

pub(crate) async fn resolved_task(manager: &SharedManager, id: &str) -> Result<DownloadTask, String> {
    let task = manager.store.get_task(id).await?.ok_or("任务不存在")?;
    if task.task_kind != TaskKind::Bt { return Ok(task); }
    let has_paths = task.bt_meta.as_ref().is_some_and(|meta|
        meta.output_folder.is_some() && !meta.owned_files.is_empty());
    if has_paths { return Ok(task); }
    let resolved = manager.bt.resolve_task_paths(&task).await?;
    manager.store.update_bt_ownership(&resolved).await?.ok_or_else(|| "任务已被删除".to_owned())
}

pub(crate) async fn content_path(task: &DownloadTask, relative: Option<&str>) -> Result<PathBuf, String> {
    if task.task_kind == TaskKind::Bt { return crate::bt::task_content_path(task, relative).await; }
    if !is_safe_path_component(&task.file_name) { return Err("任务文件名无效，请检查保存位置".into()); }
    if let Some(requested) = relative.filter(|value| !value.trim().is_empty()) {
        if requested != task.file_name { return Err("指定文件不属于当前下载任务".into()); }
    }
    let root = tokio::fs::canonicalize(&task.destination).await
        .map_err(|e| format!("下载目录不可访问：{e}"))?;
    let path = tokio::fs::canonicalize(root.join(&task.file_name)).await
        .map_err(|e| format!("任务文件尚未完成、已移动或被删除，请检查下载状态和保存位置：{e}"))?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err("任务文件不在指定保存目录内或不是普通文件".into());
    }
    Ok(path)
}

pub(crate) async fn folder_path(task: &DownloadTask) -> Result<PathBuf, String> {
    if task.task_kind == TaskKind::Bt { return crate::bt::task_output_folder(task).await; }
    let path = tokio::fs::canonicalize(Path::new(&task.destination)).await
        .map_err(|e| format!("下载目录不可访问：{e}"))?;
    if !path.is_dir() { return Err("保存位置不是文件夹".into()); }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn missing_task_file_never_opens_an_unrelated_larger_movie() {
        let temp = tempfile::tempdir().unwrap();
        let mut task = crate::bt::tests::bt_task_for_test();
        task.task_kind = TaskKind::Http;
        task.destination = temp.path().to_string_lossy().into_owned();
        task.file_name = "wanted.mp4".into();
        tokio::fs::write(temp.path().join("another-user-video.mp4"), vec![0; 4096]).await.unwrap();
        assert!(content_path(&task, None).await.is_err());
        tokio::fs::write(temp.path().join("wanted.mp4"), b"actual task").await.unwrap();
        assert_eq!(content_path(&task, None).await.unwrap(), temp.path().join("wanted.mp4").canonicalize().unwrap());
        assert!(content_path(&task, Some("another-user-video.mp4")).await.is_err());
        task.file_name = "../outside.mp4".into();
        assert!(content_path(&task, None).await.is_err());
    }
}
