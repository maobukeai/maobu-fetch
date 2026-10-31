//! 可控本地 tracker/seeder/leecher；不访问公网，不读取用户状态。
use librqbit::{AddTorrent, AddTorrentOptions, ManagedTorrent, Session, SessionOptions};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};

pub(crate) struct LocalTorrent {
    pub(crate) source: Arc<Session>,
    pub(crate) source_file: PathBuf,
    pub(crate) bytes: Vec<u8>,
    pub(crate) expected: Vec<u8>,
    peers: Arc<RwLock<Vec<SocketAddr>>>,
    pub(crate) announces: Arc<AtomicUsize>,
    tracker: tokio::task::JoinHandle<()>,
}

impl LocalTorrent {
    pub(crate) async fn new(root: &Path) -> Self {
        let source_root = root.join("source");
        tokio::fs::create_dir_all(&source_root).await.unwrap();
        let expected = (0..32 * 1024 * 1024 + 137)
            .map(|i| ((i * 17 + i / 251 + 97) % 251) as u8)
            .collect::<Vec<_>>();
        let source_file = source_root.join("payload.bin");
        tokio::fs::write(&source_file, &expected).await.unwrap();
        let source = Session::new_with_opts(source_root.clone(), local_options())
            .await
            .unwrap();
        assert!(
            source.listen_addr().unwrap().is_ipv4()
                && source.listen_addr().unwrap().ip().is_loopback()
        );
        let peers = Arc::new(RwLock::new(vec![source.listen_addr().unwrap()]));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/announce", listener.local_addr().unwrap());
        let tracker_peers = peers.clone();
        let announces = Arc::new(AtomicUsize::new(0));
        let tracker_announces = announces.clone();
        let tracker = tokio::spawn(async move {
            let app = axum::Router::new().route(
                "/announce",
                axum::routing::get(move || {
                    let peers = tracker_peers.clone();
                    tracker_announces.fetch_add(1, Ordering::Relaxed);
                    async move { tracker_response(&peers.read().unwrap()) }
                }),
            );
            axum::serve(listener, app).await.unwrap();
        });
        let spawner = librqbit::spawn_utils::BlockingSpawner::new(2);
        let bytes = librqbit::create_torrent(
            &source_file,
            librqbit::CreateTorrentOptions {
                piece_length: Some(16 * 1024),
                trackers: vec![url],
                ..Default::default()
            },
            &spawner,
        )
        .await
        .unwrap()
        .as_bytes()
        .unwrap()
        .to_vec();
        let handle = source
            .add_torrent(
                AddTorrent::from_bytes(bytes.clone()),
                Some(AddTorrentOptions {
                    output_folder: Some(source_root.to_string_lossy().into_owned()),
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
        assert!(handle.stats().finished);
        Self {
            source,
            source_file,
            bytes,
            expected,
            peers,
            announces,
            tracker,
        }
    }

    pub(crate) fn advertise(&self, peers: Vec<SocketAddr>) {
        assert!(peers.iter().all(|peer| peer.ip().is_loopback()));
        *self.peers.write().unwrap() = peers;
    }

    pub(crate) async fn leecher(&self, root: &Path, download_bps: u32) -> LocalLeecher {
        tokio::fs::create_dir_all(root).await.unwrap();
        let session = Session::new_with_opts(root.to_owned(), local_options())
            .await
            .unwrap();
        let handle = session
            .add_torrent(
                AddTorrent::from_bytes(self.bytes.clone()),
                Some(AddTorrentOptions {
                    output_folder: Some(root.to_string_lossy().into_owned()),
                    ratelimits: librqbit::limits::LimitsConfig {
                        download_bps: std::num::NonZeroU32::new(download_bps),
                        ..Default::default()
                    },
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
        assert_eq!(handle.stats().progress_bytes, 0);
        LocalLeecher {
            session,
            handle,
            file: root.join("payload.bin"),
        }
    }
}

impl Drop for LocalTorrent {
    fn drop(&mut self) {
        self.tracker.abort();
        self.source.cancellation_token().cancel();
    }
}

pub(crate) struct LocalLeecher {
    pub(crate) session: Arc<Session>,
    pub(crate) handle: Arc<ManagedTorrent>,
    pub(crate) file: PathBuf,
}
impl LocalLeecher {
    pub(crate) fn verified_bytes(&self) -> u64 {
        self.handle.stats().progress_bytes
    }
}
impl Drop for LocalLeecher {
    fn drop(&mut self) {
        self.session.cancellation_token().cancel();
    }
}

fn local_options() -> SessionOptions {
    SessionOptions {
        dht: None,
        disable_trackers: true,
        disable_local_service_discovery: true,
        listen: Some(librqbit::ListenerOptions {
            listen_addr: "127.0.0.1:0".parse().unwrap(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn tracker_response(peers: &[SocketAddr]) -> Vec<u8> {
    let mut compact = Vec::new();
    for peer in peers {
        if let SocketAddr::V4(peer) = peer {
            compact.extend(peer.ip().octets());
            compact.extend(peer.port().to_be_bytes());
        }
    }
    let mut response = format!("d8:intervali1e5:peers{}:", compact.len()).into_bytes();
    response.extend(compact);
    response.push(b'e');
    response
}
