use super::super::{seed_policy::wait_for_check, tests::bt_task_for_test, *};
use crate::{
    models::{DownloadTask, TaskStatus},
    store::Store,
};
use base64::Engine as _;
use librqbit::{ManagedTorrent, Session, TorrentStatsState};
use sha1::Digest;
use std::{sync::Arc, time::Duration};

struct Fixture {
    _root: tempfile::TempDir,
    engine: Arc<BtEngine>,
    task: DownloadTask,
    session: Arc<Session>,
    native: Arc<ManagedTorrent>,
}
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut task = bt_task_for_test();
        task.id = uuid::Uuid::new_v4().to_string();
        task.destination = root.path().join("downloads").to_string_lossy().into_owned();
        task.url = "recheck-cancellation.torrent".into();
        let mut bytes =
            b"d4:infod6:lengthi4e4:name8:file.bin12:piece lengthi4e6:pieces20:".to_vec();
        bytes.extend_from_slice(&sha1::Sha1::digest(b"DATA"));
        bytes.extend_from_slice(b"ee");
        let meta = task.bt_meta.as_mut().unwrap();
        meta.info_hash = inspect_torrent_bytes(&bytes).unwrap().info_hash;
        meta.torrent_data_base64 = Some(base64::engine::general_purpose::STANDARD.encode(bytes));
        meta.seeding_requested = true;
        prepare_task_output(&mut task).await.unwrap();
        tokio::fs::write(
            task_output_folder(&task).await.unwrap().join("file.bin"),
            b"DATA",
        )
        .await
        .unwrap();
        let engine = Arc::new(BtEngine::with_data_dir(root.path().join("session")));
        engine
            .prepare_task(&Default::default(), &mut task, false)
            .await
            .unwrap();
        let native = engine.bindings.lock().await.get(&task.id).unwrap().clone();
        tokio::time::timeout(Duration::from_secs(2), native.wait_until_initialized())
            .await
            .unwrap()
            .unwrap();
        assert!(
            native.stats().finished,
            "fixture must first pass the real piece hash"
        );
        let session = engine.session.lock().await.clone().unwrap();
        // Current-thread runtime: start queues the real recheck, then pause sets its request
        // before that queued checker can run. This deterministically reaches paused Initializing.
        native.recheck_paused().unwrap();
        session.pause(&native).await.unwrap();
        assert!(matches!(
            native.stats().state,
            TorrentStatsState::Initializing { paused: true }
        ));
        task.status = TaskStatus::Completed;
        Self {
            _root: root,
            engine,
            task,
            session,
            native,
        }
    }
    async fn blocked_preparation(&self) -> tokio::task::JoinHandle<()> {
        let engine = self.engine.clone();
        let session = self.session.clone();
        let native = self.native.clone();
        let id = self.task.id.clone();
        let waiting = tokio::spawn(async move {
            let _reconcile = engine.seed_sync.lock().await;
            let preparation = engine.begin_seed_preparation(&id);
            assert!(preparation
                .run(wait_for_check(&session, &native))
                .await
                .is_none());
            drop(preparation);
            engine.pause_task_for_preparation(&id).await.unwrap();
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !self.engine.has_seed_preparation(&self.task.id) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            !waiting.is_finished(),
            "must actually block on the native Initializing state before cancellation"
        );
        waiting
    }
    async fn settled(&self, waiting: tokio::task::JoinHandle<()>) {
        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .unwrap()
            .unwrap();
        assert!(self.engine.seed_sync.try_lock().is_ok());
    }
}

#[tokio::test]
async fn pause_cancels_a_real_paused_initializing_recheck_and_releases_reconciliation() {
    let fixture = Fixture::new().await;
    let waiting = fixture.blocked_preparation().await;
    tokio::time::timeout(
        Duration::from_secs(1),
        fixture.engine.pause_task(&fixture.task.id),
    )
    .await
    .unwrap()
    .unwrap();
    fixture.settled(waiting).await;
    let mut restarted = fixture.task.clone();
    tokio::time::timeout(
        Duration::from_secs(2),
        fixture
            .engine
            .prepare_seeding(&Default::default(), &mut restarted),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        fixture
            .engine
            .status_of(&fixture.task.id, "")
            .await
            .unwrap()
            .downloaded_bytes,
        4
    );
    fixture.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn explicit_stop_and_late_error_keep_the_completed_record_and_false_intent() {
    let fixture = Fixture::new().await;
    let data = fixture._root.path().join("database");
    let store = tokio::task::spawn_blocking(move || Store::open(data))
        .await
        .unwrap()
        .unwrap();
    store.upsert_task(&fixture.task).await.unwrap();
    let waiting = fixture.blocked_preparation().await;
    store
        .update_bt_seed_request(&fixture.task.id, false, None)
        .await
        .unwrap();
    fixture.engine.pause_task(&fixture.task.id).await.unwrap();
    fixture.settled(waiting).await;
    assert!(store
        .update_bt_seed_error(&fixture.task.id, "late cancelled preparation".into())
        .await
        .unwrap()
        .is_none());
    let current = store.get_task(&fixture.task.id).await.unwrap().unwrap();
    assert_eq!(current.status, TaskStatus::Completed);
    assert!(!current.bt_meta.unwrap().seeding_requested);
    assert!(current.error.is_none());
    store.remove_task(&fixture.task.id).await.unwrap();
    assert!(store
        .update_bt_seed_error(&fixture.task.id, "deleted preparation".into())
        .await
        .unwrap()
        .is_none());
    assert!(store.get_task(&fixture.task.id).await.unwrap().is_none());
    fixture.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn deletion_guard_cancels_a_blocked_recheck_before_waiting_for_the_lease() {
    let fixture = Fixture::new().await;
    let waiting = fixture.blocked_preparation().await;
    let removal = tokio::time::timeout(Duration::from_secs(1), fixture.engine.removal_guard())
        .await
        .unwrap();
    fixture.settled_without_lock_check(waiting).await;
    removal.remove_task_for(&fixture.task).await.unwrap();
    assert!(fixture.engine.bindings.lock().await.is_empty());
    assert_eq!(
        tokio::fs::read(
            task_output_folder(&fixture.task)
                .await
                .unwrap()
                .join("file.bin")
        )
        .await
        .unwrap(),
        b"DATA"
    );
    drop(removal);
    fixture.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn global_seed_off_and_shutdown_both_cancel_preparation() {
    for shutdown in [false, true] {
        let fixture = Fixture::new().await;
        let waiting = fixture.blocked_preparation().await;
        if shutdown {
            let engine = fixture.engine.clone();
            let stopping = tokio::spawn(async move { engine.shutdown().await });
            // Preparation itself must stop within one second. The native Session::stop
            // deliberately adds a separate one-second grace interval before it returns.
            fixture.settled_without_lock_check(waiting).await;
            assert!(!fixture.engine.has_seed_preparation(&fixture.task.id));
            tokio::time::timeout(Duration::from_secs(3), stopping)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(fixture.engine.seed_sync.try_lock().is_ok());
        } else {
            let mut settings = crate::models::AppSettings::default();
            settings.bt_seed_enabled = false;
            fixture.engine.apply_settings(&settings).await.unwrap();
            fixture.settled(waiting).await;
        }
        fixture.engine.shutdown().await.unwrap();
    }
}

impl Fixture {
    async fn settled_without_lock_check(&self, waiting: tokio::task::JoinHandle<()>) {
        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .unwrap()
            .unwrap();
    }
}
