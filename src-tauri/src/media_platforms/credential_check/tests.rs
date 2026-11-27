use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[tokio::test]
async fn injected_client_sends_only_target_scoped_cookie_and_never_follows_redirect() {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut byte = [0; 1];
        while !bytes.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            bytes.push(byte[0]);
        }
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.to_lowercase().contains("cookie: local=synthetic"));
        assert!(!text.contains("foreign=private"));
        stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://unrelated.invalid/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
    });
    let jar="# Netscape HTTP Cookie File\n127.0.0.1\tFALSE\t/\tFALSE\t0\tlocal\tsynthetic\nx.com\tFALSE\t/\tTRUE\t0\tforeign\tprivate\n";
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let result = check_target(
        "x.com",
        Some(jar),
        None,
        None,
        &client,
        &format!("http://{address}/"),
    )
    .await
    .unwrap();
    assert!(!result.valid);
    assert!(result.message.contains("重定向"));
    server.await.unwrap();
}
#[test]
fn cookie_names_and_status_200_do_not_forge_authenticated_identity() {
    for platform in [
        MediaPlatform::Bilibili,
        MediaPlatform::Douyin,
        MediaPlatform::Twitter,
        MediaPlatform::BaiduPan,
    ] {
        assert!(!response::classify(platform, b"{}").unwrap().0);
    }
    assert!(
        !response::classify(MediaPlatform::YouTube, b"SID LOGIN_INFO SAPISID")
            .unwrap()
            .0
    );
    assert!(
        response::classify(MediaPlatform::YouTube, b"\"LOGGED_IN\":true")
            .unwrap()
            .0
    );
    assert!(
        response::classify(
            MediaPlatform::Bilibili,
            br#"{"code":0,"data":{"isLogin":true}}"#
        )
        .unwrap()
        .0
    );
    assert!(target("x.com/path").is_err());
    assert!(target("user@x.com").is_err());
}

#[tokio::test]
async fn connection_failure_is_distinct_from_rejected_credentials() {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let error = check_target(
        "x.com",
        Some("session=synthetic"),
        None,
        None,
        &client,
        &format!("http://{address}/"),
    )
    .await
    .unwrap_err();
    assert!(error.contains("网络不可达"));
    assert!(error.contains("尚未判定"));
    assert!(!error.contains("synthetic"));
}

#[tokio::test]
async fn baidu_check_diagnoses_missing_bduss_and_http_400_errno_minus6() {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let missing_bduss = check_target(
        "pan.baidu.com",
        Some("BAIDUID=12345"),
        None,
        None,
        &client,
        "http://127.0.0.1:9/",
    )
    .await
    .unwrap();
    assert!(!missing_bduss.valid);
    assert!(missing_bduss.message.contains("BDUSS"));

    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut byte = [0; 1];
        while !bytes.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            bytes.push(byte[0]);
        }
        let text = String::from_utf8(bytes).unwrap().to_lowercase();
        assert!(text.contains("referer: https://pan.baidu.com/disk/home"));
        assert!(text.contains("cookie: bduss=expired"));
        let body = r#"{"errmsg":"用户未登录","errno":-6}"#;
        let resp = format!(
            "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(resp.as_bytes()).await.unwrap();
    });
    let expired = check_target(
        "pan.baidu.com",
        Some("BDUSS=expired"),
        None,
        None,
        &client,
        &format!("http://{address}/"),
    )
    .await
    .unwrap();
    assert!(!expired.valid);
    assert!(expired.message.contains("errno -6"));
    assert!(expired.message.contains("同步到媒体凭证"));
    server.await.unwrap();
}
