use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Actual yt-dlp must traverse the authenticated local proxy and selected upstream.
pub(super) async fn run(
    app: &AppHandle,
    manager: &crate::manager::SharedManager,
    fixtures: &execution_fixtures::Fixtures,
    settings: &AppSettings,
    output: &Path,
) -> Result<Value, String> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let observed = Arc::new(AtomicUsize::new(0));
    let hits = observed.clone();
    let origin = url::Url::parse(&fixtures.base_url).map_err(|error| error.to_string())?;
    let port = origin.port().ok_or("missing local fixture port")?;
    let server = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let (mut client, _) = listener.accept().await.unwrap();
            let hits = hits.clone();
            connections.spawn(async move {
                let mut header = Vec::new();
                let mut byte = [0; 1];
                while !header.ends_with(b"\r\n\r\n") {
                    if header.len() > 32 * 1024 {
                        return;
                    }
                    if client.read_exact(&mut byte).await.is_err() {
                        return;
                    }
                    header.push(byte[0]);
                }
                let text = String::from_utf8(header).unwrap();
                assert!(text.starts_with(&format!("CONNECT 127.0.0.1:{port} HTTP/1.1\r\n")));
                assert!(text.contains("Proxy-Authorization: Basic dXNlcjpzeW50aGV0aWM="));
                hits.fetch_add(1, Ordering::Relaxed);
                let mut source = tokio::net::TcpStream::connect(("127.0.0.1", port))
                    .await
                    .unwrap();
                client
                    .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                    .await
                    .unwrap();
                let _ = tokio::io::copy_bidirectional(&mut client, &mut source).await;
            });
        }
    });
    let mut task = super::execution_negative::make_task(
        fixtures,
        output,
        "actual-proxy-limit",
        "fixture.mp4",
        "best",
    );
    task.proxy_override = Some(format!("http://user:synthetic@{address}"));
    task.per_task_speed_limit = 16 * 1024;
    manager.store.upsert_task(&task).await?;
    manager.task_runtime.write().await.insert(
        task.id.clone(),
        Arc::new(crate::manager::RuntimeTaskOptions::new(&task)),
    );
    let started = std::time::Instant::now();
    let result = download::download_with_manager(
        app,
        settings,
        task,
        CancellationToken::new(),
        Vec::new(),
        manager,
    )
    .await;
    server.abort();
    let _ = server.await;
    let complete = result?;
    assert!(
        observed.load(Ordering::Relaxed) > 0,
        "actual tool must traverse explicit upstream, including loopback targets"
    );
    assert!(
        started.elapsed() >= std::time::Duration::from_secs(2),
        "actual tool bytes bypassed 16KiB/s task budget: {:?}",
        started.elapsed()
    );
    let actual = tokio::fs::read(output.join(&complete.file_name))
        .await
        .map_err(|error| error.to_string())?;
    let source = tokio::fs::read(fixtures.root.join("fixture.mp4"))
        .await
        .map_err(|error| error.to_string())?;
    assert_eq!(actual, source);
    Ok(
        serde_json::json!({"case":"actual-ytdlp-upstream-and-task-budget","upstream_connects":observed.load(Ordering::Relaxed),"bytes":actual.len(),"elapsed_ms":started.elapsed().as_millis(),"limit_bytes_per_second":16*1024,"exact_source":true}),
    )
}
