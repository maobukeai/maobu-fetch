//! 123云盘 (123pan) 公开分享解析与直链获取模块。
//!
//! 支持 `123pan.com/s/xxxx`、`123pan.cn`、`123684.com`、`*.share.123pan.cn/123pan/xxxx` 等分享链接，
//! 支持公开分享免登录解析、多层目录递归展开与 32 线程满载多并发下载。

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

pub const PAN123_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub const PAN123_API_BASE: &str = "https://www.123684.com/api";

static RE_PAN123_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)https?://(?:[a-zA-Z0-9-]+\.)?(?:123pan\.(?:com|cn)|123684\.com|123952\.com)(?:/s/|/123pan/|/)([a-zA-Z0-9_-]+)(?:\.html)?(?:\?[^\s#]*)?")
        .expect("RE_PAN123_URL regex compile")
});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pan123FileItem {
    pub id: i64,
    pub name: String,
    pub size: u64,
    pub size_formatted: String,
    pub etag: String,
    pub s3_key_flag: String,
    pub kind: String, // "file" | "folder"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pan123ShareInfo {
    pub share_key: String,
    pub share_url: String,
    pub title: String,
    pub is_folder: bool,
    pub requires_password: bool,
    pub files: Vec<Pan123FileItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pan123DirectUrlResult {
    pub url: String,
    pub headers: HashMap<String, String>,
}

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
    let parsed = parse_pan123_url(share_url)
        .ok_or_else(|| "无法识别的 123云盘分享链接".to_string())?;
    let effective_pwd = pass_code.map(|s| s.to_string()).or(parsed.pass_code.clone());

    let client = reqwest::Client::builder()
        .user_agent(PAN123_USER_AGENT)
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;

    let pwd_param = effective_pwd.as_deref().unwrap_or("");
    let mut files = Vec::new();
    let mut title = format!("123云盘分享 - {}", parsed.share_key);
    let mut is_folder = false;

    // 递归获取分享内的文件列表（支持嵌套子目录）
    let mut folder_queue = vec![0i64];
    let mut visited_folders = std::collections::HashSet::new();

    while let Some(parent_id) = folder_queue.pop() {
        if !visited_folders.insert(parent_id) {
            continue;
        }

        let list_url = format!(
            "{}/share/get?limit=100&next=0&page=1&orderBy=share_id&orderDirection=desc&shareKey={}&SharePwd={}&parentFileId={}",
            PAN123_API_BASE,
            parsed.share_key,
            pwd_param,
            parent_id
        );

        let resp = client
            .get(&list_url)
            .header("Referer", share_url)
            .header("Platform", "web")
            .header("App-Version", "3")
            .send()
            .await
            .map_err(|e| format!("请求 123云盘分享失败：{e}"))?;

        let json_text = resp.text().await.unwrap_or_default();
        let json_val: serde_json::Value = serde_json::from_str(&json_text)
            .map_err(|_| format!("123云盘接口回执异常：{json_text}"))?;

        let code = json_val.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
        if code == 5103 || code == 50001 || code == 50002 || code == 401 || json_text.contains("提取码") || json_text.contains("密码") {
            return Ok(Pan123ShareInfo {
                share_key: parsed.share_key.clone(),
                share_url: share_url.to_string(),
                title: format!("123云盘加密分享 - {}", parsed.share_key),
                is_folder: false,
                requires_password: true,
                files: Vec::new(),
            });
        }

        if code != 0 {
            let msg = json_val.get("message").and_then(|v| v.as_str()).unwrap_or("未知错误");
            return Err(format!("123云盘解析失败：{msg}"));
        }

        if let Some(data) = json_val.get("data") {
            if let Some(list) = data.get("InfoList").and_then(|v| v.as_array()) {
                for item in list {
                    let file_id = item.get("FileId").and_then(|v| v.as_i64()).unwrap_or(0);
                    let name = item.get("FileName").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let size = item.get("Size").and_then(|v| v.as_u64()).unwrap_or(0);
                    let etag = item.get("Etag").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let s3_key_flag = item.get("S3KeyFlag").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let f_type = item.get("Type").and_then(|v| v.as_i64()).unwrap_or(0);

                    if f_type == 1 {
                        // 文件夹 -> 递归排队拉取其子文件
                        is_folder = true;
                        if !name.is_empty() && title.starts_with("123云盘分享") {
                            title = name.clone();
                        }
                        folder_queue.push(file_id);
                    } else {
                        // 真实文件
                        let size_formatted = format_bytes(size);
                        files.push(Pan123FileItem {
                            id: file_id,
                            name,
                            size,
                            size_formatted,
                            etag,
                            s3_key_flag,
                            kind: "file".to_string(),
                        });
                    }
                }
            }
        }
    }

    if files.len() > 1 {
        is_folder = true;
    }

    Ok(Pan123ShareInfo {
        share_key: parsed.share_key,
        share_url: share_url.to_string(),
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
    let client = reqwest::Client::builder()
        .user_agent(PAN123_USER_AGENT)
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;

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
            if cred_trim.contains('=') && !cred_trim.starts_with("Bearer") {
                req = req.header("Cookie", cred_trim);
                if let Some(tok) = cred_trim.split(';').find_map(|p| {
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

    let json_text = resp.text().await.unwrap_or_default();
    let json_val: serde_json::Value = serde_json::from_str(&json_text)
        .map_err(|_| format!("123云盘直链接口回执异常：{json_text}"))?;

    let code = json_val.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code == 5112 {
        return Err("123云盘提示：该分享设置了【仅限登录或付费用户下载】。如需下载受限分享，请在【设置 -> 平台凭证管理】中添加 123云盘 Token/Cookie，或测试公开免登录分享。".to_string());
    }

    if code != 0 {
        let msg = json_val.get("message").and_then(|v| v.as_str()).unwrap_or("未知错误");
        return Err(format!("123云盘获取直链失败：{msg}"));
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

        let p2 = parse_pan123_url("https://1683912.share.123pan.cn/123pan/z3h9-rtFzh?notoken=1").unwrap();
        assert_eq!(p2.host, "1683912.share.123pan.cn");
        assert_eq!(p2.share_key, "z3h9-rtFzh");
    }
}
