//! PikPak 网盘免登录分享解析与直链获取模块。
//!
//! 遵循本地优先、无外部额外进程、安全合规原则：
//! 1. 运行在 Rust 原生后端，彻底规避 WebView CORS 跨域限制；
//! 2. 纯客户端直连 PikPak 官方 API，免登录获取公开或加密分享的文件树；
//! 3. 解析出的文件下载直链直连猫步下载器 HTTP Range 多连接内核。

use md5::{Digest, Md5};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const PIKPAK_CLIENT_ID: &str = "YNxT9w7GMdWvEOKa";
pub const PIKPAK_CLIENT_VERSION: &str = "1.0.0";
pub const PIKPAK_PACKAGE_NAME: &str = "mypikpak.com";
pub const PIKPAK_API_HOST: &str = "https://api-drive.mypikpak.com";
pub const PIKPAK_USER_HOST: &str = "https://user.mypikpak.com";

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn get_http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .build()
            .unwrap_or_default()
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PikPakFileItem {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub size: u64,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_extension: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_content_link: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PikPakShareInfo {
    pub share_id: String,
    pub title: String,
    pub files: Vec<PikPakFileItem>,
    pub total_size: u64,
    pub file_count: usize,
    pub folder_count: usize,
    pub pass_code_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pass_code_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PikPakDirectUrlResult {
    pub url: String,
    pub headers: HashMap<String, String>,
}

/// 解析 PikPak 分享 URL 结构
pub struct ParsedPikPakUrl {
    pub share_id: String,
    pub parent_id: Option<String>,
    pub pass_code: Option<String>,
}

pub fn parse_pikpak_url(raw: &str) -> Option<ParsedPikPakUrl> {
    let text = raw.trim();
    let re = Regex::new(
        r"(?i)https?://(?:[a-zA-Z0-9-]+\.)?mypikpak\.(?:com|net)/s/([a-zA-Z0-9_-]+)(?:/([a-zA-Z0-9_-]+))?(?:\?[^\s#]*)?",
    ).ok()?;

    let caps = re.captures(text)?;
    let share_id = caps.get(1)?.as_str().to_string();
    let parent_id = caps.get(2).map(|m| m.as_str().to_string());

    // 提取密码（URL query 或 文本提取码）
    let mut pass_code = None;
    if let Ok(url) = url::Url::parse(caps.get(0)?.as_str()) {
        for (k, v) in url.query_pairs() {
            if k == "pwd" || k == "pass_code" || k == "code" {
                pass_code = Some(v.trim().to_string());
                break;
            }
        }
    }

    if pass_code.is_none() {
        let pwd_re = Regex::new(r"(?i)(?:提取码|密码|pwd|code)[:：\s]+([a-zA-Z0-9]{4,8})").ok();
        if let Some(re) = pwd_re {
            if let Some(m) = re.captures(text) {
                if let Some(c) = m.get(1) {
                    pass_code = Some(c.as_str().trim().to_string());
                }
            }
        }
    }

    Some(ParsedPikPakUrl {
        share_id,
        parent_id,
        pass_code,
    })
}

/// 获取/生成 Captcha Token 与 Device Sign
pub async fn get_captcha_and_sign(device_id: &str) -> Result<(String, String), String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();

    let timestamp_str = now.to_string();
    let salt = "l-sark";
    let raw_sign = format!(
        "{}{}{}{}{}{}",
        PIKPAK_CLIENT_ID, PIKPAK_CLIENT_VERSION, PIKPAK_PACKAGE_NAME, device_id, timestamp_str, salt
    );

    let mut hasher = Md5::new();
    hasher.update(raw_sign.as_bytes());
    let sign_hex = hex::encode(hasher.finalize());
    let captcha_sign = format!("1.{}", sign_hex);

    let client = get_http_client();
    let payload = serde_json::json!({
        "client_id": PIKPAK_CLIENT_ID,
        "device_id": device_id,
        "client_version": PIKPAK_CLIENT_VERSION,
        "package_name": PIKPAK_PACKAGE_NAME,
        "timestamp": now,
        "captcha_sign": captcha_sign,
        "action": "GET:/drive/v1/share/detail",
        "meta": {
            "phone_model": "Chrome/120.0.0.0"
        }
    });

    let resp = client
        .post(format!("{}/v1/shield/captcha/init", PIKPAK_USER_HOST))
        .header("Content-Type", "application/json")
        .body(payload.to_string())
        .send()
        .await
        .map_err(|e| format!("连接 PikPak 验证服务失败: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("PikPak 验证服务返回异常: {}", resp.status()));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| format!("读取 PikPak 验证响应失败: {}", e))?;

    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("解析 PikPak 验证响应失败: {}", e))?;

    let token = json
        .get("captcha_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            json.get("error_description")
                .and_then(|v| v.as_str())
                .unwrap_or("未能获取有效的 PikPak 验证令牌")
                .to_string()
        })?;

    Ok((token.to_string(), captcha_sign))
}

/// 验证提取码并换取 pass_code_token
pub async fn verify_pass_code(
    share_id: &str,
    pass_code: &str,
    captcha_token: &str,
    device_sign: &str,
    device_id: &str,
) -> Result<String, String> {
    let client = get_http_client();
    let payload = serde_json::json!({
        "share_id": share_id,
        "pass_code": pass_code,
    });

    let resp = client
        .post(format!("{}/drive/v1/share/pass_code", PIKPAK_API_HOST))
        .header("Content-Type", "application/json")
        .header("X-Client-Id", PIKPAK_CLIENT_ID)
        .header("X-Client-Version", PIKPAK_CLIENT_VERSION)
        .header("X-Device-Id", device_id)
        .header("X-Device-Sign", device_sign)
        .header("X-Captcha-Token", captcha_token)
        .header("Referer", "https://mypikpak.com/")
        .body(payload.to_string())
        .send()
        .await
        .map_err(|e| format!("验证提取码请求失败: {}", e))?;

    if resp.status().as_u16() == 400 || resp.status().as_u16() == 401 {
        return Err("提取码错误，请重新输入".into());
    }

    if !resp.status().is_success() {
        return Err(format!("验证提取码失败 ({})", resp.status()));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| format!("读取提取码响应失败: {}", e))?;

    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("解析提取码响应失败: {}", e))?;

    let token = json
        .get("pass_code_token")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    Ok(token.to_string())
}

/// 队列迭代抓取分享目录树（支持多层级子目录与分页）
async fn fetch_directory_tree(
    share_id: &str,
    initial_parent_id: &str,
    pass_code_token: &mut String,
    captcha_token: &str,
    device_sign: &str,
    device_id: &str,
) -> Result<Vec<PikPakFileItem>, String> {
    let client = get_http_client();
    let mut results = Vec::new();
    let mut queue = VecDeque::new();
    queue.push_back((initial_parent_id.to_string(), String::new()));

    while let Some((parent_id, current_path)) = queue.pop_front() {
        let mut page_token: Option<String> = None;
        loop {
            let mut query = vec![("share_id", share_id.to_string()), ("limit", "100".to_string())];
            if !parent_id.is_empty() {
                query.push(("parent_id", parent_id.clone()));
            }
            if let Some(ref pt) = page_token {
                query.push(("page_token", pt.clone()));
            }
            if !pass_code_token.is_empty() {
                query.push(("pass_code_token", pass_code_token.clone()));
            }

            let resp = client
                .get(format!("{}/drive/v1/share/detail", PIKPAK_API_HOST))
                .query(&query)
                .header("X-Client-Id", PIKPAK_CLIENT_ID)
                .header("X-Client-Version", PIKPAK_CLIENT_VERSION)
                .header("X-Device-Id", device_id)
                .header("X-Device-Sign", device_sign)
                .header("X-Captcha-Token", captcha_token)
                .header("Referer", "https://mypikpak.com/")
                .send()
                .await
                .map_err(|e| format!("拉取分享内容失败: {}", e))?;

            if resp.status().as_u16() == 403 {
                return Err("NEED_PASS_CODE".into());
            }

            let text = resp
                .text()
                .await
                .map_err(|e| format!("读取分享列表响应失败: {}", e))?;

            let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();

            if json.get("error").and_then(|v| v.as_str()) == Some("need_pass_code") {
                return Err("NEED_PASS_CODE".into());
            }
            if let Some(desc) = json.get("error_description").and_then(|v| v.as_str()) {
                return Err(desc.to_string());
            }

            if pass_code_token.is_empty() {
                if let Some(token) = json.get("pass_code_token").and_then(|v| v.as_str()) {
                    *pass_code_token = token.to_string();
                }
            }

            let files = json
                .get("files")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            for item in files {
                let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let kind = item.get("kind").and_then(|v| v.as_str()).unwrap_or("drive#file").to_string();
                let size = item.get("size").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                let mime_type = item.get("mime_type").and_then(|v| v.as_str()).map(|s| s.to_string());
                let file_extension = item.get("file_extension").and_then(|v| v.as_str()).map(|s| s.to_string());
                let thumbnail_url = item.get("thumbnail_link").or_else(|| item.get("icon_link")).and_then(|v| v.as_str()).map(|s| s.to_string());
                let web_content_link = item.get("web_content_link").and_then(|v| v.as_str()).map(|s| s.to_string());

                let item_path = if current_path.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", current_path, name)
                };

                let is_folder = kind == "drive#folder";

                results.push(PikPakFileItem {
                    id: id.clone(),
                    name: name.clone(),
                    kind,
                    size,
                    path: item_path.clone(),
                    mime_type,
                    file_extension,
                    thumbnail_url,
                    web_content_link,
                });

                if is_folder {
                    queue.push_back((id, item_path));
                }
            }

            page_token = json
                .get("next_page_token")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            if page_token.is_none() || page_token.as_deref() == Some("") {
                break;
            }
        }
    }

    Ok(results)
}

/// 解析分享链接
pub async fn inspect_pikpak_share(
    raw_url: &str,
    provided_pass_code: Option<String>,
    device_id: &str,
) -> Result<PikPakShareInfo, String> {
    let parsed = parse_pikpak_url(raw_url).ok_or_else(|| {
        "无效的 PikPak 分享链接，格式应为 https://mypikpak.com/s/xxxx".to_string()
    })?;

    let (captcha_token, device_sign) = get_captcha_and_sign(device_id).await?;
    let mut pass_code_token = String::new();

    let effective_pass_code = provided_pass_code.or(parsed.pass_code);
    if let Some(ref pwd) = effective_pass_code {
        match verify_pass_code(&parsed.share_id, pwd, &captcha_token, &device_sign, device_id).await {
            Ok(token) => pass_code_token = token,
            Err(e) => {
                if e.contains("提取码错误") {
                    return Err(e);
                }
            }
        }
    }

    match fetch_directory_tree(
        &parsed.share_id,
        parsed.parent_id.as_deref().unwrap_or(""),
        &mut pass_code_token,
        &captcha_token,
        &device_sign,
        device_id,
    )
    .await
    {
        Ok(all_items) => {
            let file_count = all_items.iter().filter(|i| i.kind == "drive#file").count();
            let folder_count = all_items.iter().filter(|i| i.kind == "drive#folder").count();
            let total_size: u64 = all_items
                .iter()
                .filter(|i| i.kind == "drive#file")
                .map(|f| f.size)
                .sum();

            let title = if let Some(first) = all_items.iter().find(|i| i.kind == "drive#file") {
                first.name.clone()
            } else {
                "PikPak 分享资源".to_string()
            };

            Ok(PikPakShareInfo {
                share_id: parsed.share_id,
                title,
                files: all_items,
                total_size,
                file_count,
                folder_count,
                pass_code_required: false,
                pass_code_token: if pass_code_token.is_empty() {
                    None
                } else {
                    Some(pass_code_token)
                },
            })
        }
        Err(e) if e == "NEED_PASS_CODE" => Ok(PikPakShareInfo {
            share_id: parsed.share_id,
            title: "加密分享链接".into(),
            files: Vec::new(),
            total_size: 0,
            file_count: 0,
            folder_count: 0,
            pass_code_required: true,
            pass_code_token: None,
        }),
        Err(e) => Err(e),
    }
}

/// 解析单文件下载直链
pub async fn resolve_pikpak_file(
    share_id: &str,
    file_id: &str,
    pass_code_token: Option<&str>,
    device_id: &str,
) -> Result<PikPakDirectUrlResult, String> {
    let (captcha_token, device_sign) = get_captcha_and_sign(device_id).await?;
    let client = get_http_client();

    let mut query = vec![("share_id", share_id), ("file_id", file_id)];
    if let Some(token) = pass_code_token {
        if !token.is_empty() {
            query.push(("pass_code_token", token));
        }
    }

    let resp = client
        .get(format!("{}/drive/v1/share/file_info", PIKPAK_API_HOST))
        .query(&query)
        .header("X-Client-Id", PIKPAK_CLIENT_ID)
        .header("X-Client-Version", PIKPAK_CLIENT_VERSION)
        .header("X-Device-Id", device_id)
        .header("X-Device-Sign", &device_sign)
        .header("X-Captcha-Token", &captcha_token)
        .header("Referer", "https://mypikpak.com/")
        .send()
        .await
        .map_err(|e| format!("获取文件直链失败: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("PikPak 直链接口返回错误: {}", resp.status()));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| format!("读取直链响应失败: {}", e))?;

    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("解析直链响应失败: {}", e))?;

    let file_info = json.get("file_info").or_else(|| json.get("file"));

    let direct_url = file_info
        .and_then(|f| {
            f.get("medias")
                .and_then(|m| m.as_array())
                .and_then(|arr| arr.first())
                .and_then(|first| first.get("link"))
                .and_then(|link| link.get("url"))
                .and_then(|u| u.as_str())
                .map(|s| s.to_string())
                .or_else(|| {
                    f.get("web_content_link")
                        .and_then(|w| w.as_str())
                        .map(|s| s.to_string())
                })
        })
        .or_else(|| {
            json.get("web_content_link")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .ok_or_else(|| "PikPak 未返回该文件的有效下载直链".to_string())?;

    let mut headers = HashMap::new();
    headers.insert(
        "User-Agent".to_string(),
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36".to_string(),
    );
    headers.insert("Referer".to_string(), "https://mypikpak.com/".to_string());

    Ok(PikPakDirectUrlResult {
        url: direct_url,
        headers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_inspect_real_pikpak_share() {
        let url = "https://mypikpak.com/s/VOveL7ZI01ViAz9VVKGgSWDlo2";
        let device_id = hex::encode(rand::random::<[u8; 16]>());
        let res = inspect_pikpak_share(url, None, &device_id).await;
        println!("inspect_pikpak_share result: {:?}", res);
        if let Ok(info) = &res {
            if let Some(first) = info.files.first() {
                let direct = resolve_pikpak_file(&info.share_id, &first.id, info.pass_code_token.as_deref(), &device_id).await;
                println!("resolve_pikpak_file result: {:?}", direct);
            }
        }
    }
}
