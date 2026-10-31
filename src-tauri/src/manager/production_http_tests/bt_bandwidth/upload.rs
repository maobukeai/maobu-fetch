use super::*;

pub(super) async fn run(app: &AppHandle) {
    eprintln!("native manager: measured BT upload to an isolated real leecher");
    let harness = Harness::new(app, None).await;
    let torrent = LocalTorrent::new(&harness.data.join("upload-torrent")).await;
    torrent.advertise(Vec::new());
    let mut settings = harness.manager.settings().await;
    settings.speed_limit_kbps = 128;
    settings.bt_upload_limit_kbps = 0;
    settings.bt_seed_enabled = false;
    settings.bt_seed_ratio = 0.0;
    harness
        .manager
        .save_settings(settings.clone())
        .await
        .unwrap();
    let task = add_bt(&harness, &torrent, 64 * 1024).await;
    let output = crate::bt::task_output_folder(&task).await.unwrap();
    fs::copy(&torrent.source_file, output.join("payload.bin"))
        .await
        .unwrap();
    harness.manager.dispatch_once().await.unwrap();
    harness.wait(&task.id, TaskStatus::Completed).await;
    assert!(
        !harness
            .manager
            .bt
            .status_of(&task.id, "")
            .await
            .unwrap()
            .seeder
    );
    let leecher = torrent
        .leecher(&harness.data.join("leecher"), 4 * 1024 * 1024)
        .await;
    torrent.advertise(vec![leecher.session.listen_addr().unwrap()]);
    settings.bt_seed_enabled = true;
    harness
        .manager
        .save_settings(settings.clone())
        .await
        .unwrap();
    crate::bt::request_seeding(&harness.manager, &task.id, true)
        .await
        .unwrap();
    wait_progress("limited upload warm", || async {
        leecher.verified_bytes() >= 128 * 1024
    })
    .await;
    measure(&leecher, "upload_task_64k_global_128k", 64 * 1024).await;
    switch_off(&harness, &task.id, &leecher, &mut settings).await;
    harness
        .manager
        .update_task_options(&task.id, None, Some(0), None)
        .await
        .unwrap();
    settings.bt_seed_enabled = true;
    harness
        .manager
        .save_settings(settings.clone())
        .await
        .unwrap();
    let before = leecher.verified_bytes();
    wait_progress("global-only upload resumes", || async {
        leecher.verified_bytes() > before + 128 * 1024
    })
    .await;
    measure(&leecher, "upload_task_unlimited_global_128k", 128 * 1024).await;
    unlimited_cancel_resume(&harness, &torrent, &task.id, &leecher, settings).await;
    assert_eq!(
        torrent.source.with_torrents(|items| items
            .map(|(_, handle)| handle.stats().uploaded_bytes)
            .sum::<u64>()),
        0,
        "leecher must receive only from the application's managed seed"
    );
    crate::bt::request_seeding(&harness.manager, &task.id, false)
        .await
        .unwrap();
    assert_completed(&harness, &task.id).await;
    crate::bt::history_tests::clear_completed_seed(&harness.manager, &task.id, &harness.output)
        .await;
    harness.manager.shutdown_bt().await.unwrap();
    leecher.session.stop().await;
    torrent.source.stop().await;
}

async fn measure(leecher: &LocalLeecher, label: &str, expected: u64) {
    tokio::time::sleep(Duration::from_secs(1)).await;
    let before = leecher.verified_bytes();
    let clock = std::time::Instant::now();
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_rate(
        label,
        leecher.verified_bytes() - before,
        clock.elapsed(),
        expected,
        64 * 1024,
    );
}

async fn switch_off(
    harness: &Harness,
    id: &str,
    leecher: &LocalLeecher,
    settings: &mut AppSettings,
) {
    settings.bt_seed_enabled = false;
    harness
        .manager
        .save_settings(settings.clone())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let stopped = leecher.verified_bytes();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        leecher.verified_bytes(),
        stopped,
        "global seed-off must stop actual upload payload"
    );
    assert!(!harness.manager.bt.status_of(id, "").await.unwrap().seeder);
    assert_completed(harness, id).await;
}

async fn unlimited_cancel_resume(
    harness: &Harness,
    torrent: &LocalTorrent,
    id: &str,
    leecher: &LocalLeecher,
    mut settings: AppSettings,
) {
    let before = leecher.verified_bytes();
    settings.speed_limit_kbps = 0;
    harness.manager.save_settings(settings).await.unwrap();
    wait_progress("unlimited actual payload", || async {
        leecher.verified_bytes() >= before + 1024 * 1024
    })
    .await;
    assert!(
        leecher.verified_bytes() < torrent.expected.len() as u64,
        "cancellation must occur during an actual unfinished transfer"
    );
    crate::bt::request_seeding(&harness.manager, id, false)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let stopped = leecher.verified_bytes();
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(
        stopped,
        leecher.verified_bytes(),
        "cancel at 0/unlimited must stop upload too"
    );
    let row = assert_completed(harness, id).await;
    assert!(!row.bt_meta.as_ref().unwrap().seeding_requested);
    let start = leecher.verified_bytes();
    let clock = std::time::Instant::now();
    crate::bt::request_seeding(&harness.manager, id, true)
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(12),
        leecher.handle.wait_until_completed(),
    )
    .await
    .unwrap()
    .unwrap();
    let elapsed = clock.elapsed();
    let delta = leecher.verified_bytes() - start;
    eprintln!("NATIVE_BT_RATE label=upload_zero_unlimited_resumed bytes={delta} duration_ms={} measured_bps={:.0}", elapsed.as_millis(), delta as f64 / elapsed.as_secs_f64());
    assert!(
        delta as f64 / elapsed.as_secs_f64() > 128.0 * 1024.0 * 2.0,
        "unlimited must actually release the previous global upload cap"
    );
    let bytes = fs::read(&leecher.file).await.unwrap();
    assert_eq!(bytes.len(), torrent.expected.len());
    assert_eq!(Sha256::digest(bytes), Sha256::digest(&torrent.expected));
}
