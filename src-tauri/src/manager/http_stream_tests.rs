use super::*;
use super::{http_response::{response_plan, send_with_cancel}, http_transfer::{receive_response, StreamObserver}};
use axum::{body::Body, extract::Path as RoutePath, http::{HeaderMap, StatusCode}, response::Response, routing::get, Router};
use std::convert::Infallible;

const DATA: &[u8] = b"verified production stream fixture";

struct Observer { cancel: Option<CancellationToken> }
impl StreamObserver for Observer {
    async fn before_write(&mut self, _: u64) -> Result<(), String> { Ok(()) }
    async fn progress(&mut self, _: u64) -> Result<(), String> {
        if let Some(token) = &self.cancel { token.cancel(); }
        Ok(())
    }
}

async fn fixture(RoutePath(mode): RoutePath<String>, headers: HeaderMap) -> Response {
    if mode == "unknown" {
        return Response::builder().status(200).body(Body::from_stream(futures_util::stream::iter([
            Ok::<_, Infallible>(DATA),
        ]))).unwrap();
    }
    if mode == "short" {
        return Response::builder().status(200).header("content-length", "100").body(
            Body::from_stream(futures_util::stream::iter([Ok::<_, Infallible>(DATA)])),
        ).unwrap();
    }
    if mode == "416" { return Response::builder().status(416).body(Body::empty()).unwrap(); }
    let start = headers.get(RANGE).and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("bytes="))
        .and_then(|h| h.trim_end_matches('-').parse::<usize>().ok()).unwrap_or(0);
    if mode == "ignore" || start == 0 {
        return Response::builder().status(200).header("etag", "\"v1\"").body(Body::from(DATA)).unwrap();
    }
    let mut response = Response::builder().status(206).header("etag", if mode == "changed" { "\"V1\"" } else { "\"v1\"" });
    if mode != "missing" {
        let range_start = if mode == "offset" { start + 1 } else { start };
        let total = if mode == "total" { DATA.len() + 1 } else { DATA.len() };
        response = response.header("content-range", format!("bytes {range_start}-{}/{total}", DATA.len()-1));
    }
    response.body(Body::from(DATA[start..].to_vec())).unwrap()
}

async fn server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move { axum::serve(listener, Router::new().route("/{mode}", get(fixture))).await.unwrap(); });
    (format!("http://{address}"), handle)
}

#[tokio::test]
async fn production_stream_resumes_and_restarts_ignored_ranges_without_duplicate_bytes() {
    let (base, server) = server().await;
    let directory = tempfile::tempdir().unwrap();
    let mut task = super::tests::test_task(directory.path(), "stream.bin", CollisionPolicy::Rename);
    task.total_bytes = DATA.len() as u64;
    task.etag = Some("\"v1\"".into());
    for mode in ["good", "ignore"] {
        let target = directory.path().join(mode);
        fs::write(&target, &DATA[..8]).await.unwrap();
        let response = reqwest::Client::new().get(format!("{base}/{mode}")).header(RANGE, "bytes=8-").send().await.unwrap();
        let plan = response_plan(&task, &response, 8).unwrap();
        let received = receive_response(response, &plan, &target, &CancellationToken::new(), 4096, &mut Observer { cancel: None }).await.unwrap();
        assert_eq!(received, DATA.len() as u64);
        let bytes = fs::read(target).await.unwrap();
        assert_eq!(Sha256::digest(&bytes), Sha256::digest(DATA));
    }
    server.abort();
}

#[tokio::test]
async fn production_stream_rejects_wrong_range_and_changed_etag_before_touching_part() {
    let (base, server) = server().await;
    let directory = tempfile::tempdir().unwrap();
    let mut task = super::tests::test_task(directory.path(), "stream.bin", CollisionPolicy::Rename);
    task.total_bytes = DATA.len() as u64;
    task.etag = Some("\"v1\"".into());
    let target = directory.path().join("part");
    fs::write(&target, &DATA[..8]).await.unwrap();
    for mode in ["offset", "total", "missing", "changed", "416"] {
        let response = reqwest::Client::new().get(format!("{base}/{mode}")).header(RANGE, "bytes=8-").send().await.unwrap();
        assert!(response_plan(&task, &response, 8).is_err(), "{mode}");
        assert_eq!(fs::read(&target).await.unwrap(), &DATA[..8]);
    }
    server.abort();
}

#[tokio::test]
async fn production_stream_unknown_length_and_incomplete_body_are_distinct() {
    let (base, server) = server().await;
    let directory = tempfile::tempdir().unwrap();
    let task = super::tests::test_task(directory.path(), "stream.bin", CollisionPolicy::Rename);
    for (mode, succeeds) in [("unknown", true)] {
        let response = reqwest::get(format!("{base}/{mode}")).await.unwrap();
        let plan = response_plan(&task, &response, 0).unwrap();
        let result = receive_response(response, &plan, &directory.path().join(mode), &CancellationToken::new(), 4096, &mut Observer { cancel: None }).await;
        assert_eq!(result.is_ok(), succeeds, "{mode}: {result:?}");
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let short_server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 2048];
        socket.read(&mut request).await.unwrap();
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n").await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        socket.write_all(DATA).await.unwrap();
        socket.shutdown().await.unwrap();
    });
    let response = reqwest::get(format!("http://{address}")).await.unwrap();
    let plan = response_plan(&task, &response, 0).unwrap();
    let target = directory.path().join("short");
    let result = receive_response(response, &plan, &target, &CancellationToken::new(), 4096, &mut Observer { cancel: None }).await;
    assert!(result.is_err());
    assert_eq!(fs::read(target).await.unwrap(), DATA);
    short_server.await.unwrap();
    server.abort();
}

#[tokio::test]
async fn production_stream_cancel_flushes_once_and_does_not_keep_writing() {
    let (base, server) = server().await;
    let directory = tempfile::tempdir().unwrap();
    let task = super::tests::test_task(directory.path(), "stream.bin", CollisionPolicy::Rename);
    let response = reqwest::get(format!("{base}/ignore")).await.unwrap();
    let plan = response_plan(&task, &response, 0).unwrap();
    let token = CancellationToken::new();
    let target = directory.path().join("cancelled");
    let result = receive_response(response, &plan, &target, &token, 4096, &mut Observer { cancel: Some(token.clone()) }).await;
    assert!(result.unwrap_err().contains("暂停"));
    let before = fs::read(&target).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(fs::read(&target).await.unwrap(), before);
    server.abort();
}

#[tokio::test]
async fn production_request_cancel_interrupts_silent_response_headers() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { let (_socket, _) = listener.accept().await.unwrap(); tokio::time::sleep(Duration::from_secs(10)).await; });
    let token = CancellationToken::new();
    let cancel = token.clone();
    tokio::spawn(async move { tokio::time::sleep(Duration::from_millis(40)).await; cancel.cancel(); });
    let result = tokio::time::timeout(Duration::from_millis(500), send_with_cancel(reqwest::Client::new().get(format!("http://{address}")), &token)).await.unwrap();
    assert!(result.unwrap_err().contains("暂停"));
    server.abort();
}
