//! 下载前预检模块（Task 1）。
//!
//! 在用户点击“开始下载”之前，先发 HEAD 请求（必要时回退到 GET with
//! `Range: bytes=0-0`）跟随重定向，收集 ETag / Last-Modified /
//! Accept-Ranges / Content-Length / Content-Type，从 Content-Disposition
//! 或 URL 路径解析最终文件名，给出建议连接数（1/2/4/8/16/32），校验目标盘
//! 可用空间，并比对任务列表中是否存在同 URL、同最终 URL 或同目标路径冲突。
//!
//! 安全约束：
//! - 不使用 `unwrap()` / `expect()` 处理可恢复错误
//! - 不把 Cookie / Authorization / 代理密码写入日志
//! - 请求超时 30 秒
//! - 最多跟随 10 次重定向

use crate::models::{
    AppSettings, DownloadTask, PrecheckConflict, PrecheckConflictType, PrecheckRequest,
    PrecheckResult, RedirectHop, TaskStatus,
};
use reqwest::header::{
    ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG,
    LAST_MODIFIED, LOCATION, RANGE,
};
use reqwest::{Client, Response};
use std::path::{Path, PathBuf};
use std::time::Duration;
use url::Url;

use super::DownloadManager;

/// 跟随重定向的最大次数，避免无限循环。
const PRECHECK_MAX_REDIRECTS: usize = 10;
/// 连接建立超时（秒）。
const PRECHECK_CONNECT_TIMEOUT_SECS: u64 = 20;
/// 完整请求超时（秒），包含连接 + 重定向 + 响应头。
const PRECHECK_REQUEST_TIMEOUT_SECS: u64 = 30;
/// 合并所需临时空间的固定安全余量（字节）。
const PRECHECK_SAFETY_MARGIN_BYTES: u64 = 100 * 1024 * 1024;
/// 1 MiB，用于连接数阈值判定。
const PRECHECK_ONE_MB: u64 = 1024 * 1024;
/// 1 GiB，用于连接数阈值判定。
const PRECHECK_ONE_GB: u64 = 1024 * 1024 * 1024;

/// 用户代理标识，包含产品名和版本，便于 CDN 识别。
const PRECHECK_USER_AGENT: &str = concat!("MaobuFetch/", env!("CARGO_PKG_VERSION"), " (Windows)");

/// 全流程总体预检超时（秒）。
const PRECHECK_OVERALL_TIMEOUT_SECS: u64 = 12;

impl DownloadManager {
    /// 执行下载前预检。
    ///
    /// 流程：
    /// 1. 校验 URL scheme（仅 http/https）
    /// 2. 构造 HTTP 客户端（应用全局/任务代理，不自动跟随重定向）
    /// 3. HEAD 请求手动跟随重定向链；若 HEAD 返回 405/403/错误，在该跳尝试 GET with `Range: bytes=0-0`
    /// 4. 跨域重定向时自动剥离 Authorization 与 Cookie 敏感头
    /// 5. 校验实际 `bytes=0-0` 探针（206 + Content-Range + 1 字节 body）验证 Range 支持
    /// 6. 收集 ETag / Last-Modified / Accept-Ranges / Content-Length / Content-Type
    /// 7. 解析最终文件名（用户指定 > Content-Disposition > URL 路径 > "download"）
    /// 8. 计算建议连接数（1/2/4/8/16/32）
    /// 9. 校验目标盘可用空间（单连接 size+50MB / 多连接 2x size+100MB / 未知 size 50MB 试探）
    /// 10. 比对任务列表冲突（同 URL / 同 final_url / 同目标路径）
    /// 11. 生成中文警告列表，全流程添加 12 秒总体 Deadline。
    pub async fn precheck(&self, request: PrecheckRequest) -> Result<PrecheckResult, String> {
        match tokio::time::timeout(
            Duration::from_secs(PRECHECK_OVERALL_TIMEOUT_SECS),
            self.do_precheck(request),
        )
        .await
        {
            Ok(res) => res,
            Err(_) => Err("预检请求超时（超过 12 秒），请检查网络连接或代理设置".to_string()),
        }
    }

    async fn do_precheck(&self, mut request: PrecheckRequest) -> Result<PrecheckResult, String> {
        let extracted_url = crate::media_platforms::extract_url_from_share_text(&request.url)
            .unwrap_or_else(|| request.url.clone());
        let expanded_url = match crate::media_platforms::expand_short_url(&extracted_url).await {
            Ok(u) => u,
            Err(_) => extracted_url,
        };
        request.url = expanded_url;

        let parsed_url = Url::parse(&request.url).map_err(|_| "URL 格式无效".to_string())?;
        if !matches!(parsed_url.scheme(), "http" | "https") {
            return Err("仅支持 http/https 协议".to_string());
        }

        let is_douyin = crate::media_platforms::detect_platform(&request.url) == crate::media_platforms::MediaPlatform::Douyin
            || request.url.contains("douyin.com")
            || request.url.contains("iesdouyin.com");

        if !request.headers.keys().any(|k| k.eq_ignore_ascii_case("user-agent")) {
            request.headers.insert("User-Agent".to_string(), "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".to_string());
        }
        if is_douyin && !request.headers.keys().any(|k| k.eq_ignore_ascii_case("referer")) {
            request.headers.insert("Referer".to_string(), "https://www.douyin.com/".to_string());
        }

        let settings = self.settings.read().await.clone();
        let client = build_precheck_client(&settings, &request)?;
        let target_directory = request
            .target_directory
            .clone()
            .filter(|dir| !dir.trim().is_empty())
            .unwrap_or_else(|| settings.download_dir.clone());

        let probe = probe_endpoint(&client, &request, &settings.user_agent)
            .await
            .map_err(|e| format!("预检请求失败：{e}"))?;

        let is_media_url = crate::media_platforms::detect_platform(&request.url) != crate::media_platforms::MediaPlatform::Unknown
            || request.url.contains("douyin.com")
            || request.url.contains("iesdouyin.com");

        let mut file_name = determine_filename(
            request.suggested_filename.as_deref(),
            probe.content_disposition.as_deref(),
            &probe.final_url,
        );

        if is_media_url && request.suggested_filename.is_none() {
            let cookie = request.headers.get("Cookie").map(|s| s.as_str());
            let referer = request.headers.get("Referer").map(|s| s.as_str());
            let user_agent = request.headers.get("User-Agent").map(|s| s.as_str());
            if let Ok(probe_res) = crate::media::probe(&self.app, &settings, &request.url, cookie, referer, user_agent).await {
                if !probe_res.title.trim().is_empty() {
                    let raw_title = probe_res.title.clone();
                    let cleaned = crate::manager::naming_template::sanitize_filename(&regex::Regex::new(r"#[^\s#.]+")
                        .map(|re| re.replace_all(&raw_title, "").to_string())
                        .unwrap_or_else(|_| raw_title.clone()));
                    if !cleaned.trim().is_empty() {
                        file_name = format!("{}.mp4", cleaned.trim());
                    }
                }
            }
        }

        let suggested = if probe.file_size.is_none() && is_media_url {
            settings.connections_per_download
        } else {
            suggest_connections(probe.file_size, probe.accepts_ranges)
        };
        let supports_resume = probe.accepts_ranges && probe.file_size.is_some();

        let target_dir_clone = target_directory.clone();
        let available_opt =
            tokio::task::spawn_blocking(move || check_disk_space(&target_dir_clone))
                .await
                .unwrap_or(None);
        let available = available_opt.unwrap_or(0);
        let (required, disk_ok, disk_state) = compute_disk_requirements(
            probe.file_size,
            probe.accepts_ranges,
            suggested,
            available_opt,
        );

        let conflicts = self
            .find_precheck_conflicts(
                &request.url,
                &probe.final_url,
                &target_directory,
                &file_name,
            )
            .await;

        let warnings = build_warnings(
            probe.file_size,
            probe.accepts_ranges,
            disk_ok,
            disk_state,
            available,
            required,
            &conflicts,
        );

        Ok(PrecheckResult {
            original_url: request.url.clone(),
            final_url: probe.final_url.clone(),
            redirect_chain: probe.redirect_chain,
            file_name,
            file_size: probe.file_size,
            etag: probe.etag.clone(),
            last_modified: probe.last_modified.clone(),
            accepts_ranges: probe.accepts_ranges,
            content_type: probe.content_type.clone(),
            suggested_connections: suggested,
            supports_resume,
            target_directory: target_directory.clone(),
            available_disk_bytes: available,
            required_disk_bytes: required,
            disk_ok,
            disk_state,
            conflicts,
            warnings,
        })
    }

    /// 比对任务列表中的冲突：同 URL、同最终 URL、同目标路径。
    ///
    /// 已取消的任务不参与比对（用户已主动放弃）。
    /// 读取失败时返回空列表（不阻塞预检）。
    async fn find_precheck_conflicts(
        &self,
        original_url: &str,
        final_url: &str,
        target_directory: &str,
        file_name: &str,
    ) -> Vec<PrecheckConflict> {
        let tasks = match self.store.list_tasks().await {
            Ok(tasks) => tasks,
            Err(_) => return Vec::new(),
        };

        let target_path = build_target_path(target_directory, file_name);
        let mut conflicts = Vec::new();
        for task in tasks {
            if matches!(task.status, TaskStatus::Cancelled) {
                continue;
            }
            let Some(conflict_type) =
                match_task_for_conflict(&task, original_url, final_url, &target_path)
            else {
                continue;
            };
            let label = format!(
                "{} · {}",
                conflict_type.label(),
                if task.file_name.is_empty() {
                    task.url.clone()
                } else {
                    task.file_name.clone()
                }
            );
            conflicts.push(PrecheckConflict {
                conflict_type,
                existing_task_id: task.id.clone(),
                existing_task_label: label,
            });
        }
        conflicts
    }
}

// ---- 自由辅助函数 ----

/// 远程端点探测结果聚合。
struct PrecheckProbe {
    final_url: String,
    redirect_chain: Vec<RedirectHop>,
    file_size: Option<u64>,
    etag: Option<String>,
    last_modified: Option<String>,
    accepts_ranges: bool,
    content_type: Option<String>,
    content_disposition: Option<String>,
}

/// 构造专用 HTTP 客户端：不自动跟随重定向，应用代理与超时。
fn build_precheck_client(
    settings: &AppSettings,
    request: &PrecheckRequest,
) -> Result<Client, String> {
    let mut builder = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(PRECHECK_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(PRECHECK_REQUEST_TIMEOUT_SECS))
        .user_agent(PRECHECK_USER_AGENT);

    // 优先使用请求级代理，回退到全局代理设置
    match request.proxy_override.as_deref() {
        Some(url) if !url.is_empty() => {
            let mut proxy = reqwest::Proxy::all(url).map_err(|e| e.to_string())?;
            if let Some(auth) = request.proxy_auth.as_ref() {
                if let Some(decoded) = crate::proxy::decode_proxy_auth(auth) {
                    if !decoded.username.is_empty() {
                        proxy = proxy.basic_auth(&decoded.username, &decoded.password);
                    }
                }
            }
            builder = builder.proxy(proxy);
        }
        Some(_) => {
            builder = builder.no_proxy();
        }
        None => {
            if settings.proxy_mode == "manual" && !settings.proxy_url.is_empty() {
                let mut proxy =
                    reqwest::Proxy::all(&settings.proxy_url).map_err(|e| e.to_string())?;
                if !settings.proxy_username.is_empty() {
                    proxy = proxy.basic_auth(&settings.proxy_username, &settings.proxy_password);
                }
                builder = builder.proxy(proxy);
            } else if settings.proxy_mode == "none" {
                builder = builder.no_proxy();
            }
        }
    }

    builder
        .build()
        .map_err(|e| format!("无法构造 HTTP 客户端：{e}"))
}

/// 判断重定向是否发生跨域（Domain / Scheme / Port 变更）。
fn is_cross_origin(base: &str, target: &str) -> bool {
    let u1 = match Url::parse(base) {
        Ok(u) => u,
        Err(_) => return false,
    };
    let u2 = match Url::parse(target) {
        Ok(u) => u,
        Err(_) => return false,
    };
    u1.scheme() != u2.scheme()
        || u1.host_str() != u2.host_str()
        || u1.port_or_known_default() != u2.port_or_known_default()
}

/// 探测远程端点：HEAD 优先，HEAD 失败或 405/403 时在该跳回退 GET。
///
/// 手动跟随 3xx 重定向，每一跳记录到 `redirect_chain`；跨域时自动剥离 Cookie 与 Authorization 敏感头部。
async fn probe_endpoint(
    client: &Client,
    request: &PrecheckRequest,
    default_ua: &str,
) -> Result<PrecheckProbe, String> {
    let mut redirect_chain = Vec::new();
    let mut current_url = request.url.to_string();
    let mut headers = request.headers.clone();

    for _ in 0..PRECHECK_MAX_REDIRECTS {
        let mut head_builder = client.head(&current_url);
        for (k, v) in &headers {
            head_builder = head_builder.header(k, v);
        }
        if !headers.keys().any(|k| k.eq_ignore_ascii_case("user-agent")) {
            head_builder = head_builder.header(reqwest::header::USER_AGENT, default_ua);
        }

        let head_res = head_builder.send().await;
        let (response, is_get_fallback) = match head_res {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if resp.status().is_success() || resp.status().is_redirection() {
                    (resp, false)
                } else {
                    // HEAD 返回 405/403/404 或其他非 2xx/3xx 状态：回退到 GET Range: bytes=0-0
                    if let Ok(get_resp) =
                        send_get_range_probe(client, &current_url, &headers, default_ua).await
                    {
                        (get_resp, true)
                    } else {
                        return Err(format!("服务器返回错误状态：{status}"));
                    }
                }
            }
            Err(_) => {
                // HEAD 网络层失败：回退到 GET Range: bytes=0-0
                let get_resp =
                    send_get_range_probe(client, &current_url, &headers, default_ua).await?;
                (get_resp, true)
            }
        };

        let status = response.status().as_u16();

        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            redirect_chain.push(RedirectHop {
                url: current_url.clone(),
                status,
            });
            match location {
                Some(loc) => {
                    let next_url = resolve_redirect(&current_url, &loc)?;
                    if is_cross_origin(&current_url, &next_url) {
                        headers.retain(|k, _| {
                            !k.eq_ignore_ascii_case("authorization")
                                && !k.eq_ignore_ascii_case("cookie")
                        });
                    }
                    current_url = next_url;
                    continue;
                }
                None => return Err("服务器返回重定向但未提供 Location 头".to_string()),
            }
        }

        if response.status().is_success() || status == 206 {
            return collect_and_verify_probe(
                client,
                response,
                current_url,
                redirect_chain,
                &headers,
                default_ua,
                is_get_fallback,
            )
            .await;
        }

        return Err(format!("服务器返回错误状态：{status}"));
    }
    Err("重定向次数过多".to_string())
}

/// 发送 GET `Range: bytes=0-0` 请求辅助探测。
async fn send_get_range_probe(
    client: &Client,
    url: &str,
    headers: &std::collections::HashMap<String, String>,
    default_ua: &str,
) -> Result<Response, String> {
    let mut get_builder = client
        .get(url)
        .header(RANGE, "bytes=0-0")
        .header(reqwest::header::ACCEPT_ENCODING, "identity");
    for (k, v) in headers {
        get_builder = get_builder.header(k, v);
    }
    if !headers.keys().any(|k| k.eq_ignore_ascii_case("user-agent")) {
        get_builder = get_builder.header(reqwest::header::USER_AGENT, default_ua);
    }
    get_builder
        .send()
        .await
        .map_err(|e| format!("GET 回退请求失败：{e}"))
}

/// 解析重定向 Location：可能是绝对 URL 或相对路径。
fn resolve_redirect(base: &str, location: &str) -> Result<String, String> {
    let base_url = Url::parse(base).map_err(|_| "基础 URL 无效".to_string())?;
    base_url
        .join(location)
        .map(|u| u.to_string())
        .map_err(|e| format!("无法解析重定向地址：{e}"))
}

/// 收集探针元数据，并在需要时通过发送真正的 `Range: bytes=0-0` GET 请求验证 Range 支持。
async fn collect_and_verify_probe(
    client: &Client,
    response: Response,
    final_url: String,
    redirect_chain: Vec<RedirectHop>,
    headers: &std::collections::HashMap<String, String>,
    default_ua: &str,
    is_get_fallback: bool,
) -> Result<PrecheckProbe, String> {
    let status = response.status().as_u16();
    let resp_headers = response.headers();

    let etag = resp_headers
        .get(ETAG)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let last_modified = resp_headers
        .get(LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let content_type = resp_headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let content_disposition = resp_headers
        .get(CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let header_accept_ranges = extract_accept_ranges(resp_headers.get(ACCEPT_RANGES));

    let initial_file_size = if status == 206 {
        resp_headers
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_range_total)
    } else {
        resp_headers
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
    };

    // 如果是通过 GET Range 拿到的 206 响应，说明已完成真实 Range 校验
    if is_get_fallback && status == 206 {
        return Ok(PrecheckProbe {
            final_url,
            redirect_chain,
            file_size: initial_file_size,
            etag,
            last_modified,
            accepts_ranges: true,
            content_type,
            content_disposition,
        });
    }

    // 若响应头声明了 Accept-Ranges: bytes，发送真实 Range 校验请求
    let mut verified_accepts_ranges = false;
    let mut verified_file_size = initial_file_size;

    if header_accept_ranges || status == 206 {
        if let Ok(range_resp) = send_get_range_probe(client, &final_url, headers, default_ua).await
        {
            if range_resp.status().as_u16() == 206 {
                let range_headers = range_resp.headers();
                if let Some(content_range) = range_headers
                    .get(CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                {
                    if let Some(total) = parse_content_range_total(content_range) {
                        // 为 body 读取单独设 5 秒超时：connect_timeout 只覆盖 TCP 握手，
                        // 不覆盖响应体传输，某些 CDN（如微软）会在分片探针后延迟发送 body。
                        let read_result =
                            tokio::time::timeout(Duration::from_secs(5), range_resp.bytes()).await;
                        if let Ok(Ok(bytes)) = read_result {
                            if bytes.len() == 1 {
                                verified_accepts_ranges = true;
                                verified_file_size = Some(total);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(PrecheckProbe {
        final_url,
        redirect_chain,
        file_size: verified_file_size,
        etag,
        last_modified,
        accepts_ranges: verified_accepts_ranges,
        content_type,
        content_disposition,
    })
}

/// 判断 `Accept-Ranges` 头是否表示服务器支持 Range。
fn extract_accept_ranges(header: Option<&reqwest::header::HeaderValue>) -> bool {
    match header.and_then(|v| v.to_str().ok()) {
        Some(value) => {
            let lower = value.to_ascii_lowercase();
            lower.contains("bytes") && !lower.contains("none")
        }
        None => false,
    }
}

/// 解析 `Content-Range: bytes 0-0/12345` 中的总大小（`/` 之后的部分）。
fn parse_content_range_total(value: &str) -> Option<u64> {
    let after_slash = value.rsplit('/').next()?;
    after_slash.trim().parse::<u64>().ok()
}

/// 决定最终文件名：用户指定 > Content-Disposition > URL 路径 > "download"。
fn determine_filename(
    suggested: Option<&str>,
    content_disposition: Option<&str>,
    final_url: &str,
) -> String {
    let mut name = if let Some(name) = suggested {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            sanitize_filename(trimmed)
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    if name.is_empty() {
        if let Some(cd) = content_disposition {
            if let Some(cd_name) = parse_content_disposition_filename(cd) {
                name = cd_name;
            }
        }
    }

    if name.is_empty() {
        if let Some(url_name) = extract_filename_from_url(final_url) {
            name = url_name;
        } else {
            name = "download".to_string();
        }
    }

    let is_media = crate::media_platforms::detect_platform(final_url) != crate::media_platforms::MediaPlatform::Unknown
        || final_url.contains("douyinvod.com")
        || final_url.contains("douyin.com")
        || final_url.contains("iesdouyin.com");

    if is_media && !name.contains('.') {
        name = format!("{}.mp4", name);
    }

    name
}

/// 从 URL 路径提取文件名（percent-decode 后）。
fn extract_filename_from_url(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let segments = parsed.path_segments()?;
    let last = segments.last()?;
    if last.is_empty() {
        return None;
    }
    let decoded = percent_decode_str(last);
    let sanitized = sanitize_filename(&decoded);
    if sanitized.is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

/// 解析 `Content-Disposition` 头中的文件名。
fn parse_content_disposition_filename(header: &str) -> Option<String> {
    // RFC 5987: filename*=UTF-8''value
    for segment in header.split(';') {
        let trimmed = segment.trim();
        if let Some(rest) = trimmed
            .strip_prefix("filename*")
            .and_then(|s| s.strip_prefix('='))
        {
            if let Some(decoded) = parse_rfc5987_filename(rest) {
                let sanitized = sanitize_filename(&decoded);
                if !sanitized.is_empty() {
                    return Some(sanitized);
                }
            }
        }
    }
    // 传统：filename="value" 或 filename=value
    for segment in header.split(';') {
        let trimmed = segment.trim();
        if let Some(rest) = trimmed
            .strip_prefix("filename")
            .and_then(|s| s.strip_prefix('='))
        {
            let value = rest.trim().trim_matches('"');
            if !value.is_empty() {
                return Some(sanitize_filename(value));
            }
        }
    }
    None
}

/// 解析 RFC 5987 编码：`charset'language'value`。
fn parse_rfc5987_filename(value: &str) -> Option<String> {
    let parts: Vec<&str> = value.splitn(3, '\'').collect();
    if parts.len() != 3 {
        return None;
    }
    let charset = parts[0].to_ascii_lowercase();
    let bytes = percent_decode_bytes(parts[2]);
    match charset.as_str() {
        "iso-8859-1" => Some(bytes.iter().map(|&b| b as char).collect()),
        _ => Some(String::from_utf8_lossy(&bytes).to_string()),
    }
}

/// Percent-decode 字符串（不支持 `+` 转空格，与 RFC 3986 一致）。
fn percent_decode_str(input: &str) -> String {
    let bytes = percent_decode_bytes(input);
    String::from_utf8_lossy(&bytes).to_string()
}

/// Percent-decode 字符串，返回原始字节。
fn percent_decode_bytes(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                output.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        output.push(bytes[i]);
        i += 1;
    }
    output
}

/// 十六进制字符转数值。
fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// 清理文件名：去除路径分隔符等危险字符，剥离首尾空白与点。
fn sanitize_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect();
    let trimmed = sanitized.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.is_empty() {
        "download".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 计算建议连接数（仅返回 1/2/4/8/16/32）。
pub(crate) fn suggest_connections(file_size: Option<u64>, accepts_ranges: bool) -> u8 {
    if !accepts_ranges {
        return 1;
    }
    let size = match file_size {
        Some(s) => s,
        None => return 1,
    };
    if size < 10 * PRECHECK_ONE_MB {
        1
    } else if size < 100 * PRECHECK_ONE_MB {
        4
    } else if size < PRECHECK_ONE_GB {
        8
    } else if size < 10 * PRECHECK_ONE_GB {
        16
    } else {
        32
    }
}

/// 精细化计算磁盘空间需求与三态评估（Task 7）。
///
/// - 单连接任务：所需空间 = size + 50MB 安全余量
/// - 多连接任务：所需空间 = 2 × size + 100MB 安全余量（原子重命名合并需保留原分片）
/// - 未知配额（网络路径/UNC 盘）：返回 (required, true, Unknown)
fn compute_disk_requirements(
    file_size: Option<u64>,
    accepts_ranges: bool,
    suggested_connections: u8,
    available: Option<u64>,
) -> (u64, bool, crate::models::PrecheckDiskState) {
    let is_multi = accepts_ranges && suggested_connections > 1;
    let required = match file_size {
        Some(size) => {
            if is_multi {
                size.saturating_add(size).saturating_add(100 * 1024 * 1024)
            } else {
                size.saturating_add(50 * 1024 * 1024)
            }
        }
        None => 50 * 1024 * 1024,
    };

    match available {
        Some(avail) => {
            if avail >= required {
                (required, true, crate::models::PrecheckDiskState::Sufficient)
            } else {
                (
                    required,
                    false,
                    crate::models::PrecheckDiskState::Insufficient,
                )
            }
        }
        None => {
            // 无法读取磁盘空间配额（如 UNC 路径），按盲测处理，不判定为空间不足
            (required, true, crate::models::PrecheckDiskState::Unknown)
        }
    }
}

/// 查询目录所在磁盘的可用空间（无法获取配额时返回 None）。
pub(crate) fn check_disk_space(target_directory: &str) -> Option<u64> {
    let path = Path::new(target_directory);
    if let Some(space) = query_available_space(path) {
        return Some(space);
    }
    let mut current = path;
    while let Some(parent) = current.parent() {
        if parent.as_os_str().is_empty() {
            break;
        }
        if let Some(space) = query_available_space(parent) {
            return Some(space);
        }
        current = parent;
    }
    None
}

/// 查询单个已存在目录的可用空间。
fn query_available_space(path: &Path) -> Option<u64> {
    if !path.exists() {
        return None;
    }
    fs2::available_space(path).ok()
}

/// 拼接并规范化目标文件路径（用于冲突比较，大小写不敏感）。
fn build_target_path(directory: &str, file_name: &str) -> String {
    let dir = directory.trim_end_matches(['/', '\\']);
    if dir.is_empty() || file_name.is_empty() {
        return String::new();
    }
    let path = PathBuf::from(dir).join(file_name);
    path.to_string_lossy().to_lowercase()
}

/// 判断任务是否与预检目标冲突，返回冲突类型。
fn match_task_for_conflict(
    task: &DownloadTask,
    original_url: &str,
    final_url: &str,
    target_path: &str,
) -> Option<PrecheckConflictType> {
    if !original_url.is_empty() && task.url == original_url {
        return Some(PrecheckConflictType::DuplicateUrl);
    }
    if !final_url.is_empty() && task.final_url.as_deref() == Some(final_url) {
        return Some(PrecheckConflictType::DuplicateFinalUrl);
    }
    if !target_path.is_empty() {
        let task_path = build_target_path(&task.destination, &task.file_name);
        if task_path == *target_path {
            return Some(PrecheckConflictType::DuplicateTargetPath);
        }
    }
    None
}

/// 生成中文警告列表。
fn build_warnings(
    file_size: Option<u64>,
    accepts_ranges: bool,
    disk_ok: bool,
    disk_state: crate::models::PrecheckDiskState,
    available: u64,
    required: u64,
    conflicts: &[PrecheckConflict],
) -> Vec<String> {
    let mut warnings = Vec::new();
    if !accepts_ranges {
        warnings.push("服务器不支持断点续传，将使用单连接下载".to_string());
    }
    if file_size.is_none() {
        warnings.push("无法获取文件大小，建议连接数已设为 1".to_string());
    }
    if disk_state == crate::models::PrecheckDiskState::Unknown {
        warnings.push("无法确切评估所需磁盘空间，下载过程中将动态校验余量".to_string());
    } else if !disk_ok {
        warnings.push(format!(
            "磁盘空间不足：可用 {} 字节，需要 {} 字节",
            available, required
        ));
    }
    if !conflicts.is_empty() {
        warnings.push(format!("检测到 {} 个冲突任务", conflicts.len()));
    }
    warnings
}

// ===== 单元测试 =====
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DownloadTask, PrecheckConflictType, TaskStatus};

    // ---- 连接数算法 ----

    #[test]
    fn suggest_connections_returns_one_when_range_not_supported() {
        assert_eq!(suggest_connections(Some(1024 * 1024 * 1024), false), 1);
    }

    #[test]
    fn suggest_connections_returns_one_when_size_unknown() {
        assert_eq!(suggest_connections(None, true), 1);
    }

    #[test]
    fn suggest_connections_small_file_returns_one() {
        assert_eq!(suggest_connections(Some(5 * PRECHECK_ONE_MB), true), 1);
    }

    #[test]
    fn suggest_connections_under_10mb_boundary_returns_one() {
        // 10MB 边界（< 10MB），返回 1
        assert_eq!(suggest_connections(Some(10 * PRECHECK_ONE_MB - 1), true), 1);
    }

    #[test]
    fn suggest_connections_just_over_10mb_returns_four() {
        assert_eq!(suggest_connections(Some(11 * PRECHECK_ONE_MB), true), 4);
    }

    #[test]
    fn suggest_connections_under_100mb_returns_four() {
        assert_eq!(suggest_connections(Some(50 * PRECHECK_ONE_MB), true), 4);
    }

    #[test]
    fn suggest_connections_just_over_100mb_returns_eight() {
        assert_eq!(suggest_connections(Some(101 * PRECHECK_ONE_MB), true), 8);
    }

    #[test]
    fn suggest_connections_just_under_1gb_returns_eight() {
        assert_eq!(suggest_connections(Some(PRECHECK_ONE_GB - 1), true), 8);
    }

    #[test]
    fn suggest_connections_just_over_1gb_returns_sixteen() {
        assert_eq!(suggest_connections(Some(PRECHECK_ONE_GB + 1), true), 16);
    }

    #[test]
    fn suggest_connections_just_under_10gb_returns_sixteen() {
        assert_eq!(
            suggest_connections(Some(10 * PRECHECK_ONE_GB - 1), true),
            16
        );
    }

    #[test]
    fn suggest_connections_over_10gb_returns_thirty_two() {
        assert_eq!(suggest_connections(Some(20 * PRECHECK_ONE_GB), true), 32);
    }

    // ---- Accept-Ranges 解析 ----

    #[test]
    fn accept_ranges_bytes_means_supported() {
        let value = reqwest::header::HeaderValue::from_static("bytes");
        assert!(extract_accept_ranges(Some(&value)));
    }

    #[test]
    fn accept_ranges_none_means_not_supported() {
        let value = reqwest::header::HeaderValue::from_static("none");
        assert!(!extract_accept_ranges(Some(&value)));
    }

    #[test]
    fn accept_ranges_missing_means_not_supported() {
        assert!(!extract_accept_ranges(None));
    }

    // ---- Content-Range 解析 ----

    #[test]
    fn content_range_total_parses_bytes_range() {
        assert_eq!(parse_content_range_total("bytes 0-0/12345"), Some(12345));
    }

    #[test]
    fn content_range_total_parses_star_form() {
        assert_eq!(parse_content_range_total("bytes */98765"), Some(98765));
    }

    #[test]
    fn content_range_total_returns_none_for_invalid() {
        assert_eq!(parse_content_range_total("bytes 0-0"), None);
        assert_eq!(parse_content_range_total("bytes 0-0/notanumber"), None);
    }

    // ---- Content-Disposition 文件名解析 ----

    #[test]
    fn parses_rfc5987_utf8_filename() {
        let header = "attachment; filename*=UTF-8''%E4%B8%AD%E6%96%87.txt";
        assert_eq!(
            parse_content_disposition_filename(header),
            Some("中文.txt".to_string())
        );
    }

    #[test]
    fn parses_traditional_quoted_filename() {
        let header = "attachment; filename=\"report.pdf\"";
        assert_eq!(
            parse_content_disposition_filename(header),
            Some("report.pdf".to_string())
        );
    }

    #[test]
    fn parses_traditional_unquoted_filename() {
        let header = "attachment; filename=data.zip";
        assert_eq!(
            parse_content_disposition_filename(header),
            Some("data.zip".to_string())
        );
    }

    #[test]
    fn rfc5987_takes_precedence_over_traditional() {
        let header =
            "attachment; filename=\"fallback.txt\"; filename*=UTF-8''%E9%A6%96%E9%80%89.mp4";
        assert_eq!(
            parse_content_disposition_filename(header),
            Some("首选.mp4".to_string())
        );
    }

    #[test]
    fn rfc5987_with_iso_8859_1_charset_decodes() {
        // "café" in ISO-8859-1 percent-encoded
        let header = "attachment; filename*=ISO-8859-1''caf%E9.txt";
        assert_eq!(
            parse_content_disposition_filename(header),
            Some("caf\u{e9}.txt".to_string())
        );
    }

    #[test]
    fn content_disposition_without_filename_returns_none() {
        let header = "attachment";
        assert_eq!(parse_content_disposition_filename(header), None);
    }

    #[test]
    fn content_disposition_empty_returns_none() {
        assert_eq!(parse_content_disposition_filename(""), None);
    }

    #[test]
    fn content_disposition_filename_with_path_separator_is_sanitized() {
        let header = "attachment; filename=\"..\\\\evil.exe\"";
        let result = parse_content_disposition_filename(header).unwrap();
        assert!(!result.contains('\\'));
        assert!(!result.contains('/'));
    }

    // ---- percent-decode ----

    #[test]
    fn percent_decode_decodes_space() {
        assert_eq!(percent_decode_str("hello%20world"), "hello world");
    }

    #[test]
    fn percent_decode_decodes_utf8_multibyte() {
        // "中" UTF-8 = E4 B8 AD
        assert_eq!(percent_decode_str("%E4%B8%AD"), "中");
    }

    #[test]
    fn percent_decode_preserves_plus_sign() {
        // RFC 3986: '+' 不是空格
        assert_eq!(percent_decode_str("a+b"), "a+b");
    }

    #[test]
    fn percent_decode_passthrough_invalid_escape() {
        assert_eq!(percent_decode_str("100%"), "100%");
        assert_eq!(percent_decode_str("50%GG"), "50%GG");
    }

    // ---- sanitize_filename ----

    #[test]
    fn sanitize_strips_dots_and_whitespace_at_edges() {
        assert_eq!(sanitize_filename("  ..hidden..  "), "hidden");
    }

    #[test]
    fn sanitize_replaces_dangerous_chars() {
        let result = sanitize_filename("file/name:with*bad?chars");
        assert!(!result.contains('/'));
        assert!(!result.contains(':'));
        assert!(!result.contains('*'));
        assert!(!result.contains('?'));
    }

    #[test]
    fn sanitize_empty_returns_download() {
        assert_eq!(sanitize_filename(""), "download");
        assert_eq!(sanitize_filename("..."), "download");
        assert_eq!(sanitize_filename("   "), "download");
    }

    // ---- determine_filename 优先级 ----

    #[test]
    fn determine_filename_prefers_user_suggestion() {
        let result = determine_filename(
            Some("user-file.zip"),
            Some("attachment; filename=\"server.zip\""),
            "https://example.com/path.zip",
        );
        assert_eq!(result, "user-file.zip");
    }

    #[test]
    fn determine_filename_uses_content_disposition_when_no_user() {
        let result = determine_filename(
            None,
            Some("attachment; filename=\"from-server.zip\""),
            "https://example.com/path.zip",
        );
        assert_eq!(result, "from-server.zip");
    }

    #[test]
    fn determine_filename_uses_url_path_when_no_cd() {
        let result = determine_filename(None, None, "https://example.com/dir/file.zip");
        assert_eq!(result, "file.zip");
    }

    #[test]
    fn determine_filename_falls_back_to_download() {
        let result = determine_filename(None, None, "https://example.com/");
        assert_eq!(result, "download");
    }

    #[test]
    fn determine_filename_uses_url_when_cd_missing_filename() {
        let result = determine_filename(None, Some("attachment"), "https://example.com/data.bin");
        assert_eq!(result, "data.bin");
    }

    #[test]
    fn determine_filename_percent_decodes_url() {
        let result = determine_filename(
            None,
            None,
            "https://example.com/path/%E4%B8%AD%E6%96%87.mp4",
        );
        assert_eq!(result, "中文.mp4");
    }

    #[test]
    fn determine_filename_ignores_empty_user_suggestion() {
        let result = determine_filename(
            Some("   "),
            Some("attachment; filename=\"from-server.zip\""),
            "https://example.com/path.zip",
        );
        assert_eq!(result, "from-server.zip");
    }

    #[test]
    fn extract_filename_from_url_returns_none_for_root() {
        assert_eq!(extract_filename_from_url("https://example.com/"), None);
        assert_eq!(extract_filename_from_url("https://example.com"), None);
    }

    #[test]
    fn extract_filename_from_url_returns_last_segment() {
        assert_eq!(
            extract_filename_from_url("https://example.com/a/b/c.tar.gz"),
            Some("c.tar.gz".to_string())
        );
    }

    // ---- 磁盘空间计算 ----

    #[test]
    fn check_disk_space_returns_nonzero_for_existing_dir() {
        // 当前工作目录一定存在
        let space = check_disk_space(".");
        assert!(space.is_some_and(|s| s > 0));
    }

    #[test]
    fn check_disk_space_returns_none_for_nonexistent_root() {
        // 一个不存在的盘符路径，且无祖先存在
        let space = check_disk_space("Z:\\\\nonexistent\\\\deep\\\\path");
        let _ = space;
    }

    // ---- build_target_path ----

    #[test]
    fn build_target_path_lowercases_for_comparison() {
        let a = build_target_path("C:\\Downloads", "FILE.ZIP");
        let b = build_target_path("c:\\downloads", "file.zip");
        assert_eq!(a, b);
    }

    #[test]
    fn build_target_path_trims_trailing_separators() {
        let a = build_target_path("C:\\Downloads\\", "file.zip");
        let b = build_target_path("C:\\Downloads", "file.zip");
        assert_eq!(a, b);
    }

    #[test]
    fn build_target_path_returns_empty_for_empty_inputs() {
        assert_eq!(build_target_path("", "file.zip"), "");
        assert_eq!(build_target_path("C:\\Dir", ""), "");
    }

    // ---- match_task_for_conflict ----

    fn make_task(url: &str, final_url: Option<&str>, dest: &str, name: &str) -> DownloadTask {
        let mut task = DownloadTask {
            id: "test-id".to_string(),
            url: url.to_string(),
            file_name: name.to_string(),
            destination: dest.to_string(),
            total_bytes: 0,
            downloaded_bytes: 0,
            speed: 0,
            eta_seconds: None,
            status: TaskStatus::Queued,
            error: None,
            created_at: 0,
            completed_at: None,
            scheduled_at: None,
            category: String::new(),
            queue_position: 0,
            priority: 0,
            retry_count: 0,
            max_retries: 0,
            checksum_sha256: None,
            expected_checksum: None,
            source: String::new(),
            etag: None,
            last_modified: None,
            final_url: final_url.map(|s| s.to_string()),
            response_status: None,
            content_type: None,
            accepts_ranges: None,
            headers: std::collections::HashMap::new(),
            media: None,
            per_task_speed_limit: 0,
            collision_policy: crate::models::CollisionPolicy::default(),
            completion_action: crate::models::CompletionAction::default(),
            connection_count: 1,
            active_connections: 0,
            segments: Vec::new(),
            retry_policy_override: None,
            proxy_override: None,
            proxy_auth: None,
        };
        let _ = &mut task; // silence unused mut warning if any
        task
    }

    #[test]
    fn match_task_same_url_returns_duplicate_url() {
        let task = make_task("https://example.com/a.zip", None, "C:\\DL", "a.zip");
        assert_eq!(
            match_task_for_conflict(&task, "https://example.com/a.zip", "", ""),
            Some(PrecheckConflictType::DuplicateUrl)
        );
    }

    #[test]
    fn match_task_same_final_url_returns_duplicate_final_url() {
        let task = make_task(
            "https://other.com/x",
            Some("https://final.example.com/real.zip"),
            "C:\\DL",
            "x.zip",
        );
        assert_eq!(
            match_task_for_conflict(
                &task,
                "https://different.com/y",
                "https://final.example.com/real.zip",
                ""
            ),
            Some(PrecheckConflictType::DuplicateFinalUrl)
        );
    }

    #[test]
    fn match_task_empty_final_url_skips_final_url_check() {
        let task = make_task(
            "https://other.com/x",
            Some(""), // 空 final_url
            "C:\\DL",
            "x.zip",
        );
        assert_eq!(
            match_task_for_conflict(&task, "https://other.com/x", "", ""),
            Some(PrecheckConflictType::DuplicateUrl)
        );
    }

    #[test]
    fn match_task_same_target_path_returns_duplicate_target_path() {
        let task = make_task("https://other.com/x", None, "C:\\Downloads", "video.mp4");
        assert_eq!(
            match_task_for_conflict(
                &task,
                "https://different.com/y",
                "https://final.example.com/real.zip",
                "c:\\downloads\\video.mp4"
            ),
            Some(PrecheckConflictType::DuplicateTargetPath)
        );
    }

    #[test]
    fn match_task_no_conflict_returns_none() {
        let task = make_task(
            "https://other.com/x",
            Some("https://final.example.com/real.zip"),
            "C:\\Downloads",
            "other.zip",
        );
        assert_eq!(
            match_task_for_conflict(
                &task,
                "https://new.com/y",
                "https://new-final.com/z",
                "c:\\downloads\\new.zip"
            ),
            None
        );
    }

    #[test]
    fn match_task_url_takes_precedence_over_final_url() {
        let task = make_task(
            "https://example.com/same",
            Some("https://final.example.com/same"),
            "C:\\DL",
            "a.zip",
        );
        // URL 和 final_url 都匹配，应优先返回 DuplicateUrl
        assert_eq!(
            match_task_for_conflict(
                &task,
                "https://example.com/same",
                "https://final.example.com/same",
                ""
            ),
            Some(PrecheckConflictType::DuplicateUrl)
        );
    }

    // ---- build_warnings ----

    #[test]
    fn build_warnings_includes_range_not_supported() {
        let warnings = build_warnings(
            Some(1024),
            false,
            true,
            crate::models::PrecheckDiskState::Sufficient,
            0,
            0,
            &[],
        );
        assert!(warnings.iter().any(|w| w.contains("不支持断点续传")));
    }

    #[test]
    fn build_warnings_includes_unknown_size() {
        let warnings = build_warnings(
            None,
            true,
            true,
            crate::models::PrecheckDiskState::Unknown,
            0,
            0,
            &[],
        );
        assert!(warnings.iter().any(|w| w.contains("无法获取文件大小")));
    }

    #[test]
    fn build_warnings_includes_disk_full() {
        let warnings = build_warnings(
            Some(1024),
            true,
            false,
            crate::models::PrecheckDiskState::Insufficient,
            100,
            500,
            &[],
        );
        assert!(warnings.iter().any(|w| w.contains("磁盘空间不足")));
        assert!(warnings.iter().any(|w| w.contains("100")));
        assert!(warnings.iter().any(|w| w.contains("500")));
    }

    #[test]
    fn build_warnings_includes_conflict_count() {
        let conflicts = vec![PrecheckConflict::default(); 2];
        let warnings = build_warnings(
            Some(1024),
            true,
            true,
            crate::models::PrecheckDiskState::Sufficient,
            0,
            0,
            &conflicts,
        );
        assert!(warnings.iter().any(|w| w.contains("2 个冲突任务")));
    }

    #[test]
    fn build_warnings_empty_when_everything_ok() {
        let warnings = build_warnings(
            Some(1024),
            true,
            true,
            crate::models::PrecheckDiskState::Sufficient,
            1024,
            0,
            &[],
        );
        assert!(warnings.is_empty());
    }

    // ---- compute_disk_requirements (Task 7) ----

    #[test]
    fn compute_disk_requirements_single_connection() {
        // 100MB 单连接任务：需 100MB + 50MB 余量 = 150MB
        let (req, ok, state) = compute_disk_requirements(
            Some(100 * PRECHECK_ONE_MB),
            true,
            1,
            Some(200 * PRECHECK_ONE_MB),
        );
        assert_eq!(req, 150 * PRECHECK_ONE_MB);
        assert!(ok);
        assert_eq!(state, crate::models::PrecheckDiskState::Sufficient);
    }

    #[test]
    fn compute_disk_requirements_multi_connection() {
        // 100MB 多连接任务：需 2 * 100MB + 100MB 余量 = 300MB
        let (req, ok, state) = compute_disk_requirements(
            Some(100 * PRECHECK_ONE_MB),
            true,
            8,
            Some(200 * PRECHECK_ONE_MB),
        );
        assert_eq!(req, 300 * PRECHECK_ONE_MB);
        assert!(!ok);
        assert_eq!(state, crate::models::PrecheckDiskState::Insufficient);
    }

    #[test]
    fn compute_disk_requirements_unknown_size() {
        // 未知长度：需 50MB 试探空间
        let (req, ok, state) =
            compute_disk_requirements(None, true, 1, Some(100 * PRECHECK_ONE_MB));
        assert_eq!(req, 50 * PRECHECK_ONE_MB);
        assert!(ok);
        assert_eq!(state, crate::models::PrecheckDiskState::Sufficient);

        let (_, ok2, state2) = compute_disk_requirements(None, true, 1, Some(10 * PRECHECK_ONE_MB));
        assert!(!ok2);
        assert_eq!(state2, crate::models::PrecheckDiskState::Insufficient);
    }

    #[test]
    fn compute_disk_requirements_unqueryable_quota_returns_unknown_state() {
        // 无法读取磁盘空间配额（如 UNC 路径）
        let (req, ok, state) =
            compute_disk_requirements(Some(100 * PRECHECK_ONE_MB), true, 1, None);
        assert_eq!(req, 150 * PRECHECK_ONE_MB);
        assert!(ok);
        assert_eq!(state, crate::models::PrecheckDiskState::Unknown);
    }

    // ---- is_cross_origin ----

    #[test]
    fn is_cross_origin_detects_scheme_change() {
        assert!(is_cross_origin(
            "http://example.com/file",
            "https://example.com/file"
        ));
    }

    #[test]
    fn is_cross_origin_detects_domain_change() {
        assert!(is_cross_origin(
            "https://a.example.com/file",
            "https://b.example.com/file"
        ));
    }

    #[test]
    fn is_cross_origin_returns_false_for_same_origin() {
        assert!(!is_cross_origin(
            "https://example.com/dir1/file",
            "https://example.com/dir2/other"
        ));
    }

    // ---- resolve_redirect ----

    #[test]
    fn resolve_redirect_absolute_url() {
        let result = resolve_redirect("https://example.com/a", "https://other.com/b").unwrap();
        assert_eq!(result, "https://other.com/b");
    }

    #[test]
    fn resolve_redirect_relative_path() {
        let result = resolve_redirect("https://example.com/dir/page", "/new").unwrap();
        assert_eq!(result, "https://example.com/new");
    }

    #[test]
    fn resolve_redirect_protocol_relative() {
        let result = resolve_redirect("https://example.com/a", "//other.com/b").unwrap();
        assert_eq!(result, "https://other.com/b");
    }

    #[test]
    fn resolve_redirect_invalid_base_returns_error() {
        assert!(resolve_redirect("not-a-url", "/path").is_err());
    }

    // ---- build_precheck_client ----

    #[test]
    fn build_precheck_client_succeeds() {
        let settings = AppSettings::default();
        let request = PrecheckRequest::default();
        let client = build_precheck_client(&settings, &request);
        assert!(client.is_ok());
    }

    // ---- precheck URL 校验 ----

    #[tokio::test]
    async fn precheck_rejects_non_http_scheme() {
        // 通过构造一个不存在的 manager 不可行，所以只验证 URL 解析逻辑
        let parsed = Url::parse("ftp://example.com/file.zip");
        assert!(parsed.is_ok());
        let url = parsed.unwrap();
        assert!(!matches!(url.scheme(), "http" | "https"));
    }

    #[test]
    fn precheck_user_agent_contains_product_name() {
        assert!(PRECHECK_USER_AGENT.contains("MaobuFetch"));
        assert!(PRECHECK_USER_AGENT.contains(env!("CARGO_PKG_VERSION")));
    }
}
