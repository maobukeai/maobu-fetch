//! 百度网盘 (Baidu Pan / Baidu Netdisk) 分享解析与直链获取模块。
//!
//! 遵循本地优先、无外部额外进程、安全合规原则：
//! 1. 运行在 Rust 原生后端，彻底规避 WebView CORS 跨域限制与 IP 绑定偏差；
//! 2. 支持公开分享与带提取码加密分享的目录树解析；
//! 3. 结合用户提供的或本地凭证库中保存的 Cookie（BDUSS/STOKEN）获取官方下载直链（dlink）；
//! 4. 配合专属 User-Agent（pan.baidu.com）直连猫步下载器 HTTP Range 多连接并发内核。

use regex::Regex;
#[path = "baidupan/listing.rs"]
mod listing;
#[path = "baidupan/resolve.rs"]
mod resolve;

pub const BAIDU_API_HOST: &str = "https://pan.baidu.com";
pub const BAIDU_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
pub const BAIDU_DLINK_USER_AGENT: &str = "netdisk;P2SP;2.2.90.43;WindowsBaiduYunGuanJia;netdisk;11.4.5;android-android;11.0;JSbridge4.4.0;LogStatistic";

pub(crate) fn dlink_user_agent(url: &str) -> &'static str {
    if url.contains("-266719-") || url.contains("-498065-") || url.contains("-309847-") {
        BAIDU_DLINK_USER_AGENT
    } else {
        "pan.baidu.com"
    }
}

fn decode_json_string_literal(raw: &str) -> String {
    let wrapped = format!("\"{raw}\"");
    serde_json::from_str::<String>(&wrapped).unwrap_or_else(|_| raw.to_string())
}

fn detect_share_page_error(html: &str) -> Option<&'static str> {
    if html.contains("wappass.baidu.com")
        || html.contains("百度安全验证")
        || html.contains("passport.baidu.com/v2/?login")
    {
        return Some("百度网盘触发了安全验证或登录拦截，请在浏览器中打开分享页并同步 Cookie 后重试");
    }
    if html.contains("分享的文件已经被取消")
        || html.contains("啊哦，你所访问的页面不存在了")
        || html.contains("链接不存在")
        || html.contains("已过期")
        || html.contains("涉及侵权")
    {
        return Some("该百度网盘分享链接已失效、被取消或不存在");
    }
    None
}

pub(crate) fn is_browser_download_target(url: &url::Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|host| {
            matches!(
                host,
                "pan.baidu.com" | "yun.baidu.com" | "pcs.baidu.com" | "baidupcs.com"
            ) || host.ends_with(".pcs.baidu.com")
                || host.ends_with(".baidupcs.com")
        })
}

pub(crate) fn is_browser_download_content_type(value: &str) -> bool {
    let essence = value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let Some((kind, subtype)) = essence.split_once('/') else {
        return false;
    };
    [kind, subtype].into_iter().all(|part| {
        !part.is_empty()
            && part.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || b"!#$%&'+-.^_`|~".contains(&byte)
            })
    }) && kind != "multipart"
        && subtype != "json"
        && !subtype.ends_with("+json")
        && !matches!(essence.as_str(), "text/html" | "application/xhtml+xml")
}

#[path = "baidupan/models.rs"]
mod models;
pub use models::*;

#[derive(Debug, Clone)]
pub struct ParsedBaiduUrl {
    pub surl: String,
    pub pass_code: Option<String>,
}

/// 判断文本是否包含百度网盘分享链接
pub fn is_baidu_url(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return false;
    }
    let re =
        Regex::new(r"(?i)https?://(?:pan|yun)\.baidu\.com/(?:s/|share/init\?surl=)[a-zA-Z0-9_-]+")
            .unwrap();
    re.is_match(trimmed)
}

/// 从 URL 或文本中提取 surl 与提取码
pub fn parse_baidu_url(raw: &str) -> Option<ParsedBaiduUrl> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 匹配 surl
    // 形式 1: https://pan.baidu.com/s/1xxxx 或 https://pan.baidu.com/s/xxxx
    // 形式 2: https://pan.baidu.com/share/init?surl=xxxx
    let surl_re = Regex::new(r"(?i)https?://(?:pan|yun)\.baidu\.com/(?:s/1?([a-zA-Z0-9_-]+)|share/init\?surl=1?([a-zA-Z0-9_-]+))").unwrap();
    let caps = surl_re.captures(trimmed)?;
    let surl = caps.get(1).or_else(|| caps.get(2))?.as_str().to_string();

    // 匹配提取码
    let pass_code_re =
        Regex::new(r"(?i)(?:(?:pwd|code|提取码|密码)[：:\s=]*([a-zA-Z0-9]{4}))").unwrap();
    let pass_code = pass_code_re
        .captures(trimmed)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());

    Some(ParsedBaiduUrl { surl, pass_code })
}

pub fn url_encode(input: &str) -> String {
    let mut result = String::new();
    for b in input.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                result.push(b as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", b));
            }
        }
    }
    result
}

/// 校验百度网盘提取码，换取 BDCLND (randsk)
pub async fn verify_baidu_pass_code(surl: &str, pass_code: &str) -> Result<String, String> {
    let network = crate::cloud_request::CloudRequestContext::defaults(
        BAIDU_USER_AGENT,
        20,
        tokio_util::sync::CancellationToken::new(),
    )?;
    verify_baidu_pass_code_with_context(surl, pass_code, &network).await
}
pub async fn verify_baidu_pass_code_with_context(
    surl: &str,
    pass_code: &str,
    network: &crate::cloud_request::CloudRequestContext,
) -> Result<String, String> {
    let network = network
        .with_user_agent(BAIDU_USER_AGENT)
        .with_cookie_source(BAIDU_API_HOST);
    network
        .run(
            "百度网盘",
            verify_baidu_pass_code_impl(surl, pass_code, &network),
        )
        .await
}
async fn verify_baidu_pass_code_impl(
    surl: &str,
    pass_code: &str,
    network: &crate::cloud_request::CloudRequestContext,
) -> Result<String, String> {
    let client = network;
    let clean_surl = surl.strip_prefix('1').unwrap_or(surl);
    let verify_url = format!(
        "{}/share/verify?channel=chunlei&clienttype=0&web=1&app_id=250528&surl={}",
        BAIDU_API_HOST, clean_surl
    );

    let params = [("pwd", pass_code), ("vcode", ""), ("vcode_str", "")];

    let resp = client
        .post(&verify_url)
        .form(&params)
        .header("Referer", format!("{}/s/1{}", BAIDU_API_HOST, clean_surl))
        .header("Origin", BAIDU_API_HOST)
        .send()
        .await
        .map_err(|e| format!("验证提取码网络请求失败: {}", e))?;

    // 检查 Set-Cookie 中的 BDCLND
    let mut randsk = String::new();
    for val in resp.headers().get_all("set-cookie") {
        if let Ok(s) = val.to_str() {
            if let Some(pos) = s.find("BDCLND=") {
                let rest = &s[pos + 7..];
                let end = rest.find(';').unwrap_or(rest.len());
                randsk = rest[..end].to_string();
                break;
            }
        }
    }

    let text = network
        .text(resp)
        .await
        .map_err(|e| format!("读取验证响应失败: {}", e))?;
    let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();

    let errno = json.get("errno").and_then(|v| v.as_i64()).unwrap_or(0);
    if errno != 0 {
        if errno == -9 {
            return Err("提取码错误，请重新输入 4 位提取码".to_string());
        }
        return Err(format!(
            "验证提取码失败（代码 {errno}），请检查密码和分享权限"
        ));
    }

    if randsk.is_empty() {
        if let Some(r) = json.get("randsk").and_then(|v| v.as_str()) {
            randsk = r.to_string();
        }
    }

    if randsk.is_empty() {
        return Err("未能获取到有效的分享访问令牌 (BDCLND)".to_string());
    }

    Ok(randsk)
}

/// 解析百度网盘分享信息与文件树
pub async fn inspect_baidu_share(
    url: &str,
    pass_code: Option<&str>,
    cookie: Option<&str>,
) -> Result<BaiduShareInfo, String> {
    inspect_baidu_share_with_cancel(
        url,
        pass_code,
        cookie,
        &tokio_util::sync::CancellationToken::new(),
    )
    .await
}
pub async fn inspect_baidu_share_with_cancel(
    url: &str,
    pass_code: Option<&str>,
    cookie: Option<&str>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<BaiduShareInfo, String> {
    let network =
        crate::cloud_request::CloudRequestContext::defaults(BAIDU_USER_AGENT, 20, cancel.clone())?;
    inspect_baidu_share_with_context(url, pass_code, cookie, &network).await
}
pub async fn inspect_baidu_share_with_context(
    url: &str,
    pass_code: Option<&str>,
    cookie: Option<&str>,
    network: &crate::cloud_request::CloudRequestContext,
) -> Result<BaiduShareInfo, String> {
    let network = network
        .with_user_agent(BAIDU_USER_AGENT)
        .with_cookie_source(BAIDU_API_HOST);
    network
        .run(
            "百度网盘",
            inspect_baidu_share_impl(url, pass_code, cookie, &network),
        )
        .await
}
async fn inspect_baidu_share_impl(
    url: &str,
    pass_code: Option<&str>,
    cookie: Option<&str>,
    network: &crate::cloud_request::CloudRequestContext,
) -> Result<BaiduShareInfo, String> {
    let parsed = parse_baidu_url(url).ok_or_else(|| "无效的百度网盘分享链接".to_string())?;
    let clean_surl = parsed.surl.strip_prefix('1').unwrap_or(&parsed.surl);
    let effective_pass_code = pass_code.or(parsed.pass_code.as_deref());

    let mut current_randsk = String::new();
    let need_pass_code = false;

    // 若有提取码，优先验证提取码拿到 randsk
    if let Some(pwd) = effective_pass_code {
        if !pwd.trim().is_empty() {
            match verify_baidu_pass_code_with_context(clean_surl, pwd.trim(), network).await {
                Ok(r) => current_randsk = r,
                Err(e) => return Err(e),
            }
        }
    }

    let client = network;
    let page_url = format!("{}/s/1{}", BAIDU_API_HOST, clean_surl);

    let cookie_header = if current_randsk.is_empty() {
        cookie.unwrap_or("").to_owned()
    } else {
        crate::cloud_request::with_cookie_pair(
            cookie.unwrap_or(""),
            "BDCLND",
            &current_randsk,
            BAIDU_API_HOST,
        )?
    };

    let page_resp = client
        .get(&page_url)
        .header("Cookie", &cookie_header)
        .header("Referer", BAIDU_API_HOST)
        .send()
        .await
        .map_err(|e| format!("请求百度网盘分享页失败: {}", e))?;

    let html = network
        .text(page_resp)
        .await
        .map_err(|e| format!("读取分享页面失败: {}", e))?;

    // 判断是否需要提取码
    if html.contains("请输入提取码")
        || html.contains("share-verify-form")
        || html.contains("init?surl=")
    {
        if current_randsk.is_empty() {
            return Ok(BaiduShareInfo {
                surl: clean_surl.to_string(),
                share_id: None,
                uk: None,
                title: "加密分享文件".to_string(),
                files: Vec::new(),
                total_size: 0,
                file_count: 0,
                folder_count: 0,
                pass_code_required: true,
                randsk: None,
                sign: None,
                timestamp: None,
                seckey: None,
            });
        }
    }

    // 从页面提取 yunData 基础配置
    let mut share_id: Option<String> = None;
    let mut uk: Option<String> = None;
    let mut sign: Option<String> = None;
    let mut timestamp: Option<i64> = None;
    let mut seckey: Option<String> = None;
    let mut title = format!("百度网盘分享_{}", clean_surl);

    // 优先提取 share_uk（分享者的真实 UK，避免匹配到当前未登录访客的 uk: 0）
    let share_uk_re = Regex::new(r#"["']share_uk["']\s*:\s*["']?(\d+)["']?"#).unwrap();
    if let Some(caps) = share_uk_re.captures(&html) {
        if let Some(m) = caps.get(1) {
            uk = Some(m.as_str().to_string());
        }
    }
    if uk.is_none() {
        let uk_re = Regex::new(r#"["']uk["']\s*:\s*["']?([1-9]\d*)["']?"#).unwrap();
        if let Some(caps) = uk_re.captures(&html) {
            if let Some(m) = caps.get(1) {
                uk = Some(m.as_str().to_string());
            }
        }
    }

    let shareid_re = Regex::new(
        r#"["']shareid["']\s*:\s*["']?(\d+)["']?|["']share_id["']\s*:\s*["']?(\d+)["']?"#,
    )
    .unwrap();
    if let Some(caps) = shareid_re.captures(&html) {
        if let Some(m) = caps.get(1).or_else(|| caps.get(2)) {
            share_id = Some(m.as_str().to_string());
        }
    }

    let sign_re = Regex::new(r#"["']sign["']\s*:\s*["']([a-zA-Z0-9+/=]+)["']"#).unwrap();
    if let Some(caps) = sign_re.captures(&html) {
        if let Some(m) = caps.get(1) {
            sign = Some(m.as_str().to_string());
        }
    }

    let ts_re =
        Regex::new(r#"["']timestamp["']\s*:\s*["']?(\d+)["']?|["']servertime["']\s*,\s*(\d+)"#)
            .unwrap();
    if let Some(caps) = ts_re.captures(&html) {
        if let Some(m) = caps.get(1).or_else(|| caps.get(2)) {
            timestamp = m.as_str().parse::<i64>().ok();
        }
    }

    let seckey_re = Regex::new(r#"["']seckey["']\s*:\s*["']([a-zA-Z0-9+/=]+)["']"#).unwrap();
    if let Some(caps) = seckey_re.captures(&html) {
        if let Some(m) = caps.get(1) {
            seckey = Some(m.as_str().to_string());
        }
    }

    let title_re = Regex::new(r#""server_filename"\s*:\s*"((?:[^"\\]|\\.)+)""#).unwrap();
    if let Some(caps) = title_re.captures(&html) {
        if let Some(m) = caps.get(1) {
            let decoded = decode_json_string_literal(m.as_str());
            if !decoded.trim().is_empty() {
                title = decoded;
            }
        }
    }

    if uk.as_deref().is_none_or(str::is_empty) || share_id.as_deref().is_none_or(str::is_empty) {
        if let Some(reason) = detect_share_page_error(&html) {
            return Err(reason.to_string());
        }
    }

    let effective_uk = uk
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or("百度网盘文件列表未完整：分享者标识读取失败，请检查分享链接和权限")?;
    let effective_shareid = share_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or("百度网盘文件列表未完整：分享标识读取失败，请检查分享链接和权限")?;
    let all_files = listing::fetch(
        network.client(),
        BAIDU_API_HOST,
        clean_surl,
        effective_uk,
        effective_shareid,
        &cookie_header,
        &page_url,
        network.cancel().clone(),
        network.user_agent(),
        network.clone(),
    )
    .await?;
    let file_count = all_files
        .iter()
        .filter(|entry| entry.kind == "drive#file")
        .count();
    let folder_count = all_files.len() - file_count;
    let total_size = all_files
        .iter()
        .filter(|entry| entry.kind == "drive#file")
        .try_fold(0u64, |total, entry| {
            total
                .checked_add(entry.size)
                .ok_or("百度网盘文件总大小溢出，列表未完整")
        })?;

    Ok(BaiduShareInfo {
        surl: clean_surl.to_string(),
        share_id,
        uk,
        title,
        files: all_files,
        total_size,
        file_count,
        folder_count,
        pass_code_required: need_pass_code,
        randsk: if current_randsk.is_empty() {
            None
        } else {
            Some(current_randsk)
        },
        sign,
        timestamp,
        seckey,
    })
}

fn url_decode(input: &str) -> String {
    let mut out = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex_str) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex_str, 16) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// 解析单文件下载直链
pub async fn resolve_baidu_file(
    surl: &str,
    fs_id: &str,
    share_id: Option<&str>,
    uk: Option<&str>,
    sign: Option<&str>,
    timestamp: Option<i64>,
    seckey: Option<&str>,
    randsk: Option<&str>,
    cookie: Option<&str>,
) -> Result<BaiduDirectUrlResult, String> {
    let network = crate::cloud_request::CloudRequestContext::defaults(
        BAIDU_USER_AGENT,
        20,
        tokio_util::sync::CancellationToken::new(),
    )?;
    resolve_baidu_file_with_context(
        surl, fs_id, share_id, uk, sign, timestamp, seckey, randsk, cookie, &network,
    )
    .await
}
pub async fn resolve_baidu_file_with_context(
    surl: &str,
    fs_id: &str,
    share_id: Option<&str>,
    uk: Option<&str>,
    sign: Option<&str>,
    timestamp: Option<i64>,
    seckey: Option<&str>,
    randsk: Option<&str>,
    cookie: Option<&str>,
    network: &crate::cloud_request::CloudRequestContext,
) -> Result<BaiduDirectUrlResult, String> {
    let network = network
        .with_user_agent(BAIDU_USER_AGENT)
        .with_cookie_source(BAIDU_API_HOST);
    network
        .run(
            "百度网盘",
            resolve_baidu_file_impl(
                surl, fs_id, share_id, uk, sign, timestamp, seckey, randsk, cookie, &network,
            ),
        )
        .await
}
async fn resolve_baidu_file_impl(
    surl: &str,
    fs_id: &str,
    share_id: Option<&str>,
    uk: Option<&str>,
    sign: Option<&str>,
    timestamp: Option<i64>,
    seckey: Option<&str>,
    randsk: Option<&str>,
    cookie: Option<&str>,
    network: &crate::cloud_request::CloudRequestContext,
) -> Result<BaiduDirectUrlResult, String> {
    resolve::resolve_baidu_file_impl(
        surl, fs_id, share_id, uk, sign, timestamp, seckey, randsk, cookie, network,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_baidu_url() {
        assert!(is_baidu_url("https://pan.baidu.com/s/1xxxx"));
        assert!(is_baidu_url("http://pan.baidu.com/s/1abcdefg"));
        assert!(is_baidu_url("https://yun.baidu.com/s/1test123"));
        assert!(is_baidu_url("https://pan.baidu.com/share/init?surl=abcxyz"));
        assert!(!is_baidu_url("https://pan.quark.cn/s/123456"));
        assert!(!is_baidu_url("https://example.com/s/123"));
    }

    #[test]
    fn test_parse_baidu_url() {
        let res1 = parse_baidu_url("https://pan.baidu.com/s/1abcdefg");
        assert!(res1.is_some());
        let p1 = res1.unwrap();
        assert_eq!(p1.surl, "abcdefg");
        assert!(p1.pass_code.is_none());

        let res2 = parse_baidu_url("https://pan.baidu.com/s/1abcdefg?pwd=1234");
        assert!(res2.is_some());
        let p2 = res2.unwrap();
        assert_eq!(p2.surl, "abcdefg");
        assert_eq!(p2.pass_code.as_deref(), Some("1234"));

        let res3 = parse_baidu_url(
            "链接: https://pan.baidu.com/s/1xyz789 提取码: abcd 复制这段内容后打开百度网盘手机App",
        );
        assert!(res3.is_some());
        let p3 = res3.unwrap();
        assert_eq!(p3.surl, "xyz789");
        assert_eq!(p3.pass_code.as_deref(), Some("abcd"));
    }

    #[test]
    fn test_dlink_user_agent_matches_endpoint_app_id() {
        assert_eq!(
            dlink_user_agent("https://d.pcs.baidu.com/file/abc-250528-xyz"),
            "pan.baidu.com"
        );
        assert_eq!(
            dlink_user_agent("https://d.pcs.baidu.com/file/abc-266719-xyz"),
            BAIDU_DLINK_USER_AGENT
        );
        assert_eq!(
            dlink_user_agent("https://d.pcs.baidu.com/file/abc-498065-xyz"),
            BAIDU_DLINK_USER_AGENT
        );
    }

    #[test]
    fn test_decode_json_title_and_detect_share_errors() {
        assert_eq!(
            decode_json_string_literal(r"\u6d4b\u8bd5\u6587\u4ef6.zip"),
            "测试文件.zip"
        );
        assert!(detect_share_page_error("啊哦，你所访问的页面不存在了").is_some());
        assert!(detect_share_page_error("https://wappass.baidu.com/static/captcha").is_some());
        assert!(detect_share_page_error("<html>normal share</html>").is_none());
    }

    #[test]
    fn test_transfer_errno_111_rejected_and_https_locate_preferred() {
        let err_111 = resolve::parse_transfer_response(&serde_json::json!({ "errno": 111 }), "999")
            .err()
            .expect("errno 111 must not be treated as success");
        assert!(err_111.contains("111"));

        let err_12_empty =
            resolve::parse_transfer_response(&serde_json::json!({ "errno": 12 }), "999")
                .err()
                .expect("errno 12 without target info must fail instead of falling back to sharer fs_id");
        assert!(err_12_empty.contains("12"));

        let ok_12 = resolve::parse_transfer_response(
            &serde_json::json!({
                "errno": 12,
                "extra": { "list": [{ "to_fs_id": 12345, "to": "/saved.zip" }] }
            }),
            "999",
        )
        .unwrap();
        assert_eq!(ok_12.fs_id, "12345");
        assert_eq!(ok_12.path.as_deref(), Some("/saved.zip"));

        let chosen = resolve::select_preferred_locate_url(&[
            serde_json::json!({ "url": "http://cdn.baidupcs.com/a" }),
            serde_json::json!({ "url": "https://cdn.baidupcs.com/b" }),
        ]);
        assert_eq!(chosen.as_deref(), Some("https://cdn.baidupcs.com/b"));
    }

    #[test]
    fn test_netscape_cookie_flattened_for_dlink_and_bduss_validated() {
        let netscape = "# Netscape HTTP Cookie File\n.baidu.com\tTRUE\t/\tTRUE\t0\tBDUSS\tsynth\n.pan.baidu.com\tTRUE\t/\tTRUE\t0\tSTOKEN\tstok\n";
        let built = resolve::build_dlink_result(
            "https://d.pcs.baidu.com/file/123?app_id=250528",
            "pan.baidu.com",
            netscape,
            Some("https://pan.baidu.com/disk/home"),
        );
        let cookie_val = built.headers.get("Cookie").unwrap();
        assert!(!cookie_val.contains('\n') && !cookie_val.contains('\t'));
        assert!(cookie_val.contains("BDUSS=synth"));
        assert!(resolve::has_valid_bduss(netscape));
        assert!(!resolve::has_valid_bduss("BDUSS=; STOKEN=only"));
        assert!(!resolve::has_valid_bduss(
            ".baidu.com\tTRUE\t/\tTRUE\t1\tBDUSS\texpired\n"
        ));

        let info_json = serde_json::json!({
            "errno": 0,
            "info": [{ "dlink": "https://d.pcs.baidu.com/file/sample", "fs_id": 1 }]
        });
        assert_eq!(
            resolve::extract_filemetas_item(&info_json)
                .and_then(|v| v.get("dlink"))
                .and_then(|v| v.as_str()),
            Some("https://d.pcs.baidu.com/file/sample")
        );
    }

    #[tokio::test]
    #[ignore = "显式授权的联网验收：设置 MAOBU_TEST_BAIDU_SHARE 后单独运行"]
    async fn test_inspect_real_baidu_share() {
        let url = std::env::var("MAOBU_TEST_BAIDU_SHARE")
            .expect("请显式提供自己的合法测试分享 MAOBU_TEST_BAIDU_SHARE");
        let cookie = std::env::var("MAOBU_TEST_BAIDU_COOKIE").ok();
        let result = inspect_baidu_share(&url, None, cookie.as_deref()).await;
        assert!(result.is_ok(), "百度在线解析失败；不将失败记为跳过或成功");
        let info = result.unwrap();
        assert!(
            info.files
                .iter()
                .any(|file| file.kind == "drive#file" && file.size > 0),
            "测试分享必须包含非空文件"
        );
    }

    #[tokio::test]
    #[ignore = "显式授权的在线直链解析；只读取测试环境变量，不读取个人数据"]
    async fn test_resolve_real_baidu_file() {
        // 从测试配置取得凭证；严禁自动读取个人客户端数据库或用户信息接口。
        let url = std::env::var("MAOBU_TEST_BAIDU_SHARE").expect("请设置 MAOBU_TEST_BAIDU_SHARE");
        let cookie = std::env::var("MAOBU_TEST_BAIDU_COOKIE")
            .expect("请设置该测试分享所需的 MAOBU_TEST_BAIDU_COOKIE");
        let result = inspect_baidu_share(&url, None, Some(&cookie)).await;
        assert!(result.is_ok(), "百度在线元数据解析失败（凭证与回执不输出）");
        let info = result.unwrap();
        let file = info
            .files
            .iter()
            .find(|file| file.kind == "drive#file")
            .expect("测试分享必须包含文件");
        let result = resolve_baidu_file(
            &info.surl,
            &file.id,
            info.share_id.as_deref(),
            info.uk.as_deref(),
            info.sign.as_deref(),
            info.timestamp,
            info.seckey.as_deref(),
            info.randsk.as_deref(),
            Some(&cookie),
        )
        .await;
        assert!(
            result.is_ok(),
            "百度在线直链解析失败（凭证与签名URL不输出）"
        );
        let result = result.unwrap();
        assert!(
            url::Url::parse(&result.url).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
        );
    }
}
