//! 真实 librqbit peer 流量、分片哈希、暂停及进程会话恢复；只监听 loopback。
use super::{tests::bt_task_for_test, BtEngine};
use crate::models::AppSettings;
use librqbit::{AddTorrent, AddTorrentOptions, Session, SessionOptions};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc, time::Duration};

pub(super) struct LocalSwarm {
    pub(super) seeder: Arc<Session>,
    tracker: tokio::task::JoinHandle<()>,
    pub(super) torrent: Vec<u8>,
    pub(super) expected: Vec<u8>,
}

impl Drop for LocalSwarm {
    fn drop(&mut self) {
        self.tracker.abort();
        self.seeder.cancellation_token().cancel();
    }
}

pub(super) async fn local_swarm(root: &Path) -> LocalSwarm {
    let source = root.join("source");
    tokio::fs::create_dir(&source).await.unwrap();
    let expected = (0..1024 * 1024)
        .map(|i| ((i * 17 + i / 251) % 251) as u8)
        .collect::<Vec<_>>();
    let file = source.join("payload.bin");
    tokio::fs::write(&file, &expected).await.unwrap();
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
    let address = seeder.listen_addr().unwrap();
    assert!(address.ip().is_loopback());
    let mut response = b"d8:intervali1e5:peers6:".to_vec();
    response.extend_from_slice(&[127, 0, 0, 1]);
    response.extend_from_slice(&address.port().to_be_bytes());
    response.push(b'e');
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tracker_url = format!("http://{}/announce", listener.local_addr().unwrap());
    let tracker = tokio::spawn(async move {
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
        &file,
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
    let handle = seeder
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
    tokio::time::timeout(Duration::from_secs(10), handle.wait_until_initialized())
        .await
        .unwrap()
        .unwrap();
    assert!(
        handle.stats().finished,
        "合成源必须先由真实分片哈希验证为完整种子"
    );
    LocalSwarm {
        seeder,
        tracker,
        torrent,
        expected,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_loopback_partial_download_pause_restore_and_sha256() {
    let temp = tempfile::tempdir().unwrap();
    let swarm = local_swarm(temp.path()).await;
    let mut task = bt_task_for_test();
    task.id = "local-swarm-task".into();
    task.destination = temp.path().join("downloads").to_string_lossy().into_owned();
    task.file_name = "stable-output-folder".into();
    task.url = "local-swarm.torrent".into();
    task.per_task_speed_limit = 64 * 1024;
    use base64::Engine as _;
    task.bt_meta.as_mut().unwrap().torrent_data_base64 =
        Some(base64::engine::general_purpose::STANDARD.encode(&swarm.torrent));
    task.bt_meta.as_mut().unwrap().info_hash = super::inspect_torrent_bytes(&swarm.torrent)
        .unwrap()
        .info_hash;
    super::prepare_task_output(&mut task).await.unwrap();
    let output = super::task_output_folder(&task).await.unwrap();
    let data = temp.path().join("session");
    let engine = BtEngine::with_data_dir(data.clone());
    engine
        .add_task_inner(&AppSettings::default(), &mut task)
        .await
        .unwrap();
    let handle = engine.bindings.lock().await.get(&task.id).unwrap().clone();
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let stats = handle.stats();
            if stats.progress_bytes > 0 && stats.progress_bytes < stats.total_bytes {
                break;
            }
            assert!(
                !matches!(stats.state, librqbit::TorrentStatsState::Error),
                "{:?}",
                stats.error
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("loopback seeder 应提供经过真实哈希校验的部分内容");
    engine.pause_task(&task.id).await.unwrap();
    let checkpoint = handle.stats().progress_bytes;
    assert!(checkpoint > 0 && checkpoint < swarm.expected.len() as u64);
    let bitfield = tokio::fs::read(data.join(format!("{}.bitv", handle.info_hash().as_string())))
        .await
        .unwrap();
    let persisted_pieces = bitfield
        .iter()
        .map(|byte| byte.count_ones() as u64)
        .sum::<u64>();
    assert_eq!(
        persisted_pieces * 16 * 1024,
        checkpoint,
        "暂停返回时真实bitv必须已经落盘，不是仅放入异步队列"
    );
    let paused_bytes = tokio::fs::read(output.join("payload.bin")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        paused_bytes,
        tokio::fs::read(output.join("payload.bin")).await.unwrap(),
        "暂停成功后不得继续写文件"
    );
    engine.shutdown().await.unwrap();
    drop(handle);
    let resumed = BtEngine::with_data_dir(data);
    let session = resumed.get_or_init_session(None).await.unwrap();
    let handles = session.with_torrents(|items| items.map(|(_, h)| h.clone()).collect::<Vec<_>>());
    assert_eq!(handles.len(), 1);
    assert!(handles[0].is_paused());
    tokio::time::timeout(Duration::from_secs(10), handles[0].wait_until_initialized())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        handles[0].stats().progress_bytes,
        checkpoint,
        "重启不能回退已经验证的分片进度"
    );
    resumed
        .add_task_inner(&AppSettings::default(), &mut task)
        .await
        .unwrap();
    resumed
        .set_task_limits(&task.id, 1024 * 1024, 0)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), handles[0].wait_until_completed())
        .await
        .unwrap()
        .unwrap();
    resumed.pause_task(&task.id).await.unwrap();
    assert_eq!(super::task_output_folder(&task).await.unwrap(), output);
    let actual = tokio::fs::read(super::task_content_path(&task, None).await.unwrap())
        .await
        .unwrap();
    assert_eq!(actual.len(), swarm.expected.len());
    assert_eq!(Sha256::digest(actual), Sha256::digest(&swarm.expected));
    resumed.shutdown().await.unwrap();
    swarm.seeder.stop().await;
}
