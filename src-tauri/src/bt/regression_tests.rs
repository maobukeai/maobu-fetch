use super::{tests::bt_task_for_test, BtEngine};
use crate::models::{AppSettings, CollisionPolicy};
use std::{path::Path, time::Duration};

async fn torrent_bytes(source: &Path) -> Vec<u8> {
    let spawner = librqbit::spawn_utils::BlockingSpawner::new(2);
    librqbit::create_torrent(
        source,
        librqbit::CreateTorrentOptions {
            piece_length: Some(16 * 1024),
            ..Default::default()
        },
        &spawner,
    )
    .await
    .unwrap()
    .as_bytes()
    .unwrap()
    .to_vec()
}

async fn fixture(root: &Path) -> (crate::models::DownloadTask, Vec<u8>) {
    let source = root.join("seed");
    tokio::fs::create_dir(&source).await.unwrap();
    tokio::fs::write(
        source.join("first.bin"),
        (0..65_536).map(|i| (i % 251) as u8).collect::<Vec<_>>(),
    )
    .await
    .unwrap();
    tokio::fs::write(
        source.join("second.bin"),
        (0..65_536).map(|i| (i % 241 + 1) as u8).collect::<Vec<_>>(),
    )
    .await
    .unwrap();
    let bytes = torrent_bytes(&source).await;
    let mut task = bt_task_for_test();
    task.destination = root.join("downloads").to_string_lossy().into_owned();
    task.file_name = "same-name".into();
    task.url = "synthetic.torrent".into();
    use base64::Engine as _;
    task.bt_meta.as_mut().unwrap().torrent_data_base64 =
        Some(base64::engine::general_purpose::STANDARD.encode(&bytes));
    task.bt_meta.as_mut().unwrap().info_hash =
        super::inspect_torrent_bytes(&bytes).unwrap().info_hash;
    super::prepare_task_output(&mut task).await.unwrap();
    (task, bytes)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_engine_selection_ignores_preallocated_file_length() {
    let temp = tempfile::tempdir().unwrap();
    let (mut task, _) = fixture(temp.path()).await;
    let engine = BtEngine::with_data_dir(temp.path().join("session"));
    engine
        .add_task_inner(&AppSettings::default(), &mut task)
        .await
        .unwrap();
    let handle = engine.bindings.lock().await.get(&task.id).unwrap().clone();
    tokio::time::timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .unwrap()
        .unwrap();
    let files = engine.files_of(&task.id, &task.destination).await.unwrap();
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|file| file.downloaded_bytes == 0));
    engine.select_files(&task.id, &[2]).await.unwrap();
    assert_eq!(handle.only_files(), Some(vec![1]));
    let files = engine.files_of(&task.id, &task.destination).await.unwrap();
    assert!(!files[0].selected && files[1].selected);
    assert!(engine.select_files(&task.id, &[]).await.is_err());
    assert!(engine.select_files(&task.id, &[0]).await.is_err());
    assert!(engine.select_files(&task.id, &[3]).await.is_err());
    engine.set_task_limits(&task.id, 1024, 2048).await.unwrap();
    assert_eq!(handle.ratelimits().get_download_bps().unwrap().get(), 1024);
    assert_eq!(handle.ratelimits().get_upload_bps().unwrap().get(), 2048);
    engine.pause_task(&task.id).await.unwrap();
    engine.pause_task(&task.id).await.unwrap();
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_fastresume_restores_paused_and_keeps_data_directories_isolated() {
    let temp = tempfile::tempdir().unwrap();
    let (mut task, _) = fixture(temp.path()).await;
    let output = super::task_output_folder(&task).await.unwrap();
    for name in ["first.bin", "second.bin"] {
        tokio::fs::copy(temp.path().join("seed").join(name), output.join(name))
            .await
            .unwrap();
    }
    task.bt_meta.as_mut().unwrap().owned_files = vec!["first.bin".into(), "second.bin".into()];
    let session_root = temp.path().join("session-a");
    let engine = BtEngine::with_data_dir(session_root.clone());
    engine
        .add_task_inner(&AppSettings::default(), &mut task)
        .await
        .unwrap();
    let handle = engine.bindings.lock().await.get(&task.id).unwrap().clone();
    tokio::time::timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .unwrap()
        .unwrap();
    assert!(handle.stats().finished);
    engine.pause_task(&task.id).await.unwrap();
    engine.shutdown().await.unwrap();
    assert!(session_root.join("session.json").is_file());
    assert!(std::fs::read_dir(&session_root)
        .unwrap()
        .filter_map(Result::ok)
        .any(|f| f.path().extension().is_some_and(|e| e == "bitv")));
    let restored = BtEngine::with_data_dir(session_root);
    let session = restored.get_or_init_session(None).await.unwrap();
    let handles = session.with_torrents(|items| items.map(|(_, h)| h.clone()).collect::<Vec<_>>());
    assert_eq!(handles.len(), 1);
    assert!(handles[0].is_paused(), "恢复会话不得抢先启动下载或做种");
    tokio::time::timeout(Duration::from_secs(10), handles[0].wait_until_initialized())
        .await
        .unwrap()
        .unwrap();
    let files = restored.files_for_task(&task).await.unwrap();
    assert!(files.iter().all(|f| f.downloaded_bytes == f.length_bytes));
    assert_eq!(super::task_output_folder(&task).await.unwrap(), output);
    let isolated = BtEngine::with_data_dir(temp.path().join("session-b"));
    assert_eq!(
        isolated
            .get_or_init_session(None)
            .await
            .unwrap()
            .with_torrents(|items| items.count()),
        0
    );
    isolated.shutdown().await.unwrap();
    restored.shutdown().await.unwrap();
}

#[tokio::test]
async fn collision_and_delete_preserve_unrelated_files_and_only_record_removal() {
    let temp = tempfile::tempdir().unwrap();
    let (mut task, _) = fixture(temp.path()).await;
    let root = super::task_output_folder(&task).await.unwrap();
    tokio::fs::write(root.join("download.bin"), b"owned")
        .await
        .unwrap();
    tokio::fs::write(root.join("personal.txt"), b"unrelated")
        .await
        .unwrap();
    task.bt_meta.as_mut().unwrap().owned_files = vec!["download.bin".into()];
    let mut other = task.clone();
    other.id = "another-task".into();
    other.bt_meta.as_mut().unwrap().output_folder = None;
    other.bt_meta.as_mut().unwrap().owned_files.clear();
    super::prepare_task_output(&mut other).await.unwrap();
    assert_ne!(super::task_output_folder(&other).await.unwrap(), root);
    let engine = BtEngine::with_data_dir(temp.path().join("session"));
    engine.remove_task(&task.id).await.unwrap();
    assert!(
        root.join("download.bin").exists(),
        "仅移除引擎记录不得删除数据"
    );
    super::delete_task_files(&task).await.unwrap();
    assert!(!root.join("download.bin").exists());
    assert_eq!(
        tokio::fs::read(root.join("personal.txt")).await.unwrap(),
        b"unrelated"
    );
    other.collision_policy = CollisionPolicy::Skip;
    other.bt_meta.as_mut().unwrap().output_folder = None;
    assert!(super::prepare_task_output(&mut other).await.is_err());
}

#[tokio::test]
async fn low_rate_reserves_large_blocks_without_capacity_errors() {
    let limits = librqbit::limits::Limits::new(librqbit::limits::LimitsConfig {
        download_bps: std::num::NonZeroU32::new(8 * 1024),
        ..Default::default()
    });
    let start = std::time::Instant::now();
    limits
        .prepare_for_download(std::num::NonZeroU32::new(16 * 1024).unwrap())
        .await
        .unwrap();
    assert!(start.elapsed() >= Duration::from_millis(850));
    limits.set_download_bps(None);
    tokio::time::timeout(
        Duration::from_millis(200),
        limits.prepare_for_download(std::num::NonZeroU32::new(16 * 1024).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
}

#[test]
fn task_speed_limit_is_already_bytes_per_second() {
    let mut task = bt_task_for_test();
    task.per_task_speed_limit = 1000;
    let config = super::limits::task_limits(&task);
    assert_eq!(config.download_bps.unwrap().get(), 1000);
    assert_eq!(config.upload_bps.unwrap().get(), 1000);
    task.per_task_speed_limit = 0;
    let config = super::limits::task_limits(&task);
    assert!(config.download_bps.is_none() && config.upload_bps.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restored_torrent_does_not_require_original_source_file() {
    let temp = tempfile::tempdir().unwrap();
    let (mut task, bytes) = fixture(temp.path()).await;
    let source = temp.path().join("original.torrent");
    tokio::fs::write(&source, bytes).await.unwrap();
    task.url = source.to_string_lossy().into_owned();
    task.bt_meta.as_mut().unwrap().torrent_data_base64 = None;
    let session_root = temp.path().join("session");
    let engine = BtEngine::with_data_dir(session_root.clone());
    engine
        .add_task_inner(&AppSettings::default(), &mut task)
        .await
        .unwrap();
    let handle = engine.bindings.lock().await.get(&task.id).unwrap().clone();
    handle.wait_until_initialized().await.unwrap();
    engine.shutdown().await.unwrap();
    tokio::fs::remove_file(source).await.unwrap();
    let restored = BtEngine::with_data_dir(session_root);
    restored
        .add_task_inner(&AppSettings::default(), &mut task)
        .await
        .unwrap();
    assert_eq!(restored.bindings.lock().await.len(), 1);
    restored.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persistence_failure_is_reported_and_deletion_can_retry_without_losing_data() {
    let temp = tempfile::tempdir().unwrap();
    let (mut task, bytes) = fixture(temp.path()).await;
    let session_root = temp.path().join("session");
    let engine = BtEngine::with_data_dir(session_root.clone());
    let session = engine.get_or_init_session(None).await.unwrap();
    let hash = super::inspect_torrent_bytes(&bytes).unwrap().info_hash;
    let metadata_blocked = session_root.join(format!("{hash}.torrent.tmp"));
    tokio::fs::create_dir(&metadata_blocked).await.unwrap();
    assert!(engine
        .add_task_inner(&AppSettings::default(), &mut task)
        .await
        .is_err());
    assert_eq!(session.with_torrents(|items| items.count()), 0);
    tokio::fs::remove_dir(&metadata_blocked).await.unwrap();
    let blocked = session_root.join("session.json.tmp");
    tokio::fs::create_dir(&blocked).await.unwrap();
    assert!(engine
        .add_task_inner(&AppSettings::default(), &mut task)
        .await
        .is_err());
    assert_eq!(session.with_torrents(|items| items.count()), 0);
    tokio::fs::remove_dir(&blocked).await.unwrap();
    engine
        .add_task_inner(&AppSettings::default(), &mut task)
        .await
        .unwrap();
    let handle = engine.bindings.lock().await.get(&task.id).unwrap().clone();
    handle.wait_until_initialized().await.unwrap();
    tokio::fs::create_dir(&blocked).await.unwrap();
    assert!(engine.pause_task(&task.id).await.is_err());
    assert!(engine.remove_task(&task.id).await.is_err());
    assert_eq!(session.with_torrents(|items| items.count()), 1);
    assert!(engine.bindings.lock().await.contains_key(&task.id));
    tokio::fs::remove_dir(&blocked).await.unwrap();
    engine.remove_task(&task.id).await.unwrap();
    assert_eq!(session.with_torrents(|items| items.count()), 0);
    assert!(super::task_output_folder(&task).await.unwrap().is_dir());
    engine.shutdown().await.unwrap();
    let restored = BtEngine::with_data_dir(session_root);
    assert_eq!(
        restored
            .get_or_init_session(None)
            .await
            .unwrap()
            .with_torrents(|items| items.count()),
        0
    );
    restored.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_seed_recheck_rejects_corruption_without_redownload() {
    let temp = tempfile::tempdir().unwrap();
    let (mut task, _) = fixture(temp.path()).await;
    let output = super::task_output_folder(&task).await.unwrap();
    for name in ["first.bin", "second.bin"] {
        tokio::fs::copy(temp.path().join("seed").join(name), output.join(name))
            .await
            .unwrap();
    }
    task.bt_meta.as_mut().unwrap().owned_files = vec!["first.bin".into(), "second.bin".into()];
    task.status = crate::models::TaskStatus::Completed;
    let engine = BtEngine::with_data_dir(temp.path().join("session"));
    engine
        .prepare_seeding(&AppSettings::default(), &mut task)
        .await
        .unwrap();
    let handle = engine.bindings.lock().await.get(&task.id).unwrap().clone();
    assert!(handle.stats().finished && handle.is_paused());
    let mut corrupt = tokio::fs::read(output.join("first.bin")).await.unwrap();
    corrupt[32768] ^= 0xff;
    tokio::fs::write(output.join("first.bin"), &corrupt)
        .await
        .unwrap();
    assert!(engine
        .prepare_seeding(&AppSettings::default(), &mut task)
        .await
        .is_err());
    assert!(handle.is_paused());
    assert!(!handle.stats().finished);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        tokio::fs::read(output.join("first.bin")).await.unwrap(),
        corrupt
    );
    assert_eq!(engine.active_downloads().await, 0);
    engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn budgets_are_lazy_and_remain_effective_after_settings_refresh() {
    let temp = tempfile::tempdir().unwrap();
    let engine = BtEngine::with_data_dir(temp.path().join("session"));
    engine.set_global_budgets(2048, 1024).await.unwrap();
    assert!(!engine.is_running().await);
    assert!(!temp.path().join("session").exists());
    let settings = AppSettings {
        speed_limit_kbps: 64,
        bt_upload_limit_kbps: 8,
        ..Default::default()
    };
    let session = engine.get_or_init_session(Some(&settings)).await.unwrap();
    assert_eq!(session.ratelimits.get_download_bps().unwrap().get(), 2048);
    assert_eq!(session.ratelimits.get_upload_bps().unwrap().get(), 1024);
    engine.apply_settings(&settings).await.unwrap();
    assert_eq!(session.ratelimits.get_download_bps().unwrap().get(), 2048);
    assert_eq!(session.ratelimits.get_upload_bps().unwrap().get(), 1024);
    engine.set_global_budgets(0, 0).await.unwrap();
    assert_eq!(
        session.ratelimits.get_download_bps().unwrap().get(),
        64 * 1024
    );
    assert_eq!(session.ratelimits.get_upload_bps().unwrap().get(), 8 * 1024);
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clean_fastresume_rejects_sampled_hash_mismatch() {
    let temp = tempfile::tempdir().unwrap();
    let (mut task, _) = fixture(temp.path()).await;
    let output = super::task_output_folder(&task).await.unwrap();
    for name in ["first.bin", "second.bin"] {
        tokio::fs::copy(temp.path().join("seed").join(name), output.join(name))
            .await
            .unwrap();
    }
    task.bt_meta.as_mut().unwrap().owned_files = vec!["first.bin".into(), "second.bin".into()];
    let data = temp.path().join("session");
    let engine = BtEngine::with_data_dir(data.clone());
    engine
        .add_task_inner(&AppSettings::default(), &mut task)
        .await
        .unwrap();
    let handle = engine.bindings.lock().await.get(&task.id).unwrap().clone();
    handle.wait_until_initialized().await.unwrap();
    assert!(handle.stats().finished);
    engine.shutdown().await.unwrap();
    let mut changed = tokio::fs::read(output.join("first.bin")).await.unwrap();
    changed[0] ^= 0xff;
    tokio::fs::write(output.join("first.bin"), changed)
        .await
        .unwrap();
    let restored = BtEngine::with_data_dir(data);
    let session = restored.get_or_init_session(None).await.unwrap();
    let handle = session.with_torrents(|items| items.next().unwrap().1.clone());
    handle.wait_until_initialized().await.unwrap();
    assert!(
        !handle.stats().finished,
        "Ok(false)哈希不匹配必须使fastresume失效"
    );
    assert!(handle.is_paused());
    restored.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offline_record_removal_never_starts_a_session_or_opens_payload_files() {
    let temp = tempfile::tempdir().unwrap();
    let (mut task, _) = fixture(temp.path()).await;
    let data = temp.path().join("session");
    let engine = BtEngine::with_data_dir(data.clone());
    engine
        .add_task_inner(&AppSettings::default(), &mut task)
        .await
        .unwrap();
    let handle = engine.bindings.lock().await.get(&task.id).unwrap().clone();
    handle.wait_until_initialized().await.unwrap();
    engine.shutdown().await.unwrap();
    drop(handle);
    let output = super::task_output_folder(&task).await.unwrap();
    // 原下载目录移走；仅记录删除既不能重新创建它，也不能要求文件存在。
    let moved = temp.path().join("moved-downloads");
    tokio::fs::rename(&output, &moved).await.unwrap();
    let offline = BtEngine::with_data_dir(data.clone());
    offline.remove_task_offline(&task).await.unwrap();
    assert!(!offline.is_running().await);
    assert!(!output.exists());
    assert!(moved.join("first.bin").is_file());
    let json: serde_json::Value =
        serde_json::from_slice(&tokio::fs::read(data.join("session.json")).await.unwrap()).unwrap();
    assert!(json["torrents"].as_object().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rechecked_existing_bytes_are_not_reported_as_network_speed() {
    let temp = tempfile::tempdir().unwrap();
    let (mut task, _) = fixture(temp.path()).await;
    let output = super::task_output_folder(&task).await.unwrap();
    tokio::fs::copy(temp.path().join("seed/first.bin"), output.join("first.bin"))
        .await
        .unwrap();
    let engine = BtEngine::with_data_dir(temp.path().join("session"));
    engine
        .add_task_inner(&AppSettings::default(), &mut task)
        .await
        .unwrap();
    let handle = engine.bindings.lock().await.get(&task.id).unwrap().clone();
    handle.wait_until_initialized().await.unwrap();
    tokio::time::sleep(Duration::from_millis(900)).await;
    let progress = engine.status_of(&task.id, &task.destination).await.unwrap();
    assert!(progress.downloaded_bytes > 0 && progress.downloaded_bytes < progress.total_bytes);
    assert_eq!(
        progress.download_speed, 0,
        "磁盘重检已存在文件不能冒充网络速度"
    );
    engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelling_unstarted_magnet_removes_only_its_owned_empty_directory() {
    let temp = tempfile::tempdir().unwrap();
    let mut task = bt_task_for_test();
    task.destination = temp.path().join("downloads").to_string_lossy().into_owned();
    super::prepare_task_output(&mut task).await.unwrap();
    let root = super::task_output_folder(&task).await.unwrap();
    let engine = BtEngine::with_data_dir(temp.path().join("session"));
    engine.resolve_task_paths(&task).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), engine.remove_task_for(&task))
        .await
        .unwrap()
        .unwrap();
    assert!(!engine.is_running().await);
    super::delete_task_files(&task).await.unwrap();
    assert!(!root.exists());
    task.bt_meta.as_mut().unwrap().output_folder = None;
    super::prepare_task_output(&mut task).await.unwrap();
    let root = super::task_output_folder(&task).await.unwrap();
    tokio::fs::write(root.join("personal.txt"), b"sentinel")
        .await
        .unwrap();
    assert!(super::delete_task_files(&task).await.is_err());
    assert_eq!(
        tokio::fs::read(root.join("personal.txt")).await.unwrap(),
        b"sentinel"
    );
}
