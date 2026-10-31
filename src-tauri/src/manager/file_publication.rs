//! 完成文件一次性发布；失败时目标旧内容与恢复源均保持完整。
use super::*;

pub(crate) async fn replace_file_atomically(candidate: &Path, target: &Path) -> Result<(), String> {
    publish_file_atomically(candidate, target, true).await
}

pub(crate) async fn publish_file_atomically(candidate: &Path, target: &Path, overwrite: bool) -> Result<(), String> {
    let metadata = fs::metadata(candidate).await.map_err(|e| format!("无法读取完成临时文件：{e}"))?;
    if !metadata.is_file() || candidate == target {
        return Err("完成临时文件不是普通文件或与目标重合".into());
    }
    fs::OpenOptions::new().write(true).open(candidate).await
        .map_err(|e| format!("无法刷新完成临时文件：{e}"))?.sync_all().await
        .map_err(|e| format!("完成临时文件落盘失败：{e}"))?;
    match atomic_rename(candidate, target, overwrite).await {
        Ok(()) => return Ok(()),
        Err(error) if is_cross_device(&error) => {}
        Err(error) => return Err(format!("无法发布完成文件，已保留原文件和临时文件：{error}")),
    }
    publish_cross_device(candidate, target, metadata.len(), overwrite).await
}

async fn publish_cross_device(candidate: &Path, target: &Path, expected: u64, overwrite: bool) -> Result<(), String> {
    let parent = target.parent().ok_or("完成文件目标目录无效")?;
    let staging = parent.join(format!(".maobu-publish-{}.lumaget", Uuid::new_v4()));
    let result = async {
        let mut input = fs::File::open(candidate).await.map_err(|e| format!("读取完成文件失败：{e}"))?;
        let mut output = fs::OpenOptions::new().create_new(true).write(true).open(&staging).await
            .map_err(|e| format!("创建发布临时文件失败：{e}"))?;
        let copied = tokio::io::copy(&mut input, &mut output).await
            .map_err(|e| format!("复制完成文件失败，已保留原文件：{e}"))?;
        output.sync_all().await.map_err(|e| format!("发布临时文件落盘失败：{e}"))?;
        drop(output);
        if copied != expected || fs::metadata(&staging).await.map_err(|e| e.to_string())?.len() != expected {
            return Err("发布临时文件长度不匹配，未替换目标".into());
        }
        atomic_rename(&staging, target, overwrite).await.map_err(|e| format!("完成文件原子发布失败：{e}"))?;
        // 发布已成功，清理恢复源失败不能使用户误以为下载失败。
        if let Err(error) = fs::remove_file(candidate).await {
            tracing::warn!(error = %error, "完成文件已发布，但跨卷恢复源清理失败");
        }
        Ok(())
    }.await;
    if result.is_err() {
        if let Err(error) = fs::remove_file(&staging).await {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(error = %error, "发布失败后的临时副本清理失败");
            }
        }
    }
    result
}

#[cfg(windows)]
fn is_cross_device(error: &std::io::Error) -> bool { error.raw_os_error() == Some(17) }
#[cfg(not(windows))]
fn is_cross_device(error: &std::io::Error) -> bool { error.raw_os_error() == Some(18) }

#[cfg(windows)]
async fn atomic_rename(candidate: &Path, target: &Path, overwrite: bool) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
    let wide = |path: &Path| -> std::io::Result<Vec<u16>> {
        let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) { return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "路径含空字符")); }
        value.push(0);
        Ok(value)
    };
    let candidate = wide(candidate)?;
    let target = wide(target)?;
    let flags = MOVEFILE_WRITE_THROUGH | if overwrite { MOVEFILE_REPLACE_EXISTING } else { 0 };
    if unsafe { MoveFileExW(candidate.as_ptr(), target.as_ptr(), flags) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
async fn atomic_rename(candidate: &Path, target: &Path, overwrite: bool) -> std::io::Result<()> {
    if overwrite { return fs::rename(candidate, target).await; }
    fs::hard_link(candidate, target).await?;
    if let Err(error) = fs::remove_file(candidate).await {
        tracing::warn!(error = %error, "完成文件已发布但恢复源清理失败");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cross_device_copy_stages_before_publishing_and_preserves_source_on_failure() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::write(&source, b"new complete data").await.unwrap();
        fs::write(&target, b"old user data").await.unwrap();
        assert!(publish_cross_device(&source, &target, 123, true).await.is_err());
        assert_eq!(fs::read(&source).await.unwrap(), b"new complete data");
        assert_eq!(fs::read(&target).await.unwrap(), b"old user data");
        publish_cross_device(&source, &target, 17, true).await.unwrap();
        assert_eq!(fs::read(&target).await.unwrap(), b"new complete data");
        assert!(!source.exists());
    }

    #[tokio::test]
    async fn rename_or_skip_cannot_overwrite_a_file_created_at_publication_time() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::write(&source, b"download").await.unwrap();
        fs::write(&target, b"user file appeared later").await.unwrap();
        assert!(publish_file_atomically(&source, &target, false).await.is_err());
        assert_eq!(fs::read(&target).await.unwrap(), b"user file appeared later");
        assert_eq!(fs::read(&source).await.unwrap(), b"download");
    }
}
