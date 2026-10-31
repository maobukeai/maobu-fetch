use super::*;

async fn fixture() -> (tempfile::TempDir, Arc<Store>, DownloadTask) {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    let store = Arc::new(tokio::task::spawn_blocking(move || Store::open(data)).await.unwrap().unwrap());
    let mut task = crate::bt::tests::bt_task_for_test();
    task.task_kind = models::TaskKind::Http;
    task.status = models::TaskStatus::Paused;
    task.destination = directory.path().to_string_lossy().into_owned();
    task.file_name = "incomplete.bin".into();
    store.upsert_task(&task).await.unwrap();
    (directory, store, task)
}

#[tokio::test]
async fn offline_remove_record_preserves_incomplete_output_and_recovery_data() {
    let (directory, store, task) = fixture().await;
    let target = directory.path().join(&task.file_name);
    tokio::fs::write(&target, b"existing user file").await.unwrap();
    let temp_dir = manager::ensure_task_temp_dir(&task.destination, &task.id).await.unwrap();
    let part = temp_dir.join("part");
    tokio::fs::write(&part, b"resume data").await.unwrap();
    cli_remove(&store, &task.id, false).await.unwrap();
    assert!(store.get_task(&task.id).await.unwrap().is_none());
    assert_eq!(tokio::fs::read(&target).await.unwrap(), b"existing user file");
    assert_eq!(tokio::fs::read(&part).await.unwrap(), b"resume data");
}

#[tokio::test]
async fn offline_delete_failure_keeps_the_record_and_unrelated_files() {
    let (directory, store, task) = fixture().await;
    let target = directory.path().join(&task.file_name);
    tokio::fs::create_dir(&target).await.unwrap();
    tokio::fs::write(target.join("keep"), b"unrelated").await.unwrap();
    assert!(cli_remove(&store, &task.id, true).await.is_err());
    assert!(store.get_task(&task.id).await.unwrap().is_some());
    assert_eq!(tokio::fs::read(target.join("keep")).await.unwrap(), b"unrelated");
}
