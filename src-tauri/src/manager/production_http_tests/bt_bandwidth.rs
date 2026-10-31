//! 同一隔离 Wry manager 的实际 HTTP/BT 下载与 BT 上传吞吐。
use super::super::*;
use super::{
    fixture::{Fixture, Mode},
    harness::Harness,
};
use crate::bt::bandwidth_fixture::{LocalLeecher, LocalTorrent};
use crate::models::BtNewTaskRequest;
mod mixed;
mod received;
mod upload;

pub(super) async fn run(app: &AppHandle) {
    let selected = std::env::var("MAOBU_TEST_BT_RATE_KIND").ok();
    if selected.as_deref() != Some("upload") {
        mixed::run(app).await;
    }
    if selected.as_deref() != Some("mixed") {
        upload::run(app).await;
    }
}

async fn add_bt(harness: &Harness, torrent: &LocalTorrent, limit: u64) -> DownloadTask {
    use base64::Engine as _;
    let task = harness
        .manager
        .add_bt(BtNewTaskRequest {
            source: "bandwidth-fixture.torrent".into(),
            source_data_base64: Some(
                base64::engine::general_purpose::STANDARD.encode(&torrent.bytes),
            ),
            destination: None,
            selected_files: Vec::new(),
            start_paused: false,
            source_tag: Some("native-bandwidth-test".into()),
            streaming_priority: false,
        })
        .await
        .unwrap();
    harness
        .manager
        .update_task_options(&task.id, None, Some(limit), None)
        .await
        .unwrap()
}

async fn assert_completed(harness: &Harness, id: &str) -> DownloadTask {
    let row = harness.row(id).await;
    assert_eq!(row.status, TaskStatus::Completed);
    row
}

fn assert_rate(label: &str, bytes: u64, elapsed: Duration, limit: u64, allowance: u64) {
    let expected = limit as f64 * elapsed.as_secs_f64();
    eprintln!("NATIVE_BT_RATE label={label} bytes={bytes} duration_ms={} measured_bps={:.0} expected_bps={limit}", elapsed.as_millis(), bytes as f64 / elapsed.as_secs_f64());
    assert!(
        bytes as f64 >= expected * 0.50,
        "{label}: throughput too low to establish active transfer"
    );
    assert!(
        bytes as f64 <= expected * 1.20 + allowance as f64,
        "{label}: actual payload exceeded the rate envelope"
    );
}

async fn wait_progress(label: &str, mut ready: impl AsyncFnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(12), async {
        while !ready().await {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{label}: no real payload progress within deadline"));
}
