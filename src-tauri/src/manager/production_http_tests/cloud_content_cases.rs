//! Actual Manager/SQLite workers must not publish a Lanzou HTML intermediate as a binary.
use super::super::*;
use super::harness::Harness;
#[path = "cloud_content_fixture.rs"]
mod fixture;
use fixture::{Fixture, Mode};

pub(super) async fn run(app: &AppHandle) {
    eprintln!("native cloud content: reject declared HTML before publication, including changed stream/range and resume");
    for (mode, connections) in [(Mode::Html, 1), (Mode::StreamHtml, 1), (Mode::RangeHtml, 4)] {
        reject_html(app, mode, connections, false).await;
    }
    reject_html(app, Mode::Html, 1, true).await;
    preserve_resume(app).await;
    preserve_encoding_guard(app).await;
    for (mode, source, name) in [
        (Mode::Binary, "lanzou", "binary.exe"),
        (Mode::Html, "lanzou", "page.html"),
        (Mode::Html, "desktop", "explicit-save.exe"),
        (Mode::UnknownMime, "lanzou", "unknown-mime.exe"),
    ] {
        allowed_file(app, mode, source, name).await;
    }
}

async fn queued(
    harness: &Harness,
    fixture: &Fixture,
    source: &str,
    name: &str,
    connections: u8,
) -> DownloadTask {
    let mut task =
        super::super::tests::test_task(&harness.output, name, CollisionPolicy::Overwrite);
    task.id = Uuid::new_v4().to_string();
    task.url = fixture.url.clone();
    task.source = source.into();
    task.connection_count = connections;
    task.max_retries = 0;
    task.expected_checksum = None; // No publisher hash: the MIME guard, not a test checksum, must reject.
    task.proxy_override = Some(String::new());
    harness
        .manager
        .store
        .create_task_with_name_intent(&task, true)
        .await
        .unwrap();
    task
}

async fn reject_html(app: &AppHandle, mode: Mode, connections: u8, existing_target: bool) {
    let fixture = Fixture::new(mode).await;
    let harness = Harness::new(app, Some(8)).await;
    let task = queued(&harness, &fixture, "lanzou", "selected.exe", connections).await;
    let target = harness.output.join(&task.file_name);
    if existing_target {
        fs::write(&target, b"original user file").await.unwrap();
    }
    harness.manager.dispatch_once().await.unwrap();
    let failed = harness.wait(&task.id, TaskStatus::Failed).await;
    assert!(failed
        .error
        .as_deref()
        .is_some_and(|error| error.contains("HTML") && error.contains("浏览器")));
    assert_eq!(failed.active_connections, 0);
    assert_eq!(
        failed.downloaded_bytes, 0,
        "HTML response bytes must not be committed as file progress"
    );
    if existing_target {
        assert_eq!(fs::read(&target).await.unwrap(), b"original user file");
    } else {
        assert!(
            !target.exists(),
            "HTML intermediate must never publish the selected binary filename"
        );
    }
    let completed = harness.manager.store.list_tasks().await.unwrap();
    assert!(completed
        .iter()
        .all(|row| row.status != TaskStatus::Completed));
    harness.manager.shutdown_bt().await.unwrap();
}

async fn preserve_resume(app: &AppHandle) {
    let fixture = Fixture::new(Mode::StreamHtml).await;
    let harness = Harness::new(app, Some(8)).await;
    let mut task = queued(&harness, &fixture, "lanzou", "resumed.exe", 1).await;
    let temporary = temp_layout::ensure(&task).await.unwrap();
    let retained = b"original verified partial bytes";
    fs::write(&temporary, retained).await.unwrap();
    task.total_bytes = fixture.expected().len() as u64;
    task.downloaded_bytes = retained.len() as u64;
    task.etag = Some("\"same-resource\"".into());
    task.last_modified = Some("Wed, 21 Oct 2015 07:28:00 GMT".into());
    harness.manager.store.upsert_task(&task).await.unwrap();
    harness.manager.dispatch_once().await.unwrap();
    let failed = harness.wait(&task.id, TaskStatus::Failed).await;
    assert!(failed
        .error
        .as_deref()
        .is_some_and(|error| error.contains("HTML")));
    assert!(
        fixture.requested_start(retained.len()),
        "test must reach the real resumed response, not only its probe"
    );
    assert_eq!(
        fs::read(temporary).await.unwrap(),
        retained,
        "rejected resumed HTML cannot append to or truncate old bytes"
    );
    assert!(!harness.output.join(&task.file_name).exists());
    harness.manager.shutdown_bt().await.unwrap();
}

async fn preserve_encoding_guard(app: &AppHandle) {
    let fixture = Fixture::new(Mode::Gzip).await;
    let harness = Harness::new(app, Some(8)).await;
    let task = queued(&harness, &fixture, "lanzou", "encoded.exe", 1).await;
    harness.manager.dispatch_once().await.unwrap();
    let failed = harness.wait(&task.id, TaskStatus::Failed).await;
    assert!(failed
        .error
        .as_deref()
        .is_some_and(|error| error.contains("identity")));
    assert!(!harness.output.join(&task.file_name).exists());
    harness.manager.shutdown_bt().await.unwrap();
}

async fn allowed_file(app: &AppHandle, mode: Mode, source: &str, name: &str) {
    let fixture = Fixture::new(mode).await;
    let harness = Harness::new(app, Some(8)).await;
    let task = queued(&harness, &fixture, source, name, 1).await;
    harness.manager.dispatch_once().await.unwrap();
    harness.check_output(&task.id, fixture.expected()).await;
    harness.manager.shutdown_bt().await.unwrap();
}
