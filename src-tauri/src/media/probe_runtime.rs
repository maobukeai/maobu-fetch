use super::*;

pub(super) struct ProbeSession<'a> {
    pub app: &'a AppHandle,
    pub settings: &'a AppSettings,
    pub source_url: &'a str,
    pub platform: MediaPlatform,
    pub cookie: Option<&'a str>,
    pub referer: Option<&'a str>,
    pub user_agent: Option<&'a str>,
    pub include_collection: bool,
    pub client: &'a reqwest::Client,
    pub proxy_address: &'a str,
    pub cancel: &'a CancellationToken,
    pub metadata_task: Option<&'a DownloadTask>,
    pub data_dir: &'a Path,
}
impl ProbeSession<'_> {
    pub(super) fn cloud_context(
        &self,
    ) -> Result<crate::cloud_request::CloudRequestContext, String> {
        let no_redirect = reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(self.proxy_address).map_err(|_| "媒体网盘代理配置失败")?)
            .timeout(std::time::Duration::from_secs(12))
            .connect_timeout(std::time::Duration::from_secs(6))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "创建媒体网盘连接失败")?;
        Ok(crate::cloud_request::CloudRequestContext::new(
            self.client.clone(),
            no_redirect,
            self.cancel.clone(),
        ))
    }
    pub(super) async fn yt_dlp(&self, url: &str) -> Result<Value, String> {
        if self.cancel.is_cancelled() {
            return Err("任务已暂停".into());
        }
        probe_via_yt_dlp(
            self.app,
            self.settings,
            url,
            self.source_url,
            self.data_dir,
            self.platform,
            self.cookie,
            self.referer,
            self.user_agent,
            self.include_collection,
            self.proxy_address,
            self.cancel,
        )
        .await
    }
}

pub async fn probe(
    app: &AppHandle,
    settings: &AppSettings,
    url: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
) -> Result<MediaProbeResult, String> {
    probe_with_options(app, settings, url, cookie, referer, user_agent, false).await
}

/// 桌面用户明确分析合集；扩展和自动任务仍只分析当前视频。
pub async fn probe_collection(
    app: &AppHandle,
    settings: &AppSettings,
    url: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
) -> Result<MediaProbeResult, String> {
    probe_with_options(app, settings, url, cookie, referer, user_agent, true).await
}

async fn probe_with_options(
    app: &AppHandle,
    settings: &AppSettings,
    url: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
    include_collection: bool,
) -> Result<MediaProbeResult, String> {
    probe_controlled(
        app,
        settings,
        url,
        cookie,
        referer,
        user_agent,
        include_collection,
        network_policy::NetworkPolicy::resolve(settings, None)?,
        &CancellationToken::new(),
        None,
        None,
        None,
    )
    .await
}

/// Task probes inherit task-level proxy overrides and cancellation without changing /v1.
pub async fn probe_for_task(
    app: &AppHandle,
    settings: &AppSettings,
    task: &DownloadTask,
    cancel: &CancellationToken,
    manager: &crate::manager::DownloadManager,
) -> Result<MediaProbeResult, String> {
    let (cookie, referer, user_agent, headers) = split_auth_headers(&task.headers);
    auth_scope::validate_external_headers(
        headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
    )?;
    let runtime = manager
        .task_runtime
        .read()
        .await
        .get(&task.id)
        .cloned()
        .unwrap_or_else(|| std::sync::Arc::new(crate::manager::RuntimeTaskOptions::new(task)));
    let budget = traffic_proxy::TrafficBudget::new(
        task.id.clone(),
        manager.bandwidth_scheduler.clone(),
        runtime,
    );
    probe_controlled(
        app,
        settings,
        &task.url,
        cookie.as_deref(),
        referer.as_deref(),
        user_agent.as_deref(),
        false,
        network_policy::NetworkPolicy::resolve(settings, Some(task))?,
        cancel,
        Some(budget),
        Some(task),
        Some(manager.store.data_dir()),
    )
    .await
}

async fn probe_controlled(
    app: &AppHandle,
    settings: &AppSettings,
    url: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
    include_collection: bool,
    policy: network_policy::NetworkPolicy,
    cancel: &CancellationToken,
    budget: Option<traffic_proxy::TrafficBudget>,
    metadata_task: Option<&DownloadTask>,
    data_dir: Option<&Path>,
) -> Result<MediaProbeResult, String> {
    let data_dir = data_dir
        .map(Path::to_owned)
        .unwrap_or_else(|| portable::resolve_data_dir(app));
    let operation_cancel = cancel.child_token();
    let proxy = traffic_proxy::TrafficProxy::start(policy, budget, &operation_cancel).await?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(proxy.address()).map_err(|_| "媒体探测通道配置失败")?)
        .timeout(std::time::Duration::from_secs(12))
        .connect_timeout(std::time::Duration::from_secs(6))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|_| "创建媒体探测通道失败")?;
    let limit = metadata_task
        .map(|task| {
            crate::manager::effective_retry_policy(task, settings)
                .task_timeout_secs
                .unwrap_or(90)
                .clamp(1, 90)
        })
        .unwrap_or(90);
    let mut operation = Box::pin(probe_inner(
        app,
        settings,
        url,
        cookie,
        referer,
        user_agent,
        include_collection,
        &client,
        proxy.address(),
        &operation_cancel,
        metadata_task,
        &data_dir,
    ));
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            operation_cancel.cancel();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), operation.as_mut()).await;
            Err("任务已暂停".into())
        },
        _ = tokio::time::sleep(std::time::Duration::from_secs(limit)) => {
            operation_cancel.cancel();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), operation.as_mut()).await;
            Err("媒体分析超过时限，请检查网络、代理与登录权限后重试".into())
        },
        result = operation.as_mut() => result,
    };
    drop(operation);
    proxy.shutdown().await;
    result
}

async fn probe_inner(
    app: &AppHandle,
    settings: &AppSettings,
    url: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
    include_collection: bool,
    client: &reqwest::Client,
    proxy_address: &str,
    cancel: &CancellationToken,
    metadata_task: Option<&DownloadTask>,
    data_dir: &Path,
) -> Result<MediaProbeResult, String> {
    let ffmpeg_available = resolve_ffmpeg(app, settings).is_some();
    // Task 41.2：用户可能粘贴分享文本（如"xxx https://v.douyin.com/yyy 复制此链接..."），
    // 先从中提取首个 URL。纯 URL 输入会被原样返回；无 URL 时回退到原输入，
    // 让后续 url::Url::parse 给出"媒体地址无效"中文错误（AGENTS.md §7）。
    let extracted_url = extract_url_from_share_text(url).unwrap_or_else(|| url.to_string());
    let parsed = url::Url::parse(&extracted_url).map_err(|_| "媒体地址无效".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("媒体地址无效".into());
    }
    // Task 37.3：识别平台，用于失败时返回平台特定的中文错误。
    let platform = detect_platform(&extracted_url);
    // Task 37.3 / Task 41.1：短链先跟随重定向到最终 URL，再传给 yt-dlp。
    // 失败时不阻断流程（yt-dlp 自身也能处理部分短链），仅在错误信息中体现。
    let expanded =
        match crate::media_platforms::expand_short_url_with_client(&extracted_url, client).await {
            Ok(final_url) => final_url,
            Err(_) => extracted_url.clone(),
        };
    // Task 41.3：剥离跟踪参数（utm_* / fbclid / gclid 等白名单），
    // 保留业务必需参数（sign / token / auth / X-Amz-Signature 等）。
    let effective_url =
        crate::media_platforms::normalize_bilibili_video_url(&strip_tracking_params(&expanded));
    // 抖音优先走直连解析（绕过 byted_acrawler 反爬）。
    // 抖音 web 页面首次访问会返回 JS 挑战页，yt-dlp 无法获取元数据；
    // 而抖音 aweme_detail API 仅需 ttwid + UA + Referer，无需复杂签名。
    // 失败时回退到 yt-dlp 流程，保证用户已安装的旧版 yt-dlp 仍可用。
    //
    // 抖音直播（live.douyin.com）yt-dlp 官方不支持，需要直连请求直播页面 HTML，
    // 从中提取 HLS/FLV 流地址和元数据（标题、主播名），交给 yt-dlp 录制 HLS 流。
    let session = ProbeSession {
        app,
        settings,
        source_url: &extracted_url,
        platform,
        cookie,
        referer,
        user_agent,
        include_collection,
        client,
        proxy_address,
        cancel,
        metadata_task,
        data_dir,
    };
    let value = match platform {
        MediaPlatform::Douyin => super::probe_platforms::douyin(&session, &effective_url).await?,
        MediaPlatform::Twitter if !is_twitter_space(&effective_url) => {
            super::probe_platforms::twitter(&session, &effective_url).await?
        }
        MediaPlatform::PikPak => {
            return super::probe_platforms::pikpak(&session, &effective_url, ffmpeg_available).await
        }
        _ => session.yt_dlp(&effective_url).await?,
    };
    drm::ensure_unprotected(&value)?;
    if let Some(task) = metadata_task {
        output::record_probe_metadata(task, &value, platform).await?;
    } else {
        let metadata = probe_cache::minimal(&value, platform);
        probe_cache::save(data_dir, url, cookie, &metadata).await?;
    }
    super::probe_result::parse(
        value,
        &effective_url,
        platform,
        include_collection,
        ffmpeg_available,
    )
}

/// 通过 yt-dlp 子进程探测媒体元数据（probe 的 yt-dlp 路径）。
///
/// 抽出为独立函数以便抖音直连解析失败时复用同一段 yt-dlp 调用逻辑。
///
/// 安全约束（AGENTS.md §3 / §7）：
/// - Cookie / Referer / User-Agent 通过 `attach_auth_args` 安全传递（不写入命令行）
/// - stderr 在分类前由 `redact_sensitive` 脱敏，不泄露认证信息
/// - 错误按平台分类后返回中文文案
async fn probe_via_yt_dlp(
    app: &AppHandle,
    settings: &AppSettings,
    effective_url: &str,
    cookie_source_url: &str,
    data_dir: &Path,
    platform: MediaPlatform,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
    include_collection: bool,
    proxy_address: &str,
    cancel: &CancellationToken,
) -> Result<Value, String> {
    let yt = resolve_yt_dlp(app, settings)
        .ok_or("MEDIA_YT_DLP_MISSING: 分析媒体需要先安装 yt-dlp 基础组件")?;
    let mut command = create_hidden_tokio_command(yt);
    traffic_proxy::configure_command(&mut command, proxy_address);
    command
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1");
    command.args(["--dump-single-json", "--no-warnings", "--no-check-formats"]);
    collection::configure_probe_playlist(&mut command, effective_url, platform, include_collection);
    // YouTube 反爬虫对策（AGENTS.md §3：不得绕过 DRM，但 PO Token 不是 DRM）：
    // YouTube 自 2024 年起强制 PO Token 验证，yt-dlp 无法自行生成，
    // 普通浏览器 Cookie 会被 YouTube 频繁轮换失效。
    // 使用 player_client 按序回退：默认客户端优先（格式最全），
    // android_vr 客户端不需要 PO Token（限制："Made for kids" 视频不可用），
    // web_safari 客户端的 HLS 格式不需要 GVS PO Token。
    // 参考：https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide
    let cookie_guard = attach_auth_args_in_dir(
        &mut command,
        &data_dir.join("cookies_tmp"),
        cookie_source_url,
        cookie,
        referer,
        user_agent,
    )
    .await?;

    let mut options = vec![
        ("--proxy", proxy_address.to_owned()),
        ("--socket-timeout", "15".into()),
    ];
    if platform == MediaPlatform::YouTube {
        options.push((
            "--extractor-args",
            build_youtube_extractor_arg(&settings.youtube_po_token, cookie_guard.path().is_some()),
        ));
    }
    let private = command_config::PrivateConfig::create(
        &data_dir.join("cookies_tmp"),
        &options,
        Some(effective_url),
    )
    .await?;
    private.attach(&mut command);
    let output = process::capture(
        &mut command,
        cancel,
        std::time::Duration::from_secs(70),
        8 * 1024 * 1024,
    )
    .await?;
    // 显式删除临时 cookie 文件（即使分析失败也会通过 drop 删除，这里主动清理）
    cookie_guard.consume().await;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        // Task 37.6：按平台分类错误并返回中文文案。
        // AGENTS.md §3：认证信息不得泄露；stderr 在分类前由 redact_sensitive 脱敏。
        let redacted = crate::manager::redact_sensitive(&stderr);

        let platform_error = classify_platform_error(platform, &redacted);
        let chinese = platform_error_to_chinese_with_evidence(platform_error, platform, &redacted);
        // DrmProtected 错误前缀加 MEDIA_DRM_ 以便前端识别并拒绝（AGENTS.md §6）
        if platform_error == crate::manager::diagnose::MediaPlatformError::DrmProtected {
            return Err(format!("MEDIA_DRM_PROTECTED: {chinese}"));
        }
        return Err(chinese);
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|_| "媒体工具返回的探测数据不是有效JSON，请更新组件后重试".into())
}
