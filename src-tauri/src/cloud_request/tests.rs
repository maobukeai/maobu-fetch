use super::*;
#[path = "cookie_scope_tests.rs"]
mod cookie_scope_tests;
#[path = "fixture.rs"]
mod fixture;
#[path = "replay_tests.rs"]
mod replay_tests;
use fixture::{Mode, ProxyFixture};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

#[tokio::test]
async fn five_cloud_platforms_all_phases_use_the_same_authenticated_proxy() {
    let fixture = ProxyFixture::start(Mode::Normal).await;
    let context = fixture.app_network(CancellationToken::new());
    let baidu = crate::baidupan::inspect_baidu_share_with_context(
        "https://pan.baidu.com/s/1synthetic",
        Some("synthetic-password"),
        Some("BDUSS=synthetic-cookie"),
        &context,
    )
    .await
    .unwrap();
    assert_eq!(baidu.file_count, 1);
    let direct = crate::baidupan::resolve_baidu_file_with_context(
        &baidu.surl,
        "1",
        baidu.share_id.as_deref(),
        baidu.uk.as_deref(),
        baidu.sign.as_deref(),
        baidu.timestamp,
        baidu.seckey.as_deref(),
        baidu.randsk.as_deref(),
        Some("BDUSS=synthetic-cookie"),
        &context,
    )
    .await
    .unwrap();
    assert!(direct.url.starts_with("https://download.example.invalid/"));
    let quark = crate::quark::inspect_quark_share_with_context(
        "https://pan.quark.cn/s/synthetic",
        None,
        Some("synthetic-cookie".into()),
        &context,
    )
    .await
    .unwrap();
    assert_eq!(quark.file_count, 1);
    crate::quark::resolve_quark_file_with_context(
        "synthetic",
        "1",
        Some("synthetic-sign"),
        quark.stoken.as_deref(),
        Some("synthetic-cookie"),
        &context,
    )
    .await
    .unwrap();
    let pikpak = crate::pikpak::inspect_pikpak_share_with_context(
        "https://mypikpak.com/s/synthetic",
        None,
        "synthetic-device",
        &context,
    )
    .await
    .unwrap();
    assert_eq!(pikpak.file_count, 1);
    crate::pikpak::resolve_pikpak_file_with_context(
        "synthetic",
        "1",
        None,
        "synthetic-device",
        &context,
    )
    .await
    .unwrap();
    let pan123 = crate::pan123::inspect_pan123_share_with_context(
        "https://www.123pan.com/s/synthetic",
        None,
        &context,
    )
    .await
    .unwrap();
    assert_eq!(pan123.files.len(), 1);
    crate::pan123::resolve_pan123_file_with_context(
        "synthetic",
        1,
        "synthetic",
        3,
        "synthetic",
        None,
        Some("synthetic-cookie=value"),
        &context,
    )
    .await
    .unwrap();
    let lanzou = crate::lanzou::inspect_lanzou_share_with_context(
        "https://wwfixture.lanzouq.com/synthetic",
        None,
        &context,
    )
    .await
    .unwrap();
    assert_eq!(lanzou.files.len(), 1);
    crate::lanzou::resolve_lanzou_file_with_context(
        "https://wwfixture.lanzouq.com/synthetic",
        "synthetic",
        None,
        &context,
    )
    .await
    .unwrap();
    for phase in [
        "/pan.baidu.com/share/verify",
        "/pan.baidu.com/s/1synthetic",
        "/pan.baidu.com/share/list",
        "/pan.baidu.com/api/sharedownload",
        "/drive.quark.cn/1/clouddrive/share/sharepage/token",
        "/drive.quark.cn/1/clouddrive/share/sharepage/detail",
        "/drive-pc.quark.cn/1/clouddrive/file/download",
        "/user.mypikpak.com/v1/shield/captcha/init",
        "/api-drive.mypikpak.com/drive/v1/share",
        "/api-drive.mypikpak.com/drive/v1/share/file_info",
        "/www.123684.com/api/share/get",
        "/www.123684.com/api/share/download/info",
        "/wwfixture.lanzouq.com/synthetic",
        "/wwfixture.lanzouq.com/fn",
        "/wwfixture.lanzouq.com/ajaxm.php",
        "/cdn.example.invalid/file/synthetic",
    ] {
        fixture.require(phase);
    }
    assert_eq!(fixture.state.rejected.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn cloud_factory_honors_task_proxy_auth_direct_override_and_private_cdn_rules() {
    let global = ProxyFixture::start(Mode::Normal).await;
    let task_proxy = ProxyFixture::start(Mode::Normal).await;
    let mut settings = global.settings();
    let mut task:crate::models::DownloadTask=serde_json::from_value(serde_json::json!({
        "id":"cloud-network-fixture","url":"http://127.0.0.1/private-cdn","file_name":"fixture.bin","destination":"C:/fixture", "total_bytes":0,"downloaded_bytes":0,"speed":0,"status":"queued","created_at":0,"category":"other","queue_position":0,"priority":0,"retry_count":0,"max_retries":0,"source":"desktop","headers":{},"per_task_speed_limit":0,"collision_policy":"rename","connection_count":1
    })).unwrap();
    // A private CDN inherited from the prior transfer must not disable the proxy for public cloud APIs.
    task.final_url = Some("http://127.0.0.1/private-cdn".into());
    let network =
        crate::manager::cloud_request_context(&settings, Some(&task), CancellationToken::new())
            .unwrap()
            .with_test_endpoint("http://cloud-fixture.invalid");
    crate::pan123::inspect_pan123_share_with_context(
        "https://123pan.com/s/synthetic",
        None,
        &network,
    )
    .await
    .unwrap();
    global.require("/www.123684.com/api/share/get");
    let global_hits = global.state.seen.lock().unwrap().values().sum::<usize>();
    settings.proxy_password = "incorrect-global-password".into();
    task.proxy_override = Some(task_proxy.address.clone());
    task.proxy_auth = Some(crate::models::ProxyAuth {
        username: "fixture-user".into(),
        password: "fixture-password".into(),
    });
    let network =
        crate::manager::cloud_request_context(&settings, Some(&task), CancellationToken::new())
            .unwrap()
            .with_test_endpoint("http://cloud-fixture.invalid");
    crate::pan123::inspect_pan123_share_with_context(
        "https://123pan.com/s/synthetic",
        None,
        &network,
    )
    .await
    .unwrap();
    task_proxy.require("/www.123684.com/api/share/get");
    assert_eq!(
        global.state.seen.lock().unwrap().values().sum::<usize>(),
        global_hits
    );
    let origin = ProxyFixture::start(Mode::DirectOrigin).await;
    task.url = "https://123pan.com/s/synthetic".into();
    task.final_url = None;
    task.proxy_override = Some(String::new());
    let network =
        crate::manager::cloud_request_context(&settings, Some(&task), CancellationToken::new())
            .unwrap()
            .with_test_endpoint(&origin.address);
    crate::pan123::inspect_pan123_share_with_context(
        "https://123pan.com/s/synthetic",
        None,
        &network,
    )
    .await
    .unwrap();
    origin.require("/www.123684.com/api/share/get");
    assert_eq!(global.state.rejected.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn pcs_no_redirect_and_quark_save_poll_paths_keep_proxy_authentication() {
    let fixture = ProxyFixture::start(Mode::BaiduPcs).await;
    let context = fixture.network(CancellationToken::new(), "fixture-password");
    crate::baidupan::resolve_baidu_file_with_context(
        "synthetic",
        "1",
        Some("456"),
        Some("123"),
        None,
        None,
        None,
        None,
        Some("BDUSS=synthetic-cookie"),
        &context,
    )
    .await
    .unwrap();
    fixture.require("/pan.baidu.com/share/transfer");
    fixture.require("/d.pcs.baidu.com/rest/2.0/pcs/file");
    assert_eq!(fixture.state.rejected.load(Ordering::Acquire), 0);
    let fixture = ProxyFixture::start(Mode::QuarkSaved).await;
    let context = fixture.network(CancellationToken::new(), "fixture-password");
    crate::quark::resolve_quark_file_with_context(
        "synthetic",
        "1",
        Some("synthetic-sign"),
        None,
        Some("synthetic-cookie"),
        &context,
    )
    .await
    .unwrap();
    fixture.require("/drive.quark.cn/1/clouddrive/share/sharepage/token");
    fixture.require("/drive.quark.cn/1/clouddrive/share/sharepage/save");
    fixture.require("/drive.quark.cn/1/clouddrive/task");
    assert_eq!(fixture.state.rejected.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn wrong_proxy_credentials_cannot_succeed_or_fall_back_to_another_exit() {
    let fixture = ProxyFixture::start(Mode::Normal).await;
    let context = fixture.network(CancellationToken::new(), "incorrect-synthetic-password");
    let result = crate::pan123::inspect_pan123_share_with_context(
        "https://www.123pan.com/s/synthetic",
        None,
        &context,
    )
    .await;
    assert!(result.is_err());
    assert!(fixture.state.rejected.load(Ordering::Acquire) > 0);
    assert!(fixture.state.seen.lock().unwrap().is_empty());
    let error = result.err().unwrap();
    assert!(!error.contains("incorrect-synthetic-password") && !error.contains("fixture-password"));
}

#[tokio::test]
async fn cancellation_interrupts_initial_headers_and_body_for_all_five_platforms() {
    for mode in [Mode::SilentHeaders, Mode::SilentBody] {
        for platform in 0..5 {
            let fixture = ProxyFixture::start(mode).await;
            let cancel = CancellationToken::new();
            let context = fixture.network(cancel.clone(), "fixture-password");
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(30)).await;
                cancel.cancel();
            });
            let started = Instant::now();
            let result = match platform {
                0 => crate::baidupan::inspect_baidu_share_with_context(
                    "https://pan.baidu.com/s/1synthetic",
                    None,
                    None,
                    &context,
                )
                .await
                .map(|_| ()),
                1 => crate::quark::inspect_quark_share_with_context(
                    "https://pan.quark.cn/s/synthetic",
                    None,
                    None,
                    &context,
                )
                .await
                .map(|_| ()),
                2 => crate::pikpak::inspect_pikpak_share_with_context(
                    "https://mypikpak.com/s/synthetic",
                    None,
                    "synthetic-device",
                    &context,
                )
                .await
                .map(|_| ()),
                3 => crate::pan123::inspect_pan123_share_with_context(
                    "https://123pan.com/s/synthetic",
                    None,
                    &context,
                )
                .await
                .map(|_| ()),
                _ => crate::lanzou::inspect_lanzou_share_with_context(
                    "https://wwfixture.lanzouq.com/synthetic",
                    None,
                    &context,
                )
                .await
                .map(|_| ()),
            };
            assert!(result.unwrap_err().contains("取消"));
            assert!(started.elapsed() < Duration::from_millis(500));
        }
    }
}
