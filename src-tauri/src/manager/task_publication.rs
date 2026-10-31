//! A durable journal bridges atomic file publication and the SQLite completion transaction.
use super::*;

const JOURNAL: &str = "task-publication.json";
#[derive(serde::Serialize, serde::Deserialize)]
struct Journal {
    version: u32,
    task_id: String,
    file_name: String,
    length: u64,
    sha256: String,
}

pub(crate) async fn record_task_publication(
    task: &DownloadTask,
    source: &Path,
    target: &Path,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| is_safe_path_component(name))
        .ok_or("完成文件名无效，未发布文件")?;
    let destination = fs::canonicalize(&task.destination)
        .await
        .map_err(|e| format!("读取保存目录失败：{e}"))?;
    let parent = fs::canonicalize(target.parent().ok_or("完成文件路径无效")?)
        .await
        .map_err(|e| format!("读取完成路径失败：{e}"))?;
    if parent != destination {
        return Err("完成文件不在任务保存目录内，未发布文件".into());
    }
    let (length, sha256) = fingerprint(source, cancel).await?;
    let journal = Journal {
        version: 1,
        task_id: task.id.clone(),
        file_name: name.into(),
        length,
        sha256,
    };
    let directory = ensure_task_temp_dir(&task.destination, &task.id).await?;
    let staged = directory.join(format!(".publication-{}", Uuid::new_v4()));
    let bytes = serde_json::to_vec(&journal).map_err(|_| "完成恢复记录序列化失败")?;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&staged)
        .await
        .map_err(|e| format!("创建完成恢复记录失败：{e}"))?;
    file.write_all(&bytes)
        .await
        .map_err(|e| format!("保存完成恢复记录失败：{e}"))?;
    file.sync_all()
        .await
        .map_err(|e| format!("完成恢复记录落盘失败：{e}"))?;
    drop(file);
    if cancel.is_cancelled() {
        return Err("任务已停止，原文件与恢复记录已保留".into());
    }
    publish_file_atomically(&staged, &directory.join(JOURNAL), true).await
}

pub(super) async fn recover(
    task: &DownloadTask,
    cancel: &CancellationToken,
) -> Result<Option<DownloadTask>, String> {
    temp_layout::validate_ancestors(task).await?;
    let path = task_temp_dir(&task.destination, &task.id).join(JOURNAL);
    let metadata = match fs::symlink_metadata(&path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("读取完成恢复记录失败：{error}")),
    };
    if !metadata.is_file() || temp_layout::redirected(&metadata) || metadata.len() > 4096 {
        return Err("完成恢复记录类型或长度无效，已保留全部文件".into());
    }
    let journal: Journal =
        serde_json::from_slice(&fs::read(path).await.map_err(|e| e.to_string())?)
            .map_err(|_| "完成恢复记录损坏，已保留全部文件")?;
    if journal.version != 1
        || journal.task_id != task.id
        || !is_safe_path_component(&journal.file_name)
        || journal.sha256.len() != 64
        || !journal.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("完成恢复记录身份或路径不匹配，未继续下载".into());
    }
    let target = PathBuf::from(&task.destination).join(&journal.file_name);
    match fs::symlink_metadata(&target).await {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("读取已发布文件失败：{error}")),
        Ok(metadata) if !metadata.is_file() || temp_layout::redirected(&metadata) => {
            return Err("已发布路径不是普通文件，已保留恢复数据".into())
        }
        Ok(metadata) if metadata.len() != journal.length => {
            return Err("已发布文件长度已变化，未覆盖原文件，请检查后再操作".into())
        }
        Ok(_) => {}
    }
    let (length, hash) = fingerprint(&target, cancel).await?;
    if length != journal.length || hash != journal.sha256 {
        return Err("已发布文件校验发生变化，未重新下载或覆盖，请检查现有文件".into());
    }
    let mut completed = task.clone();
    completed.file_name = journal.file_name;
    completed.category = category(&completed.file_name);
    completed.total_bytes = length;
    completed.downloaded_bytes = length;
    completed.speed = 0;
    completed.active_connections = 0;
    completed.eta_seconds = Some(0);
    completed.status = TaskStatus::Completed;
    completed.error = None;
    for segment in &mut completed.segments {
        segment.status = "completed".into();
        segment.downloaded_bytes = segment
            .end_byte
            .saturating_sub(segment.start_byte)
            .saturating_add(1);
    }
    Ok(Some(completed))
}

async fn fingerprint(path: &Path, cancel: &CancellationToken) -> Result<(u64, String), String> {
    let metadata = fs::symlink_metadata(path)
        .await
        .map_err(|e| format!("读取完成文件失败：{e}"))?;
    if !metadata.is_file() || temp_layout::redirected(&metadata) {
        return Err("完成文件不是安全的普通文件".into());
    }
    let mut file = fs::File::open(path)
        .await
        .map_err(|e| format!("打开完成文件失败：{e}"))?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let count = tokio::select! {biased; _=cancel.cancelled()=>return Err("任务已停止，完成文件与恢复数据已保留".into()), result=file.read(&mut buffer)=>result.map_err(|e|format!("校验完成文件失败：{e}"))?};
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        total = total.checked_add(count as u64).ok_or("完成文件过大")?;
    }
    if total != metadata.len() {
        return Err("校验期间文件长度变化，已保留文件".into());
    }
    Ok((total, hex::encode(hasher.finalize())))
}

#[cfg(test)]
mod tests;
