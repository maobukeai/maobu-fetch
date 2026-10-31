use std::{io::ErrorKind, path::Path};

pub(super) async fn validate_completed_file(path: &Path, expected: u64) -> Result<(), String> {
    if expected == 0 { return Err("媒体完成文件为空，已保留分片".into()); }
    validate_regular_file_length(path, expected).await
}

pub(super) async fn validate_regular_file_length(path: &Path, expected: u64) -> Result<(), String> {
    let metadata = tokio::fs::metadata(path).await
        .map_err(|error| format!("完成文件缺失或不可读取，已保留恢复数据：{error}"))?;
    if !metadata.is_file() || metadata.len() != expected {
        return Err("完成文件类型或长度不正确，已保留分片，不能标记下载完成".into());
    }
    Ok(())
}

pub(super) async fn remove_optional_file(path: &Path) -> Result<(), String> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("文件删除失败，任务记录已保留，请关闭占用程序或检查权限：{error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn completion_requires_actual_regular_file_with_exact_length() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("output");
        assert!(validate_completed_file(&path, 7).await.is_err());
        assert!(validate_completed_file(directory.path(), 7).await.is_err());
        tokio::fs::write(&path, b"STALE").await.unwrap();
        assert!(validate_completed_file(&path, 7).await.is_err());
        tokio::fs::write(&path, b"CORRECT").await.unwrap();
        validate_completed_file(&path, 7).await.unwrap();
    }
    #[tokio::test]
    async fn deletion_errors_are_propagated_and_unrelated_file_is_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let sentinel = directory.path().join("keep");
        tokio::fs::write(&sentinel, b"user data").await.unwrap();
        assert!(remove_optional_file(directory.path()).await.is_err());
        assert_eq!(tokio::fs::read(&sentinel).await.unwrap(), b"user data");
        remove_optional_file(&directory.path().join("missing")).await.unwrap();
    }
}
