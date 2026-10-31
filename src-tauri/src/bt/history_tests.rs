//! A terminal-history operation must stop the native seed and retain every user file.
use super::*;
use crate::{
    manager::SharedManager,
    models::{DownloadTask, TaskKind, TaskStatus},
};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};

pub(crate) async fn clear_completed_seed(manager: &SharedManager, id: &str, folder: &Path) {
    request_seeding(manager, id, true).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !manager.bt.status_of(id, "").await.unwrap().seeder {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let row = manager.store.get_task(id).await.unwrap().unwrap();
    assert_eq!(row.status, TaskStatus::Completed);
    assert!(row.bt_meta.as_ref().unwrap().seeding_requested);
    let content = task_content_path(&row, None).await.unwrap();
    let digest = file_digest(&content).await;
    let sentinel = folder.join("unrelated-history-sentinel.txt");
    tokio::fs::write(&sentinel, b"unrelated user file must remain")
        .await
        .unwrap();
    let sentinel_digest = file_digest(&sentinel).await;
    let cancelled = cancelled_http_record(&row, folder);
    let cancelled_file = folder.join(&cancelled.file_name);
    tokio::fs::write(&cancelled_file, b"retained cancelled HTTP payload")
        .await
        .unwrap();
    let cancelled_digest = file_digest(&cancelled_file).await;
    manager.store.upsert_task(&cancelled).await.unwrap();

    manager.clear_history(true).await.unwrap();
    assert!(manager.store.list_tasks().await.unwrap().is_empty());
    assert!(manager.bt.status_of(id, "").await.is_err());
    assert!(manager.bt.bindings.lock().await.is_empty());
    assert_native_empty(manager).await;
    assert_eq!(file_digest(&content).await, digest);
    assert_eq!(file_digest(&sentinel).await, sentinel_digest);
    assert_eq!(file_digest(&cancelled_file).await, cancelled_digest);
    // Exceed the seed monitor's poll interval: a late callback must not resurrect the row/native task.
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert!(manager.store.list_tasks().await.unwrap().is_empty());
    assert!(manager.bt.bindings.lock().await.is_empty());
    assert_native_empty(manager).await;
    eprintln!("NATIVE_BT_HISTORY requested_seed_removed=true cancelled_http_removed=true native_remaining=0 user_files_sha256_unchanged=true");
}

fn cancelled_http_record(row: &DownloadTask, folder: &Path) -> DownloadTask {
    let mut cancelled = row.clone();
    cancelled.id = uuid::Uuid::new_v4().to_string();
    cancelled.destination = folder.to_string_lossy().into_owned();
    cancelled.file_name = "cancelled-history.bin".into();
    cancelled.url = "http://127.0.0.1/never-started-history-record".into();
    cancelled.task_kind = TaskKind::Http;
    cancelled.status = TaskStatus::Cancelled;
    cancelled.downloaded_bytes = b"retained cancelled HTTP payload".len() as u64;
    cancelled.completed_at = None;
    cancelled.bt_meta = None;
    cancelled.bt_runtime = None;
    cancelled
}

async fn assert_native_empty(manager: &SharedManager) {
    if let Some(session) = manager.bt.session.lock().await.as_ref() {
        let count = session.with_torrents(|items| {
            let mut count = 0;
            for _ in items {
                count += 1;
            }
            count
        });
        assert_eq!(count, 0);
    }
}

async fn file_digest(path: &Path) -> Vec<u8> {
    Sha256::digest(tokio::fs::read(path).await.unwrap()).to_vec()
}
