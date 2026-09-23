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

#[cfg(windows)]
pub fn detect_windows_system_proxy() -> Option<String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, KEY_READ, REG_DWORD, REG_SZ,
    };

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    unsafe {
        let subkey = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings");
        let mut hkey = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut hkey) != 0 {
            return None;
        }

        // 1. Check ProxyEnable (DWORD)
        let proxy_enable_val = to_wide("ProxyEnable");
        let mut enabled: u32 = 0;
        let mut enabled_size = std::mem::size_of::<u32>() as u32;
        let mut val_type: u32 = 0;
        let res = RegQueryValueExW(
            hkey,
            proxy_enable_val.as_ptr(),
            std::ptr::null_mut(),
            &mut val_type,
            &mut enabled as *mut _ as *mut u8,
            &mut enabled_size,
        );
        if res != 0 || val_type != REG_DWORD || enabled == 0 {
            RegCloseKey(hkey);
            return None;
        }

        // 2. Read ProxyServer (REG_SZ)
        let proxy_server_val = to_wide("ProxyServer");
        let mut buf_size: u32 = 0;
        let res = RegQueryValueExW(
            hkey,
            proxy_server_val.as_ptr(),
            std::ptr::null_mut(),
            &mut val_type,
            std::ptr::null_mut(),
            &mut buf_size,
        );
        if res != 0 || val_type != REG_SZ || buf_size == 0 {
            RegCloseKey(hkey);
            return None;
        }

        let mut buffer: Vec<u16> = vec![0; (buf_size as usize / 2) + 1];
        let res = RegQueryValueExW(
            hkey,
            proxy_server_val.as_ptr(),
            std::ptr::null_mut(),
            &mut val_type,
            buffer.as_mut_ptr() as *mut u8,
            &mut buf_size,
        );
        RegCloseKey(hkey);

        if res != 0 {
            return None;
        }

        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        let server_str = String::from_utf16_lossy(&buffer[..len]);
        let trimmed = server_str.trim();
        if trimmed.is_empty() {
            return None;
        }

        // Parse: e.g. "127.0.0.1:7890" or "http=127.0.0.1:7890;https=127.0.0.1:7890" or "socks=127.0.0.1:1080"
        let (proxy_addr, default_scheme) = if trimmed.contains('=') {
            let mut chosen = None;
            for part in trimmed.split(';') {
                let p = part.trim();
                if let Some(rest) = p.strip_prefix("https=") {
                    chosen = Some((rest, "http://"));
                    break;
                } else if let Some(rest) = p.strip_prefix("http=") {
                    chosen = Some((rest, "http://"));
                    break;
                } else if let Some(rest) = p.strip_prefix("socks=") {
                    chosen = Some((rest, "socks5h://"));
                }
            }
            chosen.unwrap_or((trimmed, "http://"))
        } else {
            (trimmed, "http://")
        };

        if proxy_addr.starts_with("http://")
            || proxy_addr.starts_with("https://")
            || proxy_addr.starts_with("socks5://")
            || proxy_addr.starts_with("socks5h://")
        {
            Some(normalize_proxy_scheme(proxy_addr))
        } else {
            Some(format!("{default_scheme}{proxy_addr}"))
        }
    }
}

#[cfg(not(windows))]
pub fn detect_windows_system_proxy() -> Option<String> {
    None
}

/// 规范化代理 scheme。
/// 1. 若没有 scheme（如用户输入的纯 `host:port`，如 `127.0.0.1:7890`），自动补全 `http://` 前缀，
///    避免 `reqwest::Proxy::all` 解析相对 URL 时报错。
/// 2. 特别地，将 `socks5://` 自动升级为 `socks5h://`，强制让远程代理服务器执行 DNS 域名解析，
///    彻底避免客户端本地 DNS 污染导致访问境外/受限域名（如 chatgpt.com）时 TLS 握手异常重置。
pub fn normalize_proxy_scheme(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("socks5://") {
        format!("socks5h://{}", &trimmed["socks5://".len()..])
    } else if lower.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    }
}

/// 获取当前系统生效的代理配置。
///
/// 优先级设计（对 Windows 用户友好）：
/// 1. 在 Windows 上：**优先**探测 Windows 注册表中的系统代理（`detect_windows_system_proxy`）。
///    因为 Windows 用户在系统设置或代理客户端（如 Clash、v2rayN、Surge）中开启“系统代理”时，
///    修改的是系统注册表 WinINet Internet Settings。
///    若注册表 `ProxyEnable == 1`，说明系统代理处于开启状态，必须优先遵循此配置。
/// 2. 若 Windows 系统代理未开启，或在非 Windows 系统上，回退探测环境变量：
///    `HTTPS_PROXY` / `https_proxy` / `HTTP_PROXY` / `http_proxy` / `ALL_PROXY` / `all_proxy`。
/// 3. 所有返回的代理 URL 均通过 [`normalize_proxy_scheme`] 规范化（包括将 `socks5://` 自动转为 `socks5h://`）。
pub fn get_effective_system_proxy() -> Option<String> {
    #[cfg(windows)]
    {
        if let Some(win_proxy) = detect_windows_system_proxy() {
            let normalized = normalize_proxy_scheme(&win_proxy);
            if !normalized.is_empty() {
                return Some(normalized);
            }
        }
    }

    for var in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        if let Ok(val) = std::env::var(var) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(normalize_proxy_scheme(trimmed));
            }
        }
    }

    #[cfg(not(windows))]
    {
        detect_windows_system_proxy().map(|s| normalize_proxy_scheme(&s))
    }
    #[cfg(windows)]
    {
        None
    }
}

/// 判断主机名或 IP 是否属于私有网络、本地环回或 Tailscale 网络。
///
/// 匹配规则：
/// - 单标签主机名（不含点，如 `desktop-pc`、`nas`、`localhost` 等本地 Intranet/mDNS/NetBIOS/Tailscale 机器名）
/// - localhost 以及以 `.localhost` 结尾的域名
/// - 本地网络与 mDNS 域名：`.local`、`.lan`、`.home.arpa`、`.internal`
/// - Tailscale MagicDNS 域名：`*.ts.net`、`*.tailscale.net`
/// - 本地环回：`127.0.0.0/8`、`::1`
/// - 未指定与广播：`0.0.0.0/8`、`255.255.255.255`、`::`
/// - RFC 1918 私有 IPv4 地址：
///   - `10.0.0.0/8`
///   - `172.16.0.0/12`（`172.16.0.0` - `172.31.255.255`）
///   - `192.168.0.0/16`
/// - 链路本地 IPv4：`169.254.0.0/16`
/// - Tailscale CGNAT IPv4（RFC 6598）：`100.64.0.0/10`（`100.64.0.0` - `100.127.255.255`）
/// - IPv4 映射的 IPv6 地址（`::ffff:a.b.c.d`）继承上述 IPv4 规则
/// - IPv6 私有地址（ULA）：`fc00::/7`
/// - Tailscale IPv6：`fd7a:115c:a1e0::/48`
/// - IPv6 链路本地：`fe80::/10`
pub fn is_private_or_tailscale_host(host: &str) -> bool {
    let host = host.trim();
    if host.is_empty() {
        return false;
    }
    // 处理 [ipv6]:port 或 [ipv6]
    let host_cleaned = if host.starts_with('[') {
        if let Some(close_bracket) = host.find(']') {
            &host[1..close_bracket]
        } else {
            host.trim_matches(['[', ']'])
        }
    } else if let Some(colon_idx) = host.rfind(':') {
        // 若只有一个冒号，为 host:port；若有多个冒号且无中括号，则为裸 IPv6 地址
        if host.find(':') == Some(colon_idx) {
            &host[..colon_idx]
        } else {
            host
        }
    } else {
        host
    };

    let host_cleaned = host_cleaned.trim();
    if host_cleaned.is_empty() {
        return false;
    }

    let host_lower = host_cleaned.to_ascii_lowercase();
    let host_norm = host_lower.trim_end_matches('.');
    if host_norm.is_empty() {
        return false;
    }

    // 1. IP 地址匹配优先（支持 IPv4、IPv6、IPv4 映射的 IPv6）
    if let Ok(ip) = host_norm.parse::<std::net::IpAddr>() {
        return is_private_or_tailscale_ip(ip);
    }

    // 2. 单标签主机名匹配（Intranet / NetBIOS / mDNS / Tailscale MagicDNS 裸主机名）
    // 依据 RFC 6761 以及 Windows/Chrome 的 `<local>` Intranet 代理绕过标准，
    // 不含点号的主机名（如 `desktop-pc`、`nas`、`localhost` 等）属于局域网或内部机器名。
    if !host_norm.contains('.') {
        return true;
    }

    // 3. 域名后缀匹配
    if host_norm.ends_with(".localhost")
        || host_norm.ends_with(".local")
        || host_norm.ends_with(".lan")
        || host_norm.ends_with(".home.arpa")
        || host_norm.ends_with(".internal")
        || host_norm == "ts.net"
        || host_norm.ends_with(".ts.net")
        || host_norm == "tailscale.net"
        || host_norm.ends_with(".tailscale.net")
    {
        return true;
    }

    false
}

fn is_private_or_tailscale_ipv4(ipv4: std::net::Ipv4Addr) -> bool {
    let octets = ipv4.octets();
    // 本地环回 127.0.0.0/8
    if octets[0] == 127 {
        return true;
    }
    // 未指定 / 本网络 0.0.0.0/8 与广播 255.255.255.255
    if octets[0] == 0 || ipv4.is_broadcast() {
        return true;
    }
    // RFC 1918 私有 IPv4 地址
    // 10.0.0.0/8
    if octets[0] == 10 {
        return true;
    }
    // 172.16.0.0/12 (172.16.0.0 - 172.31.255.255)
    if octets[0] == 172 && (16..=31).contains(&octets[1]) {
        return true;
    }
    // 192.168.0.0/16
    if octets[0] == 192 && octets[1] == 168 {
        return true;
    }
    // 链路本地 169.254.0.0/16
    if octets[0] == 169 && octets[1] == 254 {
        return true;
    }
    // Tailscale CGNAT IPv4: 100.64.0.0/10 (100.64.0.0 - 100.127.255.255)
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return true;
    }
    false
}

fn is_private_or_tailscale_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ipv4) => is_private_or_tailscale_ipv4(ipv4),
        std::net::IpAddr::V6(ipv6) => {
            // IPv4 映射的 IPv6 地址（::ffff:a.b.c.d）或兼容 IPv4 地址
            if let Some(v4) = ipv6.to_ipv4() {
                return is_private_or_tailscale_ipv4(v4);
            }
            // 环回 ::1
            if ipv6.is_loopback() {
                return true;
            }
            // 未指定 ::
            if ipv6.is_unspecified() {
                return true;
            }
            let segments = ipv6.segments();
            // IPv6 私有/唯一本地地址 ULA: fc00::/7 (前缀 fc00::/8 与 fd00::/8)
            if (segments[0] & 0xfe00) == 0xfc00 {
                return true;
            }
            // Tailscale IPv6: fd7a:115c:a1e0::/48
            if segments[0] == 0xfd7a && segments[1] == 0x115c && segments[2] == 0xa1e0 {
                return true;
            }
            // 链路本地: fe80::/10
            if (segments[0] & 0xffc0) == 0xfe80 {
                return true;
            }
            false
        }
    }
}

/// 判断 URL 是否指向局域网或 Tailscale 私有网络。
pub fn is_private_or_tailscale_url(url_str: &str) -> bool {
    let trimmed = url_str.trim();
    if trimmed.is_empty() {
        return false;
    }
    if let Ok(parsed) = url::Url::parse(trimmed) {
        if let Some(host) = parsed.host_str() {
            return is_private_or_tailscale_host(host);
        }
    } else if let Ok(parsed) = url::Url::parse(&format!("http://{trimmed}")) {
        if let Some(host) = parsed.host_str() {
            return is_private_or_tailscale_host(host);
        }
    }
    false
}

/// 解析任务实际使用的代理 URL。
///
/// 优先级（高 → 低）：
/// 1. `task.proxy_override = Some(url)`（非空字符串）：使用任务级 URL。
///    若 URL 中包含 `userinfo`（`http://user:pass@host`），同时使用
///    `task.proxy_auth`（任务级）的明文用户名/密码。
/// 2. `task.proxy_override = Some("")`：显式禁用代理，返回 `None`。
/// 3. `task.proxy_override = None`：回退到全局。
///    - 若目标 URL 或重定向目标指向局域网或 Tailscale 私网地址，自动绕过代理，返回 `None`。
///    - `settings.proxy_mode = "manual"` 且 `proxy_url` 非空：返回全局 URL。
///    - 其他模式（system / none / pac）：返回 `None`（由 reqwest 默认处理）。
///
/// 返回值是 reqwest 可识别的代理 URL 字符串（含 `http://`/`https://`/`socks5://` 前缀）。
/// 调用方在 reqwest::Proxy::all 失败时回退到"无代理"状态。
pub fn resolve_proxy(settings: &AppSettings, task: &DownloadTask) -> Option<String> {
    match task.proxy_override.as_deref() {
        Some(url) if !url.trim().is_empty() => Some(normalize_proxy_scheme(url)),
        // Some("") 显式禁用代理。
        Some(_) => None,
        None => {
            // 私有网络或 Tailscale 目标自动直连绕过代理
            if is_private_or_tailscale_url(&task.url)
                || task
                    .final_url
                    .as_deref()
                    .map_or(false, is_private_or_tailscale_url)
            {
                return None;
            }
            if settings.proxy_mode == "manual" && !settings.proxy_url.trim().is_empty() {
                Some(normalize_proxy_scheme(&settings.proxy_url))
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
        .ok_or_else(|| "代理地址必须以 http://、https://、socks5:// 或 socks5h:// 开头".to_string())?;
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
    let normalized_url = normalize_proxy_scheme(proxy_url);
    if let Err(reason) = validate_proxy_url(&normalized_url) {
        return ProxyTestResult {
            success: false,
            exit_ip: None,
            latency_ms: 0,
            error: Some(reason),
        };
    }

    let mut proxy = match reqwest::Proxy::all(&normalized_url) {
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
            task_kind: Default::default(),
            bt_meta: None,
            bt_runtime: None,
            cloud_refresh: None,
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
    fn resolve_proxy_normalizes_bare_host_port_for_global_and_task() {
        let mut settings = minimal_settings();
        settings.proxy_mode = "manual".into();
        settings.proxy_url = "127.0.0.1:7890".into();
        let mut task = minimal_task();
        assert_eq!(
            resolve_proxy(&settings, &task).as_deref(),
            Some("http://127.0.0.1:7890")
        );

        task.proxy_override = Some("127.0.0.1:1080".into());
        assert_eq!(
            resolve_proxy(&settings, &task).as_deref(),
            Some("http://127.0.0.1:1080")
        );

        task.proxy_override = Some("socks5://127.0.0.1:1080".into());
        assert_eq!(
            resolve_proxy(&settings, &task).as_deref(),
            Some("socks5h://127.0.0.1:1080")
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

    #[test]
    fn effective_system_proxy_detection_runs() {
        // 验证探测函数可正常调用且不发生 panic 或内存异常
        let proxy = get_effective_system_proxy();
        if let Some(p) = proxy {
            assert!(
                p.starts_with("http://")
                    || p.starts_with("https://")
                    || p.starts_with("socks5://")
                    || p.starts_with("socks5h://")
            );
        }
    }

    #[test]
    fn test_normalize_proxy_scheme() {
        assert_eq!(
            normalize_proxy_scheme("socks5://127.0.0.1:7890"),
            "socks5h://127.0.0.1:7890"
        );
        assert_eq!(
            normalize_proxy_scheme("SOCKS5://127.0.0.1:7890"),
            "socks5h://127.0.0.1:7890"
        );
        assert_eq!(
            normalize_proxy_scheme("http://127.0.0.1:7890"),
            "http://127.0.0.1:7890"
        );
        assert_eq!(
            normalize_proxy_scheme("https://127.0.0.1:7890"),
            "https://127.0.0.1:7890"
        );
        assert_eq!(
            normalize_proxy_scheme("socks5h://127.0.0.1:7890"),
            "socks5h://127.0.0.1:7890"
        );
        assert_eq!(
            normalize_proxy_scheme("127.0.0.1:7890"),
            "http://127.0.0.1:7890"
        );
        assert_eq!(
            normalize_proxy_scheme("  127.0.0.1:7890  "),
            "http://127.0.0.1:7890"
        );
        assert_eq!(
            normalize_proxy_scheme("localhost:1080"),
            "http://localhost:1080"
        );
        assert_eq!(
            normalize_proxy_scheme("user:pass@127.0.0.1:7890"),
            "http://user:pass@127.0.0.1:7890"
        );
        assert_eq!(
            normalize_proxy_scheme("socks5://alice:secret@127.0.0.1:1080"),
            "socks5h://alice:secret@127.0.0.1:1080"
        );
        assert_eq!(normalize_proxy_scheme(""), "");
        assert_eq!(normalize_proxy_scheme("   "), "");
    }

    #[test]
    fn test_is_private_or_tailscale_host() {
        // localhost
        assert!(is_private_or_tailscale_host("localhost"));
        assert!(is_private_or_tailscale_host("LOCALHOST"));
        assert!(is_private_or_tailscale_host("app.localhost"));
        assert!(is_private_or_tailscale_host("localhost:8080"));
        assert!(is_private_or_tailscale_host("localhost."));

        // Single-label hostnames without dot (Windows NetBIOS / mDNS / Tailscale MagicDNS short name / intranet)
        assert!(is_private_or_tailscale_host("desktop-pc"));
        assert!(is_private_or_tailscale_host("nas"));
        assert!(is_private_or_tailscale_host("workstation:8000"));
        assert!(is_private_or_tailscale_host("my-laptop"));
        assert!(is_private_or_tailscale_host("ubuntu:3000"));

        // .local and .lan
        assert!(is_private_or_tailscale_host("my-mac.local"));
        assert!(is_private_or_tailscale_host("local"));
        assert!(is_private_or_tailscale_host("printer.lan"));
        assert!(is_private_or_tailscale_host("lan"));
        assert!(is_private_or_tailscale_host("my-mac.local:3000"));
        assert!(is_private_or_tailscale_host("my-mac.local."));

        // .home.arpa (RFC 8375) and .internal
        assert!(is_private_or_tailscale_host("router.home.arpa"));
        assert!(is_private_or_tailscale_host("service.internal"));
        assert!(is_private_or_tailscale_host("node.internal:8080"));

        // Tailscale MagicDNS domains
        assert!(is_private_or_tailscale_host("my-laptop.ts.net"));
        assert!(is_private_or_tailscale_host("ts.net"));
        assert!(is_private_or_tailscale_host("node.tailscale.net"));
        assert!(is_private_or_tailscale_host("tailscale.net"));
        assert!(is_private_or_tailscale_host("my-laptop.ts.net:8080"));
        assert!(is_private_or_tailscale_host("my-laptop.ts.net."));

        // IPv4 Loopback
        assert!(is_private_or_tailscale_host("127.0.0.1"));
        assert!(is_private_or_tailscale_host("127.1.2.3"));
        assert!(is_private_or_tailscale_host("127.255.255.255"));
        assert!(is_private_or_tailscale_host("127.0.0.1:7890"));

        // Unspecified and broadcast
        assert!(is_private_or_tailscale_host("0.0.0.0"));
        assert!(is_private_or_tailscale_host("255.255.255.255"));

        // RFC 1918 Private IPv4
        // 10.0.0.0/8
        assert!(is_private_or_tailscale_host("10.0.0.1"));
        assert!(is_private_or_tailscale_host("10.255.255.255"));
        assert!(is_private_or_tailscale_host("10.0.0.1:8000"));
        // 172.16.0.0/12
        assert!(is_private_or_tailscale_host("172.16.0.1"));
        assert!(is_private_or_tailscale_host("172.20.1.2"));
        assert!(is_private_or_tailscale_host("172.31.255.255"));
        // 192.168.0.0/16
        assert!(is_private_or_tailscale_host("192.168.0.1"));
        assert!(is_private_or_tailscale_host("192.168.1.100"));
        assert!(is_private_or_tailscale_host("192.168.1.100:9000"));

        // Link-local 169.254.0.0/16
        assert!(is_private_or_tailscale_host("169.254.0.1"));
        assert!(is_private_or_tailscale_host("169.254.169.254"));

        // Tailscale CGNAT IPv4: 100.64.0.0/10 (100.64.0.0 - 100.127.255.255)
        assert!(is_private_or_tailscale_host("100.64.0.1"));
        assert!(is_private_or_tailscale_host("100.100.100.100"));
        assert!(is_private_or_tailscale_host("100.127.255.254"));
        assert!(is_private_or_tailscale_host("100.100.100.100:8080"));

        // IPv6 loopback and unspecified
        assert!(is_private_or_tailscale_host("::1"));
        assert!(is_private_or_tailscale_host("[::1]"));
        assert!(is_private_or_tailscale_host("[::1]:8080"));
        assert!(is_private_or_tailscale_host("::"));
        assert!(is_private_or_tailscale_host("[::]"));

        // IPv4-mapped IPv6
        assert!(is_private_or_tailscale_host("::ffff:192.168.1.1"));
        assert!(is_private_or_tailscale_host("[::ffff:192.168.1.1]:8080"));
        assert!(is_private_or_tailscale_host("::ffff:100.100.100.100"));
        assert!(is_private_or_tailscale_host("::ffff:127.0.0.1"));

        // IPv6 ULA (fc00::/7)
        assert!(is_private_or_tailscale_host("fc00::1"));
        assert!(is_private_or_tailscale_host("fd00::1"));
        assert!(is_private_or_tailscale_host("[fd00::1]"));

        // Tailscale IPv6: fd7a:115c:a1e0::/48
        assert!(is_private_or_tailscale_host("fd7a:115c:a1e0::1"));
        assert!(is_private_or_tailscale_host("[fd7a:115c:a1e0::1]"));
        assert!(is_private_or_tailscale_host("[fd7a:115c:a1e0::1]:8080"));

        // IPv6 link-local (fe80::/10)
        assert!(is_private_or_tailscale_host("fe80::1"));

        // Public IPs and domains should return false
        assert!(!is_private_or_tailscale_host("8.8.8.8"));
        assert!(!is_private_or_tailscale_host("1.1.1.1"));
        assert!(!is_private_or_tailscale_host("11.0.0.1"));
        assert!(!is_private_or_tailscale_host("172.15.0.1"));
        assert!(!is_private_or_tailscale_host("172.32.0.1"));
        assert!(!is_private_or_tailscale_host("192.167.1.1"));
        assert!(!is_private_or_tailscale_host("192.169.1.1"));
        assert!(!is_private_or_tailscale_host("100.63.255.255"));
        assert!(!is_private_or_tailscale_host("100.128.0.1"));
        assert!(!is_private_or_tailscale_host("example.com"));
        assert!(!is_private_or_tailscale_host("google.com"));
        assert!(!is_private_or_tailscale_host(""));
        assert!(!is_private_or_tailscale_host("   "));
    }

    #[test]
    fn test_is_private_or_tailscale_url() {
        assert!(is_private_or_tailscale_url("http://localhost:8080/sync"));
        assert!(is_private_or_tailscale_url("http://desktop-pc:8000/download"));
        assert!(is_private_or_tailscale_url("http://127.0.0.1:8080/sync"));
        assert!(is_private_or_tailscale_url("http://192.168.1.50:9000/stream"));
        assert!(is_private_or_tailscale_url("http://100.100.100.100:8000/download"));
        assert!(is_private_or_tailscale_url("http://my-peer.ts.net:3000/file.bin"));
        assert!(is_private_or_tailscale_url("http://[::1]:8080/file"));
        assert!(is_private_or_tailscale_url("http://[fd7a:115c:a1e0::1]:8080/file"));
        assert!(is_private_or_tailscale_url("http://[::ffff:192.168.1.1]:8080/file"));

        // Bare host without scheme
        assert!(is_private_or_tailscale_url("100.100.100.100:8000/download"));
        assert!(is_private_or_tailscale_url("192.168.1.100:9000/stream"));

        assert!(!is_private_or_tailscale_url("https://example.com/file.zip"));
        assert!(!is_private_or_tailscale_url("http://8.8.8.8/file.zip"));
        assert!(!is_private_or_tailscale_url(""));
        assert!(!is_private_or_tailscale_url("   "));
    }

    #[test]
    fn test_resolve_proxy_tailscale_bypass() {
        let mut settings = minimal_settings();
        settings.proxy_mode = "manual".into();
        settings.proxy_url = "http://127.0.0.1:7890".into();

        // 1. Tailscale URL without task-level override: should bypass proxy (return None)
        let mut task = minimal_task();
        task.url = "http://100.100.100.100:8000/download".into();
        task.proxy_override = None;
        assert_eq!(resolve_proxy(&settings, &task), None);

        // 2. LAN URL without task-level override: should bypass proxy (return None)
        task.url = "http://192.168.1.100:8000/share".into();
        assert_eq!(resolve_proxy(&settings, &task), None);

        // 3. Tailscale MagicDNS URL without task-level override: should bypass proxy (return None)
        task.url = "http://peer.ts.net:8080/file".into();
        assert_eq!(resolve_proxy(&settings, &task), None);

        // 4. Single-label host without task-level override: should bypass proxy (return None)
        task.url = "http://desktop-pc:8080/file".into();
        assert_eq!(resolve_proxy(&settings, &task), None);

        // 5. Public task.url with redirected final_url pointing to Tailscale: should bypass proxy!
        task.url = "https://short.link/123".into();
        task.final_url = Some("http://100.100.100.100:8000/download".into());
        assert_eq!(resolve_proxy(&settings, &task), None);

        // 6. Public URL without task-level override: should use global manual proxy
        task.url = "https://example.com/file.zip".into();
        task.final_url = None;
        assert_eq!(
            resolve_proxy(&settings, &task).as_deref(),
            Some("http://127.0.0.1:7890")
        );

        // 7. Tailscale URL WITH explicit task-level proxy override: should honor the manual task override
        task.url = "http://100.100.100.100:8000/download".into();
        task.proxy_override = Some("http://corp-proxy:1080".into());
        assert_eq!(
            resolve_proxy(&settings, &task).as_deref(),
            Some("http://corp-proxy:1080")
        );
    }
}
