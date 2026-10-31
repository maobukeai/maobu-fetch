use super::super::*;
use super::fixture::Fixture;

pub struct Harness {
    pub manager: SharedManager,
    _root: tempfile::TempDir,
    pub output: PathBuf,
    pub data: PathBuf,
}
impl Harness {
    pub async fn new(app: &AppHandle, timeout: Option<u64>) -> Self {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("database");
        let output = root.path().join("downloads");
        fs::create_dir_all(&output).await.unwrap();
        let manager = construct(app.clone(), data.clone(), output.clone(), timeout).await;
        Self {
            manager,
            _root: root,
            output,
            data,
        }
    }
    pub async fn task(&self, fixture: &Fixture, connections: u8) -> DownloadTask {
        let task = self.queued_task(fixture, connections).await;
        self.manager.dispatch_once().await.unwrap();
        task
    }
    pub async fn queued_task(&self, fixture: &Fixture, connections: u8) -> DownloadTask {
        fixture
            .state
            .expected_parallel
            .store(connections as usize, Ordering::Release);
        let mut task =
            super::super::tests::test_task(&self.output, "verified.bin", CollisionPolicy::Rename);
        task.id = Uuid::new_v4().to_string();
        task.url = fixture.url.clone();
        task.max_retries = 0;
        task.connection_count = connections;
        task.created_at = now();
        task.expected_checksum = Some(hex::encode(Sha256::digest(fixture.expected())));
        task.proxy_override = Some(String::new());
        self.manager.store.upsert_task(&task).await.unwrap();
        task
    }
    pub async fn reopen(&mut self, app: &AppHandle) {
        self.manager.shutdown_bt().await.unwrap();
        self.manager = construct(app.clone(), self.data.clone(), self.output.clone(), None).await;
        self.manager.run_startup_selfcheck().await.unwrap();
        self.manager.recover_interrupted().await.unwrap();
    }
    pub async fn row(&self, id: &str) -> DownloadTask {
        self.manager.store.get_task(id).await.unwrap().unwrap()
    }
    pub async fn wait(&self, id: &str, status: TaskStatus) -> DownloadTask {
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                let row = self.row(id).await;
                if row.status == status && !self.manager.controls.lock().await.contains_key(id) {
                    return row;
                }
                assert!(
                    !matches!(row.status, TaskStatus::Failed | TaskStatus::RemoteChanged)
                        || row.status == status,
                    "unexpected task state {:?}: {:?}",
                    row.status,
                    row.error
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("native DownloadManager must settle within bounded time")
    }
    pub async fn pause(&self, id: &str) {
        tokio::time::timeout(Duration::from_secs(2), self.manager.action(id, "pause"))
            .await
            .expect("pause must stop real sockets and flush promptly")
            .unwrap();
        let row = self.row(id).await;
        assert_eq!(row.status, TaskStatus::Paused);
        assert_eq!(row.active_connections, 0);
    }
    pub async fn resume(&self, id: &str) {
        self.manager.action(id, "resume").await.unwrap();
        self.manager.dispatch_once().await.unwrap();
    }
    pub async fn check_output(&self, id: &str, expected: &[u8]) -> DownloadTask {
        let row = self.wait(id, TaskStatus::Completed).await;
        let path = Path::new(&row.destination).join(&row.file_name);
        let bytes = fs::read(path).await.unwrap();
        assert_eq!(bytes.len(), expected.len());
        assert_eq!(Sha256::digest(bytes), Sha256::digest(expected));
        assert_eq!(row.total_bytes, expected.len() as u64);
        assert_eq!(row.downloaded_bytes, expected.len() as u64);
        assert_eq!(row.active_connections, 0);
        row
    }
    pub async fn fingerprints(&self, task: &DownloadTask) -> Vec<(String, u64, String)> {
        let temporary = task_temp_path(&task.destination, &task.id, &task.file_name);
        let mut entries = fs::read_dir(temporary.parent().unwrap()).await.unwrap();
        let mut values = Vec::new();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            if entry.metadata().await.unwrap().is_file() {
                let bytes = fs::read(entry.path()).await.unwrap();
                values.push((
                    entry.file_name().to_string_lossy().into_owned(),
                    bytes.len() as u64,
                    hex::encode(Sha256::digest(bytes)),
                ));
            }
        }
        values.sort();
        values
    }
}

async fn construct(
    app: AppHandle,
    data: PathBuf,
    output: PathBuf,
    timeout: Option<u64>,
) -> SharedManager {
    let directory = data.clone();
    let store = Arc::new(
        tokio::task::spawn_blocking(move || Store::open(directory))
            .await
            .unwrap()
            .unwrap(),
    );
    let mut settings = AppSettings::default();
    settings.download_dir = output.to_string_lossy().into_owned();
    settings.auto_start = false;
    settings.notifications = false;
    settings.notify_on_complete = false;
    settings.notify_on_failure = false;
    settings.clipboard_monitor = false;
    settings.proxy_mode = "none".into();
    settings.concurrent_downloads = 1;
    settings.connections_per_download = 8;
    settings.max_retries = 0;
    settings.default_retry_policy.max_retries = 0;
    settings.default_retry_policy.task_timeout_secs = timeout;
    settings.default_retry_policy.connection_timeout_secs = 2;
    settings.verify_after_download = false;
    settings.media_tool_auto_update = false;
    settings.bt_tracker_auto_update = false;
    settings.auto_check_app_update = false;
    store.save_settings(&settings).await.unwrap();
    let client = build_client(&settings).unwrap();
    Arc::new(DownloadManager {
        store,
        settings: RwLock::new(settings),
        settings_operation: Mutex::new(()),
        client: RwLock::new(client),
        controls: Mutex::new(HashMap::new()),
        task_operations: Mutex::new(HashMap::new()),
        task_runtime: RwLock::new(HashMap::new()),
        path_reservation: Mutex::new(()),
        power_action: Mutex::new(PowerActionRuntime::default()),
        dispatcher: Notify::new(),
        shutting_down: AtomicBool::new(false),
        bandwidth_allocation: Mutex::new(()),
        app,
        bandwidth_scheduler: BandwidthScheduler::new(0),
        bt: crate::bt::BtEngine::with_data_dir(data.join("session")),
    })
}
