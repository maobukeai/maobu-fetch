//! 123云盘 (123pan) 公开分享解析与直链获取模块。
//!
//! 支持 `123pan.com/s/xxxx`、`123pan.cn`、`123684.com`、`*.share.123pan.cn/123pan/xxxx` 等分享链接，
//! 支持公开分享免登录解析、多层目录递归展开与 32 线程满载多并发下载。

use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;
#[path = "pan123/listing.rs"]
mod listing;

pub const PAN123_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub const PAN123_API_BASE: &str = "https://www.123684.com/api";

static RE_PAN123_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)https?://(?:[a-zA-Z0-9-]+\.)?(?:123pan\.(?:com|cn)|123684\.com|123952\.com)(?:/s/|/123pan/|/)([a-zA-Z0-9_-]+)(?:\.html)?(?:\?[^\s#]*)?")
        .expect("RE_PAN123_URL regex compile")
});

#[path = "pan123/models.rs"]
mod models;
pub use models::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPan123Url {
    pub host: String,
    pub share_key: String,
    pub pass_code: Option<String>,
}

pub fn parse_pan123_url(raw: &str) -> Option<ParsedPan123Url> {
    let parsed = url::Url::parse(raw).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    if !is_pan123_host(&host) {
        return None;
    }

    let path_segments: Vec<&str> = parsed.path_segments()?.filter(|s| !s.is_empty()).collect();
    let last_seg = path_segments.last()?;
    let share_key = last_seg.trim_end_matches(".html").to_string();
    if share_key.is_empty() {
        return None;
    }

    let mut pass_code = None;
    for (k, v) in parsed.query_pairs() {
        if (k == "pwd" || k == "p" || k == "passcode" || k == "SharePwd") && !v.is_empty() {
            pass_code = Some(v.to_string());
            break;
        }
    }

    Some(ParsedPan123Url {
        host,
        share_key,
        pass_code,
    })
}

pub fn is_pan123_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host.contains("123pan") || host.contains("123684") || host.contains("123952")
}

pub async fn inspect_pan123_share(
    share_url: &str,
    pass_code: Option<&str>,
) -> Result<Pan123ShareInfo, String> {
    inspect_pan123_share_with_cancel(
        share_url,
        pass_code,
        &tokio_util::sync::CancellationToken::new(),
    )
    .await
}
pub async fn inspect_pan123_share_with_cancel(
    share_url: &str,
    pass_code: Option<&str>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Pan123ShareInfo, String> {
    let network =
        crate::cloud_request::CloudRequestContext::defaults(PAN123_USER_AGENT, 15, cancel.clone())?;
    inspect_pan123_share_with_context(share_url, pass_code, &network).await
}
pub async fn inspect_pan123_share_with_context(
    share_url: &str,
    pass_code: Option<&str>,
    network: &crate::cloud_request::CloudRequestContext,
) -> Result<Pan123ShareInfo, String> {
    let network = network
        .with_user_agent(PAN123_USER_AGENT)
        .with_cookie_source(PAN123_API_BASE);
    network
        .run(
            "123云盘",
            inspect_pan123_share_impl(share_url, pass_code, &network),
        )
        .await
}
async fn inspect_pan123_share_impl(
    share_url: &str,
    pass_code: Option<&str>,
    network: &crate::cloud_request::CloudRequestContext,
) -> Result<Pan123ShareInfo, String> {
    let parsed =
        parse_pan123_url(share_url).ok_or_else(|| "无法识别的 123云盘分享链接".to_string())?;
    let effective_pwd = pass_code
        .map(|s| s.to_string())
        .or(parsed.pass_code.clone());

    let mut source = listing::Pan123Source {
        context: crate::cloud_pagination::Context::new(
            network.client(),
            PAN123_API_BASE,
            network.cancel().clone(),
            "123云盘",
        )
        .with_user_agent(network.user_agent())
        .with_network(network.clone()),
        share_key: &parsed.share_key,
        password: effective_pwd.as_deref().unwrap_or(""),
        referer: share_url,
    };
    let entries = match crate::cloud_pagination::collect(&mut source, "0").await {
        Ok(entries) => entries,
        Err(error) if error == "NEED_PASS_CODE" => {
            return Ok(Pan123ShareInfo {
                share_key: parsed.share_key.clone(),
                share_url: share_url.into(),
                title: "123云盘加密分享".into(),
                is_folder: false,
                requires_password: true,
                files: Vec::new(),
            })
        }
        Err(error) => return Err(error),
    };
    let is_folder = entries.iter().any(|entry| entry.folder)
        || entries.iter().filter(|entry| !entry.folder).count() > 1;
    let title = entries
        .iter()
        .find(|entry| entry.folder)
        .map(|entry| entry.name.clone())
        .unwrap_or_else(|| "123云盘分享".into());
    let mut files = Vec::new();
    for entry in entries.into_iter().filter(|entry| !entry.folder) {
        let id = entry
            .id
            .parse::<i64>()
            .map_err(|_| "123云盘文件列表未完整：文件标识无效")?;
        files.push(Pan123FileItem {
            id,
            name: entry.name,
            size: entry.size,
            size_formatted: format_bytes(entry.size),
            etag: entry
                .raw
                .get("Etag")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .into(),
            s3_key_flag: entry
                .raw
                .get("S3KeyFlag")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .into(),
            kind: "file".into(),
        });
    }
    Ok(Pan123ShareInfo {
        share_key: parsed.share_key,
        share_url: share_url.into(),
        title,
        is_folder,
        requires_password: false,
        files,
    })
}

pub async fn resolve_pan123_file(
    share_key: &str,
    file_id: i64,
    s3_key_flag: &str,
    size: u64,
    etag: &str,
    pass_code: Option<&str>,
    token_or_cookie: Option<&str>,
) -> Result<Pan123DirectUrlResult, String> {
    let network = crate::cloud_request::CloudRequestContext::defaults(
        PAN123_USER_AGENT,
        15,
        tokio_util::sync::CancellationToken::new(),
    )?;
    resolve_pan123_file_with_context(
        share_key,
        file_id,
        s3_key_flag,
        size,
        etag,
        pass_code,
        token_or_cookie,
        &network,
    )
    .await
}
pub async fn resolve_pan123_file_with_context(
    share_key: &str,
    file_id: i64,
    s3_key_flag: &str,
    size: u64,
    etag: &str,
    pass_code: Option<&str>,
    token_or_cookie: Option<&str>,
    network: &crate::cloud_request::CloudRequestContext,
) -> Result<Pan123DirectUrlResult, String> {
    let network = network
        .with_user_agent(PAN123_USER_AGENT)
        .with_cookie_source(PAN123_API_BASE);
    network
        .run(
            "123云盘",
            resolve_pan123_file_impl(
                share_key,
                file_id,
                s3_key_flag,
                size,
                etag,
                pass_code,
                token_or_cookie,
                &network,
            ),
        )
        .await
}
async fn resolve_pan123_file_impl(
    share_key: &str,
    file_id: i64,
    s3_key_flag: &str,
    size: u64,
    etag: &str,
    pass_code: Option<&str>,
    token_or_cookie: Option<&str>,
    network: &crate::cloud_request::CloudRequestContext,
) -> Result<Pan123DirectUrlResult, String> {
    let client = network;

    let api_url = format!("{}/share/download/info", PAN123_API_BASE);

    let body = serde_json::json!({
        "ShareKey": share_key,
        "FileID": file_id,
        "S3keyFlag": s3_key_flag,
        "Size": size,
        "Etag": etag,
        "SharePwd": pass_code.unwrap_or("")
    });

    let mut req = client
        .post(&api_url)
        .header("Referer", format!("https://www.123pan.com/s/{}", share_key))
        .header("Platform", "web")
        .header("App-Version", "3")
        .json(&body);

    if let Some(cred) = token_or_cookie {
        let cred_trim = cred.trim();
        if !cred_trim.is_empty() {
            if crate::media_cookies::is_netscape_format(cred_trim)
                || cred_trim.contains('=') && !cred_trim.starts_with("Bearer")
            {
                req = req.header("Cookie", cred_trim);
                let scoped =
                    crate::media_cookies::header_for_url(cred_trim, PAN123_API_BASE, &api_url);
                if let Some(tok) = scoped.split(';').find_map(|p| {
                    let p_trim = p.trim();
                    if p_trim.starts_with("token=") || p_trim.starts_with("authorization=") {
                        Some(p_trim.split('=').nth(1)?.trim())
                    } else {
                        None
                    }
                }) {
                    req = req.header("Authorization", format!("Bearer {tok}"));
                }
            } else {
                let tok = cred_trim.trim_start_matches("Bearer ").trim();
                req = req.header("Authorization", format!("Bearer {tok}"));
            }
        }
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求 123云盘直链失败：{e}"))?;

    let json_text = network.text(resp).await?;
    let json_val: serde_json::Value =
        serde_json::from_str(&json_text).map_err(|_| "123云盘直链接口返回无效 JSON".to_owned())?;

    let code = json_val.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code == 5112 {
        return Err("123云盘提示：该分享设置了【仅限登录或付费用户下载】。如需下载受限分享，请在【设置 -> 平台凭证管理】中添加 123云盘 Token/Cookie，或测试公开免登录分享。".to_string());
    }

    if code != 0 {
        return Err(format!(
            "123云盘获取直链失败（代码 {code}），请检查分享权限和登录状态"
        ));
    }

    let dlink = json_val
        .get("data")
        .and_then(|d| d.get("DownloadURL"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "123云盘未返回 DownloadURL 直链".to_string())?
        .to_string();

    let mut headers = HashMap::new();
    headers.insert("User-Agent".to_string(), PAN123_USER_AGENT.to_string());
    headers.insert(
        "Referer".to_string(),
        format!("https://www.123pan.com/s/{}", share_key),
    );

    Ok(Pan123DirectUrlResult {
        url: dlink,
        headers,
    })
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pan123_url() {
        let p1 = parse_pan123_url("https://www.123pan.com/s/Abcd-Efgh.html").unwrap();
        assert_eq!(p1.host, "www.123pan.com");
        assert_eq!(p1.share_key, "Abcd-Efgh");

        let p2 = parse_pan123_url("https://1683912.share.123pan.cn/123pan/z3h9-rtFzh?notoken=1")
            .unwrap();
        assert_eq!(p2.host, "1683912.share.123pan.cn");
        assert_eq!(p2.share_key, "z3h9-rtFzh");
    }
}
