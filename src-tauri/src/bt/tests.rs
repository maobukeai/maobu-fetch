use super::worker::{apply_metadata_transition, apply_progress};
use super::*;
use crate::models::BtTaskMeta;
use crate::models::{AppSettings, BtFileEntry, DownloadTask, TaskStatus};
use std::{path::PathBuf, sync::Arc};

#[test]
fn enrich_magnet_url_injects_trackers() {
    let mut settings = AppSettings::default();
    settings.bt_extra_trackers = "  https://custom.tracker.org/announce\n# comment\n\n".into();
    let enriched = BtEngine::enrich_magnet_url(
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=test",
        &settings,
    );
    assert!(enriched.contains("tr="));
    assert!(enriched.contains("custom.tracker.org"));
    assert!(enriched.contains("opentrackr.org"));
}

#[test]
fn metadata_transition_fills_name_and_hash_only_once() {
    let mut task = bt_task_for_test();
    let progress = status::BtProgress {
        state: "active".into(),
        total_bytes: 1024,
        downloaded_bytes: 512,
        download_speed: 128,
        info_hash: "ABCDEF0123456789ABCDEF0123456789ABCDEF01".into(),
        display_name: Some("My Torrent".into()),
        files: vec![BtFileEntry {
            index: 1,
            path: "D:/dl/My Torrent/video.mp4".into(),
            length_bytes: 1024,
            selected: true,
            downloaded_bytes: 512,
        }],
        metadata_fetching: false,
        ..Default::default()
    };
    apply_metadata_transition(&mut task, &progress);
    let meta = task.bt_meta.as_ref().unwrap();
    assert!(meta.metadata_ready);
    assert_eq!(meta.info_hash, "abcdef0123456789abcdef0123456789abcdef01");
    assert_eq!(task.file_name, "video.mp4");

    // 第二次调用不得覆盖用户可能修改过的名称。
    task.file_name = "renamed.mp4".into();
    let again = status::BtProgress {
        state: "active".into(),
        total_bytes: 1024,
        info_hash: "abcdef0123456789abcdef0123456789abcdef01".into(),
        display_name: Some("Other".into()),
        metadata_fetching: false,
        ..Default::default()
    };
    apply_metadata_transition(&mut task, &again);
    assert_eq!(task.file_name, "renamed.mp4");
}

#[test]
fn progress_mapping_handles_metadata_phase() {
    let mut task = bt_task_for_test();
    let progress = status::BtProgress {
        state: "active".into(),
        total_bytes: 0,
        downloaded_bytes: 0,
        download_speed: 0,
        metadata_fetching: true,
        ..Default::default()
    };
    apply_progress(&mut task, &progress);
    assert_eq!(task.total_bytes, 0);
    assert_eq!(task.downloaded_bytes, 0);
    let runtime = task.bt_runtime.as_ref().unwrap();
    assert!(runtime.fetching_metadata);
}

#[test]
fn validate_torrent_bytes_rejects_truncated_bencode() {
    assert!(validate_torrent_bytes(b"d8:announce").is_err());
    assert!(validate_torrent_bytes(b"invalid").is_err());
    assert!(validate_torrent_bytes(b"d").is_err());
}

#[test]
fn ratelimits_conversion_works() {
    let mut settings = AppSettings::default();
    settings.speed_limit_kbps = 1000;
    settings.bt_upload_limit_kbps = 500;
    let dl_bps = std::num::NonZeroU32::new(
        (settings.speed_limit_kbps.saturating_mul(1024)).min(u32::MAX as u64) as u32,
    );
    let ul_bps = std::num::NonZeroU32::new(
        (settings.bt_upload_limit_kbps.saturating_mul(1024)).min(u32::MAX as u64) as u32,
    );
    assert_eq!(dl_bps.unwrap().get(), 1000 * 1024);
    assert_eq!(ul_bps.unwrap().get(), 500 * 1024);

    settings.speed_limit_kbps = 0;
    settings.bt_upload_limit_kbps = 0;
    let dl_zero = if settings.speed_limit_kbps > 0 {
        std::num::NonZeroU32::new(
            (settings.speed_limit_kbps.saturating_mul(1024)).min(u32::MAX as u64) as u32,
        )
    } else {
        None
    };
    assert!(dl_zero.is_none());
}

#[test]
fn engine_with_data_dir_stores_correctly() {
    let dir = PathBuf::from("test/data/dir");
    let engine = BtEngine::with_data_dir(dir.clone());
    assert_eq!(engine.resolve_data_dir(), dir);
}

#[tokio::test]
async fn engine_apply_settings_with_no_session_succeeds() {
    let engine = BtEngine::new();
    let mut settings = AppSettings::default();
    settings.speed_limit_kbps = 500;
    settings.bt_upload_limit_kbps = 200;
    assert!(engine.apply_settings(&settings).await.is_ok());
}

#[tokio::test]
async fn engine_preserves_settings_applied_before_session() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = BtEngine::with_data_dir(tmp.path().to_path_buf());
    let mut settings = AppSettings::default();
    settings.speed_limit_kbps = 800;
    settings.bt_upload_limit_kbps = 400;
    assert!(engine.apply_settings(&settings).await.is_ok());
    assert_eq!(
        engine
            .last_settings
            .lock()
            .await
            .as_ref()
            .unwrap()
            .speed_limit_kbps,
        800
    );
    assert_eq!(
        engine
            .last_settings
            .lock()
            .await
            .as_ref()
            .unwrap()
            .bt_upload_limit_kbps,
        400
    );
}

#[tokio::test]
async fn engine_inspect_magnet_respects_settings_applied_before() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = BtEngine::with_data_dir(tmp.path().to_path_buf());
    let mut settings = AppSettings::default();
    settings.speed_limit_kbps = 600;
    settings.bt_upload_limit_kbps = 300;
    assert!(engine.apply_settings(&settings).await.is_ok());
    assert_eq!(
        engine
            .last_settings
            .lock()
            .await
            .as_ref()
            .unwrap()
            .speed_limit_kbps,
        600
    );
    assert_eq!(
        engine
            .last_settings
            .lock()
            .await
            .as_ref()
            .unwrap()
            .bt_upload_limit_kbps,
        300
    );
}

#[tokio::test]
async fn engine_apply_settings_updates_live_session_ratelimits() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = BtEngine::with_data_dir(tmp.path().to_path_buf());
    let session = engine.get_or_init_session(None).await.unwrap();

    // 初始状态（无设置）未限速
    assert_eq!(session.ratelimits.get_download_bps(), None);
    assert_eq!(session.ratelimits.get_upload_bps(), None);

    // 应用限速设置
    let mut settings = AppSettings::default();
    settings.speed_limit_kbps = 1500;
    settings.bt_upload_limit_kbps = 750;
    engine.apply_settings(&settings).await.unwrap();

    // 验证 live session 的 ratelimits 真实生效
    assert_eq!(
        session.ratelimits.get_download_bps().map(|v| v.get()),
        Some(1500 * 1024)
    );
    assert_eq!(
        session.ratelimits.get_upload_bps().map(|v| v.get()),
        Some(750 * 1024)
    );

    // 验证将限速设置为 0 时取消限速 (None)
    settings.speed_limit_kbps = 0;
    settings.bt_upload_limit_kbps = 0;
    engine.apply_settings(&settings).await.unwrap();

    assert_eq!(session.ratelimits.get_download_bps(), None);
    assert_eq!(session.ratelimits.get_upload_bps(), None);
}

#[tokio::test]
async fn engine_ensure_started_updates_existing_session_ratelimits() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = BtEngine::with_data_dir(tmp.path().to_path_buf());
    let mut settings = AppSettings::default();
    settings.speed_limit_kbps = 500;
    settings.bt_upload_limit_kbps = 250;
    let session1 = engine.get_or_init_session(Some(&settings)).await.unwrap();

    assert_eq!(
        session1.ratelimits.get_download_bps().map(|v| v.get()),
        Some(500 * 1024)
    );
    assert_eq!(
        session1.ratelimits.get_upload_bps().map(|v| v.get()),
        Some(250 * 1024)
    );

    // 重新调用带有新设置的 session 获取
    settings.speed_limit_kbps = 2000;
    settings.bt_upload_limit_kbps = 1000;
    let session2 = engine.get_or_init_session(Some(&settings)).await.unwrap();

    assert!(Arc::ptr_eq(&session1, &session2));
    assert_eq!(
        session2.ratelimits.get_download_bps().map(|v| v.get()),
        Some(2000 * 1024)
    );
    assert_eq!(
        session2.ratelimits.get_upload_bps().map(|v| v.get()),
        Some(1000 * 1024)
    );
}

pub(crate) fn bt_task_for_test() -> DownloadTask {
    DownloadTask {
        id: "t1".into(),
        url: "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567".into(),
        file_name: String::new(),
        destination: "D:/dl".into(),
        total_bytes: 0,
        downloaded_bytes: 0,
        speed: 0,
        eta_seconds: None,
        status: TaskStatus::Downloading,
        error: None,
        created_at: 0,
        completed_at: None,
        scheduled_at: None,
        category: "other".into(),
        queue_position: 0,
        priority: 0,
        retry_count: 0,
        max_retries: 3,
        checksum_sha256: None,
        expected_checksum: None,
        source: "desktop".into(),
        etag: None,
        last_modified: None,
        final_url: None,
        response_status: None,
        content_type: None,
        accepts_ranges: None,
        headers: Default::default(),
        media: None,
        per_task_speed_limit: 0,
        collision_policy: Default::default(),
        completion_action: Default::default(),
        connection_count: 8,
        active_connections: 0,
        segments: Vec::new(),
        retry_policy_override: None,
        proxy_override: None,
        proxy_auth: None,
        task_kind: crate::models::TaskKind::Bt,
        bt_meta: Some(BtTaskMeta {
            info_hash: "0123456789abcdef0123456789abcdef01234567".into(),
            selected_files: Vec::new(),
            display_name: None,
            metadata_ready: false,
            torrent_data_base64: None,
            streaming_priority: false,
            ..Default::default()
        }),
        bt_runtime: None,
        cloud_refresh: None,
        method: None,
        body: None,
    }
}
