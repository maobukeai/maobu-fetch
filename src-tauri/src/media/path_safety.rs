pub(super) fn is_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn actual_windows_junction_is_rejected_before_output_or_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let foreign = directory.path().join("foreign-user-files");
        tokio::fs::create_dir(&foreign).await.unwrap();
        let sentinel = foreign.join("payload.mp4");
        tokio::fs::write(&sentinel, b"user file").await.unwrap();
        let mut task = crate::media::test_support::task();
        task.destination = directory.path().to_string_lossy().into_owned();
        let link = crate::media::output::work_directory(&task);
        assert_eq!(link.parent(), Some(directory.path()));
        let mut command = crate::media_tools::create_hidden_tokio_command("cmd.exe");
        command
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&foreign);
        let result = crate::media::process::capture(
            &mut command,
            &tokio_util::sync::CancellationToken::new(),
            std::time::Duration::from_secs(5),
            4096,
        )
        .await
        .unwrap();
        assert!(
            result.status.success(),
            "Windows junction fixture must actually be created"
        );
        assert!(is_link(&tokio::fs::symlink_metadata(&link).await.unwrap()));
        assert!(crate::media::output::Workspace::open(&task, "best")
            .await
            .is_err());
        assert!(crate::media::cleanup_task_workspace(&task).await.is_err());
        assert_eq!(tokio::fs::read(&sentinel).await.unwrap(), b"user file");
        tokio::fs::remove_dir(&link).await.unwrap();
        assert_eq!(tokio::fs::read(sentinel).await.unwrap(), b"user file");
    }
}
