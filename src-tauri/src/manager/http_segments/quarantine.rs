use super::super::work_stealing::WindowTransferHandle;
use super::*;

fn marker(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.invalid", path.to_string_lossy()))
}

pub(super) async fn ensure_usable(path: &Path) -> Result<(), String> {
    match fs::metadata(marker(path)).await {
        Ok(_) => Err(format!(
            "{REMOTE_CHANGED_PREFIX}旧分片的无效响应隔离尚未完成，已保留数据；请选择重新下载"
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("读取分片隔离标记失败：{error}")),
    }
}

/// Never leave a full but unvalidated window under a name that resume can accept by length alone.
pub(super) async fn isolate(
    session: &Session,
    window: &RangeWindow,
    handle: &WindowTransferHandle,
    runtime: &SegmentRuntime,
) -> Result<(), String> {
    let flag = marker(&window.path);
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&flag)
        .await
        .map_err(|error| {
            format!(
                "{REMOTE_CHANGED_PREFIX}无法标记无效响应分片，旧数据已保留，请重新下载：{error}"
            )
        })?;
    file.write_all(b"invalid HTTP Range response; do not resume this window\n")
        .await
        .map_err(|error| format!("{REMOTE_CHANGED_PREFIX}保存分片隔离标记失败：{error}"))?;
    file.sync_all()
        .await
        .map_err(|error| format!("{REMOTE_CHANGED_PREFIX}同步分片隔离标记失败：{error}"))?;
    drop(file);
    let rejected = PathBuf::from(format!(
        "{}.rejected-{}",
        window.path.to_string_lossy(),
        Uuid::new_v4().simple()
    ));
    fs::rename(&window.path, &rejected).await.map_err(|error| {
        format!(
            "{REMOTE_CHANGED_PREFIX}无法隔离无效响应分片，已留下阻止续传标记，请重新下载：{error}"
        )
    })?;
    let bytes = handle.downloaded_bytes.swap(0, Ordering::Relaxed);
    runtime.downloaded_bytes.fetch_sub(bytes, Ordering::Relaxed);
    session.progress.fetch_sub(bytes, Ordering::Relaxed);
    pending::remove(&window.path).await?;
    fs::remove_file(&flag).await.map_err(|error| {
        format!("{REMOTE_CHANGED_PREFIX}无效分片已隔离，但清理隔离标记失败，请重新下载：{error}")
    })?;
    Ok(())
}
