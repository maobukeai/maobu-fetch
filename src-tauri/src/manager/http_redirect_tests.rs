use super::*;
use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::Request as HttpRequest,
    response::Response as HttpResponse,
    Router,
};

#[derive(Clone, Debug)]
struct Seen {
    path: String,
    method: String,
    body: Vec<u8>,
    headers: reqwest::header::HeaderMap,
}
type Reply = Arc<dyn Fn(&Seen) -> (u16, Option<String>) + Send + Sync>;
struct Fixture {
    url: String,
    seen: Arc<StdMutex<Vec<Seen>>>,
    server: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn start(reply: impl Fn(&Seen) -> (u16, Option<String>) + Send + Sync + 'static) -> Self {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let router = Router::new()
            .fallback(handle)
            .with_state((seen.clone(), Arc::new(reply) as Reply));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            url: format!("http://{address}"),
            seen,
            server,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
async fn handle(
    State((seen, reply)): State<(Arc<StdMutex<Vec<Seen>>>, Reply)>,
    request: HttpRequest<Body>,
) -> HttpResponse {
    let (parts, body) = request.into_parts();
    let entry = Seen {
        path: parts.uri.path().into(),
        method: parts.method.to_string(),
        headers: parts.headers,
        body: to_bytes(body, 1024 * 1024).await.unwrap().to_vec(),
    };
    let (status, location) = reply(&entry);
    seen.lock().unwrap().push(entry);
    let mut response = HttpResponse::builder().status(status);
    if let Some(location) = location {
        response = response.header("location", location);
    }
    response.body(Body::from("ok")).unwrap()
}
fn client() -> Client {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

#[tokio::test]
async fn every_redirect_rechecks_cookie_scope_auth_and_authoritative_range_headers() {
    let remote = Fixture::start(|_| (200, None)).await;
    let remote_url = format!("{}/media", remote.url);
    let source = Fixture::start(move |entry| match entry.path.as_str() {
        "/first" => (302, Some("nested/next".into())),
        _ => (302, Some(remote_url.clone())),
    })
    .await;
    let jar="# Netscape HTTP Cookie File\n127.0.0.1\tFALSE\t/first\tFALSE\t0\tsid\tfirst\n127.0.0.1\tFALSE\t/media\tFALSE\t0\tsid\tmedia\nforeign.invalid\tFALSE\t/\tFALSE\t0\tbad\tnever";
    let raw = HashMap::from([
        ("Cookie".into(), jar.into()),
        ("Authorization".into(), "Bearer synthetic".into()),
        ("Range".into(), "bytes=99-999".into()),
        ("If-Range".into(), "bad".into()),
        ("Accept-Encoding".into(), "gzip".into()),
    ]);
    let client = client();
    let source_url = format!("{}/first", source.url);
    let request = client
        .get(&source_url)
        .header(RANGE, "bytes=2-3")
        .header(IF_RANGE, "\"expected\"")
        .header(ACCEPT_ENCODING, "identity");
    assert_eq!(
        send(
            &client,
            request,
            &raw,
            &source_url,
            &CancellationToken::new()
        )
        .await
        .unwrap()
        .status(),
        200
    );
    let source = source.seen.lock().unwrap();
    assert_eq!(source[0].headers.get("cookie").unwrap(), "sid=first");
    assert!(source[1].headers.get("cookie").is_none());
    assert!(source
        .iter()
        .all(|entry| entry.headers.contains_key("authorization")));
    let remote = remote.seen.lock().unwrap();
    assert_eq!(remote[0].headers.get("cookie").unwrap(), "sid=media");
    assert!(remote[0].headers.get("authorization").is_none());
    for entry in source.iter().chain(remote.iter()) {
        assert_eq!(entry.headers.get("range").unwrap(), "bytes=2-3");
        assert_eq!(entry.headers.get("if-range").unwrap(), "\"expected\"");
        assert_eq!(entry.headers.get("accept-encoding").unwrap(), "identity");
    }
}

#[tokio::test]
async fn post_redirects_rewrite_only_301_302_303_and_replay_exact_bytes_for_307_308() {
    for status in [301, 302, 303, 307, 308] {
        let fixture = Fixture::start(move |entry| {
            if entry.path == "/post" {
                (status, Some("/done".into()))
            } else {
                (200, None)
            }
        })
        .await;
        let client = client();
        let url = format!("{}/post", fixture.url);
        let bytes = b"synthetic=post-body";
        send(
            &client,
            client
                .post(&url)
                .header("content-type", "application/octet-stream")
                .body(bytes.to_vec()),
            &HashMap::new(),
            &url,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let seen = fixture.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].body, bytes);
        if status <= 303 {
            assert_eq!(seen[1].method, "GET");
            assert!(seen[1].body.is_empty());
            assert!(!seen[1].headers.contains_key("content-type"));
        } else {
            assert_eq!(seen[1].method, "POST");
            assert_eq!(seen[1].body, bytes);
        }
    }
}

#[tokio::test]
async fn explicit_builder_basic_auth_is_kept_only_at_its_source_origin() {
    let remote = Fixture::start(|_| (200, None)).await;
    let target = remote.url.clone();
    let source = Fixture::start(move |_| (302, Some(target.clone()))).await;
    let client = client();
    send(
        &client,
        client
            .get(&source.url)
            .basic_auth("synthetic", Some("password")),
        &HashMap::new(),
        &source.url,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(source.seen.lock().unwrap()[0]
        .headers
        .contains_key("authorization"));
    assert!(!remote.seen.lock().unwrap()[0]
        .headers
        .contains_key("authorization"));
}

#[tokio::test]
async fn loops_protocols_hop_limits_streaming_bodies_and_pre_cancel_are_rejected() {
    for kind in 0..3 {
        let fixture = Fixture::start(move |entry| match kind {
            0 => (302, Some("/first".into())),
            1 => (302, Some("file:///private".into())),
            _ => (
                302,
                Some(format!(
                    "/{}",
                    entry
                        .path
                        .trim_start_matches('/')
                        .parse::<u8>()
                        .unwrap_or(0)
                        + 1
                )),
            ),
        })
        .await;
        let client = client();
        let url = format!("{}/first", fixture.url);
        assert!(send(
            &client,
            client.get(&url),
            &HashMap::new(),
            &url,
            &CancellationToken::new()
        )
        .await
        .is_err());
        assert!(fixture.seen.lock().unwrap().len() <= 11);
    }
    let fixture = Fixture::start(|_| (200, None)).await;
    let client = client();
    let body = reqwest::Body::wrap_stream(futures_util::stream::iter([Ok::<_, std::io::Error>(
        b"synthetic".to_vec(),
    )]));
    assert!(send(
        &client,
        client.post(&fixture.url).body(body),
        &HashMap::new(),
        &fixture.url,
        &CancellationToken::new()
    )
    .await
    .unwrap_err()
    .contains("流式正文"));
    let token = CancellationToken::new();
    token.cancel();
    assert!(send(
        &client,
        client.get(&fixture.url),
        &HashMap::new(),
        &fixture.url,
        &token
    )
    .await
    .unwrap_err()
    .contains("暂停"));
    assert!(fixture.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn per_task_read_timeout_bounds_silent_headers_without_a_hidden_total_timeout() {
    use tokio::io::AsyncReadExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/silent", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0u8; 4096];
        assert!(socket.read(&mut bytes).await.unwrap() > 0);
        tokio::time::sleep(Duration::from_secs(4)).await;
    });
    let mut task = crate::bt::tests::bt_task_for_test();
    task.url = url;
    task.proxy_override = Some(String::new());
    let mut settings = AppSettings::default();
    settings.default_retry_policy.connection_timeout_secs = 10;
    task.retry_policy_override = Some(RetryPolicy {
        connection_timeout_secs: 1,
        task_timeout_secs: None,
        ..Default::default()
    });
    let client =
        build_task_client_with_redirect_policy(&settings, &task, reqwest::redirect::Policy::none())
            .unwrap();
    let start = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        send(
            &client,
            client.get(&task.url),
            &HashMap::new(),
            &task.url,
            &CancellationToken::new(),
        ),
    )
    .await;
    assert!(result.unwrap().is_err());
    assert!(start.elapsed() >= Duration::from_millis(900));
    assert!(start.elapsed() < Duration::from_secs(3));
    server.abort();
    let _ = server.await;
}
