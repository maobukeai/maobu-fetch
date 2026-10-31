//! 显式删除必须报告失败；路径核验后仅清理该任务的恢复数据。
use super::*;
use std::io::ErrorKind;

pub(crate) async fn remove_task_temp_dir_safely(
    destination: &str,
    task_id: &str,
) -> Result<(), String> {
    if !is_safe_path_component(task_id) {
        return Err("临时任务标识包含无效路径，未删除任何文件".into());
    }
    let destination = fs::canonicalize(destination)
        .await
        .map_err(|e| format!("读取下载目录失败：{e}"))?;
    let root = destination.join("_maobu_tmp");
    let task_dir = root.join(task_id);
    for path in [&root, &task_dir] {
        match fs::symlink_metadata(path).await {
            Ok(metadata) if redirected(&metadata) || !metadata.is_dir() => {
                return Err("临时目录已变为链接或不是目录，未继续删除".into());
            }
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("读取临时目录属性失败：{e}")),
        }
    }
    let canonical_root = fs::canonicalize(&root).await.map_err(|e| e.to_string())?;
    let canonical_dir = fs::canonicalize(&task_dir)
        .await
        .map_err(|e| e.to_string())?;
    if canonical_root.parent() != Some(destination.as_path())
        || canonical_dir.parent() != Some(canonical_root.as_path())
    {
        return Err("临时目录超出任务保存范围，未删除任何文件".into());
    }
    fs::remove_dir_all(&canonical_dir)
        .await
        .map_err(|e| format!("删除任务临时目录失败，请关闭占用程序后重试：{e}"))?;
    match fs::remove_dir(&canonical_root).await {
        Ok(()) => {}
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty) => {}
        Err(e) => return Err(format!("清理空临时目录失败：{e}")),
    }
    Ok(())
}

fn redirected(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

pub(crate) async fn clear_task_parts(task: &DownloadTask, store: &Store) -> Result<(), String> {
    match fs::symlink_metadata(&task.destination).await {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("检查任务保存目录失败：{error}")),
        Ok(_) => {}
    }
    let source = temp_layout::ensure_owned(task, store).await?;
    // Capture the exact adopted layout before removing its private ownership marker.
    let legacy = if source.parent() == Some(Path::new(&task.destination)) {
        temp_layout::legacy_files_at(&source).await?
    } else {
        Vec::new()
    };
    for path in legacy {
        file_integrity::remove_optional_file(&path).await?;
    }
    remove_task_temp_dir_safely(&task.destination, &task.id).await
}

impl DownloadManager {
    pub(super) async fn clear_parts(&self, task: &DownloadTask) -> Result<(), String> {
        clear_task_parts(task, &self.store).await
    }
    pub(super) async fn cleanup_published_parts(&self, task: &mut DownloadTask) {
        let cleanup = async {
            crate::media::cleanup_task_workspace(task).await?;
            clear_task_parts(task, &self.store).await
        }
        .await;
        if let Err(error) = cleanup {
            task.error = Some(format!("下载已完成，但临时数据清理失败：{error}"));
            tracing::warn!(task_id = %task.id, error = %redact_sensitive(&error), "完成文件已保存，恢复数据尚未清理");
        }
    }
}
