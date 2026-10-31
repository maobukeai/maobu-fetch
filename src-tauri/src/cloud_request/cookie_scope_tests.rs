use super::*;

fn cookie(fixture: &ProxyFixture, path: &str) -> String {
    fixture
        .state
        .headers
        .lock()
        .unwrap()
        .iter()
        .find(|(seen, _)| seen == path)
        .expect("the actual platform phase must run")
        .1
        .get("cookie")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn quark_actual_inspect_rechecks_raw_jar_domain_path_secure_and_expiry_at_each_redirect() {
    for jar in [false, true] {
        let fixture = ProxyFixture::start(Mode::CookieRedirect).await;
        let network = fixture.app_network(CancellationToken::new());
        let credentials = if jar {
            "# Netscape HTTP Cookie File\ndrive.quark.cn\tFALSE\t/1/clouddrive/share\tTRUE\t0\tapi\tsynthetic-api\ndrive.quark.cn\tFALSE\t/redirected\tTRUE\t0\tredirected\tsynthetic-path\ncdn.example.invalid\tFALSE\t/token\tTRUE\t0\tcdn\tsynthetic-cdn\ndrive.quark.cn\tFALSE\t/\tTRUE\t1\texpired\tnever\nforeign.invalid\tFALSE\t/\tFALSE\t0\tforeign\tnever"
        } else {
            "sid=synthetic-flat"
        };
        let listing = crate::quark::inspect_quark_share_with_context(
            "https://pan.quark.cn/s/synthetic",
            None,
            Some(credentials.into()),
            &network,
        )
        .await
        .unwrap();
        assert_eq!(listing.file_count, 1);
        let first = cookie(
            &fixture,
            "/drive.quark.cn/1/clouddrive/share/sharepage/token",
        );
        let next = cookie(&fixture, "/drive.quark.cn/redirected");
        let foreign = cookie(&fixture, "/cdn.example.invalid/token");
        let listing = cookie(
            &fixture,
            "/drive.quark.cn/1/clouddrive/share/sharepage/detail",
        );
        if jar {
            assert_eq!(first, "api=synthetic-api");
            assert_eq!(next, "redirected=synthetic-path");
            assert_eq!(foreign, "cdn=synthetic-cdn");
            assert_eq!(listing, "api=synthetic-api");
        } else {
            assert_eq!(first, "sid=synthetic-flat");
            assert_eq!(next, "sid=synthetic-flat");
            assert!(foreign.is_empty());
            assert_eq!(listing, "sid=synthetic-flat");
        }
    }
}

#[tokio::test]
async fn baidu_pcs_and_quark_pc_keep_only_their_explicit_jar_scope() {
    let fixture = ProxyFixture::start(Mode::BaiduPcs).await;
    let network = fixture.app_network(CancellationToken::new());
    let jar="# Netscape HTTP Cookie File\npan.baidu.com\tFALSE\t/\tTRUE\t0\tBDUSS\tsynthetic-pan\nd.pcs.baidu.com\tFALSE\t/rest\tTRUE\t0\tpcs\tsynthetic-pcs";
    crate::baidupan::resolve_baidu_file_with_context(
        "synthetic",
        "1",
        Some("456"),
        Some("123"),
        None,
        None,
        None,
        Some("synthetic-sign"),
        Some(jar),
        &network,
    )
    .await
    .unwrap();
    assert_eq!(
        cookie(&fixture, "/d.pcs.baidu.com/rest/2.0/pcs/file"),
        "pcs=synthetic-pcs"
    );
    let pan = cookie(&fixture, "/pan.baidu.com/share/transfer");
    assert!(pan.contains("BDUSS=synthetic-pan") && pan.contains("BDCLND=synthetic-sign"));
    assert!(!pan.contains("synthetic-pcs"));
    for jar in [false, true] {
        let fixture = ProxyFixture::start(Mode::Normal).await;
        let network = fixture.app_network(CancellationToken::new());
        let credentials = if jar {
            "# Netscape HTTP Cookie File\ndrive.quark.cn\tFALSE\t/\tTRUE\t0\tsid\tsynthetic-api\ndrive-pc.quark.cn\tFALSE\t/1/clouddrive/file\tTRUE\t0\tpc\tsynthetic-pc"
        } else {
            "sid=synthetic-api"
        };
        crate::quark::resolve_quark_file_with_context(
            "synthetic",
            "1",
            None,
            None,
            Some(credentials),
            &network,
        )
        .await
        .unwrap();
        let pc = cookie(&fixture, "/drive-pc.quark.cn/1/clouddrive/file/download");
        if jar {
            assert_eq!(pc, "pc=synthetic-pc");
        } else {
            assert!(pc.is_empty());
        }
    }
}

#[tokio::test]
async fn pan123_jar_is_not_mistaken_for_a_bearer_token_or_sent_raw() {
    let fixture = ProxyFixture::start(Mode::Normal).await;
    let network = fixture.app_network(CancellationToken::new());
    let jar="# Netscape HTTP Cookie File\nwww.123684.com\tFALSE\t/api/share/download\tTRUE\t0\ttoken\tsynthetic-token\nforeign.invalid\tFALSE\t/\tTRUE\t0\tbad\tnever";
    crate::pan123::resolve_pan123_file_with_context(
        "synthetic",
        1,
        "synthetic",
        3,
        "synthetic",
        None,
        Some(jar),
        &network,
    )
    .await
    .unwrap();
    assert_eq!(
        cookie(&fixture, "/www.123684.com/api/share/download/info"),
        "token=synthetic-token"
    );
    let headers = fixture.state.headers.lock().unwrap();
    let request = &headers
        .iter()
        .find(|(path, _)| path == "/www.123684.com/api/share/download/info")
        .unwrap()
        .1;
    assert_eq!(
        request.get("authorization").unwrap(),
        "Bearer synthetic-token"
    );
}

#[tokio::test]
async fn logical_https_to_http_redirect_strips_secure_cookies_and_authorization_before_test_mapping(
) {
    let fixture = ProxyFixture::start(Mode::Normal).await;
    let network = fixture.app_network(CancellationToken::new());
    let jar="# Netscape HTTP Cookie File\ndrive.quark.cn\tFALSE\t/\tTRUE\t0\tsecure\tsynthetic-secure\ndrive.quark.cn\tFALSE\t/\tFALSE\t0\tplain\tsynthetic-plain";
    network
        .get("https://drive.quark.cn/secure")
        .header("Cookie", jar)
        .header("Authorization", "Bearer synthetic")
        .header("X-Device-Sign", "synthetic-sign")
        .header("X-Captcha-Token", "synthetic-token")
        .send()
        .await
        .unwrap();
    assert_eq!(
        cookie(&fixture, "/drive.quark.cn/secure"),
        "secure=synthetic-secure; plain=synthetic-plain"
    );
    assert_eq!(
        cookie(&fixture, "/drive.quark.cn/plain"),
        "plain=synthetic-plain"
    );
    let seen = fixture.state.headers.lock().unwrap();
    assert!(!seen
        .iter()
        .find(|(path, _)| path == "/drive.quark.cn/plain")
        .unwrap()
        .1
        .contains_key("authorization"));
    let next = &seen
        .iter()
        .find(|(path, _)| path == "/drive.quark.cn/plain")
        .unwrap()
        .1;
    assert!(!next.contains_key("x-device-sign") && !next.contains_key("x-captcha-token"));
}
