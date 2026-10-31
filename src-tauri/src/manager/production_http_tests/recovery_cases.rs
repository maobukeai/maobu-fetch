use super::super::*;
use super::{
    fixture::{Fixture, Mode, LENGTH},
    harness::Harness,
    support::{await_started, stable_after_pause},
};

pub(super) async fn rapid_pause_resume(app: &AppHandle) {
    eprintln!("native manager: four rapid stop/resume generations");
    let fixture = Fixture::new(Mode::SilentBody).await;
    let harness = Harness::new(app, None).await;
    let task = harness.task(&fixture, 4).await;
    let mut sent = 0;
    for round in 0..4 {
        await_started(&fixture, sent).await;
        harness.pause(&task.id).await;
        stable_after_pause(&harness, &fixture, &task).await;
        sent = fixture.state.sent.load(Ordering::Acquire);
        if round != 3 {
            harness.resume(&task.id).await;
        }
    }
    fixture.resume();
    harness.resume(&task.id).await;
    harness.check_output(&task.id, &fixture.expected()).await;
    assert!(
        fixture.state.peak.load(Ordering::Acquire) <= 4,
        "no overlapping worker generations"
    );
    harness.manager.shutdown_bt().await.unwrap();
}

pub(super) async fn reopen_and_resume(app: &AppHandle) {
    eprintln!("native manager: reopen the real SQLite store and resume shards");
    let fixture = Fixture::new(Mode::SilentBody).await;
    let mut harness = Harness::new(app, None).await;
    let task = harness.task(&fixture, 4).await;
    await_started(&fixture, 0).await;
    harness.pause(&task.id).await;
    let old = harness.fingerprints(&task).await;
    assert!(old.iter().any(|(_, size, _)| *size > 0));
    // Emulate an interrupted persisted status, then invoke the actual startup recovery.
    let mut interrupted = harness.row(&task.id).await;
    let expected_next: Vec<_> = interrupted
        .segments
        .iter()
        .filter(|segment| {
            segment.downloaded_bytes > 0
                && segment.start_byte + segment.downloaded_bytes <= segment.end_byte
        })
        .map(|segment| segment.start_byte + segment.downloaded_bytes)
        .collect();
    assert!(
        !expected_next.is_empty(),
        "the pause must save a real partial shard"
    );
    let old_requests = fixture.state.requests.lock().unwrap().len();
    interrupted.status = TaskStatus::Downloading;
    harness
        .manager
        .store
        .upsert_task(&interrupted)
        .await
        .unwrap();
    harness.reopen(app).await;
    assert_eq!(harness.row(&task.id).await.status, TaskStatus::Interrupted);
    assert_eq!(
        harness.fingerprints(&task).await,
        old,
        "opening/recovery cannot alter shard bytes"
    );
    harness.manager.action(&task.id, "resume").await.unwrap();
    assert_eq!(harness.row(&task.id).await.status, TaskStatus::Queued);
    fixture.resume();
    harness.manager.dispatch_once().await.unwrap();
    harness.check_output(&task.id, &fixture.expected()).await;
    assert!(
        fixture.state.requests.lock().unwrap()[old_requests..]
            .iter()
            .any(|request| !request.probe && expected_next.contains(&request.start)),
        "resume from the exact persisted partial-shard next byte"
    );
    harness.manager.shutdown_bt().await.unwrap();
}

pub(super) async fn changed_etag_preserves_data(app: &AppHandle) {
    eprintln!("native manager: ETag case change preserves old shards");
    let fixture = Fixture::new(Mode::SilentBody).await;
    let harness = Harness::new(app, None).await;
    let task = harness.task(&fixture, 4).await;
    await_started(&fixture, 0).await;
    harness.pause(&task.id).await;
    let old = harness.fingerprints(&task).await;
    assert!(old.iter().any(|(_, size, _)| *size > 0));
    fixture.state.version.store(2, Ordering::Release);
    fixture.resume();
    harness.resume(&task.id).await;
    let changed = harness.wait(&task.id, TaskStatus::RemoteChanged).await;
    assert_eq!(changed.etag.as_deref(), Some("\"ABC\""));
    assert_eq!(harness.fingerprints(&task).await, old);
    assert!(!harness.output.join(changed.file_name).exists());
    harness.manager.shutdown_bt().await.unwrap();
}

pub(super) async fn completed_temporary_recovers_without_416(app: &AppHandle) {
    eprintln!("native manager: fully downloaded temporary survives pre-publication restart");
    let fixture = Fixture::new(Mode::Good).await;
    let mut harness = Harness::new(app, None).await;
    let mut task = harness.queued_task(&fixture, 1).await;
    task.total_bytes = LENGTH as u64;
    task.downloaded_bytes = LENGTH as u64;
    task.etag = Some("\"ABC\"".into());
    task.last_modified = Some("Wed, 21 Oct 2015 07:28:00 GMT".into());
    task.accepts_ranges = Some(true);
    task.status = TaskStatus::Downloading;
    ensure_task_temp_dir(&task.destination, &task.id)
        .await
        .unwrap();
    fs::write(
        task_temp_path(&task.destination, &task.id, &task.file_name),
        fixture.expected(),
    )
    .await
    .unwrap();
    harness.manager.store.upsert_task(&task).await.unwrap();
    harness.reopen(app).await;
    assert_eq!(harness.row(&task.id).await.status,TaskStatus::Interrupted);
    harness.resume(&task.id).await;
    harness.check_output(&task.id, &fixture.expected()).await;
    let requests = fixture.state.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "only the source-validator probe is needed for a complete temporary"
    );
    assert!(
        requests[0].probe,
        "never request bytes=total- from a complete local entity"
    );
    drop(requests);
    harness.manager.shutdown_bt().await.unwrap();
}

pub(super) async fn publication_failure_preserves_all_shards(app: &AppHandle) {
    eprintln!("native manager: failed publication retains every verified source shard");
    let fixture = Fixture::new(Mode::SilentBody).await;
    let harness = Harness::new(app, None).await;
    let task = harness.task(&fixture, 4).await;
    await_started(&fixture, 0).await;
    harness.pause(&task.id).await;
    let mut paused = harness.row(&task.id).await;
    paused.collision_policy = CollisionPolicy::Overwrite;
    harness.manager.store.upsert_task(&paused).await.unwrap();
    let blocked_output = harness.output.join(&paused.file_name);
    fs::create_dir(&blocked_output).await.unwrap();
    let sentinel = blocked_output.join("unrelated-user-file");
    fs::write(&sentinel, b"PRESERVE").await.unwrap();
    fixture.resume();
    harness.resume(&task.id).await;
    let failed = harness.wait(&task.id, TaskStatus::Failed).await;
    assert!(failed.error.is_some());
    assert_eq!(fs::read(&sentinel).await.unwrap(), b"PRESERVE");
    let files = harness.fingerprints(&task).await;
    let shards: Vec<_> = files
        .iter()
        .filter(|(name, _, _)| name.contains(".part"))
        .collect();
    assert_eq!(
        shards.len(),
        4,
        "every source fragment must survive final publish failure"
    );
    assert_eq!(
        shards.iter().map(|(_, bytes, _)| *bytes).sum::<u64>(),
        LENGTH as u64
    );
    assert!(files
        .iter()
        .any(|(name, bytes, _)| name.ends_with(".lumaget") && *bytes == LENGTH as u64));
    // Remove only the two explicitly created blocking fixtures, then resume.
    fs::remove_file(&sentinel).await.unwrap();
    fs::remove_dir(&blocked_output).await.unwrap();
    harness.resume(&task.id).await;
    harness.check_output(&task.id, &fixture.expected()).await;
    harness.manager.shutdown_bt().await.unwrap();
}

pub(super) async fn network_wait_obeys_total_deadline(app: &AppHandle) {
    eprintln!("native manager: network-recovery wait remains inside task deadline");
    let fixture = Fixture::new(Mode::Good).await;
    fixture.stop_serving();
    tokio::time::sleep(Duration::from_millis(10)).await;
    let harness = Harness::new(app, Some(1)).await;
    let started = Instant::now();
    let task = harness.task(&fixture, 1).await;
    let row = harness.wait(&task.id, TaskStatus::Failed).await;
    assert!(started.elapsed() < Duration::from_secs(4));
    assert!(row
        .error
        .as_deref()
        .is_some_and(|error| error.contains("超时")));
    assert_eq!(row.active_connections, 0);
    assert!(!harness.output.join(row.file_name).exists());
    harness.manager.shutdown_bt().await.unwrap();
}
