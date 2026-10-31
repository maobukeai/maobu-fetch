use super::super::*;
use super::{fixture::Mode, lifecycle_cases::*, recovery_cases::*, transfer_cases::*};

pub async fn run(app: AppHandle) {
    if std::env::var("MAOBU_TEST_NATIVE_ONLY").as_deref()==Ok("cloud-content") {
        super::cloud_content_cases::run(&app).await; return;
    }
    if std::env::var("MAOBU_TEST_NATIVE_ONLY").as_deref()==Ok("startup-failure") {
        super::startup_cases::run(&app).await; return;
    }
    if std::env::var("MAOBU_TEST_NATIVE_ONLY").as_deref()==Ok("cloud-file") {
        super::cloud_file_case::run(&app).await; return;
    }
    if std::env::var("MAOBU_TEST_NATIVE_ONLY").as_deref()==Ok("public-samples") {
        super::site_cases::public_samples(&app).await; return;
    }
    if std::env::var("MAOBU_TEST_NATIVE_ONLY").as_deref()==Ok("recording-flow") {
        super::recording_cases::run(&app).await; return;
    }
    if std::env::var("MAOBU_TEST_NATIVE_ONLY").as_deref() == Ok("bilibili-sample") {
        super::site_cases::bilibili_sample(&app).await;
        return;
    }
    if std::env::var("MAOBU_TEST_BRIDGE_BROWSER_ONLY").as_deref() == Ok("1") {
        assert_eq!(
            std::env::var("MAOBU_TEST_BRIDGE_BROWSER").as_deref(),
            Ok("1")
        );
        super::bridge_cases::run(&app).await;
        return;
    }
    if std::env::var("MAOBU_TEST_NATIVE_ONLY").ok().as_deref() == Some("range-integrity") {
        super::protocol_cases::run(&app).await;
        return;
    }
    if std::env::var("MAOBU_TEST_NATIVE_ONLY").ok().as_deref() == Some("bt-bandwidth") {
        super::bt_bandwidth::run(&app).await;
        return;
    }
    if std::env::var_os("MAOBU_TEST_EXTERNAL_MEDIA_ONLY").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        assert_eq!(
            std::env::var("MAOBU_TEST_EXTERNAL_MEDIA").as_deref(),
            Ok("1"),
            "tools-only execution requires explicit external-media opt-in"
        );
        opt_in_external_media(&app).await;
        return;
    }
    eprintln!("NATIVE_STAGE start=startup");
    super::startup_cases::run(&app).await;
    eprintln!("NATIVE_STAGE done=startup start=cloud-content");
    super::cloud_content_cases::run(&app).await;
    eprintln!("NATIVE_STAGE done=cloud-content start=creation");
    super::creation_cases::run(&app).await;
    eprintln!("NATIVE_STAGE done=creation start=publication");
    super::publication_cases::run(&app).await;
    eprintln!("NATIVE_STAGE done=publication start=http-core");
    for connections in [1, 4, 8] {
        integral_ranges(&app, connections).await;
    }
    for mode in [Mode::Ignore, Mode::Unknown] {
        fallback_stream(&app, mode).await;
    }
    for mode in [Mode::WrongRange, Mode::Range416] {
        invalid_range(&app, mode).await;
    }
    for mode in [Mode::SilentHeaders, Mode::SilentBody] {
        pause_silent_connection(&app, mode).await;
        timeout_silent_connection(&app, mode).await;
    }
    rapid_pause_resume(&app).await;
    reopen_and_resume(&app).await;
    changed_etag_preserves_data(&app).await;
    completed_temporary_recovers_without_416(&app).await;
    publication_failure_preserves_all_shards(&app).await;
    network_wait_obeys_total_deadline(&app).await;
    eprintln!("NATIVE_STAGE start=http-protocol");
    super::protocol_cases::run(&app).await;
    eprintln!("NATIVE_STAGE done=http-protocol done=http-core");
    if std::env::var("MAOBU_TEST_NATIVE_ONLY").ok().as_deref() == Some("http-core") {
        raw_share_requires_selection_and_never_spawns_children(&app).await;
        return;
    }
    eprintln!("NATIVE_STAGE start=bt-lifecycle");
    bt_lifecycle_and_background_seeding(&app).await;
    eprintln!("NATIVE_STAGE done=bt-lifecycle start=raw-share");
    raw_share_requires_selection_and_never_spawns_children(&app).await;
    eprintln!("NATIVE_STAGE done=raw-share start=mixed-bandwidth");
    super::bt_bandwidth::run(&app).await;
    eprintln!("NATIVE_STAGE done=mixed-bandwidth start=recording");
    super::recording_cases::run(&app).await;
    eprintln!("NATIVE_STAGE done=recording start=external-media");
    opt_in_external_media(&app).await;
    eprintln!("NATIVE_STAGE done=external-media start=browser-bridge");
    super::bridge_cases::run(&app).await;
    eprintln!("NATIVE_STAGE done=browser-bridge done=aggregate");
}
