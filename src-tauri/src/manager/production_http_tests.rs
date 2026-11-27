//! Windows native-context tests of the actual manager, database and HTTP workers.
//! No windows, single-instance plugin, bridge listener or real user data are created.
use super::*;
mod bridge_cases;
mod bt_bandwidth;
mod creation_cases;
mod fixture;
mod harness;
mod lifecycle_cases;
mod protocol_cases;
mod publication_cases;
mod recovery_cases;
mod recording_fixture;
mod recording_cases;
mod scenarios;
mod site_cases;
mod startup_cases;
mod cloud_file_case;
mod cloud_content_cases;
mod cloud_content_fixture;
mod support;
mod transfer_cases;

#[test]
fn native_manager_http_downloads_are_integral_resumable_and_cancellable() {
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier = format!("app.maobu.test.{}", Uuid::new_v4().simple());
    let app = tauri::Builder::default()
        .any_thread()
        .build(context)
        .expect("build isolated Wry context without any visible window");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    // Keep the aggregate on the heap instead of moving its large, ungrouped
    // future through block_on's test-thread stack. The runtime/thread stack
    // configuration and every scenario remain unchanged.
    let scenarios = Box::pin(scenarios::run(app.handle().clone()));
    eprintln!(
        "NATIVE_MANAGER_FUTURE ungrouped_bytes={}",
        std::mem::size_of_val(scenarios.as_ref().get_ref())
    );
    if std::env::var("MAOBU_TEST_NATIVE_ONLY").as_deref() == Ok("future-size") {
        eprintln!("NATIVE_MANAGER_FUTURE diagnostic_only=true scenarios_executed=false");
        return;
    }
    runtime.block_on(scenarios);
}

#[test]
#[ignore = "显式授权的百度网盘端到端真实下载测试；仅读取环境变量，不读取或输出任何个人凭证"]
fn live_baidu_share_downloads_end_to_end() {
    let share_url = std::env::var("MAOBU_TEST_BAIDU_SHARE").expect("请设置 MAOBU_TEST_BAIDU_SHARE");
    let cookie = std::env::var("MAOBU_TEST_BAIDU_COOKIE").expect("请设置 MAOBU_TEST_BAIDU_COOKIE");

    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier = format!("app.maobu.baidu.live.{}", Uuid::new_v4().simple());
    let app = tauri::Builder::default()
        .any_thread()
        .build(context)
        .expect("build isolated Wry context");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async move {
        let harness = harness::Harness::new(app.handle(), Some(240)).await;
        let mut settings = harness.manager.settings().await;
        settings.connections_per_download = 16;
        settings.default_retry_policy.connection_timeout_secs = 20;
        settings.default_retry_policy.max_retries = 2;
        harness.manager.save_settings(settings).await.unwrap();

        let info = crate::baidupan::inspect_baidu_share(&share_url, None, Some(&cookie))
            .await
            .expect("inspect_baidu_share failed");
        let file = info
            .files
            .iter()
            .find(|f| f.kind == "drive#file")
            .expect("share must contain at least one file");
        let direct = crate::baidupan::resolve_baidu_file(
            &info.surl,
            &file.id,
            info.share_id.as_deref(),
            info.uk.as_deref(),
            info.sign.as_deref(),
            info.timestamp,
            info.seckey.as_deref(),
            info.randsk.as_deref(),
            Some(&cookie),
        )
        .await
        .expect("resolve_baidu_file failed");

        let parsed_direct = url::Url::parse(&direct.url).ok();
        eprintln!(
            "BAIDU_LIVE_DIRECT host={:?} path={:?} has_cookie={} ua={:?} referer={:?}",
            parsed_direct.as_ref().and_then(|u| u.host_str()),
            parsed_direct.as_ref().map(|u| u.path()),
            direct.headers.keys().any(|k| k.eq_ignore_ascii_case("cookie")),
            direct.headers.get("User-Agent"),
            direct.headers.get("Referer"),
        );

        let mut task = super::tests::test_task(
            &harness.output,
            &file.name,
            CollisionPolicy::Rename,
        );
        task.id = Uuid::new_v4().to_string();
        task.url = direct.url;
        task.headers = direct.headers;
        task.connection_count = 16;
        task.max_retries = 2;
        task.created_at = now();
        task.proxy_override = None;
        harness
            .manager
            .store
            .create_task_with_name_intent(&task, true)
            .await
            .unwrap();
        harness.manager.dispatch_once().await.unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
        let final_row = loop {
            let row = harness.row(&task.id).await;
            let final_host = row.final_url.as_deref().and_then(|u| url::Url::parse(u).ok()).and_then(|u| u.host_str().map(str::to_owned));
            eprintln!(
                "BAIDU_LIVE status={:?} final_host={:?} bytes={}/{} conns={} speed={}",
                row.status, final_host, row.downloaded_bytes, row.total_bytes, row.active_connections, row.speed
            );
            if row.status == TaskStatus::Completed {
                break row;
            }
            assert!(
                !matches!(row.status, TaskStatus::Failed | TaskStatus::RemoteChanged | TaskStatus::Cancelled),
                "baidu download failed: {:?} err={:?}",
                row.status,
                row.error.as_deref().map(redact_sensitive)
            );
            assert!(tokio::time::Instant::now() < deadline, "baidu live download timed out after 180s");
            tokio::time::sleep(Duration::from_secs(2)).await;
        };
        let out_path = Path::new(&final_row.destination).join(&final_row.file_name);
        let meta = fs::metadata(&out_path).await.expect("completed output file must exist");
        assert!(meta.len() > 0);
        assert_eq!(meta.len(), final_row.total_bytes);
        eprintln!(
            "BAIDU_LIVE_COMPLETE file={} size={}",
            final_row.file_name,
            meta.len()
        );
    });
}
