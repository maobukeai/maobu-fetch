use crate::{
    manager::SharedManager,
    media,
    models::{MediaProbeResult, NewTaskRequest, PairingInfo},
};
use axum::{
    body::Bytes,
    extract::State,
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
}

pub async fn run(manager: SharedManager, pairing: PairingService, app: AppHandle) {
    let state = BridgeState {
        manager,
        pairing,
        app,
        requests: Arc::new(Mutex::new(VecDeque::new())),
    };
    let router = Router::new()
        .route("/v1/health", get(health))
        .route("/v1/pair", post(pair))
        .route("/v1/tasks", post(add_task))
        .route("/v1/media/probe", post(probe_media))
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
async fn probe_media(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<MediaProbeResult>, (StatusCode, String)> {
    authorize(&state, &headers, &body).await?;
    let request: ProbeRequest = serde_json::from_slice(&body)
        .map_err(|_| (StatusCode::BAD_REQUEST, "媒体参数无效".into()))?;
    media::probe(&state.app, &request.url)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))
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
}
