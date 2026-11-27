use super::*;

pub(super) struct TransferConfig {
    pub client: reqwest::Client,
    pub headers: HashMap<String, String>,
    pub url: String,
    pub task_id: String,
    pub content_policy: cloud_content::ResponsePolicy,
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

pub(super) fn resolve_session_target(
    task: &DownloadTask,
    resolved_url: Option<&str>,
) -> (String, HashMap<String, String>) {
    match resolved_url
        .map(str::trim)
        .filter(|url| !url.is_empty() && *url != task.url)
    {
        Some(target) => (
            target.to_string(),
            request_headers::retarget(&task.headers, &task.url, target),
        ),
        None => (task.url.clone(), task.headers.clone()),
    }
}

impl Session {
    pub async fn new(
        manager: &DownloadManager,
        task: &DownloadTask,
        client: &reqwest::Client,
        temp: &Path,
        total: u64,
        connections: u8,
        resolved_url: Option<&str>,
        token: CancellationToken,
        limiter: Arc<RateLimiter>,
    ) -> Result<Self, String> {
        let connections = connections.clamp(1, 32);
        let plan = plan::prepare(task, temp, total, connections).await?;
        let progress = Arc::new(AtomicU64::new(plan.initial));
        let (effective_url, effective_headers) = resolve_session_target(task, resolved_url);
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
            .any(|host| task.url.contains(host) || effective_url.contains(host))
        {
            adaptive.user_disabled.store(1, Ordering::Relaxed);
        }
        let settings = manager.settings().await;
        let config = TransferConfig {
            client: client.clone(),
            // Keep the Netscape jar intact; the per-hop sender formats it for each actual URL.
            headers: effective_headers,
            url: effective_url,
            task_id: task.id.clone(),
            content_policy: cloud_content::ResponsePolicy::for_task(task),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirected_target_retargets_headers_without_mutating_task() {
        let mut task = super::super::super::tests::test_task(
            Path::new("C:\\downloads"),
            "123.zip",
            CollisionPolicy::Rename,
        );
        task.url = "https://d.pcs.baidu.com/file/sample?sign=abc".into();
        task.headers.insert("Cookie".into(), "BDUSS=synthetic".into());
        task.headers.insert("User-Agent".into(), "pan.baidu.com".into());
        task.headers.insert("Referer".into(), "https://pan.baidu.com/disk/home".into());

        let cdn = "https://bdbl-cm01.baidupcs.com/file/sample?sig=cdn";
        let (url, headers) = resolve_session_target(&task, Some(cdn));
        assert_eq!(url, cdn);
        assert!(!headers.keys().any(|k| k.eq_ignore_ascii_case("cookie")));
        assert_eq!(headers.get("User-Agent").map(String::as_str), Some("pan.baidu.com"));
        assert_eq!(
            headers.get("Referer").map(String::as_str),
            Some("https://pan.baidu.com/disk/home")
        );

        let (same_url, same_headers) = resolve_session_target(&task, Some(&task.url));
        assert_eq!(same_url, task.url);
        assert_eq!(
            same_headers.get("Cookie").map(String::as_str),
            Some("BDUSS=synthetic")
        );
    }
}
