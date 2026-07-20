//! Task 31：代理配置精细化。
//!
//! 提供：
//! - [`resolve_proxy`]：根据全局 `AppSettings` 与任务级 `proxy_override` 决定
//!   实际使用的代理 URL，遵循"任务级覆盖全局"的优先级。
//! - [`test_proxy`]：通过 reqwest 向 `https://api.ipify.org` 发起 HTTPS 请求，
//!   测量延迟并返回出口 IP。错误信息已脱敏，不含代理 URL 中的认证字段。
//! - [`ProxyTestResult`]：测试结果，序列化为 JSON 返回前端。
//!
//! ## 安全约束（AGENTS.md §3、§7）
//!
//! - 代理 URL 中的 `userinfo`（`http://user:pass@host`）不会出现在错误或日志中。
//! - `test_proxy` 的失败信息使用 `redact_proxy_url` 脱敏后返回前端。
//! - 不对认证字段使用 `unwrap()`/`expect()`；所有 IO 错误通过 `Result` 返回。

use crate::models::{AppSettings, DownloadTask, ProxyAuth, ProxyTestResult};
use crate::secure_storage::decrypt_password;
use std::time::Instant;

/// 代理测试目标 URL。使用 HTTPS 端点避免明文代理泄露请求内容。
const PROXY_TEST_URL: &str = "https://api.ipify.org/format=json";
/// 代理测试超时（毫秒）。10 秒是网络代理验证的常见阈值。
const PROXY_TEST_TIMEOUT_SECS: u64 = 10;

/// 解析任务实际使用的代理 URL。
///
/// 优先级（高 → 低）：
/// 1. `task.proxy_override = Some(url)`（非空字符串）：使用任务级 URL。
///    若 URL 中包含 `userinfo`（`http://user:pass@host`），同时使用
///    `task.proxy_auth`（任务级）的明文用户名/密码。
/// 2. `task.proxy_override = Some("")`：显式禁用代理，返回 `None`。
/// 3. `task.proxy_override = None`：回退到全局。
///    - `settings.proxy_mode = "manual"` 且 `proxy_url` 非空：返回全局 URL。
///    - 其他模式（system / none / pac）：返回 `None`（由 reqwest 默认处理）。
///
/// 返回值是 reqwest 可识别的代理 URL 字符串（含 `http://`/`https://`/`socks5://` 前缀）。
/// 调用方在 reqwest::Proxy::all 失败时回退到"无代理"状态。
pub fn resolve_proxy(settings: &AppSettings, task: &DownloadTask) -> Option<String> {
    match task.proxy_override.as_deref() {
        Some(url) if !url.is_empty() => Some(url.to_string()),
        // Some("") 显式禁用代理。
        Some(_) => None,
        None => {
            if settings.proxy_mode == "manual" && !settings.proxy_url.is_empty() {
                Some(settings.proxy_url.clone())
            } else {
                None
            }
        }
    }
}

/// 校验代理 URL 格式是否合法。
///
/// 合法格式：
/// - `http://host[:port]`
/// - `https://host[:port]`
/// - `socks5://host[:port]` / `socks5h://host[:port]`
///
/// 不允许：无 scheme 的纯 IP/域名、`ftp://`、`file://` 等。
/// URL 中可包含 `userinfo`（`user:pass@`），由 reqwest 解析。
pub fn validate_proxy_url(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err("代理地址不能为空".into());
    }
    let lower = trimmed.to_ascii_lowercase();
    let allowed = ["http://", "https://", "socks5://", "socks5h://"];
    let prefix = allowed
        .iter()
        .find(|p| lower.starts_with(*p))
        .ok_or_else(|| "代理地址必须以 http://、https:// 或 socks5:// 开头".to_string())?;
    // 显式校验 authority 段：`scheme://` 之后必须紧跟非空主机（不允许直接出现 `/`、`?`、`#` 或结束）。
    // 这样可以拦截 `http://` 和 `http:///path`（authority 为空），
    // 弥补 `url::Url::parse` 对这类输入的解析差异。
    let after_scheme = &trimmed[prefix.len()..];
    let next = after_scheme.chars().next();
    if next.is_none() || matches!(next, Some('/') | Some('?') | Some('#')) {
        return Err("代理地址缺少主机名".into());
    }
    // 解析校验：必须有 host 段。
    let parsed = url::Url::parse(trimmed).map_err(|_| "代理地址格式无效".to_string())?;
    if parsed.host_str().is_none() || parsed.host_str().unwrap_or("").is_empty() {
        return Err("代理地址缺少主机名".into());
    }
    Ok(())
}

/// 测试指定代理的连通性，返回出口 IP 与延迟。
///
/// `proxy_url` 必须是合法的代理 URL（见 [`validate_proxy_url`]）。
/// `auth` 可选；若提供且 `username` 非空，将以 `basic_auth` 方式附加到 reqwest 代理。
/// `auth.password` 期望为已解密的明文密码（调用方负责从 `secure_storage` 解密）。
///
/// 返回 [`ProxyTestResult`]。失败时 `success = false` 且 `error` 为脱敏后的中文说明。
/// 脱敏规则：代理 URL 中的 `userinfo` 段被替换为 `***`，不暴露用户名/密码。
pub async fn test_proxy(proxy_url: &str, auth: Option<&ProxyAuth>) -> ProxyTestResult {
    if let Err(reason) = validate_proxy_url(proxy_url) {
        return ProxyTestResult {
            success: false,
            exit_ip: None,
            latency_ms: 0,
            error: Some(reason),
        };
    }

    let mut proxy = match reqwest::Proxy::all(proxy_url) {
        Ok(p) => p,
        Err(error) => {
            return ProxyTestResult {
                success: false,
                exit_ip: None,
                latency_ms: 0,
                error: Some(format!("代理配置无效：{}", redact_reqwest_error(&error))),
            };
        }
    };
    if let Some(auth) = auth {
        if !auth.username.is_empty() {
            proxy = proxy.basic_auth(&auth.username, &auth.password);
        }
    }

    let client = match reqwest::Client::builder()
        .proxy(proxy)
        .redirect(reqwest::redirect::Policy::limited(5))
        .connect_timeout(std::time::Duration::from_secs(PROXY_TEST_TIMEOUT_SECS))
        .timeout(std::time::Duration::from_secs(PROXY_TEST_TIMEOUT_SECS))
        .build()
    {
        Ok(c) => c,
        Err(error) => {
            return ProxyTestResult {
                success: false,
                exit_ip: None,
                latency_ms: 0,
                error: Some(format!(
                    "无法创建 HTTP 客户端：{}",
                    redact_reqwest_error(&error)
                )),
            };
        }
    };

    let start = Instant::now();
    let response = client.get(PROXY_TEST_URL).send().await;
    let latency_ms = start.elapsed().as_millis() as u64;

    let response = match response {
        Ok(r) => r,
        Err(error) => {
            return ProxyTestResult {
                success: false,
                exit_ip: None,
                latency_ms,
                error: Some(format!("代理请求失败：{}", redact_reqwest_error(&error))),
            };
        }
    };

    let status = response.status();
    if !status.is_success() {
        return ProxyTestResult {
            success: false,
            exit_ip: None,
            latency_ms,
            error: Some(format!("代理返回 HTTP {}", status.as_u16())),
        };
    }

    let body = match response.text().await {
        Ok(t) => t,
        Err(error) => {
            return ProxyTestResult {
                success: false,
                exit_ip: None,
                latency_ms,
                error: Some(format!("读取响应体失败：{}", redact_reqwest_error(&error))),
            };
        }
    };

    // ipify format=json 返回 {"ip":"1.2.3.4"}。
    let parsed: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            return ProxyTestResult {
                success: false,
                exit_ip: None,
                latency_ms,
                error: Some("响应不是有效的 JSON".into()),
            };
        }
    };
    let ip = parsed
        .get("ip")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    match ip {
        Some(ip) if !ip.is_empty() => ProxyTestResult {
            success: true,
            exit_ip: Some(ip),
            latency_ms,
            error: None,
        },
        _ => ProxyTestResult {
            success: false,
            exit_ip: None,
            latency_ms,
            error: Some("响应中未找到 IP 字段".into()),
        },
    }
}

/// 对 reqwest 错误进行脱敏：移除 URL 中的 `userinfo` 段。
///
/// reqwest 在错误信息中常包含完整 URL（如 `http://user:pass@host:port/...`），
/// 直接返回前端会泄露代理认证字段（AGENTS.md §3、§7）。
/// 这里通过简单字符串扫描移除 `://` 与 `@` 之间的内容。
fn redact_reqwest_error(error: &reqwest::Error) -> String {
    let raw = error.to_string();
    redact_url_userinfo(&raw)
}

/// 移除字符串中所有 `scheme://user:pass@host` 模式的 `user:pass@` 段。
///
/// 仅用于错误信息脱敏，不要求输入是合法 URL。
/// 若输入中不包含 `userinfo`，原样返回。
fn redact_url_userinfo(input: &str) -> String {
    // 简单扫描：查找 `://` 与下一个 `@`（如果在 `/` 之前）。
    // 这是一段保守的实现，宁可漏脱敏也不要误删非 URL 文本。
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    loop {
        let Some(scheme_idx) = rest.find("://") else {
            output.push_str(rest);
            break;
        };
        let after_scheme = scheme_idx + 3;
        output.push_str(&rest[..after_scheme]);
        let tail = &rest[after_scheme..];
        // 在 tail 中查找第一个 `@`；若它在 `/` 之前，认为存在 userinfo。
        let at_idx = tail.find('@');
        let slash_idx = tail.find('/');
        let has_userinfo = match (at_idx, slash_idx) {
            (Some(a), Some(s)) => a < s,
            (Some(_), None) => true,
            _ => false,
        };
        if has_userinfo {
            let at_pos = at_idx.unwrap();
            // 跳过 userinfo（不写入），从 `@` 之后开始拷贝。
            output.push_str("***@");
            rest = &tail[at_pos + 1..];
        } else {
            output.push_str(tail);
            break;
        }
    }
    output
}

/// 从任务级 `ProxyAuth` 提取明文认证（尝试 DPAPI 解密）。
///
/// `task.proxy_auth` 中的 `password` 在持久化时由 [`crate::secure_storage::encrypt_password`]
/// 加密。读取后调用本函数解密为明文，供 reqwest 使用。
///
/// 解密失败（密文损坏、用户上下文变化）时返回 `None`，调用方应退化为
/// "无认证"状态，不阻塞下载。
pub fn decode_proxy_auth(auth: &ProxyAuth) -> Option<ProxyAuth> {
    if auth.username.is_empty() {
        return None;
    }
    // 兼容明文存储（旧版本数据库可能存的是明文）：先尝试直接使用。
    // 通过判断是否能 base64 解码 + DPAPI 解密来区分；失败则假定是明文。
    if auth.password.is_empty() {
        return Some(auth.clone());
    }
    match decrypt_password(&auth.password) {
        Ok(plain) => Some(ProxyAuth {
            username: auth.username.clone(),
            password: plain,
        }),
        Err(_) => {
            // 解密失败：假定是明文（旧版本数据），原样返回。
            Some(auth.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AppSettings, DownloadTask, ProxyAuth};

    fn minimal_settings() -> AppSettings {
        AppSettings {
            proxy_mode: "system".into(),
            proxy_url: String::new(),
            ..AppSettings::default()
        }
    }

    fn minimal_task() -> DownloadTask {
        DownloadTask {
            id: "task-1".into(),
            url: "https://example.com/file".into(),
            file_name: "file".into(),
            destination: ".".into(),
            total_bytes: 0,
            downloaded_bytes: 0,
            speed: 0,
            eta_seconds: None,
            status: crate::models::TaskStatus::Queued,
            error: None,
            created_at: 0,
            completed_at: None,
            scheduled_at: None,
            category: "other".into(),
            queue_position: 0,
            priority: 0,
            retry_count: 0,
            max_retries: 3,
            checksum_sha256: None,
            expected_checksum: None,
            source: "desktop".into(),
            etag: None,
            last_modified: None,
            headers: std::collections::HashMap::new(),
            media: None,
            per_task_speed_limit: 0,
            collision_policy: crate::models::CollisionPolicy::Rename,
            completion_action: crate::models::CompletionAction::None,
            connection_count: 1,
            active_connections: 0,
            segments: Vec::new(),
            retry_policy_override: None,
            proxy_override: None,
            proxy_auth: None,
            final_url: None,
            response_status: None,
            content_type: None,
            accepts_ranges: None,
        }
    }

    #[test]
    fn resolve_proxy_returns_none_when_no_override_and_global_not_manual() {
        let settings = minimal_settings();
        let task = minimal_task();
        assert_eq!(resolve_proxy(&settings, &task), None);
    }

    #[test]
    fn resolve_proxy_uses_global_when_no_override_and_mode_is_manual() {
        let mut settings = minimal_settings();
        settings.proxy_mode = "manual".into();
        settings.proxy_url = "http://127.0.0.1:7890".into();
        let task = minimal_task();
        assert_eq!(
            resolve_proxy(&settings, &task).as_deref(),
            Some("http://127.0.0.1:7890")
        );
    }

    #[test]
    fn resolve_proxy_task_override_wins_over_global_manual() {
        let mut settings = minimal_settings();
        settings.proxy_mode = "manual".into();
        settings.proxy_url = "http://global:7890".into();
        let mut task = minimal_task();
        task.proxy_override = Some("http://task:1080".into());
        assert_eq!(
            resolve_proxy(&settings, &task).as_deref(),
            Some("http://task:1080")
        );
    }

    #[test]
    fn resolve_proxy_empty_string_override_disables_proxy() {
        let mut settings = minimal_settings();
        settings.proxy_mode = "manual".into();
        settings.proxy_url = "http://global:7890".into();
        let mut task = minimal_task();
        task.proxy_override = Some(String::new());
        assert_eq!(resolve_proxy(&settings, &task), None);
    }

    #[test]
    fn resolve_proxy_none_override_uses_global() {
        let mut settings = minimal_settings();
        settings.proxy_mode = "manual".into();
        settings.proxy_url = "http://global:7890".into();
        let mut task = minimal_task();
        task.proxy_override = None;
        assert_eq!(
            resolve_proxy(&settings, &task).as_deref(),
            Some("http://global:7890")
        );
    }

    #[test]
    fn validate_proxy_url_rejects_empty_input() {
        assert!(validate_proxy_url("").is_err());
        assert!(validate_proxy_url("   ").is_err());
    }

    #[test]
    fn validate_proxy_url_rejects_unknown_scheme() {
        assert!(validate_proxy_url("ftp://127.0.0.1:21").is_err());
        assert!(validate_proxy_url("file:///etc/passwd").is_err());
        assert!(validate_proxy_url("127.0.0.1:7890").is_err());
    }

    #[test]
    fn validate_proxy_url_accepts_http_https_socks5() {
        assert!(validate_proxy_url("http://127.0.0.1:7890").is_ok());
        assert!(validate_proxy_url("https://proxy.example.com").is_ok());
        assert!(validate_proxy_url("socks5://127.0.0.1:1080").is_ok());
        assert!(validate_proxy_url("socks5h://127.0.0.1:1080").is_ok());
    }

    #[test]
    fn validate_proxy_url_accepts_userinfo() {
        // 用户名:密码形式的代理 URL 合法，由 reqwest 解析。
        assert!(validate_proxy_url("http://alice:secret@127.0.0.1:7890").is_ok());
        assert!(validate_proxy_url("socks5://bob:p@ss@127.0.0.1:1080").is_ok());
    }

    #[test]
    fn validate_proxy_url_rejects_missing_host() {
        assert!(validate_proxy_url("http://").is_err());
        assert!(validate_proxy_url("http:///path").is_err());
    }

    #[test]
    fn redact_url_userinfo_removes_credentials_from_url() {
        let redacted = redact_url_userinfo("connection to http://alice:secret@proxy:7890 failed");
        assert!(!redacted.contains("alice"));
        assert!(!redacted.contains("secret"));
        assert!(redacted.contains("***@proxy:7890"));
    }

    #[test]
    fn redact_url_userinfo_leaves_url_without_credentials_unchanged() {
        let input = "connection to http://proxy:7890 failed";
        assert_eq!(redact_url_userinfo(input), input);
    }

    #[test]
    fn redact_url_userinfo_handles_multiple_urls() {
        let input = "from http://a:b@host1/ to socks5://c:d@host2/";
        let redacted = redact_url_userinfo(input);
        assert!(!redacted.contains("a:b"));
        assert!(!redacted.contains("c:d"));
        assert!(redacted.contains("***@host1"));
        assert!(redacted.contains("***@host2"));
    }

    #[test]
    fn redact_url_userinfo_preserves_plain_text_without_url() {
        let input = "网络不可达，请检查代理设置";
        assert_eq!(redact_url_userinfo(input), input);
    }

    #[test]
    fn redact_url_userinfo_does_not_corrupt_path_with_at_sign() {
        // 路径中包含 @ 字符时，若 @ 出现在 / 之后则不视为 userinfo。
        let input = "http://example.com/path/@v1/file";
        let redacted = redact_url_userinfo(input);
        assert_eq!(redacted, input);
    }

    #[test]
    fn test_proxy_returns_error_for_invalid_url() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let result = runtime.block_on(test_proxy("ftp://x", None));
        assert!(!result.success);
        assert!(result.error.is_some());
        assert_eq!(result.latency_ms, 0);
    }

    #[test]
    fn test_proxy_returns_error_for_empty_url() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let result = runtime.block_on(test_proxy("", None));
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("不能为空"));
    }

    #[test]
    fn decode_proxy_auth_returns_none_for_empty_username() {
        let auth = ProxyAuth {
            username: String::new(),
            password: "secret".into(),
        };
        assert!(decode_proxy_auth(&auth).is_none());
    }

    #[test]
    fn decode_proxy_auth_returns_plain_when_password_empty() {
        let auth = ProxyAuth {
            username: "alice".into(),
            password: String::new(),
        };
        let decoded = decode_proxy_auth(&auth).unwrap();
        assert_eq!(decoded.username, "alice");
        assert!(decoded.password.is_empty());
    }

    #[test]
    fn decode_proxy_auth_falls_back_to_plain_on_decrypt_failure() {
        // 非法 base64 字符串作为密码：DPAPI 解密失败，回退为明文。
        let auth = ProxyAuth {
            username: "alice".into(),
            // 旧版本数据库存储的明文密码。
            password: "plain-secret".into(),
        };
        let decoded = decode_proxy_auth(&auth).unwrap();
        assert_eq!(decoded.username, "alice");
        // 非法 base64 应触发回退到原值。
        assert_eq!(decoded.password, "plain-secret");
    }
}
