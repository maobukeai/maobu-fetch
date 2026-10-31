use super::super::*;
use super::{
    fixture::{Fixture, Mode},
    harness::Harness,
};

pub(super) async fn await_started(fixture: &Fixture, sent_before: u64) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let ready = if matches!(fixture.state.mode, Mode::SilentHeaders) {
                fixture.state.hits.load(Ordering::Acquire) > 0
            } else {
                fixture.state.sent.load(Ordering::Acquire) > sent_before + 1024
            };
            if ready {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("real worker must reach the local fixture");
    // Give the production writer time to persist the fixture's 64 KiB first chunk.
    if matches!(fixture.state.mode, Mode::SilentBody) {
        tokio::time::sleep(Duration::from_millis(35)).await;
    }
}

pub(super) async fn stable_after_pause(harness: &Harness, fixture: &Fixture, task: &DownloadTask) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.state.active.load(Ordering::Acquire) > 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("server must observe all response bodies dropped");
    let old = if task_temp_dir(&task.destination, &task.id).exists() {
        harness.fingerprints(task).await
    } else {
        vec![]
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    let new = if task_temp_dir(&task.destination, &task.id).exists() {
        harness.fingerprints(task).await
    } else {
        vec![]
    };
    assert_eq!(
        old, new,
        "no writer may remain after pause/deadline returns"
    );
    assert!(!harness.manager.controls.lock().await.contains_key(&task.id));
}
