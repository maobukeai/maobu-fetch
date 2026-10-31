use super::{output, DownloadTask};
use sha2::{Digest, Sha256};

/// Remove only verified task-owned temporary files, after completion commit or explicit deletion.
pub async fn cleanup_task_workspace(task: &DownloadTask) -> Result<(), String> {
    let root = output::work_directory(task);
    let metadata = match tokio::fs::symlink_metadata(&root).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("无法检查媒体任务临时目录".into()),
    };
    if !metadata.is_dir() || super::path_safety::is_link(&metadata) {
        return Err("媒体任务临时路径不安全，未清理现有文件".into());
    }
    let marker = root.join("owner.json");
    let metadata = tokio::fs::symlink_metadata(&marker)
        .await
        .map_err(|_| "媒体临时目录缺少归属记录，已保留")?;
    if !metadata.is_file() || super::path_safety::is_link(&metadata) || metadata.len() > 4096 {
        return Err("媒体临时目录归属记录无效，已保留".into());
    }
    let value: serde_json::Value = serde_json::from_slice(
        &tokio::fs::read(&marker)
            .await
            .map_err(|_| "读取媒体临时归属记录失败")?,
    )
    .map_err(|_| "媒体临时归属记录损坏，已保留")?;
    if value.get("binding").and_then(serde_json::Value::as_str)
        != Some(&hex::encode(Sha256::digest(task.id.as_bytes())))
    {
        return Err("媒体临时目录不属于该任务，已保留".into());
    }
    let mut entries = tokio::fs::read_dir(&root)
        .await
        .map_err(|_| "读取媒体临时目录失败")?;
    let mut files = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|_| "读取媒体临时文件失败")?
    {
        if files.len() >= 4096 {
            return Err("媒体临时文件数量异常，未执行清理".into());
        }
        let metadata = tokio::fs::symlink_metadata(entry.path())
            .await
            .map_err(|_| "检查媒体临时文件失败")?;
        let name = entry.file_name();
        let name = name.to_str().ok_or("媒体临时文件名异常，已保留")?;
        if !metadata.is_file() || super::path_safety::is_link(&metadata) || !owned_name(name) {
            return Err("媒体临时目录包含非预期文件，未执行清理".into());
        }
        files.push(entry.path());
    }
    // Keep the owner marker until all payload files have gone, so interrupted cleanup is retryable.
    files.sort_by_key(|path| path.file_name().is_some_and(|name| name == "owner.json"));
    for path in files {
        tokio::fs::remove_file(path)
            .await
            .map_err(|_| "清理媒体临时文件失败，请检查权限后重试")?;
    }
    tokio::fs::remove_dir(root)
        .await
        .map_err(|_| "移除媒体空临时目录失败".to_owned())
}

fn owned_name(name: &str) -> bool {
    matches!(name, "owner.json" | "transfer.json" | "runtime.json")
        || [
            "payload.",
            "payload-",
            "metadata-",
            "merged-",
            "recording-final-",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cleanup_is_non_recursive_bound_to_owner_and_preserves_user_targets() {
        let root = tempfile::tempdir().unwrap();
        let mut task = crate::media::test_support::task();
        task.destination = root.path().to_string_lossy().into_owned();
        let workspace = output::Workspace::open(&task, "best").await.unwrap();
        let target = root.path().join("video.mp4");
        tokio::fs::write(&target, b"user completed file")
            .await
            .unwrap();
        let unexpected = workspace.root.join("user-notes.txt");
        tokio::fs::write(&unexpected, b"notes").await.unwrap();
        assert!(cleanup_task_workspace(&task).await.is_err());
        assert!(workspace.root.join("owner.json").exists());
        assert!(unexpected.exists());
        tokio::fs::remove_file(unexpected).await.unwrap();
        tokio::fs::write(workspace.root.join("payload.mp4.part"), b"temp")
            .await
            .unwrap();
        cleanup_task_workspace(&task).await.unwrap();
        assert!(!workspace.root.exists());
        assert_eq!(
            tokio::fs::read(target).await.unwrap(),
            b"user completed file"
        );
    }
}
