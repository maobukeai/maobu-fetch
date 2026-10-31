use super::*;

pub(super) struct TransferConfig {
    pub client: reqwest::Client,
    pub headers: HashMap<String, String>,
    pub url: String,
    pub task_id: String,
    pub cloud_binary_expected: bool,
    pub if_range: Option<String>,
    pub etag: Option<String>,
    pub modified: Option<String>,
    pub retry: RetryPolicy,
    pub buffer_size: usize,
    pub limiter: Arc<RateLimiter>,
    pub bandwidth: BandwidthScheduler,
}

pub(super) struct Session {
    pub plan: plan::Plan,
    pub connections: u8,
    pub total: u64,
    pub progress: Arc<AtomicU64>,
    pub adaptive: Arc<AdaptiveConnectionGate>,
    pub options: Arc<RuntimeTaskOptions>,
    pub token: CancellationToken,
    pub worker_cancel: CancellationToken,
    pub reporter_stop: CancellationToken,
    pub disk: Arc<disk::DiskState>,
    pub config: TransferConfig,
}

impl Session {
    pub async fn new(
        manager: &DownloadManager,
        task: &DownloadTask,
        client: &reqwest::Client,
        temp: &Path,
        total: u64,
        connections: u8,
        token: CancellationToken,
        limiter: Arc<RateLimiter>,
    ) -> Result<Self, String> {
        let connections = connections.clamp(1, 32);
        let plan = plan::prepare(task, temp, total, connections).await?;
        let progress = Arc::new(AtomicU64::new(plan.initial));
        let adaptive = Arc::new(AdaptiveConnectionGate::new(connections));
        if connections >= 16
            || [
                "mypikpak.com",
                "pikpak",
                "quark.cn",
                "baidupcs.com",
                "123pan",
                "lanzou",
            ]
            .iter()
            .any(|host| task.url.contains(host))
        {
            adaptive.user_disabled.store(1, Ordering::Relaxed);
        }
        let settings = manager.settings().await;
        let config = TransferConfig {
            client: client.clone(),
            // Keep the Netscape jar intact; the per-hop sender formats it for each actual URL.
            headers: task.headers.clone(),
            url: task.url.clone(),
            task_id: task.id.clone(),
            cloud_binary_expected: cloud_content::requires_binary(task),
            if_range: if plan.initial > 0 {
                http_response::if_range_validator(task).map(str::to_owned)
            } else {
                None
            },
            etag: task.etag.clone(),
            modified: task.last_modified.clone(),
            retry: effective_retry_policy(task, &settings),
            buffer_size: if settings.low_memory_mode {
                64 * 1024
            } else {
                1024 * 1024
            },
            limiter,
            bandwidth: manager.bandwidth_scheduler.clone(),
        };
        Ok(Self {
            plan,
            connections,
            total,
            progress,
            adaptive,
            options: manager.runtime_task_options(task).await,
            worker_cancel: token.child_token(),
            token,
            reporter_stop: CancellationToken::new(),
            disk: Arc::new(disk::DiskState::default()),
            config,
        })
    }
}
