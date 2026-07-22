use crate::{
    manager::SharedManager,
    media,
    models::{MediaProbeResult, NewTaskRequest, PairingInfo},
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use hmac::{Hmac, Mac};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::AppHandle;
use tokio::sync::Mutex;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct PairingService {
    inner: Arc<Mutex<PairCode>>,
    manager: SharedManager,
}
#[derive(Clone)]
struct PairCode {
    code: String,
    expires_at: u64,
}

impl PairingService {
    pub fn new(manager: SharedManager) -> Self {
        Self {
            inner: Arc::new(Mutex::new(new_code())),
            manager,
        }
    }
    pub async fn info(&self) -> Result<PairingInfo, String> {
        let mut code = self.inner.lock().await;
        if code.expires_at <= now() {
            *code = new_code()
        }
        let paired = self.manager.store.get_pairing().await?.map(|p| p.0);
        Ok(PairingInfo {
            code: code.code.clone(),
            expires_at: code.expires_at,
            paired_extension: paired,
        })
    }
    pub async fn rotate(&self) -> PairingInfo {
        let mut code = self.inner.lock().await;
        *code = new_code();
        let paired = self
            .manager
            .store
            .get_pairing()
            .await
            .ok()
            .flatten()
            .map(|p| p.0);
        PairingInfo {
            code: code.code.clone(),
            expires_at: code.expires_at,
            paired_extension: paired,
        }
    }
    async fn consume(&self, value: &str) -> bool {
        let mut code = self.inner.lock().await;
        if code.expires_at > now() && code.code == value {
            *code = new_code();
            true
        } else {
            false
        }
    }
}

#[derive(Clone)]
struct BridgeState {
    manager: SharedManager,
    pairing: PairingService,
    app: AppHandle,
    requests: Arc<Mutex<VecDeque<Instant>>>,
    /// `/v1/tasks/recent` 专用速率限制队列：每秒最多 5 次。
    recent_requests: Arc<Mutex<VecDeque<Instant>>>,
}
#[derive(Deserialize)]
struct PairRequest {
    code: String,
    extension_id: String,
}
#[derive(Serialize)]
struct PairResponse {
    token: String,
    api_version: u8,
}
#[derive(Deserialize)]
struct ProbeRequest {
    url: String,
    #[serde(default)]
    cookie: Option<String>,
    #[serde(default)]
    referer: Option<String>,
    #[serde(default)]
    user_agent: Option<String>,
}

#[derive(Deserialize)]
struct MediaCredentialSyncRequest {
    domain: String,
    cookie: String,
}

/// `/v1/tasks/recent` 单条任务摘要（SubTask 13.1）。
///
/// 仅暴露扩展弹窗需要的最小字段集，不包含 destination、headers、media 等敏感
/// 或冗余数据。`progress` 为 0.0..=1.0；`total_bytes == 0` 时为 0。
#[derive(Serialize)]
struct RecentTaskSummary {
    id: String,
    url: String,
    file_name: String,
    status: String,
    progress: f64,
    speed: u64,
    error: Option<String>,
}

#[derive(Serialize)]
struct RecentTasksResponse {
    tasks: Vec<RecentTaskSummary>,
}

/// `/v1/tasks/{id}/action` 请求体（SubTask 13.2）。
///
/// `action` 仅允许 `pause` / `resume` / `open_file` 三个值，
/// 其它值返回 400。
#[derive(Deserialize)]
struct ActionRequest {
    action: String,
}

#[derive(Serialize)]
struct ActionResponse {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub async fn run(manager: SharedManager, pairing: PairingService, app: AppHandle) {
    let state = BridgeState {
        manager,
        pairing,
        app,
        requests: Arc::new(Mutex::new(VecDeque::new())),
        recent_requests: Arc::new(Mutex::new(VecDeque::new())),
    };
    let router = Router::new()
        .route("/v1/health", get(health))
        .route("/v1/pair", post(pair))
        .route("/v1/tasks", post(add_task))
        .route("/v1/tasks/recent", get(recent_tasks))
        .route("/v1/tasks/{id}/action", post(task_action))
        .route("/v1/media/probe", post(probe_media))
        .route("/v1/media/credentials/sync", post(sync_media_credentials))
        .with_state(state);
    if let Ok(listener) = tokio::net::TcpListener::bind("127.0.0.1:17433").await {
        let _ = axum::serve(listener, router).await;
    }
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({"name":"Maobu Fetch","api_version":1,"ready":true}))
}
async fn pair(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Json(request): Json<PairRequest>,
) -> Result<Json<PairResponse>, (StatusCode, String)> {
    validate_extension_id(&request.extension_id)?;
    validate_origin(&headers, &request.extension_id)?;
    if !state.pairing.consume(request.code.trim()).await {
        return Err((StatusCode::UNAUTHORIZED, "配对码无效或已过期".into()));
    }
    let token = hex::encode(rand::random::<[u8; 32]>());
    let hash = hex::encode(Sha256::digest(token.as_bytes()));
    state
        .manager
        .store
        .save_pairing(&request.extension_id, &hash)
        .await
        .map_err(internal)?;
    Ok(Json(PairResponse {
        token,
        api_version: 1,
    }))
}
async fn add_task(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    authorize(&state, &headers, &body).await?;
    let mut request: NewTaskRequest = serde_json::from_slice(&body)
        .map_err(|_| (StatusCode::BAD_REQUEST, "任务参数无效".into()))?;
    request.source = Some("browser".into());
    let task = state
        .manager
        .add(request)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok((StatusCode::CREATED, Json(task)))
}

/// `GET /v1/tasks/recent`（SubTask 13.1）。
///
/// 返回最近 5 条由扩展发送的任务（`source == "browser"`，与 `add_task` 设置一致）。
/// 走完整 HMAC + 时间戳 + Origin 校验；额外应用每秒 5 次的专用速率限制。
/// 排序：按 `created_at` 倒序取前 5 条。
async fn recent_tasks(
    State(state): State<BridgeState>,
    headers: HeaderMap,
) -> Result<Json<RecentTasksResponse>, (StatusCode, String)> {
    // GET 请求无 body，签名覆盖 `timestamp\n`（空 body），与扩展端 `signedGet` 一致。
    authorize(&state, &headers, &[]).await?;
    rate_limit_recent(&state.recent_requests).await?;
    let all = state.manager.list().await.map_err(internal)?;
    let mut extension_tasks: Vec<_> = all
        .into_iter()
        .filter(|task| task.source == "browser")
        .collect();
    extension_tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    let summaries = extension_tasks
        .into_iter()
        .take(5)
        .map(|task| {
            let progress = if task.total_bytes > 0 {
                (task.downloaded_bytes as f64 / task.total_bytes as f64).clamp(0.0, 1.0)
            } else {
                0.0
            };
            RecentTaskSummary {
                id: task.id,
                url: task.url,
                file_name: task.file_name,
                status: task.status.as_str().to_string(),
                progress,
                speed: task.speed,
                error: task.error,
            }
        })
        .collect();
    Ok(Json(RecentTasksResponse { tasks: summaries }))
}

/// `POST /v1/tasks/{id}/action`（SubTask 13.2）。
///
/// 支持 `pause` / `resume` / `open_file`：
/// - `pause`/`resume` 复用 `DownloadManager::action`，与桌面端 `task_action` 命令一致。
/// - `open_file` 复用 `open::that`，与桌面端 `task_open_file` 命令一致；
///   仅对已完成任务有意义，但不在桥层强制状态校验，由桌面端 UI/文件系统负责。
async fn task_action(
    State(state): State<BridgeState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    authorize(&state, &headers, &body).await?;
    let request: ActionRequest = serde_json::from_slice(&body)
        .map_err(|_| (StatusCode::BAD_REQUEST, "操作参数无效".into()))?;
    // 统一错误类型为 `Result<(), String>`，最终包装为 `ActionResponse`。
    // 这样 HTTP 层始终返回 200 + `{success, error?}`，便于扩展弹窗统一处理。
    let result: Result<(), String> = match request.action.as_str() {
        "pause" | "resume" => state.manager.action(&id, &request.action).await,
        "open_file" => open_file_for_task(&state, &id).await,
        other => Err(format!("不支持的操作: {other}")),
    };
    match result {
        Ok(()) => Ok(Json(ActionResponse {
            success: true,
            error: None,
        })),
        Err(error) => Ok(Json(ActionResponse {
            success: false,
            error: Some(error),
        })),
    }
}

/// 打开任务目标文件（SubTask 13.2 辅助函数）。
///
/// 与 `lib.rs::task_open_file` 行为一致：通过 `open::that` 调用系统默认程序。
/// 错误统一返回 `String`，由调用方包装为 `ActionResponse.error`。
async fn open_file_for_task(state: &BridgeState, id: &str) -> Result<(), String> {
    let task = match state.manager.store.get_task(id).await {
        Ok(Some(task)) => task,
        Ok(None) => return Err("任务不存在".into()),
        Err(error) => return Err(error),
    };
    open::that(std::path::PathBuf::from(task.destination).join(task.file_name))
        .map_err(|e| e.to_string())
}
async fn probe_media(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<MediaProbeResult>, (StatusCode, String)> {
    authorize(&state, &headers, &body).await?;
    let request: ProbeRequest = serde_json::from_slice(&body)
        .map_err(|_| (StatusCode::BAD_REQUEST, "媒体参数无效".into()))?;
    let settings = state.manager.settings().await;
    media::probe(
        &state.app,
        &settings,
        &request.url,
        request.cookie.as_deref(),
        request.referer.as_deref(),
        request.user_agent.as_deref(),
    )
    .await
    .map(Json)
    .map_err(|e| (StatusCode::BAD_REQUEST, e))
}

async fn sync_media_credentials(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    authorize(&state, &headers, &body).await?;
    let req: MediaCredentialSyncRequest = serde_json::from_slice(&body)
        .map_err(|_| (StatusCode::BAD_REQUEST, "参数无效".into()))?;

    let domain = req.domain.trim().to_lowercase();
    if domain.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "域名不能为空".into()));
    }

    let supported = [
        "douyin.com",
        "iesdouyin.com",
        "douyinvod.com",
        "amemv.com",
        "tiktok.com",
        "bilibili.com",
        "weibo.com",
        "weibo.cn",
        "youtube.com",
        "twitter.com",
        "x.com",
    ];
    let is_supported = supported
        .iter()
        .any(|&d| domain == d || domain.ends_with(&format!(".{}", d)));
    if !is_supported {
        return Err((StatusCode::BAD_REQUEST, "不支持的媒体域名".into()));
    }

    let mut cred = crate::models::MediaCredential {
        domain: domain.clone(),
        cookie: req.cookie,
        referer: None,
        user_agent: None,
        updated_at: crate::now_iso8601_utc(),
    };

    if let Ok(Some(existing)) = state.manager.store.media_credential_get_matching(&domain).await {
        // 保护用户手动配置：如果已存在的 Cookie 是 Netscape cookies.txt 格式
        // （只能由用户从「导出 cookies.txt」按钮导出后手动导入，扩展自动同步
        // 永远只发 HTTP 头格式），则不允许被自动同步覆盖。这避免了
        // "用户导出无痕窗口 cookies.txt 导入 → 打开 youtube.com 网页 →
        //   扩展自动同步把普通窗口 Cookie 覆盖掉手动配置" 的数据丢失场景。
        // 参见 AGENTS.md §7：不得改变用户设置，除非操作由用户明确触发。
        if crate::media_cookies::should_skip_auto_sync(&existing.cookie, &cred.cookie) {
            return Ok(StatusCode::OK);
        }
        cred.referer = existing.referer;
        cred.user_agent = existing.user_agent;
    }

    state
        .manager
        .store
        .media_credential_upsert(cred)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(StatusCode::OK)
}


async fn authorize(
    state: &BridgeState,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(), (StatusCode, String)> {
    rate_limit(&state.requests).await?;
    let extension = headers
        .get("x-luma-extension")
        .and_then(|v| v.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "缺少扩展标识".into()))?;
    validate_origin(headers, extension)?;
    let timestamp = headers
        .get("x-luma-timestamp")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "缺少时间戳".into()))?;
    if now().abs_diff(timestamp) > 5 * 60 * 1000 {
        return Err((StatusCode::UNAUTHORIZED, "请求已过期".into()));
    }
    let signature = headers
        .get("x-luma-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "缺少签名".into()))?;
    let Some((paired, hash)) = state.manager.store.get_pairing().await.map_err(internal)? else {
        return Err((StatusCode::UNAUTHORIZED, "尚未配对".into()));
    };
    if paired != extension {
        return Err((StatusCode::UNAUTHORIZED, "扩展标识不匹配".into()));
    }
    let key = hex::decode(hash)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "配对数据损坏".into()))?;
    let mut mac = HmacSha256::new_from_slice(&key)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "无法验证签名".into()))?;
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b"\n");
    mac.update(body);
    let supplied =
        hex::decode(signature).map_err(|_| (StatusCode::UNAUTHORIZED, "签名格式无效".into()))?;
    mac.verify_slice(&supplied)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "签名验证失败".into()))
}
async fn rate_limit(queue: &Mutex<VecDeque<Instant>>) -> Result<(), (StatusCode, String)> {
    let mut queue = queue.lock().await;
    let cutoff = Instant::now() - Duration::from_secs(60);
    while queue.front().is_some_and(|v| *v < cutoff) {
        queue.pop_front();
    }
    if queue.len() >= 120 {
        return Err((StatusCode::TOO_MANY_REQUESTS, "请求过于频繁".into()));
    }
    queue.push_back(Instant::now());
    Ok(())
}

/// `/v1/tasks/recent` 专用速率限制：滑动 1 秒窗口内最多 5 次请求。
///
/// 独立于全局 `rate_limit`（120 次/分钟），避免扩展弹窗的轮询挤占其他端点配额；
/// 两个限制器叠加生效，取更严格的那个。
async fn rate_limit_recent(queue: &Mutex<VecDeque<Instant>>) -> Result<(), (StatusCode, String)> {
    let mut queue = queue.lock().await;
    let now = Instant::now();
    let cutoff = now - Duration::from_secs(1);
    while queue.front().is_some_and(|v| *v < cutoff) {
        queue.pop_front();
    }
    if queue.len() >= 5 {
        return Err((StatusCode::TOO_MANY_REQUESTS, "请求过于频繁".into()));
    }
    queue.push_back(now);
    Ok(())
}
fn validate_origin(headers: &HeaderMap, extension: &str) -> Result<(), (StatusCode, String)> {
    let origin = headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let chrome = format!("chrome-extension://{extension}");
    let edge = format!("edge-extension://{extension}");
    if origin == chrome || origin == edge {
        Ok(())
    } else {
        Err((StatusCode::FORBIDDEN, "Origin 不受信任".into()))
    }
}
fn validate_extension_id(value: &str) -> Result<(), (StatusCode, String)> {
    if value.len() >= 16
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        Ok(())
    } else {
        Err((StatusCode::BAD_REQUEST, "扩展标识无效".into()))
    }
}
fn new_code() -> PairCode {
    let code = format!("{:06}", rand::rng().random_range(0..1_000_000));
    PairCode {
        code,
        expires_at: now() + 10 * 60 * 1000,
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn internal(error: String) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    #[test]
    fn validates_extension_identifiers() {
        assert!(validate_extension_id("abcdefghijklmnop").is_ok());
        assert!(validate_extension_id("bad id").is_err())
    }
    #[test]
    fn requires_exact_extension_origin() {
        let id = "abcdefghijklmnop";
        let mut headers = HeaderMap::new();
        assert!(validate_origin(&headers, id).is_err());
        headers.insert(
            "origin",
            HeaderValue::from_static("chrome-extension://abcdefghijklmnop"),
        );
        assert!(validate_origin(&headers, id).is_ok());
        headers.insert("origin", HeaderValue::from_static("http://localhost"));
        assert!(validate_origin(&headers, id).is_err());
    }

    #[tokio::test]
    async fn rate_limit_recent_allows_five_per_second_then_rejects() {
        let queue: Arc<Mutex<VecDeque<Instant>>> = Arc::new(Mutex::new(VecDeque::new()));
        for _ in 0..5 {
            assert!(rate_limit_recent(&queue).await.is_ok());
        }
        let blocked = rate_limit_recent(&queue).await;
        assert!(blocked.is_err());
        let (status, message) = blocked.unwrap_err();
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(message, "请求过于频繁");
    }

    #[tokio::test]
    async fn rate_limit_recent_recovers_after_window_expires() {
        // 预填 5 条已过期记录（1.5 秒前），新请求应被允许。
        let queue: Arc<Mutex<VecDeque<Instant>>> = Arc::new(Mutex::new(VecDeque::from_iter(
            std::iter::repeat(Instant::now() - Duration::from_millis(1500)).take(5),
        )));
        assert!(rate_limit_recent(&queue).await.is_ok());
    }

    #[test]
    fn recent_task_summary_omits_sensitive_fields() {
        // 序列化字段集合必须与协议文档一致：不包含 destination/headers/media/etag 等。
        let summary = RecentTaskSummary {
            id: "task-1".into(),
            url: "https://example.com/file.zip".into(),
            file_name: "file.zip".into(),
            status: "downloading".into(),
            progress: 0.5,
            speed: 1024,
            error: None,
        };
        let value = serde_json::to_value(&summary).unwrap();
        let object = value.as_object().unwrap();
        assert!(object.contains_key("id"));
        assert!(object.contains_key("url"));
        assert!(object.contains_key("file_name"));
        assert!(object.contains_key("status"));
        assert!(object.contains_key("progress"));
        assert!(object.contains_key("speed"));
        assert!(object.contains_key("error"));
        assert!(!object.contains_key("destination"));
        assert!(!object.contains_key("headers"));
        assert!(!object.contains_key("media"));
        assert!(!object.contains_key("etag"));
    }

    #[test]
    fn action_response_omits_error_when_success() {
        let success = ActionResponse {
            success: true,
            error: None,
        };
        let value = serde_json::to_string(&success).unwrap();
        assert_eq!(value, "{\"success\":true}");
    }

    #[test]
    fn action_request_deserializes_known_actions() {
        for action in ["pause", "resume", "open_file"] {
            let request: ActionRequest =
                serde_json::from_str(&format!("{{\"action\":\"{action}\"}}")).unwrap();
            assert_eq!(request.action, action);
        }
    }

    // Task 45.6：扩展 /v1/tasks/add 端点接收 cookie/referer/user_agent 字段后
    // 必须正确传递给 manager.add。通过反序列化验证 NewTaskRequest 能从扩展
    // 发来的 JSON body 中正确读取 headers.Cookie / headers.Referer / headers.User-Agent。
    #[test]
    fn add_task_request_deserializes_cookie_headers_from_extension() {
        // 模拟扩展 popup.js 通过 signedFetch 发送的请求 body：
        // { url, headers: { Cookie, Referer, "User-Agent" }, source, ... }
        let body = serde_json::json!({
            "url": "https://example.com/file.zip",
            "headers": {
                "Cookie": "session=abc123; token=xyz789",
                "Referer": "https://example.com/page",
                "User-Agent": "TestBrowser/1.0"
            },
            "priority": 0,
            "source": "browser",
            "collision_policy": "rename"
        });
        let request: NewTaskRequest = serde_json::from_value(body).unwrap();
        assert_eq!(request.url, "https://example.com/file.zip");
        assert_eq!(
            request.headers.get("Cookie").unwrap(),
            "session=abc123; token=xyz789"
        );
        assert_eq!(
            request.headers.get("Referer").unwrap(),
            "https://example.com/page"
        );
        assert_eq!(
            request.headers.get("User-Agent").unwrap(),
            "TestBrowser/1.0"
        );
        assert_eq!(request.source.as_deref(), Some("browser"));
    }

    // Task 45.6：旧版扩展请求（不含 headers 字段）必须安全反序列化为空 headers，
    // 不破坏向后兼容（AGENTS.md §2）。
    #[test]
    fn add_task_request_defaults_headers_to_empty_when_missing() {
        let body = serde_json::json!({
            "url": "https://example.com/file.zip"
        });
        let request: NewTaskRequest = serde_json::from_value(body).unwrap();
        assert!(request.headers.is_empty());
    }

    // Task 45.4：下载完成后 manager 必须清空 task.headers 中的认证头，
    // 避免临时登录态被持久化。直接测试 clear_auth_headers 辅助函数。
    #[test]
    fn clear_auth_headers_removes_cookie_referer_user_agent_case_insensitively() {
        use crate::manager::clear_auth_headers;
        let mut headers = std::collections::HashMap::new();
        headers.insert("Cookie".to_string(), "session=abc".to_string());
        headers.insert("referer".to_string(), "https://example.com".to_string());
        headers.insert("REFERRER".to_string(), "https://example.com".to_string());
        headers.insert("User-Agent".to_string(), "TestBrowser/1.0".to_string());
        headers.insert("X-Custom".to_string(), "keep-me".to_string());
        clear_auth_headers(&mut headers);
        // 认证头被移除
        assert!(!headers.keys().any(|k| k.eq_ignore_ascii_case("cookie")));
        assert!(!headers.keys().any(|k| k.eq_ignore_ascii_case("referer")));
        assert!(!headers.keys().any(|k| k.eq_ignore_ascii_case("referrer")));
        assert!(!headers.keys().any(|k| k.eq_ignore_ascii_case("user-agent")));
        // 非认证头保留
        assert_eq!(headers.get("X-Custom").unwrap(), "keep-me");
    }

    // Task 45：has_auth_headers 正确识别包含认证头的任务（用于前端展示"临时登录态"标记）。
    #[test]
    fn has_auth_headers_detects_auth_headers_case_insensitively() {
        use crate::manager::has_auth_headers;
        let mut headers = std::collections::HashMap::new();
        assert!(!has_auth_headers(&headers));
        headers.insert("X-Custom".to_string(), "value".to_string());
        assert!(!has_auth_headers(&headers));
        headers.insert("cookie".to_string(), "session=abc".to_string());
        assert!(has_auth_headers(&headers));
        headers.clear();
        headers.insert("Referer".to_string(), "https://example.com".to_string());
        assert!(has_auth_headers(&headers));
        headers.clear();
        headers.insert("USER-AGENT".to_string(), "TestBrowser/1.0".to_string());
        assert!(has_auth_headers(&headers));
    }
}
