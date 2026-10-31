//! Fail the real SQLite completion update after real HTTP bytes were atomically published.
use super::super::*;
use super::{
    fixture::{Fixture, Mode, LENGTH},
    harness::Harness,
};

pub(super) async fn run(app: &AppHandle) {
    for connections in [1, 4] {
        completion_commit_fault_preserves_published_bytes_and_offline_recovery(app, connections)
            .await;
    }
}

async fn completion_commit_fault_preserves_published_bytes_and_offline_recovery(
    app: &AppHandle,
    connections: u8,
) {
    eprintln!("native manager: SQLite completion fault after {connections}-connection publication recovers without network");
    let fixture = Fixture::new(Mode::Good).await;
    let mut harness = Harness::new(app, None).await;
    let database = rusqlite::Connection::open(harness.data.join("lumaget.db")).unwrap();
    database.execute_batch("CREATE TRIGGER reject_test_completion BEFORE UPDATE OF status ON tasks WHEN NEW.status = 'completed' BEGIN SELECT RAISE(FAIL, 'synthetic completed commit fault'); END;").unwrap();
    let task = harness.task(&fixture, connections).await;
    tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            if !harness.manager.controls.lock().await.contains_key(&task.id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("completion commit fault must stop the real worker within a bounded deadline");
    let row = harness.row(&task.id).await;
    assert_eq!(row.status,TaskStatus::Downloading,"a failed completion transaction must not persist a false terminal status or remove its recovery evidence");
    assert_eq!(row.active_connections, 0);
    let published = harness.output.join(&row.file_name);
    let bytes = fs::read(&published).await.unwrap();
    assert_eq!(bytes.len(), LENGTH);
    let expected_hash = Sha256::digest(fixture.expected());
    assert_eq!(Sha256::digest(&bytes), expected_hash);
    let evidence = harness.fingerprints(&task).await;
    assert!(evidence
        .iter()
        .any(|(name, bytes, _)| name == "task-publication.json" && *bytes > 0));
    if connections > 1 {
        let parts: Vec<_> = evidence
            .iter()
            .filter(|(name, _, _)| name.contains(".part"))
            .collect();
        assert_eq!(parts.len(), connections as usize);
        assert_eq!(
            parts.iter().map(|(_, bytes, _)| *bytes).sum::<u64>(),
            LENGTH as u64
        );
    }
    let hits = fixture.state.hits.load(Ordering::Acquire);
    fixture.stop_serving();
    database
        .execute_batch("DROP TRIGGER reject_test_completion;")
        .unwrap();
    drop(database);
    harness.reopen(app).await;
    harness.resume(&task.id).await;
    let completed = harness.wait(&task.id, TaskStatus::Completed).await;
    assert_eq!(completed.file_name, row.file_name);
    assert_eq!(completed.downloaded_bytes, LENGTH as u64);
    assert_eq!(
        Sha256::digest(fs::read(&published).await.unwrap()),
        expected_hash
    );
    assert_eq!(fixture.state.hits.load(Ordering::Acquire),hits,"journal recovery must verify the already-published local SHA without probing or downloading the source again");
    harness.manager.shutdown_bt().await.unwrap();
}
