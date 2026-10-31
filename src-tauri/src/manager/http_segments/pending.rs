use super::*;

#[derive(serde::Serialize, serde::Deserialize)]
struct Pending {
    version: u8,
    task_id: String,
    segment: u8,
    start: u64,
    end: u64,
}
fn path(window: &Path) -> PathBuf {
    PathBuf::from(format!("{}.pending-validation", window.to_string_lossy()))
}

pub(super) async fn begin(task_id: &str, window: &RangeWindow) -> Result<(), String> {
    let record = Pending {
        version: 1,
        task_id: task_id.into(),
        segment: window.segment_index,
        start: window.start_byte,
        end: window.end_byte,
    };
    let bytes = serde_json::to_vec(&record).map_err(|_| "分片验证记录序列化失败")?;
    let staged = PathBuf::from(format!(
        "{}.pending-{}",
        window.path.to_string_lossy(),
        Uuid::new_v4().simple()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)
        .await
        .map_err(|error| format!("创建分片待验证记录失败：{error}"))?;
    file.write_all(&bytes)
        .await
        .map_err(|error| format!("保存分片待验证记录失败：{error}"))?;
    file.sync_all()
        .await
        .map_err(|error| format!("同步分片待验证记录失败：{error}"))?;
    drop(file);
    replace_file_atomically(&staged, &path(&window.path)).await
}

pub(super) async fn remove(window: &Path) -> Result<(), String> {
    match fs::remove_file(path(window)).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("清理分片待验证记录失败，已保留原始数据：{error}")),
    }
}

pub(super) async fn resume(
    task: &DownloadTask,
    window: &Path,
    index: u8,
    start: u64,
    end: u64,
    bytes: u64,
) -> Result<u64, String> {
    let marker = path(window);
    let metadata = match fs::symlink_metadata(&marker).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(bytes),
        Err(error) => return Err(format!("读取分片待验证记录失败：{error}")),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 4096 {
        return Err("分片待验证记录类型或长度无效，已保留数据".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("分片待验证记录已变成链接，未继续下载".into());
        }
    }
    let file = fs::File::open(&marker)
        .await
        .map_err(|error| error.to_string())?;
    let mut encoded = Vec::new();
    file.take(4097)
        .read_to_end(&mut encoded)
        .await
        .map_err(|error| error.to_string())?;
    if encoded.len() > 4096 {
        return Err("分片待验证记录超过安全长度".into());
    }
    let record: Pending =
        serde_json::from_slice(&encoded).map_err(|_| "分片待验证记录损坏，已保留原始数据")?;
    if record.version != 1
        || record.task_id != task.id
        || record.segment != index
        || record.start != start
        || record.end != end
    {
        return Err("分片待验证记录身份或范围不一致，未继续下载".into());
    }
    if bytes == end - start + 1 {
        let rejected = PathBuf::from(format!(
            "{}.rejected-{}",
            window.to_string_lossy(),
            Uuid::new_v4().simple()
        ));
        fs::rename(window, &rejected)
            .await
            .map_err(|error| format!("隔离尚未验证完整响应的分片失败：{error}"))?;
        remove(window).await?;
        return Ok(0);
    }
    if bytes > 0 && http_response::if_range_validator(task).is_none() {
        return Err(format!(
            "{REMOTE_CHANGED_PREFIX}待验证分片缺少可用于 If-Range 的身份，已保留数据，请重新下载"
        ));
    }
    if bytes == 0 {
        remove(window).await?;
    }
    Ok(bytes)
}
