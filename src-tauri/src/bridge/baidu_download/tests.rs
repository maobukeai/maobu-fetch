use super::*;
use std::net::Ipv4Addr;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const VALID_RESPONSE: &str = "HTTP/1.1 206 Partial Content\r\nContent-Type: application/octet-stream\r\nContent-Range: bytes 0-0/25\r\nContent-Length: 1\r\n\r\nA";

fn parse_request(value: serde_json::Value) -> Request {
    serde_json::from_value(value).unwrap()
}

#[test]
fn signed_body_shape_only_allows_browser_context_headers() {
    let request = parse_request(serde_json::json!({
        "url": "https://download.baidupcs.com/file?signature=synthetic",
        "total_bytes": 25,
        "headers": {
            "Cookie": "sid=synthetic",
            "Referer": "https://pan.baidu.com/s/sample",
            "User-Agent": "SyntheticBrowser/1.0",
            "Accept": "*/*",
            "Accept-Language": "zh-CN",
            "Host": "another.example",
            "Authorization": "synthetic",
            "Proxy-Authorization": "synthetic",
            "Range": "bytes=2-2",
        },
    }));
    assert!(request.url().is_ok());
    let headers = request.headers().unwrap();
    assert_eq!(headers.len(), 5);
    assert_eq!(headers[COOKIE], "sid=synthetic");
    assert_eq!(headers[REFERER], "https://pan.baidu.com/s/sample");
    assert!(headers[COOKIE].is_sensitive());
    assert!(headers[REFERER].is_sensitive());
}

#[test]
fn wrong_site_referer_is_omitted_and_invalid_headers_are_rejected() {
    let request = parse_request(serde_json::json!({
        "url": "https://pan.baidu.com/file",
        "total_bytes": 25,
        "headers": { "Referer": "https://elsewhere.example/private" },
    }));
    assert!(request.headers().unwrap().is_empty());
    for headers in [
        serde_json::json!({ "Cookie": "a=b\r\nHost: elsewhere.example" }),
        serde_json::json!({ "Cookie": "a=b", "cookie": "c=d" }),
    ] {
        let request = parse_request(serde_json::json!({
            "url": "https://pan.baidu.com/file",
            "total_bytes": 25,
            "headers": headers,
        }));
        assert_eq!(request.headers(), Err(INVALID_REQUEST));
    }
}

#[tokio::test]
async fn invalid_input_is_rejected_before_network_requests() {
    let settings = crate::models::AppSettings::default();
    for (url, total_bytes, result) in [
        ("https://pan.baidu.com/file", 0, INVALID_REQUEST),
        (
            "https://pan.baidu.com/file",
            MAX_BROWSER_FILE_SIZE + 1,
            INVALID_REQUEST,
        ),
        ("https://@pan.baidu.com/file", 1, UNSUPPORTED_TARGET),
        (
            "https://user:pass@pan.baidu.com/file",
            1,
            UNSUPPORTED_TARGET,
        ),
        (
            "https://pan.baidu.com.evil.example/file",
            1,
            UNSUPPORTED_TARGET,
        ),
        ("https://pan.baidu.com:444/file", 1, UNSUPPORTED_TARGET),
        ("http://pan.baidu.com/file", 1, UNSUPPORTED_TARGET),
        ("http://127.0.0.1/file", 1, UNSUPPORTED_TARGET),
    ] {
        let body =
            serde_json::to_vec(&serde_json::json!({ "url": url, "total_bytes": total_bytes }))
                .unwrap();
        assert_eq!(verify_download(&settings, &body).await, Err(result));
    }
}

#[tokio::test]
async fn probes_the_final_download_with_an_exact_one_byte_range() {
    let (result, request) = probe_literal(VALID_RESPONSE, 25).await;
    assert_eq!(result, Ok(()));
    assert!(request.starts_with("GET /file?sig=synthetic HTTP/1.1\r\n"));
    assert!(request.contains("\r\nrange: bytes=0-0\r\n"));
    assert!(request.contains("\r\naccept-encoding: identity\r\n"));
    assert!(request.contains("\r\ncookie: sid=synthetic\r\n"));

    let chunked = VALID_RESPONSE.replace(
        "Content-Length: 1\r\n\r\nA",
        "Transfer-Encoding: chunked\r\n\r\n1\r\nA\r\n0\r\n\r\n",
    );
    assert_eq!(probe_literal(&chunked, 25).await.0, Ok(()));
}

#[tokio::test]
async fn refuses_responses_without_evidence_of_the_file() {
    assert_eq!(
        probe_literal(VALID_RESPONSE, 26).await.0,
        Err(UNVERIFIED_FILE)
    );
    for (before, after) in [
        ("206 Partial Content", "200 OK"),
        ("206 Partial Content", "403 Forbidden"),
        ("206 Partial Content", "416 Range Not Satisfiable"),
        ("bytes 0-0/25", "bytes 1-1/25"),
        ("bytes 0-0/25", "bytes 0-0/*"),
        ("Content-Range: bytes 0-0/25\r\n", ""),
        ("Content-Type: application/octet-stream\r\n", ""),
        ("application/octet-stream", "text/html; charset=UTF-8"),
        ("application/octet-stream", "application/baidu+json"),
        (
            "application/octet-stream",
            "multipart/byteranges; boundary=sample",
        ),
        (
            "Content-Length: 1",
            "Content-Encoding: gzip\r\nContent-Length: 1",
        ),
        ("Content-Length: 1", "Content-Length: 2"),
        ("\r\n\r\nA", "\r\n\r\n"),
        (
            "Content-Length: 1\r\n\r\nA",
            "Transfer-Encoding: chunked\r\n\r\n1\r\nA\r\n1\r\nB\r\n0\r\n\r\n",
        ),
    ] {
        let (result, _) = probe_literal(&VALID_RESPONSE.replace(before, after), 25).await;
        assert_eq!(
            result,
            Err(UNVERIFIED_FILE),
            "response mutation: {before:?} → {after:?}"
        );
    }
}

#[tokio::test]
async fn never_follows_redirects_with_browser_credentials() {
    let redirect_target = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let location = format!(
        "http://{}/login?code=synthetic",
        redirect_target.local_addr().unwrap()
    );
    let response =
        format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n");
    assert_eq!(probe_literal(&response, 25).await.0, Err(UNVERIFIED_FILE));
    assert!(
        tokio::time::timeout(Duration::from_millis(25), redirect_target.accept())
            .await
            .is_err()
    );
}

async fn probe_literal(response: &str, total_bytes: u64) -> (Result<(), BridgeError>, String) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let url = Url::parse(&format!(
        "http://{}/file?sig=synthetic",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let response = response.to_owned();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            bytes.push(socket.read_u8().await.unwrap());
        }
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
        std::str::from_utf8(&bytes).unwrap().to_owned()
    });
    let client = Client::builder()
        .no_proxy()
        .no_gzip()
        .no_deflate()
        .no_brotli()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(COOKIE, HeaderValue::from_static("sid=synthetic"));
    let result = probe(&client, url, headers, total_bytes).await;
    let request = tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
    (result, request)
}
