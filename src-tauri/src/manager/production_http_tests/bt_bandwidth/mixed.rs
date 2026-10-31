use super::*;

const GLOBAL_BPS: u64 = 512 * 1024;

pub(super) async fn run(app: &AppHandle) {
    eprintln!("native manager: measured aggregate HTTP Range + BT payload budget");
    let harness = Harness::new(app, None).await;
    let torrent = LocalTorrent::new(&harness.data.join("mixed-torrent")).await;
    let http = Fixture::new(Mode::Good).await;
    let mut settings = harness.manager.settings().await;
    settings.concurrent_downloads = 2;
    settings.speed_limit_kbps = GLOBAL_BPS / 1024;
    harness
        .manager
        .save_settings(settings.clone())
        .await
        .unwrap();
    let http_task = harness.queued_task(&http, 4).await;
    let bt_task = add_bt(&harness, &torrent, 0).await;
    let received = received::Received::new(app, &http_task.id);
    harness.manager.dispatch_once().await.unwrap();
    warm_both(&harness, &http_task, &bt_task, &torrent, &received).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let before = transfer_snapshot(&harness, &received, &bt_task).await;
    let clock = std::time::Instant::now();
    tokio::time::sleep(Duration::from_secs(6)).await;
    let after = transfer_snapshot(&harness, &received, &bt_task).await;
    let elapsed = clock.elapsed();
    let http_delta = after.0 - before.0;
    let bt_delta = after.1 - before.1;
    assert_rate(
        "mixed_http",
        http_delta,
        elapsed,
        GLOBAL_BPS / 2,
        128 * 1024,
    );
    assert_rate("mixed_bt", bt_delta, elapsed, GLOBAL_BPS / 2, 32 * 1024);
    assert_rate(
        "mixed_aggregate",
        http_delta + bt_delta,
        elapsed,
        GLOBAL_BPS,
        160 * 1024,
    );
    assert!(
        http.state.peak.load(Ordering::Acquire) == 4,
        "the HTTP side must exercise all four real Range connections"
    );
    harness.pause(&http_task.id).await;
    harness.pause(&bt_task.id).await;
    assert_eq!(
        received.bytes(),
        http_bytes(&http_task).await,
        "final client-received event must equal real flushed shards after pause"
    );
    assert_eq!(
        harness.row(&http_task.id).await.downloaded_bytes,
        received.bytes()
    );
    let stopped = snapshot(&harness, &http_task, &bt_task).await;
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(
        stopped,
        snapshot(&harness, &http_task, &bt_task).await,
        "pause must stop all actual writers"
    );
    finish_unlimited(&harness, &torrent, &http, &http_task, &bt_task, settings).await;
    harness.manager.shutdown_bt().await.unwrap();
    torrent.source.stop().await;
}

async fn warm_both(
    harness: &Harness,
    http: &DownloadTask,
    bt: &DownloadTask,
    torrent: &LocalTorrent,
    received: &received::Received,
) {
    let warmed = tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            assert_not_failed(harness, &http.id).await;
            assert_not_failed(harness, &bt.id).await;
            if received.bytes() >= 192 * 1024
                && harness
                    .manager
                    .bt
                    .status_of(&bt.id, "")
                    .await
                    .is_ok_and(|progress| progress.downloaded_bytes >= 192 * 1024)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    if warmed.is_err() {
        let http_row = harness.row(&http.id).await;
        let bt_row = harness.row(&bt.id).await;
        eprintln!("MIXED_WARM http_fs={} http_db={} http_status={:?} http_connections={} bt_db={} bt_status={:?} controls={}",
            http_bytes(http).await, http_row.downloaded_bytes, http_row.status, http_row.active_connections,
            bt_row.downloaded_bytes, bt_row.status, harness.manager.controls.lock().await.len());
        eprintln!(
            "MIXED_WARM http_actual_client_received={}",
            received.bytes()
        );
        match harness.manager.bt.status_of(&bt.id, "").await {
            Ok(progress) => eprintln!(
                "MIXED_WARM bt_native_state={} bt_verified={} bt_peers={}",
                progress.state, progress.downloaded_bytes, progress.num_peers
            ),
            Err(error) => eprintln!("MIXED_WARM bt_native_error={error}"),
        }
        eprintln!(
            "MIXED_WARM tracker_announces={} source_stats={:?}",
            torrent.announces.load(Ordering::Relaxed),
            torrent
                .source
                .with_torrents(|items| items.map(|(_, handle)| handle.stats()).collect::<Vec<_>>())
        );
        let directory = task_temp_dir(&http.destination, &http.id);
        if let Ok(mut files) = fs::read_dir(directory).await {
            while let Some(file) = files.next_entry().await.unwrap() {
                eprintln!(
                    "MIXED_WARM http_file={} size={}",
                    file.file_name().to_string_lossy(),
                    file.metadata().await.unwrap().len()
                );
            }
        }
    }
    assert!(
        warmed.is_ok(),
        "both real transports must become active before measuring aggregate rate"
    );
}

async fn finish_unlimited(
    harness: &Harness,
    torrent: &LocalTorrent,
    http: &Fixture,
    http_task: &DownloadTask,
    bt_task: &DownloadTask,
    mut settings: AppSettings,
) {
    let before = snapshot(harness, http_task, bt_task).await;
    settings.speed_limit_kbps = 0;
    harness.manager.save_settings(settings).await.unwrap();
    harness
        .manager
        .action(&http_task.id, "resume")
        .await
        .unwrap();
    harness.manager.action(&bt_task.id, "resume").await.unwrap();
    let clock = std::time::Instant::now();
    harness.manager.dispatch_once().await.unwrap();
    let (http_done, bt_done) = tokio::join!(
        harness.wait(&http_task.id, TaskStatus::Completed),
        harness.wait(&bt_task.id, TaskStatus::Completed)
    );
    let duration = clock.elapsed();
    let transferred = http_done.total_bytes - before.0 + bt_done.total_bytes - before.1;
    eprintln!("NATIVE_BT_RATE label=mixed_zero_unlimited bytes={transferred} duration_ms={} measured_bps={:.0}", duration.as_millis(), transferred as f64 / duration.as_secs_f64());
    assert!(
        transferred as f64 / duration.as_secs_f64() > GLOBAL_BPS as f64 * 1.5,
        "0 must release both quotas instead of stopping or keeping the old cap"
    );
    harness.check_output(&http_task.id, &http.expected()).await;
    let file = crate::bt::task_content_path(&bt_done, None).await.unwrap();
    let bytes = fs::read(file).await.unwrap();
    assert_eq!(bytes.len(), torrent.expected.len());
    assert_eq!(Sha256::digest(bytes), Sha256::digest(&torrent.expected));
    assert_eq!(
        harness.manager.store.list_tasks().await.unwrap().len(),
        2,
        "pause/resume/unlimited keeps both task records"
    );
}

async fn snapshot(harness: &Harness, http: &DownloadTask, bt: &DownloadTask) -> (u64, u64) {
    (
        http_bytes(http).await,
        harness
            .manager
            .bt
            .status_of(&bt.id, "")
            .await
            .unwrap()
            .downloaded_bytes,
    )
}

async fn transfer_snapshot(
    harness: &Harness,
    received: &received::Received,
    bt: &DownloadTask,
) -> (u64, u64) {
    (
        received.fresh_bytes().await,
        harness
            .manager
            .bt
            .status_of(&bt.id, "")
            .await
            .unwrap()
            .downloaded_bytes,
    )
}

async fn http_bytes(task: &DownloadTask) -> u64 {
    let base = temp_layout::resolve(task).await.unwrap();
    let prefix = format!("{}.part", base.file_name().unwrap().to_string_lossy());
    let mut directory = match fs::read_dir(base.parent().unwrap()).await {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return 0,
        Err(error) => panic!("read actual HTTP shard directory: {error}"),
    };
    let mut total = 0;
    while let Some(entry) = directory.next_entry().await.unwrap() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let legacy = name.strip_prefix(&prefix).is_some_and(|index| {
            !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
        });
        if legacy || is_window_part_name(&name, &prefix) {
            let metadata = fs::symlink_metadata(entry.path()).await.unwrap();
            assert!(metadata.is_file() && !temp_layout::redirected(&metadata));
            total += metadata.len();
        }
    }
    total
}

#[tokio::test]
async fn physical_payload_counter_excludes_validation_and_rejected_sidecars() {
    let root = tempfile::tempdir().unwrap();
    let mut task = crate::bt::tests::bt_task_for_test();
    task.destination = root.path().to_string_lossy().into_owned();
    task.file_name = "verified.bin".into();
    let source = temp_layout::ensure(&task).await.unwrap();
    for (suffix, bytes) in [
        (".part0.w0", 7),
        (".part1", 11),
        (".part0.w0.pending-validation", 98),
        (".part0.w0.pending-staged", 104),
        (".part1.rejected-evidence", 104),
    ] {
        fs::write(
            format!("{}{suffix}", source.to_string_lossy()),
            vec![0u8; bytes],
        )
        .await
        .unwrap();
    }
    assert_eq!(http_bytes(&task).await, 18);
}

async fn assert_not_failed(harness: &Harness, id: &str) {
    let task = harness.row(id).await;
    assert_ne!(task.status, TaskStatus::Failed, "{:?}", task.error);
}
