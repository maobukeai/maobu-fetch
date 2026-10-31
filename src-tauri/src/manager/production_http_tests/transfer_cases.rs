use super::super::*;
use super::{
    fixture::{Fixture, Mode, LENGTH},
    harness::Harness,
    support::{await_started, stable_after_pause},
};

pub(super) async fn integral_ranges(app: &AppHandle, connections: u8) {
    eprintln!("native manager: exact coverage with {connections} connection(s)");
    let fixture = Fixture::new(Mode::Good).await;
    let harness = Harness::new(app, None).await;
    let task = harness.task(&fixture, connections).await;
    let row = harness.check_output(&task.id, &fixture.expected()).await;
    assert_eq!(row.connection_count, connections);
    assert_eq!(row.segments.len(), connections as usize);
    let mut requests: Vec<_> = fixture
        .state
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| !request.probe && matches!(request.status, 200 | 206))
        .cloned()
        .collect();
    requests.sort_by_key(|request| request.start);
    let mut next = 0;
    for request in &requests {
        assert_eq!(
            request.start, next,
            "overlapping, duplicate or missing actual HTTP range: {requests:?}"
        );
        assert!(request.end >= request.start);
        next = request.end + 1;
    }
    assert_eq!(next, LENGTH as u64);
    assert!(!requests.is_empty());
    assert_eq!(
        fixture.state.peak.load(Ordering::Acquire),
        connections as usize,
        "the fixture must observe the configured number of simultaneous real HTTP connections"
    );
    harness.manager.shutdown_bt().await.unwrap();
}

pub(super) async fn fallback_stream(app: &AppHandle, mode: Mode) {
    eprintln!("native manager: safe single stream {mode:?}");
    let fixture = Fixture::new(mode).await;
    let harness = Harness::new(app, None).await;
    let task = harness.task(&fixture, 8).await;
    let row = harness.check_output(&task.id, &fixture.expected()).await;
    assert_eq!(row.connection_count, 1);
    assert_eq!(row.segments.len(), 1);
    assert_eq!(row.accepts_ranges, Some(false));
    assert_eq!(
        fixture.state.requests.lock().unwrap().len(),
        1,
        "reuse the ignored-Range probe body"
    );
    harness.manager.shutdown_bt().await.unwrap();
}

pub(super) async fn invalid_range(app: &AppHandle, mode: Mode) {
    eprintln!("native manager: reject {mode:?}");
    let fixture = Fixture::new(mode).await;
    let harness = Harness::new(app, Some(3)).await;
    let task = harness.task(&fixture, 4).await;
    let failed = harness.wait(&task.id, TaskStatus::Failed).await;
    assert!(failed.error.is_some());
    assert!(
        !harness.output.join(&failed.file_name).exists(),
        "an invalid response must never publish an output"
    );
    assert!(
        fixture.state.hits.load(Ordering::Acquire) <= 1 + 4 * MAX_EMPTY_STEPS as usize,
        "no unbounded retries on protocol failure"
    );
    assert_eq!(failed.active_connections, 0);
    harness.manager.shutdown_bt().await.unwrap();
}

pub(super) async fn pause_silent_connection(app: &AppHandle, mode: Mode) {
    eprintln!("native manager: pause {mode:?}");
    let fixture = Fixture::new(mode).await;
    let harness = Harness::new(app, None).await;
    let task = harness.task(&fixture, 4).await;
    await_started(&fixture, 0).await;
    harness.pause(&task.id).await;
    stable_after_pause(&harness, &fixture, &task).await;
    fixture.resume();
    harness.resume(&task.id).await;
    harness.check_output(&task.id, &fixture.expected()).await;
    harness.manager.shutdown_bt().await.unwrap();
}

pub(super) async fn timeout_silent_connection(app: &AppHandle, mode: Mode) {
    eprintln!("native manager: total deadline {mode:?}");
    let fixture = Fixture::new(mode).await;
    let harness = Harness::new(app, Some(1)).await;
    let started = Instant::now();
    let task = harness.task(&fixture, 4).await;
    let row = harness.wait(&task.id, TaskStatus::Failed).await;
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "task deadline includes silent headers/body"
    );
    assert!(row
        .error
        .as_deref()
        .is_some_and(|error| error.contains("超时")));
    assert!(!harness.output.join(&row.file_name).exists());
    stable_after_pause(&harness, &fixture, &task).await;
    harness.manager.shutdown_bt().await.unwrap();
}
