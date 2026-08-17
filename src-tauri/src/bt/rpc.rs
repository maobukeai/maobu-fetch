//! aria2 JSON-RPC 客户端（roadmap BT-03）。
//!
//! 安全约束（AGENTS.md §3 BT/磁力内核）：
//! - 只连接 `127.0.0.1` 上的 RPC 端口，客户端强制 `no_proxy`（本地环回
//!   不得走任何代理）；
//! - `--rpc-secret` 令牌只存在于内存与请求体；`SecretToken` 的 Debug 实现
//!   脱敏输出，任何错误信息不得包含令牌。
//!
//! 请求/响应构建拆分为纯函数（`build_request` / `parse_response`），
//! 无网络即可单元测试（AGENTS.md §9）。

use serde_json::{json, Value};
use std::time::Duration;

/// aria2 RPC 认证令牌。Debug/Display 输出脱敏，防止日志泄露（§3）。
#[derive(Clone)]
pub struct SecretToken(String);

impl SecretToken {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// 随机生成 32 位十六进制令牌（uuid v4 字节转 hex，无新增依赖）。
    pub fn generate() -> Self {
        let bytes = uuid::Uuid::new_v4().into_bytes();
        Self(hex::encode(bytes))
    }

    fn as_str(&self) -> &str {
        &self.0
    }

    /// 仅供构建 aria2 启动参数使用（process::build_args）；
    /// 不得用于任何日志输出（用 `Debug`/`Display` 的脱敏输出）。
    pub(crate) fn expose_for_args(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretToken(***)")
    }
}

impl std::fmt::Display for SecretToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "***")
    }
}

/// aria2 JSON-RPC 客户端。`endpoint` 形如 `http://127.0.0.1:PORT/jsonrpc`。
pub struct Aria2Rpc {
    client: reqwest::Client,
    endpoint: String,
    secret: SecretToken,
}

impl Aria2Rpc {
    /// 构建指向本地端口的客户端。强制 `no_proxy()`：RPC 是本机环回流量，
    /// 走用户代理会造成劫持/失败（§3）。
    pub fn new(port: u16, secret: SecretToken) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|error| format!("初始化 aria2 RPC 客户端失败：{error}"))?;
        Ok(Self {
            client,
            endpoint: format!("http://127.0.0.1:{port}/jsonrpc"),
            secret,
        })
    }

    /// 调用任意 aria2 方法并返回 `result` 字段。
    pub async fn call(&self, method: &str, params: &[Value]) -> Result<Value, String> {
        let id = next_request_id();
        let body = build_request(id, method, params, &self.secret).to_string();
        let response = self
            .client
            .post(&self.endpoint)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|error| format!("无法连接 aria2 RPC（127.0.0.1）：{error}"))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| format!("读取 aria2 RPC 响应失败：{error}"))?;
        let payload: Value = serde_json::from_str(&text)
            .map_err(|error| format!("解析 aria2 RPC 响应失败：{error}"))?;
        if !status.is_success() {
            // HTTP 层错误：aria2 对未授权请求返回 401/400 且 body 可能为空。
            return Err(format!("aria2 RPC 返回 HTTP {status}"));
        }
        parse_response(&payload)
    }

    // ---- 任务级操作（参数见 aria2 RPC 文档） ----

    pub async fn add_uri(&self, uris: &[String], options: &Value) -> Result<String, String> {
        self.call("aria2.addUri", &[json!(uris), options.clone()])
            .await
            .and_then(|v| expect_gid(v, "addUri"))
    }

    pub async fn add_torrent(
        &self,
        torrent_base64: &str,
        uris: &[String],
        options: &Value,
    ) -> Result<String, String> {
        self.call(
            "aria2.addTorrent",
            &[json!(torrent_base64), json!(uris), options.clone()],
        )
        .await
        .and_then(|v| expect_gid(v, "addTorrent"))
    }

    pub async fn pause(&self, gid: &str) -> Result<(), String> {
        self.call("aria2.pause", &[json!(gid)]).await.map(|_| ())
    }

    pub async fn unpause(&self, gid: &str) -> Result<(), String> {
        self.call("aria2.unpause", &[json!(gid)]).await.map(|_| ())
    }

    /// 彻底移除下载（含未完成分片控制文件）。对已完成的 gid 无效，
    /// 需先 `remove_download_result` 清理结果列表。
    pub async fn remove(&self, gid: &str) -> Result<(), String> {
        self.call("aria2.remove", &[json!(gid)]).await.map(|_| ())
    }

    pub async fn remove_download_result(&self, gid: &str) -> Result<(), String> {
        self.call("aria2.removeDownloadResult", &[json!(gid)])
            .await
            .map(|_| ())
    }

    pub async fn change_option(&self, gid: &str, options: &Value) -> Result<(), String> {
        self.call("aria2.changeOption", &[json!(gid), options.clone()])
            .await
            .map(|_| ())
    }

    /// 查询任务状态。`keys` 为空时返回全部字段。
    pub async fn tell_status(&self, gid: &str, keys: &[&str]) -> Result<Value, String> {
        let params: Vec<Value> = if keys.is_empty() {
            vec![json!(gid)]
        } else {
            vec![json!(gid), json!(keys)]
        };
        self.call("aria2.tellStatus", &params).await
    }

    pub async fn tell_active(&self) -> Result<Vec<Value>, String> {
        self.call_list("aria2.tellActive", &[]).await
    }

    pub async fn tell_waiting(&self) -> Result<Vec<Value>, String> {
        // 等待队列（被暂停的任务）：offset/count 语义为取全部前 1000 条。
        self.call_list("aria2.tellWaiting", &[json!(0), json!(1000)])
            .await
    }

    pub async fn tell_stopped(&self) -> Result<Vec<Value>, String> {
        self.call_list("aria2.tellStopped", &[json!(0), json!(1000)])
            .await
    }

    pub async fn change_global_option(&self, options: &Value) -> Result<(), String> {
        self.call("aria2.changeGlobalOption", &[options.clone()])
            .await
            .map(|_| ())
    }

    pub async fn get_version(&self) -> Result<String, String> {
        self.call("aria2.getVersion", &[])
            .await
            .and_then(|v| {
                v.get("version")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| "aria2 版本响应缺少 version 字段".to_string())
            })
    }

    /// 立即把会话写入 `--save-session` 文件（优雅退出前调用）。
    pub async fn save_session(&self) -> Result<(), String> {
        self.call("aria2.saveSession", &[]).await.map(|_| ())
    }

    async fn call_list(&self, method: &str, prefix: &[Value]) -> Result<Vec<Value>, String> {
        let value = self.call(method, prefix).await?;
        value
            .as_array()
            .cloned()
            .ok_or_else(|| format!("aria2 {method} 响应不是数组"))
    }
}

fn expect_gid(value: Value, method: &str) -> Result<String, String> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("aria2 {method} 响应缺少 gid"))
}

/// 请求 id 计数器（原子递增，仅用于配对请求/响应）。
fn next_request_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// 构建标准 JSON-RPC 2.0 请求体（纯函数，便于测试）。
pub fn build_request(id: u64, method: &str, params: &[Value], secret: &SecretToken) -> Value {
    let mut all_params = Vec::with_capacity(params.len() + 1);
    all_params.push(Value::String(format!("token:{}", secret.as_str())));
    all_params.extend(params.iter().cloned());
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": all_params,
    })
}

/// 解析 JSON-RPC 响应（纯函数，便于测试）。错误信息不得回显请求参数，
/// 防止令牌以外的敏感选项进入日志。
pub fn parse_response(payload: &Value) -> Result<Value, String> {
    if let Some(error) = payload.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
        // aria2 错误 message 一律为英文底层描述，包装为可操作中文。
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("未知错误");
        return Err(map_aria2_error(code, message));
    }
    payload
        .get("result")
        .cloned()
        .ok_or_else(|| "aria2 RPC 响应缺少 result 字段".to_string())
}

/// aria2 错误码 → 中文可操作信息。常见码：
/// 1 = 操作被拒绝（如重复添加）；2 = 超时；3 = 非法参数。
/// 重复下载使用 `BT_DOWNLOAD_DUPLICATE:` 前缀标记，引擎识别后改为绑定
/// 已有任务，该前缀不直接暴露给用户。
fn map_aria2_error(code: i64, message: &str) -> String {
    match code {
        1 if message.contains("Duplicate") => format!("BT_DOWNLOAD_DUPLICATE: {message}"),
        1 => format!("aria2 拒绝了该操作：{message}"),
        2 => format!("aria2 操作超时：{message}"),
        3 => format!("aria2 参数非法：{message}"),
        _ => format!("aria2 操作失败：{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_includes_token_param_first() {
        let secret = SecretToken::new("abcdef".into());
        let body = build_request(7, "aria2.addUri", &[json!(["magnet:?x=1"]), json!({})], &secret);
        assert_eq!(body["jsonrpc"], "2.0");
        assert_eq!(body["id"], 7);
        assert_eq!(body["method"], "aria2.addUri");
        assert_eq!(body["params"][0], "token:abcdef");
        assert_eq!(body["params"][1], json!(["magnet:?x=1"]));
    }

    #[test]
    fn secret_debug_and_display_are_redacted() {
        let secret = SecretToken::new("topsecret".into());
        assert!(!format!("{secret:?}").contains("topsecret"));
        assert!(!format!("{secret}").contains("topsecret"));
    }

    #[test]
    fn parses_successful_result() {
        let payload = json!({"jsonrpc": "2.0", "id": 1, "result": "2089b05ecca3d829"});
        assert_eq!(
            parse_response(&payload).unwrap(),
            json!("2089b05ecca3d829")
        );
    }

    #[test]
    fn maps_duplicate_error_to_marker() {
        let payload = json!({
            "jsonrpc": "2.0", "id": 1,
            "error": {"code": 1, "message": ".getActiveDownloads() failed"}
        });
        let err = parse_response(&payload).unwrap_err();
        assert!(!err.contains("BT_DOWNLOAD_DUPLICATE"));
        let dup = json!({
            "jsonrpc": "2.0", "id": 1,
            "error": {"code": 1, "message": "Cannot add download: Duplicate download"}
        });
        assert!(parse_response(&dup)
            .unwrap_err()
            .starts_with("BT_DOWNLOAD_DUPLICATE"));
    }

    #[test]
    fn error_message_never_contains_token() {
        let payload = json!({
            "jsonrpc": "2.0", "id": 1,
            "error": {"code": 3, "message": "bad option"}
        });
        let err = parse_response(&payload).unwrap_err();
        assert!(!err.contains("token:"));
    }

    #[test]
    fn generated_token_is_32_hex_chars() {
        let token = SecretToken::generate();
        assert_eq!(token.as_str().len(), 32);
        assert!(token.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }
}
