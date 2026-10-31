//! 透明 loopback TCP 代理记录真实 Request 消息，证明选片顺序确实进入引擎。
use super::{tests::bt_task_for_test, BtEngine};
use crate::models::AppSettings;
use librqbit::{AddTorrent, AddTorrentOptions, Session, SessionOptions};
use std::{
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

struct Proxy {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<u32>>>,
    stop: CancellationToken,
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

async fn proxy(target: SocketAddr) -> Proxy {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let stop = CancellationToken::new();
    let token = stop.clone();
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! { _ = token.cancelled() => break, accepted = listener.accept() => accepted };
            let (client, _) = accepted.unwrap();
            let records = recorded.clone();
            let child = token.child_token();
            tokio::spawn(async move {
                let server = tokio::net::TcpStream::connect(target).await.unwrap();
                let (mut incoming, mut outgoing) = client.into_split();
                let (mut replies, mut requests) = server.into_split();
                let record = async move {
                    let mut handshake = [0; 68];
                    incoming.read_exact(&mut handshake).await?;
                    requests.write_all(&handshake).await?;
                    loop {
                        let size = incoming.read_u32().await?;
                        if size > 2 * 1024 * 1024 {
                            return Err::<(), _>(std::io::Error::other("oversized peer frame"));
                        }
                        let mut payload = vec![0; size as usize];
                        incoming.read_exact(&mut payload).await?;
                        if payload.len() >= 13 && payload[0] == 6 {
                            records
                                .lock()
                                .unwrap()
                                .push(u32::from_be_bytes(payload[1..5].try_into().unwrap()));
                        }
                        requests.write_u32(size).await?;
                        requests.write_all(&payload).await?;
                    }
                };
                tokio::select! {
                    _ = child.cancelled() => {},
                    _ = record => {},
                    _ = tokio::io::copy(&mut replies, &mut outgoing) => {},
                }
            });
        }
    });
    Proxy {
        address,
        requests,
        stop,
    }
}

async fn request_order(root: &Path, enabled: bool, selected: Vec<u32>) -> Vec<u32> {
    tokio::fs::create_dir_all(root).await.unwrap();
    let source = root.join("source");
    tokio::fs::create_dir(&source).await.unwrap();
    for (name, offset) in [("first.bin", 11), ("second.bin", 23)] {
        tokio::fs::write(
            source.join(name),
            (0..512 * 1024)
                .map(|n| ((n + offset) % 251) as u8)
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
    }
    let seeder = Session::new_with_opts(
        source.clone(),
        SessionOptions {
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            listen: Some(librqbit::ListenerOptions {
                listen_addr: "127.0.0.1:0".parse().unwrap(),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let proxy = proxy(seeder.listen_addr().unwrap()).await;
    let address = proxy.address;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tracker_url = format!("http://{}/announce", listener.local_addr().unwrap());
    let tracker = tokio::spawn(async move {
        let mut response = b"d8:intervali1e5:peers6:".to_vec();
        response.extend_from_slice(&[127, 0, 0, 1]);
        response.extend_from_slice(&address.port().to_be_bytes());
        response.push(b'e');
        let app = axum::Router::new().route(
            "/announce",
            axum::routing::get(move || {
                let response = response.clone();
                async move { response }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });
    let spawner = librqbit::spawn_utils::BlockingSpawner::new(2);
    let torrent = librqbit::create_torrent(
        &source,
        librqbit::CreateTorrentOptions {
            piece_length: Some(16 * 1024),
            trackers: vec![tracker_url],
            ..Default::default()
        },
        &spawner,
    )
    .await
    .unwrap()
    .as_bytes()
    .unwrap()
    .to_vec();
    let seed = seeder
        .add_torrent(
            AddTorrent::from_bytes(torrent.clone()),
            Some(AddTorrentOptions {
                output_folder: Some(source.to_string_lossy().into_owned()),
                overwrite: true,
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    seed.wait_until_initialized().await.unwrap();
    assert!(seed.stats().finished);
    let mut task = bt_task_for_test();
    task.url = "ordering.torrent".into();
    task.destination = root.join("downloads").to_string_lossy().into_owned();
    use base64::Engine as _;
    let meta = task.bt_meta.as_mut().unwrap();
    meta.streaming_priority = enabled;
    meta.info_hash = super::inspect_torrent_bytes(&torrent).unwrap().info_hash;
    meta.selected_files = selected;
    meta.torrent_data_base64 = Some(base64::engine::general_purpose::STANDARD.encode(torrent));
    super::prepare_task_output(&mut task).await.unwrap();
    let engine = BtEngine::with_data_dir(root.join("session"));
    engine
        .add_task_inner(&AppSettings::default(), &mut task)
        .await
        .unwrap();
    let handle = engine.bindings.lock().await.get(&task.id).unwrap().clone();
    assert_eq!(handle.streaming_priority(), enabled);
    tokio::time::timeout(Duration::from_secs(15), handle.wait_until_completed())
        .await
        .unwrap()
        .unwrap();
    engine.shutdown().await.unwrap();
    seeder.stop().await;
    tracker.abort();
    let order = proxy.requests.lock().unwrap().clone();
    order
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_peer_requests_prioritize_all_selected_file_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let ordinary = request_order(&temp.path().join("ordinary"), false, vec![]).await;
    let streaming = request_order(&temp.path().join("streaming"), true, vec![]).await;
    assert_eq!(&ordinary[..4], &[0, 31, 1, 2]);
    assert_eq!(
        &streaming[..4],
        &[0, 31, 32, 63],
        "所有文件首尾必须先于中间片进入真实Request流"
    );
    let only_second = request_order(&temp.path().join("selected"), true, vec![2]).await;
    assert!(
        only_second.iter().all(|piece| *piece >= 32),
        "首尾优先仍须严格尊重用户文件选择"
    );
}
