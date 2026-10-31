use super::super::*;
use super::{
    fixture::{Fixture, Mode},
    harness::Harness,
    support::await_started,
};

pub(super) async fn run(app: &AppHandle) {
    late_extra_response(app).await;
    full_pending_checkpoint_is_not_a_completed_window(app).await;
    unvalidated_resume(app).await;
    for (connections, zero_database_progress) in [(4, false), (1, false), (4, true)] {
        legacy_missing_validator_with_fresh_probe(app, connections, zero_database_progress).await;
    }
}

async fn terminal(harness: &Harness, id: &str) -> DownloadTask {
    tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            let task = harness.row(id).await;
            if matches!(
                task.status,
                TaskStatus::Completed | TaskStatus::Failed | TaskStatus::RemoteChanged
            ) && !harness.manager.controls.lock().await.contains_key(id)
            {
                return task;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("protocol failure must terminate within a bounded deadline")
}

async fn late_extra_response(app: &AppHandle) {
    eprintln!("native manager: reject a chunk arriving after the complete declared Range");
    let fixture = Fixture::new(Mode::LateExtra).await;
    let mut harness = Harness::new(app, None).await;
    let task = harness.task(&fixture, 4).await;
    let failed = terminal(&harness, &task.id).await;
    assert_eq!(
        failed.status,
        TaskStatus::Failed,
        "a valid prefix followed by extra chunked bytes is an invalid Range response"
    );
    assert!(!harness.output.join(&task.file_name).exists());
    let old = harness.fingerprints(&task).await;
    assert!(
        old.iter()
            .any(|(name, size, _)| name.contains(".rejected-") && *size > 0),
        "invalid full window must be retained outside the planned namespace"
    );
    let requests = fixture.state.hits.load(Ordering::Acquire);
    harness.reopen(app).await;
    harness.resume(&task.id).await;
    assert_eq!(
        terminal(&harness, &task.id).await.status,
        TaskStatus::Failed
    );
    assert!(
        fixture.state.hits.load(Ordering::Acquire) > requests,
        "restart must fetch again instead of accepting full rejected window bytes"
    );
    let after = harness.fingerprints(&task).await;
    for entry in old
        .iter()
        .filter(|(name, _, _)| name.contains(".rejected-"))
    {
        assert!(
            after.contains(entry),
            "rejected evidence must survive a retry and restart"
        );
    }
    assert!(!harness.output.join(&task.file_name).exists());
    harness.manager.shutdown_bt().await.unwrap();
}

async fn unvalidated_resume(app: &AppHandle) {
    eprintln!("native manager: missing validators cannot authorize partial shard reuse");
    let fixture = Fixture::new(Mode::NoValidatorBody).await;
    let harness = Harness::new(app, None).await;
    let task = harness.task(&fixture, 4).await;
    await_started(&fixture, 0).await;
    harness.pause(&task.id).await;
    let before = harness.fingerprints(&task).await;
    assert!(before.iter().any(|(_, bytes, _)| *bytes > 0));
    fixture.state.version.store(2, Ordering::Release);
    fixture.resume();
    harness.resume(&task.id).await;
    assert_eq!(
        terminal(&harness, &task.id).await.status,
        TaskStatus::RemoteChanged
    );
    assert_eq!(
        harness.fingerprints(&task).await,
        before,
        "ambiguous remote identity must preserve every old byte"
    );
    assert!(!harness.output.join(&task.file_name).exists());
    harness.manager.shutdown_bt().await.unwrap();
}

async fn legacy_missing_validator_with_fresh_probe(
    app: &AppHandle,
    connections: u8,
    zero_progress: bool,
) {
    eprintln!("native manager: reject legacy missing identity with {connections} connection(s), DB progress cleared={zero_progress}");
    let fixture = Fixture::new(Mode::SilentBody).await;
    let harness = Harness::new(app, None).await;
    let task = harness.task(&fixture, connections).await;
    await_started(&fixture, 0).await;
    harness.pause(&task.id).await;
    let before = harness.fingerprints(&task).await;
    assert!(before.iter().any(|(_, bytes, _)| *bytes > 0));
    let mut legacy = harness.row(&task.id).await;
    legacy.etag = None;
    legacy.last_modified = None;
    if zero_progress {
        legacy.downloaded_bytes = 0;
        for segment in &mut legacy.segments {
            segment.downloaded_bytes = 0;
            segment.status = "pending".into();
        }
    }
    harness.manager.store.upsert_task(&legacy).await.unwrap();
    fixture.resume();
    harness.resume(&task.id).await;
    let row = terminal(&harness, &task.id).await;
    assert_eq!(row.status,TaskStatus::RemoteChanged,"a fresh probe validator cannot identify bytes downloaded before that validator was recorded");
    assert_eq!(harness.fingerprints(&task).await, before);
    assert!(!harness.output.join(&task.file_name).exists());
    harness.manager.shutdown_bt().await.unwrap();
}

async fn full_pending_checkpoint_is_not_a_completed_window(app: &AppHandle) {
    eprintln!("native manager: a full pending-validation crash checkpoint must be fetched again");
    let fixture = Fixture::new(Mode::Good).await;
    let mut harness = Harness::new(app, None).await;
    let mut task = harness.queued_task(&fixture, 4).await;
    let total = super::fixture::LENGTH as u64;
    let ranges = planned_segment_ranges(&task, total, 4);
    let (index, start, end) = ranges[0];
    let expected = end - start + 1;
    let temporary = temp_layout::ensure(&task).await.unwrap();
    let part = window_part_path(&temporary, index, start);
    let source = fixture.expected();
    fs::write(&part, &source[start as usize..=end as usize])
        .await
        .unwrap();
    let marker = PathBuf::from(format!("{}.pending-validation", part.display()));
    fs::write(&marker,serde_json::to_vec(&serde_json::json!({"version":1,"task_id":task.id,"segment":index,"start":start,"end":end})).unwrap()).await.unwrap();
    task.status = TaskStatus::Downloading;
    task.total_bytes = total;
    task.downloaded_bytes = expected;
    task.etag = Some("\"ABC\"".into());
    task.last_modified = Some("Wed, 21 Oct 2015 07:28:00 GMT".into());
    task.segments = ranges
        .iter()
        .map(|&(index, start, end)| DownloadSegment {
            index,
            start_byte: start,
            end_byte: end,
            downloaded_bytes: if index == 0 { expected } else { 0 },
            status: "downloading".into(),
        })
        .collect();
    harness.manager.store.upsert_task(&task).await.unwrap();
    harness.reopen(app).await;
    assert_eq!(fs::metadata(&part).await.unwrap().len(), expected);
    assert!(
        marker.exists(),
        "startup must retain the incomplete response checkpoint"
    );
    harness.resume(&task.id).await;
    harness.check_output(&task.id, &source).await;
    assert!(
        fixture
            .state
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| !request.probe && request.start == start && request.end == end),
        "a full file with pending EOF validation cannot be reused solely by its length"
    );
    harness.manager.shutdown_bt().await.unwrap();
}
