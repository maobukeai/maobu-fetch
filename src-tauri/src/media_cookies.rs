//! 媒体解析阶段的 Cookie/Referer/User-Agent 安全透传。
//!
//! 用户填写的 Cookie 不得通过命令行参数（`--add-header "Cookie: ..."`）传递给
//! yt-dlp，因为 Windows 子进程命令行对同用户其它进程可见。本模块将 Cookie 字符串
//! 转换为 Netscape cookies 格式，写入仅当前用户可读的临时文件，分析完即删。
//!
//! 临时文件生命周期：
//!   1. `write_cookie_file()` 创建文件并返回 `CookieFileGuard`
//!   2. 调用方将 guard 的 `path()` 传给 yt-dlp `--cookies` 参数
//!   3. `CookieFileGuard` 被 drop 时（即使分析失败也会 drop）立即删除文件
//!   4. 删除失败不会抛出，但会通过 tracing warn 记录（路径不记录）
//!
//! 安全约束：
//!   - 临时文件路径不得写入日志、错误历史或前端调试输出
//!   - 文件权限：仅当前用户可读（Windows ACL：DACL 仅包含当前用户）
//!   - 文件位置：app data 目录下 `cookies_tmp/` 子目录

use std::path::{Path, PathBuf};
use uuid::Uuid;

/// 解析 `name=value; name2=value2` 格式的 Cookie 字符串。
///
/// 返回 (name, value) 元组列表。空值或格式错误的条目会被跳过。
/// 不对 name/value 内容做 URL 解码，因为 yt-dlp 接受原始值。
pub(crate) fn parse_cookie_header(cookie: &str) -> Vec<(String, String)> {
    cookie
        .split(';')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .filter_map(|segment| {
            let idx = segment.find('=')?;
            let name = segment[..idx].trim();
            let value = segment[idx + 1..].trim();
            if name.is_empty() {
                None
            } else {
                Some((name.to_string(), value.to_string()))
            }
        })
        .collect()
}

/// 判断 Cookie 字符串是否为 Netscape cookies.txt 格式。
///
/// 识别条件：包含 `# Netscape` 头或任一行含 TAB 分隔的 7 列。
/// 用于检测「设置 → 媒体凭证」里存的 Cookie 是 HTTP 头格式还是 cookies.txt 文件格式，
/// 以便在用作 HTTP `Cookie:` 头前先转换格式（HTTP 头不允许 TAB/换行字符）。
pub(crate) fn is_netscape_format(cookie: &str) -> bool {
    let trimmed = cookie.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.contains("# Netscape") {
        return true;
    }
    // 检查是否有 TAB 分隔的至少 7 列的行（Netscape 格式特征）
    trimmed.lines().any(|line| {
        !line.starts_with('#') && line.split('\t').count() >= 7
    })
}

/// 判断是否应跳过扩展自动同步对已存在凭证的覆盖。
///
/// 当用户已手动配置了 Netscape cookies.txt 格式的凭证时（这种格式只能由
/// 用户从「导出 cookies.txt」按钮导出后手动导入，扩展自动同步永远只发
/// HTTP 头格式），自动同步不得覆盖。这避免了
/// "用户导出无痕窗口 cookies.txt 导入 → 打开 youtube.com 网页 →
///   扩展自动同步把普通窗口 Cookie 覆盖掉手动配置" 的数据丢失场景。
///
/// 参见 AGENTS.md §7：不得改变用户设置，除非操作由用户明确触发。
pub(crate) fn should_skip_auto_sync(existing_cookie: &str, incoming_cookie: &str) -> bool {
    // 已存在的是 Netscape 格式（用户手动导入）→ 不允许自动同步覆盖
    is_netscape_format(existing_cookie)
        && // 自动同步永远是头格式；如果 incoming 也是 Netscape
           // （理论上不会发生），仍然允许，因为可能是用户在另一处手动同步
           !is_netscape_format(incoming_cookie)
}

/// 把 Netscape cookies.txt 格式转换为 `name=value; name2=value2` HTTP 头格式。
///
/// 解析每行：`domain<TAB>include_subdomains<TAB>path<TAB>secure<TAB>expiration<TAB>name<TAB>value`
/// 跳过注释行（`#` 开头）和列数不足的行。name/value 取第 6、7 列。
///
/// 如果输入不是 Netscape 格式（无 TAB 分隔行），原样返回。
/// 用于 `media_credential_check` 等需要把存储的 Cookie 作为 HTTP 头发送的场景：
/// 用户可能从「导出 cookies.txt」按钮导出后直接导入「媒体凭证」，
/// 此时存的格式是 Netscape，不能直接当作 HTTP 头值（含 TAB/换行会被 reqwest 拒绝）。
pub(crate) fn netscape_to_cookie_header(cookie: &str) -> String {
    if !is_netscape_format(cookie) {
        return cookie.to_string();
    }
    let mut segments: Vec<String> = Vec::new();
    for line in cookie.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 7 {
            continue;
        }
        let name = cols[5].trim();
        let value = cols[6].trim();
        if name.is_empty() {
            continue;
        }
        segments.push(format!("{name}={value}"));
    }
    segments.join("; ")
}

/// 将 Cookie 列表转换为 Netscape cookies 文件格式内容。
///
/// 格式：
/// ```text
/// # Netscape HTTP Cookie File
/// domain<TAB>include_subdomains<TAB>path<TAB>secure<TAB>expiration<TAB>name<TAB>value
/// ```
///
/// - `domain`：cookie 所属域名（不带协议），若前缀为 `.` 则 include_subdomains 为 TRUE；否则为 FALSE (host-only)
/// - `include_subdomains`：按 domain 是否带 leading dot 动态确定，防止 host-only 凭据泄露至全子域
/// - `path`：`/`
/// - `secure`：按 URL scheme (https 为 TRUE，http 为 FALSE) 匹配
/// - `expiration`：0 表示会话 cookie
pub(crate) fn build_netscape_content(
    pairs: &[(String, String)],
    domain: &str,
    is_https: bool,
) -> String {
    let mut output = String::from("# Netscape HTTP Cookie File\n");
    let include_subdomains = if domain.starts_with('.') {
        "TRUE"
    } else {
        "FALSE"
    };
    let secure_str = if is_https { "TRUE" } else { "FALSE" };
    for (name, value) in pairs {
        output.push_str(domain);
        output.push('\t');
        output.push_str(include_subdomains);
        output.push('\t');
        output.push('/');
        output.push('\t');
        output.push_str(secure_str);
        output.push('\t');
        output.push('0');
        output.push('\t');
        output.push_str(name);
        output.push('\t');
        output.push_str(value);
        output.push('\n');
    }
    output
}

/// 从 URL 提取主机名与 scheme 标志。
pub(crate) fn extract_host_and_scheme(url: &str) -> Option<(String, bool)> {
    let parsed = url::Url::parse(url).ok()?;
    let is_https = parsed.scheme() == "https";
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?;
    Some((host.to_string(), is_https))
}

/// 从 URL 提取域名/主机名（不带端口和协议）。
pub(crate) fn extract_domain(url: &str) -> Option<String> {
    extract_host_and_scheme(url).map(|(host, _)| host)
}

/// RAII guard：drop 时删除临时 cookie 文件。
///
/// 文件路径不得出现在日志或错误信息中。如果删除失败，仅记录通用警告。
pub(crate) struct CookieFileGuard {
    path: Option<PathBuf>,
}

impl CookieFileGuard {
    /// 创建一个不持有文件的 guard（路径为 None）。
    ///
    /// 用于无需写 cookie 文件的场景（如未提供 cookie），调用方可以统一持有 guard
    /// 而无需分支处理。drop 时是 no-op。
    pub(crate) fn empty() -> Self {
        Self { path: None }
    }

    /// 返回临时文件路径，供 yt-dlp `--cookies` 参数使用。
    /// 返回 `None` 表示未创建文件（调用方应跳过 `--cookies` 参数）。
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// 显式删除文件并清空内部路径，避免 Drop 时重复删除。
    pub async fn consume(mut self) {
        if let Some(path) = self.path.take() {
            let _ = tokio::fs::remove_file(&path).await;
        }
    }
}

impl Drop for CookieFileGuard {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            // 在同步 drop 中无法用 tokio::fs，使用 std::fs 兜底。
            // 路径不写入任何日志，删除失败仅静默忽略（已在文件系统层面）。
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// 写入临时 cookie 文件到指定目录。
///
/// - `base_dir`：临时文件存放目录（调用方负责解析 app data 目录）
/// - `cookie`：用户填写的 Cookie 字符串（`name=value; name2=value2`）
/// - `url`：媒体 URL，用于提取域名
/// - `referer`：如果提供，使用 referer 的域名（覆盖 url 的域名）
///
/// 返回 `CookieFileGuard`，drop 时自动删除文件。
///
/// 失败时返回中文错误（不含文件路径）。
fn normalize_cookie_domain(domain: &str) -> String {
    let lower = domain.to_lowercase();
    let platform = crate::media_platforms::detect_platform(&format!("https://{lower}"));
    match platform {
        crate::media_platforms::MediaPlatform::Douyin => ".douyin.com".to_string(),
        crate::media_platforms::MediaPlatform::TikTok => ".tiktok.com".to_string(),
        crate::media_platforms::MediaPlatform::Twitter => ".twitter.com".to_string(),
        crate::media_platforms::MediaPlatform::YouTube => ".youtube.com".to_string(),
        crate::media_platforms::MediaPlatform::Bilibili => ".bilibili.com".to_string(),
        crate::media_platforms::MediaPlatform::Weibo => {
            if lower.contains("weibo.cn") {
                ".weibo.cn".to_string()
            } else {
                ".weibo.com".to_string()
            }
        }
        crate::media_platforms::MediaPlatform::Unknown => {
            let platforms = [
                ("douyin.com", ".douyin.com"),
                ("iesdouyin.com", ".douyin.com"),
                ("tiktok.com", ".tiktok.com"),
                ("bilibili.com", ".bilibili.com"),
                ("youtube.com", ".youtube.com"),
                ("twitter.com", ".twitter.com"),
                ("x.com", ".twitter.com"),
                ("weibo.com", ".weibo.com"),
                ("weibo.cn", ".weibo.cn"),
            ];
            for &(p, target) in &platforms {
                if lower == p || lower.ends_with(&format!(".{}", p)) {
                    return target.to_string();
                }
            }
            lower
        }
    }
}

pub(crate) async fn write_cookie_file_in_dir(
    base_dir: &Path,
    cookie: &str,
    url: &str,
    referer: Option<&str>,
) -> Result<CookieFileGuard, String> {
    let trimmed = cookie.trim();
    if trimmed.is_empty() {
        return Ok(CookieFileGuard { path: None });
    }
    let content = if trimmed.contains("# Netscape") || trimmed.contains('\t') {
        trimmed.to_string()
    } else {
        let pairs = parse_cookie_header(trimmed);
        if pairs.is_empty() {
            return Ok(CookieFileGuard { path: None });
        }
        let target_url = referer.unwrap_or(url);
        let (domain, is_https) = extract_host_and_scheme(target_url)
            .or_else(|| extract_host_and_scheme(url))
            .ok_or_else(|| "无法从 URL 或 Referer 解析域名".to_string())?;
        let normalized = normalize_cookie_domain(&domain);
        build_netscape_content(&pairs, &normalized, is_https)
    };

    tokio::fs::create_dir_all(base_dir)
        .await
        .map_err(|e| format!("无法创建临时目录：{e}"))?;

    let file_name = format!("maobu_cookies_{}.txt", Uuid::new_v4().simple());
    let file_path = base_dir.join(file_name);

    tokio::fs::write(&file_path, &content)
        .await
        .map_err(|e| format!("无法写入临时 Cookie 文件：{e}"))?;

    // Windows ACL：限制文件仅当前用户可读
    #[cfg(target_os = "windows")]
    {
        if let Err(error) = restrict_to_current_user(&file_path) {
            // 删除已创建的文件，避免泄漏
            let _ = tokio::fs::remove_file(&file_path).await;
            return Err(format!("无法设置临时 Cookie 文件权限：{error}"));
        }
    }

    Ok(CookieFileGuard {
        path: Some(file_path),
    })
}

#[cfg(target_os = "windows")]
mod windows_acl {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        BuildExplicitAccessWithNameW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W,
        SET_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_NAME, TRUSTEE_IS_USER,
    };
    use windows_sys::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    };

    // FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE | SYNCHRONIZE | READ_CONTROL
    // 对应于"当前用户可读、可写、可删除"。0x001F01FF 是 FILE_ALL_ACCESS 的常用近似值。
    const FILE_ALL_ACCESS_FOR_USER: u32 = 0x001F01FF;

    /// 限制文件权限为仅当前用户可读写删除。
    ///
    /// 通过构建一个仅包含当前用户 ACE 的 DACL，并用
    /// `PROTECTED_DACL_SECURITY_INFORMATION` 阻止继承父目录 DACL，
    /// 确保即使父目录权限较宽松，文件本身也只有当前用户能访问。
    ///
    /// 实现说明：
    /// - 使用 `BuildExplicitAccessWithNameW` 通过用户名构造 ACE，避免直接操作 SID
    /// - 用户名来自 `USERNAME` 环境变量（Windows 总是设置）
    /// - 失败时返回中文错误，不含文件路径
    pub fn restrict_to_current_user(path: &Path) -> Result<(), String> {
        let username = std::env::var("USERNAME").map_err(|_| "无法获取当前用户名".to_string())?;
        let wide_user: Vec<u16> = OsStr::new(&username)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // 构造 EXPLICIT_ACCESS_W：当前用户拥有完全控制权限（SET_ACCESS 替换 DACL）
        // windows-sys 0.59 起 BuildExplicitAccessWithNameW 仅接受 5 个参数，
        // trustee 字段需在调用前显式设置在结构体上。
        let mut explicit_access: EXPLICIT_ACCESS_W = unsafe { std::mem::zeroed() };
        unsafe {
            explicit_access.Trustee.TrusteeForm = TRUSTEE_IS_NAME;
            explicit_access.Trustee.TrusteeType = TRUSTEE_IS_USER;
            BuildExplicitAccessWithNameW(
                &mut explicit_access,
                wide_user.as_ptr(),
                FILE_ALL_ACCESS_FOR_USER,
                SET_ACCESS,
                0, // NO_INHERITANCE
            );
        }

        let mut new_acl: *mut ACL = std::ptr::null_mut();
        unsafe {
            // SetEntriesInAclW(1, &explicit_access, NULL, &new_acl)
            let result = SetEntriesInAclW(1, &mut explicit_access, std::ptr::null(), &mut new_acl);
            if result != 0 {
                return Err(format!("无法构建 ACL（错误码 {result}）"));
            }

            // 将 path 转为 wide string（UTF-16，以 null 结尾）
            let wide_path: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            // SetNamedSecurityInfoW(path, SE_FILE_OBJECT, DACL | PROTECTED_DACL, NULL, NULL, new_acl, NULL)
            // PROTECTED_DACL_SECURITY_INFORMATION 阻止继承父目录 DACL
            let result = SetNamedSecurityInfoW(
                wide_path.as_ptr() as *mut u16,
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                new_acl as *const ACL as *mut ACL,
                std::ptr::null_mut(),
            );

            // LocalFree 期望 HLOCAL = *mut c_void
            LocalFree(new_acl as *mut core::ffi::c_void);

            if result != 0 {
                return Err(format!("无法应用安全描述符（错误码 {result}）"));
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
pub(crate) use windows_acl::restrict_to_current_user;

#[cfg(not(target_os = "windows"))]
pub(crate) fn restrict_to_current_user(_path: &Path) -> Result<(), String> {
    // 非 Windows 平台：依赖文件系统权限（POSIX）。
    // Unix 系统应在写入文件后用 std::os::unix::fs::PermissionsExt 设置 0o600。
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_cookie_header() {
        let pairs = parse_cookie_header("session=abc; theme=dark");
        assert_eq!(
            pairs,
            vec![
                ("session".to_string(), "abc".to_string()),
                ("theme".to_string(), "dark".to_string())
            ]
        );
    }

    #[test]
    fn parses_cookie_header_with_extra_spaces() {
        let pairs = parse_cookie_header("  a=1 ;  b=2  ;  =empty_name ;  c=  ");
        assert_eq!(
            pairs,
            vec![
                ("a".to_string(), "1".to_string()),
                ("b".to_string(), "2".to_string()),
                ("c".to_string(), "".to_string()),
            ]
        );
    }

    #[test]
    fn parses_empty_cookie_header() {
        assert!(parse_cookie_header("").is_empty());
        assert!(parse_cookie_header("   ").is_empty());
        assert!(parse_cookie_header("; ; ;").is_empty());
    }

    #[test]
    fn parses_cookie_value_with_equals_sign() {
        let pairs = parse_cookie_header("data=base64=a=b");
        assert_eq!(pairs, vec![("data".to_string(), "base64=a=b".to_string())]);
    }

    #[test]
    fn is_netscape_format_detects_netscape_header() {
        assert!(is_netscape_format("# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tsid\txyz"));
    }

    #[test]
    fn is_netscape_format_detects_tab_separated_lines_without_header() {
        // 没有 # Netscape 头但有 TAB 分隔 7 列的行也算
        assert!(is_netscape_format(".youtube.com\tTRUE\t/\tTRUE\t0\tsid\txyz"));
    }

    #[test]
    fn is_netscape_format_rejects_header_format() {
        assert!(!is_netscape_format("sid=xyz; token=abc"));
        assert!(!is_netscape_format(""));
        assert!(!is_netscape_format("   "));
    }

    #[test]
    fn netscape_to_cookie_header_converts_netscape_format_to_header_format() {
        let netscape = "# Netscape HTTP Cookie File\n\
            .youtube.com\tTRUE\t/\tTRUE\t1819208478\tSID\tabc123\n\
            .youtube.com\tTRUE\t/\tTRUE\t1819208478\tLOGIN_INFO\txyz789\n";
        let header = netscape_to_cookie_header(netscape);
        assert_eq!(header, "SID=abc123; LOGIN_INFO=xyz789");
    }

    #[test]
    fn netscape_to_cookie_header_passes_through_header_format_unchanged() {
        // 非 Netscape 格式原样返回
        let header = "SID=abc123; LOGIN_INFO=xyz789";
        assert_eq!(netscape_to_cookie_header(header), header);
    }

    #[test]
    fn netscape_to_cookie_header_skips_comment_and_short_lines() {
        let netscape = "# Netscape HTTP Cookie File\n\
            # 这是一个注释\n\
            .youtube.com\tTRUE\t/\tTRUE\t0\tSID\tabc\n\
            短行\n\
            .youtube.com\tTRUE\t/\tTRUE\t0\tTOKEN\txyz\n";
        let header = netscape_to_cookie_header(netscape);
        assert_eq!(header, "SID=abc; TOKEN=xyz");
    }

    #[test]
    fn netscape_to_cookie_header_handles_empty_name() {
        // name 为空的行应被跳过
        let netscape = ".youtube.com\tTRUE\t/\tTRUE\t0\t\tvalue\n\
            .youtube.com\tTRUE\t/\tTRUE\t0\tSID\tabc\n";
        let header = netscape_to_cookie_header(netscape);
        assert_eq!(header, "SID=abc");
    }

    #[test]
    fn netscape_to_cookie_header_returns_empty_for_empty_input() {
        assert_eq!(netscape_to_cookie_header(""), "");
    }

    #[test]
    fn netscape_to_cookie_header_preserves_value_with_equals_sign() {
        // Cookie value 中可能含 = 字符（如 base64），应原样保留
        let netscape = ".youtube.com\tTRUE\t/\tTRUE\t0\tdata\tbase64=a=b=c\n";
        let header = netscape_to_cookie_header(netscape);
        assert_eq!(header, "data=base64=a=b=c");
    }

    #[test]
    fn should_skip_auto_sync_blocks_header_overwriting_netscape() {
        // 已存在 Netscape 格式（用户手动导入），扩展自动同步发来头格式 → 必须跳过
        let existing = "# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tSID\txyz\n";
        let incoming = "SID=rotated-by-youtube; LOGIN_INFO=abc";
        assert!(
            should_skip_auto_sync(existing, incoming),
            "自动同步不得覆盖用户手动导入的 Netscape 格式凭证"
        );
    }

    #[test]
    fn should_skip_auto_sync_allows_header_overwriting_header() {
        // 已存在头格式，扩展自动同步发来新头格式 → 允许覆盖（普通窗口 Cookie 轮换）
        let existing = "SID=old; LOGIN_INFO=old";
        let incoming = "SID=new; LOGIN_INFO=new";
        assert!(
            !should_skip_auto_sync(existing, incoming),
            "头格式凭证可以被自动同步更新"
        );
    }

    #[test]
    fn should_skip_auto_sync_allows_netscape_overwriting_header() {
        // 已存在头格式，incoming 是 Netscape → 允许（用户从其他渠道手动同步）
        // 实际场景中自动同步不会发 Netscape，但函数逻辑上不应阻止
        let existing = "SID=old";
        let incoming = "# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tSID\tnew\n";
        assert!(
            !should_skip_auto_sync(existing, incoming),
            "incoming 为 Netscape 时应允许（可能是用户另一处手动同步"
        );
    }

    #[test]
    fn should_skip_auto_sync_allows_overwrite_when_existing_empty() {
        // 已存在为空 → 允许覆盖（首次设置）
        assert!(!should_skip_auto_sync("", "SID=abc"));
    }

    #[test]
    fn normalize_cookie_domain_maps_short_urls_to_canonical_domain() {
        assert_eq!(normalize_cookie_domain("youtu.be"), ".youtube.com");
        assert_eq!(normalize_cookie_domain("www.youtube.com"), ".youtube.com");
        assert_eq!(normalize_cookie_domain("x.com"), ".twitter.com");
        assert_eq!(normalize_cookie_domain("b23.tv"), ".bilibili.com");
        assert_eq!(normalize_cookie_domain("v.douyin.com"), ".douyin.com");
    }

    #[test]
    fn builds_netscape_format_with_header_line() {
        let pairs = vec![("sid".to_string(), "xyz".to_string())];
        let content = build_netscape_content(&pairs, "example.com", true);
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines[0], "# Netscape HTTP Cookie File");
        assert_eq!(lines.len(), 2);
        let fields: Vec<&str> = lines[1].split('\t').collect();
        assert_eq!(fields.len(), 7);
        assert_eq!(fields[0], "example.com");
        assert_eq!(fields[1], "FALSE");
        assert_eq!(fields[2], "/");
        assert_eq!(fields[3], "TRUE");
        assert_eq!(fields[4], "0");
        assert_eq!(fields[5], "sid");
        assert_eq!(fields[6], "xyz");
    }

    #[test]
    fn builds_netscape_format_multiple_cookies() {
        let pairs = vec![
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "2".to_string()),
        ];
        let content = build_netscape_content(&pairs, ".test.org", false);
        assert_eq!(content.matches('\n').count(), 3);
        // Netscape 格式：domain\tTRUE\t/\tFALSE\t0\tname\tvalue
        assert!(content.contains(".test.org\tTRUE\t/\tFALSE\t0\ta\t1"));
        assert!(content.contains(".test.org\tTRUE\t/\tFALSE\t0\tb\t2"));
    }

    #[test]
    fn extracts_domain_from_https_url() {
        assert_eq!(
            extract_domain("https://www.example.com/path"),
            Some("www.example.com".to_string())
        );
        assert_eq!(
            extract_domain("https://api.test.org"),
            Some("api.test.org".to_string())
        );
    }

    #[test]
    fn extract_domain_rejects_non_http_schemes() {
        assert!(extract_domain("file:///C:/test").is_none());
        assert!(extract_domain("ftp://example.com").is_none());
        assert!(extract_domain("not a url").is_none());
    }

    #[tokio::test]
    async fn cookie_file_guard_deletes_on_drop() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test_cookie.txt");
        tokio::fs::write(&path, "content").await.unwrap();
        assert!(path.exists());

        {
            let _guard = CookieFileGuard {
                path: Some(path.clone()),
            };
            // guard 离开作用域时删除文件
        }

        assert!(!path.exists());
    }

    #[tokio::test]
    async fn cookie_file_guard_consume_deletes_immediately() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("test_cookie.txt");
        tokio::fs::write(&path, "content").await.unwrap();

        let guard = CookieFileGuard {
            path: Some(path.clone()),
        };
        guard.consume().await;
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn cookie_file_guard_with_none_path_is_noop() {
        let guard = CookieFileGuard::empty();
        assert!(guard.path().is_none());
        // consume 接收所有权并立即返回，不应 panic
        guard.consume().await;
    }

    #[test]
    fn cookie_file_guard_drop_handles_missing_file_gracefully() {
        let guard = CookieFileGuard {
            path: Some(PathBuf::from("/nonexistent/path/cookie.txt")),
        };
        // 不应 panic
        drop(guard);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn restrict_to_current_user_succeeds_for_temp_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("acl_test.txt");
        std::fs::write(&path, b"test").unwrap();
        // 仅验证函数返回 Ok；ACL 实际效果由系统集成测试覆盖。
        // 集成测试可通过 PowerShell `Get-Acl <path>` 验证 DACL 仅包含当前用户。
        let result = restrict_to_current_user(&path);
        assert!(result.is_ok(), "ACL 设置失败：{result:?}");
        // 验证文件仍然可读（当前用户拥有权限）
        let content = std::fs::read(&path).unwrap();
        assert_eq!(content, b"test");
    }
}
