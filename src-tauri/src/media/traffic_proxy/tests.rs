use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::sync::atomic::AtomicUsize;

async fn source(bytes: usize) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let hits = count.clone();
    let router = axum::Router::new().route(
        "/bytes",
        axum::routing::get(move || {
            let hits = hits.clone();
            async move {
                hits.fetch_add(1, Ordering::Relaxed);
                vec![0x5au8; bytes]
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{address}/bytes"), count, server)
}

fn policy() -> NetworkPolicy {
    let mut settings = crate::models::AppSettings::default();
    settings.proxy_mode = "none".into();
    NetworkPolicy::resolve(&settings, None).unwrap()
}
fn client(proxy: &TrafficProxy) -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(proxy.address()).unwrap())
        .build()
        .unwrap()
}

#[test]
fn subprocess_proxy_environment_cannot_inherit_no_proxy_bypass() {
    let before = std::env::var_os("NO_PROXY");
    let mut command = tokio::process::Command::new("synthetic-tool");
    configure_command(&mut command, "http://maobu:synthetic@127.0.0.1:17434");
    for name in ["NO_PROXY", "no_proxy"] {
        assert!(command.as_std().get_envs().any(|(key, value)| key
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
            && value.is_some_and(|value| value.is_empty())));
    }
    for name in [
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
    ] {
        assert!(command.as_std().get_envs().any(|(key, value)| key
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
            && value.is_some_and(|value| value == "http://maobu:synthetic@127.0.0.1:17434")));
    }
    assert_eq!(std::env::var_os("NO_PROXY"), before);
}

#[tokio::test]
async fn unauthorized_clients_never_reach_origin_and_http_body_is_exact() {
    let (url, hits, server) = source(32 * 1024).await;
    let cancel = CancellationToken::new();
    let proxy = TrafficProxy::start(policy(), None, &cancel).await.unwrap();
    let proxy_url = url::Url::parse(proxy.address()).unwrap();
    let mut unauthorized = TcpStream::connect(("127.0.0.1", proxy_url.port().unwrap()))
        .await
        .unwrap();
    unauthorized
        .write_all(format!("GET {url} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut rejected = Vec::new();
    unauthorized.read_to_end(&mut rejected).await.unwrap();
    assert!(rejected.starts_with(b"HTTP/1.1 407"));
    assert_eq!(hits.load(Ordering::Relaxed), 0);
    let body = client(&proxy)
        .get(&url)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(body.as_ref(), vec![0x5a; 32 * 1024]);
    assert_eq!(hits.load(Ordering::Relaxed), 1);
    proxy.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn connect_preserves_opaque_bytes_and_peer_half_close_without_tls_inspection() {
    let origin = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = origin.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = origin.accept().await.unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await.unwrap();
        stream.write_all(&bytes).await.unwrap();
    });
    let proxy = TrafficProxy::start(policy(), None, &CancellationToken::new())
        .await
        .unwrap();
    let url = url::Url::parse(proxy.address()).unwrap();
    let mut client = TcpStream::connect(("127.0.0.1", url.port().unwrap()))
        .await
        .unwrap();
    let auth = STANDARD.encode(format!("maobu:{}", url.password().unwrap()));
    client
        .write_all(
            format!("CONNECT {address} HTTP/1.1\r\nProxy-Authorization: Basic {auth}\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    assert!(request::read_header(&mut client)
        .await
        .unwrap()
        .starts_with(b"HTTP/1.1 200"));
    assert_eq!(proxy.stats.active(), 1);
    let opaque = b"\x16\x03\x03\x00\x08opaque\x00\xff";
    client.write_all(opaque).await.unwrap();
    client.shutdown().await.unwrap();
    let mut echoed = Vec::new();
    client.read_to_end(&mut echoed).await.unwrap();
    assert_eq!(echoed, opaque);
    server.await.unwrap();
    proxy.shutdown().await;
}

#[tokio::test]
async fn two_media_tunnels_and_native_http_share_one_global_budget() {
    use crate::manager::{bandwidth::BandwidthScheduler, RuntimeTaskOptions};
    use futures_util::StreamExt;
    let (url, _, server) = source(64 * 1024).await;
    let scheduler = BandwidthScheduler::new(64 * 1024);
    let mut task = super::super::test_support::task();
    let one = TrafficBudget::new(
        "one".into(),
        scheduler.clone(),
        Arc::new(RuntimeTaskOptions::new(&task)),
    );
    task.id = "two".into();
    let two = TrafficBudget::new(
        "two".into(),
        scheduler.clone(),
        Arc::new(RuntimeTaskOptions::new(&task)),
    );
    let cancel = CancellationToken::new();
    let p1 = TrafficProxy::start(policy(), Some(one), &cancel)
        .await
        .unwrap();
    let p2 = TrafficProxy::start(policy(), Some(two), &cancel)
        .await
        .unwrap();
    let started = tokio::time::Instant::now();
    let first = client(&p1).get(&url).send();
    let second = client(&p2).get(&url).send();
    let native = async {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut response = client.get(&url).send().await.unwrap().bytes_stream();
        let mut total = 0;
        while let Some(chunk) = response.next().await {
            let chunk = chunk.unwrap();
            scheduler
                .acquire("native", chunk.len() as u64, 0, &cancel)
                .await;
            total += chunk.len();
        }
        total
    };
    let results = tokio::join!(
        async { first.await.unwrap().bytes().await.unwrap() },
        async { second.await.unwrap().bytes().await.unwrap() },
        native
    );
    assert_eq!(results.0.len(), 64 * 1024);
    assert_eq!(results.1.len(), 64 * 1024);
    assert_eq!(results.2, 64 * 1024);
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(2400),
        "three 64KiB flows must share 64KiB/s, not receive a cap each: {:?}",
        started.elapsed()
    );
    p1.shutdown().await;
    p2.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn task_limit_updates_and_cancellation_reach_all_tunnels() {
    use crate::manager::{bandwidth::BandwidthScheduler, RuntimeTaskOptions};
    let (url, _, server) = source(128 * 1024).await;
    let mut task = super::super::test_support::task();
    task.per_task_speed_limit = 16 * 1024;
    let runtime = Arc::new(RuntimeTaskOptions::new(&task));
    let budget = TrafficBudget::new(task.id.clone(), BandwidthScheduler::new(0), runtime.clone());
    let cancel = CancellationToken::new();
    let proxy = TrafficProxy::start(policy(), Some(budget), &cancel)
        .await
        .unwrap();
    let request = tokio::spawn({
        let client = client(&proxy);
        let url = url.clone();
        async move { client.get(url).send().await?.bytes().await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    assert!(
        !request.is_finished(),
        "single-task limit must use bytes/sec without a 1024 multiplier"
    );
    runtime.speed_limit.store(0, Ordering::Relaxed);
    let body = tokio::time::timeout(std::time::Duration::from_secs(3), request)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        body.len(),
        128 * 1024,
        "live task limit removal must affect subsequent chunks"
    );
    runtime.speed_limit.store(1024, Ordering::Relaxed);
    let request = tokio::spawn({
        let client = client(&proxy);
        async move { client.get(url).send().await?.bytes().await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let stats = proxy.stats.clone();
    cancel.cancel();
    proxy.shutdown().await;
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), request)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert_eq!(stats.active(), 0);
    server.abort();
}
