use super::*;
async fn fixture() -> (tempfile::TempDir, DownloadTask, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let mut task = crate::bt::tests::bt_task_for_test();
    task.task_kind = TaskKind::Http;
    task.destination = root.path().to_string_lossy().into_owned();
    task.file_name = "output.bin".into();
    let source = temp_layout::ensure(&task).await.unwrap();
    let target = root.path().join(&task.file_name);
    fs::write(&source, b"verified payload").await.unwrap();
    (root, task, source, target)
}
#[tokio::test]
async fn atomic_file_without_completion_commit_recovers_offline_without_redownload() {
    let (_root, task, source, target) = fixture().await;
    let cancel = CancellationToken::new();
    record_task_publication(&task, &source, &target, &cancel)
        .await
        .unwrap();
    assert!(recover(&task, &cancel).await.unwrap().is_none());
    publish_file_atomically(&source, &target, false)
        .await
        .unwrap();
    let complete = recover(&task, &cancel).await.unwrap().unwrap();
    assert_eq!(complete.status, TaskStatus::Completed);
    assert_eq!(complete.downloaded_bytes, 16);
    assert_eq!(fs::read(target).await.unwrap(), b"verified payload");
}
#[tokio::test]
async fn changed_or_foreign_final_files_are_never_accepted_or_overwritten() {
    let (root, task, source, target) = fixture().await;
    let cancel = CancellationToken::new();
    record_task_publication(&task, &source, &target, &cancel)
        .await
        .unwrap();
    fs::write(&target, b"different bytes!").await.unwrap();
    assert!(recover(&task, &cancel).await.unwrap_err().contains("校验"));
    assert_eq!(fs::read(&target).await.unwrap(), b"different bytes!");
    let path = task_temp_dir(&task.destination, &task.id).join(JOURNAL);
    let foreign = Journal {
        version: 1,
        task_id: "foreign".into(),
        file_name: "../outside".into(),
        length: 16,
        sha256: "0".repeat(64),
    };
    fs::write(path, serde_json::to_vec(&foreign).unwrap())
        .await
        .unwrap();
    assert!(recover(&task, &cancel).await.is_err());
    assert!(root.path().join("output.bin").exists());
}
#[tokio::test]
async fn cancelled_publication_hashing_preserves_source_and_existing_target() {
    let (_root, task, source, target) = fixture().await;
    let cancel = CancellationToken::new();
    cancel.cancel();
    fs::write(&target, b"user target").await.unwrap();
    assert!(record_task_publication(&task, &source, &target, &cancel)
        .await
        .is_err());
    assert_eq!(fs::read(source).await.unwrap(), b"verified payload");
    assert_eq!(fs::read(target).await.unwrap(), b"user target");
}
