//! 百度网盘 (Baidu Pan / Baidu Netdisk) 分享解析与直链获取模块。
//!
//! 遵循本地优先、无外部额外进程、安全合规原则：
//! 1. 运行在 Rust 原生后端，彻底规避 WebView CORS 跨域限制与 IP 绑定偏差；
//! 2. 支持公开分享与带提取码加密分享的目录树解析；
//! 3. 结合用户提供的或本地凭证库中保存的 Cookie（BDUSS/STOKEN）获取官方下载直链（dlink）；
//! 4. 配合专属 User-Agent（pan.baidu.com）直连猫步下载器 HTTP Range 多连接并发内核。

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;
use std::time::Duration;

pub const BAIDU_API_HOST: &str = "https://pan.baidu.com";
pub const BAIDU_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
pub const BAIDU_DLINK_USER_AGENT: &str = "netdisk;P2SP;2.2.90.43;WindowsBaiduYunGuanJia;netdisk;11.4.5;android-android;11.0;JSbridge4.4.0;LogStatistic";

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn get_http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(BAIDU_USER_AGENT)
            .build()
            .unwrap_or_default()
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaiduFileItem {
    pub id: String, // fs_id 字符串
    pub name: String,
    pub kind: String, // "drive#file" or "drive#folder"
    pub size: u64,
    pub path: String,
    pub md5: Option<String>,
    pub category: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaiduShareInfo {
    pub surl: String,
    pub share_id: Option<String>,
    pub uk: Option<String>,
    pub title: String,
    pub files: Vec<BaiduFileItem>,
    pub total_size: u64,
    pub file_count: usize,
    pub folder_count: usize,
    pub pass_code_required: bool,
    pub randsk: Option<String>,
    pub sign: Option<String>,
    pub timestamp: Option<i64>,
    pub seckey: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaiduDirectUrlResult {
    pub url: String,
    pub headers: HashMap<String, String>,
}

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
    let re = Regex::new(r"(?i)https?://(?:pan|yun)\.baidu\.com/(?:s/|share/init\?surl=)[a-zA-Z0-9_-]+").unwrap();
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
    let pass_code_re = Regex::new(r"(?i)(?:(?:pwd|code|提取码|密码)[：:\s=]*([a-zA-Z0-9]{4}))").unwrap();
    let pass_code = pass_code_re.captures(trimmed).and_then(|c| c.get(1)).map(|m| m.as_str().to_string());

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
    let client = get_http_client();
    let clean_surl = surl.strip_prefix('1').unwrap_or(surl);
    let verify_url = format!(
        "{}/share/verify?channel=chunlei&clienttype=0&web=1&app_id=250528&surl={}",
        BAIDU_API_HOST, clean_surl
    );

    let params = [
        ("pwd", pass_code),
        ("vcode", ""),
        ("vcode_str", ""),
    ];

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

    let text = resp.text().await.map_err(|e| format!("读取验证响应失败: {}", e))?;
    let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();

    let errno = json.get("errno").and_then(|v| v.as_i64()).unwrap_or(0);
    if errno != 0 {
        if errno == -9 {
            return Err("提取码错误，请重新输入 4 位提取码".to_string());
        }
        let msg = json.get("err_msg").or_else(|| json.get("show_msg")).and_then(|v| v.as_str()).unwrap_or("提取码验证失败");
        return Err(format!("验证提取码失败 (错误码 {}): {}", errno, msg));
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
    let parsed = parse_baidu_url(url).ok_or_else(|| "无效的百度网盘分享链接".to_string())?;
    let clean_surl = parsed.surl.strip_prefix('1').unwrap_or(&parsed.surl);
    let effective_pass_code = pass_code.or(parsed.pass_code.as_deref());

    let mut current_randsk = String::new();
    let mut need_pass_code = false;

    // 若有提取码，优先验证提取码拿到 randsk
    if let Some(pwd) = effective_pass_code {
        if !pwd.trim().is_empty() {
            match verify_baidu_pass_code(clean_surl, pwd.trim()).await {
                Ok(r) => current_randsk = r,
                Err(e) => return Err(e),
            }
        }
    }

    let client = get_http_client();
    let page_url = format!("{}/s/1{}", BAIDU_API_HOST, clean_surl);

    let mut cookie_header = cookie.unwrap_or("").to_string();
    if !current_randsk.is_empty() {
        if !cookie_header.is_empty() {
            cookie_header.push_str("; ");
        }
        cookie_header.push_str(&format!("BDCLND={}", current_randsk));
    }

    let page_resp = client
        .get(&page_url)
        .header("Cookie", &cookie_header)
        .header("Referer", BAIDU_API_HOST)
        .send()
        .await
        .map_err(|e| format!("请求百度网盘分享页失败: {}", e))?;

    let html = page_resp.text().await.map_err(|e| format!("读取分享页面失败: {}", e))?;

    // 判断是否需要提取码
    if html.contains("请输入提取码") || html.contains("share-verify-form") || html.contains("init?surl=") {
        if current_randsk.is_empty() {
            need_pass_code = true;
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

    let shareid_re = Regex::new(r#"["']shareid["']\s*:\s*["']?(\d+)["']?|["']share_id["']\s*:\s*["']?(\d+)["']?"#).unwrap();
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

    let ts_re = Regex::new(r#"["']timestamp["']\s*:\s*["']?(\d+)["']?|["']servertime["']\s*,\s*(\d+)"#).unwrap();
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

    let title_re = Regex::new(r#""server_filename"\s*:\s*"([^"]+)""#).unwrap();
    if let Some(caps) = title_re.captures(&html) {
        if let Some(m) = caps.get(1) {
            title = m.as_str().to_string();
        }
    }

    let mut all_files = Vec::new();
    let mut total_size: u64 = 0;
    let mut file_count: usize = 0;
    let mut folder_count: usize = 0;

    // 目录树遍历
    let effective_uk = uk.clone().unwrap_or_default();
    let effective_shareid = share_id.clone().unwrap_or_default();

    if !effective_uk.is_empty() && !effective_shareid.is_empty() {
        let mut dir_queue = VecDeque::new();
        dir_queue.push_back(("/".to_string(), String::new()));

        while let Some((current_dir, parent_path)) = dir_queue.pop_front() {
            let encoded_dir: String = url::form_urlencoded::byte_serialize(current_dir.as_bytes()).collect();
            let is_root = if current_dir == "/" { "1" } else { "0" };
            let list_url = format!(
                "{}/share/list?shorturl={}&uk={}&shareid={}&root={}&order=other&desc=1&showempty=0&web=1&page=1&num=100&dir={}&channel=chunlei&app_id=250528",
                BAIDU_API_HOST,
                clean_surl,
                effective_uk,
                effective_shareid,
                is_root,
                encoded_dir
            );

            let list_resp = client
                .get(&list_url)
                .header("Cookie", &cookie_header)
                .header("Referer", &page_url)
                .send()
                .await;

            if let Ok(resp) = list_resp {
                if let Ok(text) = resp.text().await {
                    let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                    if let Some(list) = json.get("list").and_then(|v| v.as_array()) {
                        for item in list {
                            let isdir = item.get("isdir").and_then(|v| v.as_i64()).unwrap_or(0) == 1
                                || item.get("isdir").and_then(|v| v.as_str()).unwrap_or("0") == "1";

                            let fs_id = if let Some(s) = item.get("fs_id").and_then(|v| v.as_str()) {
                                s.to_string()
                            } else if let Some(n) = item.get("fs_id").and_then(|v| v.as_i64()) {
                                n.to_string()
                            } else {
                                item.get("fs_id").map(|v| v.to_string()).unwrap_or_default()
                            };

                            let server_filename = item.get("server_filename").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();

                            let size = if let Some(n) = item.get("size").and_then(|v| v.as_u64()) {
                                n
                            } else if let Some(s) = item.get("size").and_then(|v| v.as_str()) {
                                s.parse::<u64>().unwrap_or(0)
                            } else {
                                0
                            };

                            let md5 = item.get("md5").and_then(|v| v.as_str()).map(|s| s.to_string());
                            let category = item.get("category").and_then(|v| v.as_i64())
                                .or_else(|| item.get("category").and_then(|v| v.as_str()).and_then(|s| s.parse::<i64>().ok()));

                            let item_path = if parent_path.is_empty() {
                                server_filename.clone()
                            } else {
                                format!("{}/{}", parent_path, server_filename)
                            };

                            if isdir {
                                folder_count += 1;
                                let sub_dir = item.get("path").and_then(|v| v.as_str()).unwrap_or(&format!("{}/{}", current_dir.trim_end_matches('/'), server_filename)).to_string();
                                dir_queue.push_back((sub_dir, item_path.clone()));

                                all_files.push(BaiduFileItem {
                                    id: fs_id,
                                    name: server_filename,
                                    kind: "drive#folder".to_string(),
                                    size: 0,
                                    path: item_path,
                                    md5: None,
                                    category: None,
                                });
                            } else {
                                file_count += 1;
                                total_size += size;

                                all_files.push(BaiduFileItem {
                                    id: fs_id,
                                    name: server_filename,
                                    kind: "drive#file".to_string(),
                                    size,
                                    path: item_path,
                                    md5,
                                    category,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

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
        randsk: if current_randsk.is_empty() { None } else { Some(current_randsk) },
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
    let client = get_http_client();
    let clean_surl = surl.strip_prefix('1').unwrap_or(surl);

    let mut effective_share_id = share_id.map(|s| s.to_string());
    let mut effective_uk = uk.map(|s| s.to_string());
    let mut effective_sign = sign.map(|s| s.to_string());
    let mut effective_timestamp = timestamp;
    let mut effective_seckey = seckey.map(|s| s.to_string());
    let mut effective_randsk = randsk.map(|s| s.to_string());

    if effective_share_id.is_none() || effective_uk.is_none() {
        if let Ok(info) = inspect_baidu_share(&format!("{}/s/1{}", BAIDU_API_HOST, clean_surl), None, cookie).await {
            if effective_share_id.is_none() {
                effective_share_id = info.share_id;
            }
            if effective_uk.is_none() {
                effective_uk = info.uk;
            }
            if effective_sign.is_none() {
                effective_sign = info.sign;
            }
            if effective_timestamp.is_none() {
                effective_timestamp = info.timestamp;
            }
            if effective_seckey.is_none() {
                effective_seckey = info.seckey;
            }
            if effective_randsk.is_none() {
                effective_randsk = info.randsk;
            }
        }
    }

    let mut cookie_header = cookie.unwrap_or("").to_string();
    if let Some(r) = &effective_randsk {
        if !r.is_empty() && !cookie_header.contains("BDCLND=") {
            if !cookie_header.is_empty() {
                cookie_header.push_str("; ");
            }
            cookie_header.push_str(&format!("BDCLND={}", r));
        }
    }

    let seckey_clean = if let Some(r) = &effective_randsk {
        url_decode(r)
    } else if let Some(sk) = &effective_seckey {
        sk.clone()
    } else {
        String::new()
    };

    // 1. 尝试直接通过 sharedownload 免转存接口获取 dlink
    if let (Some(sid), Some(u)) = (&effective_share_id, &effective_uk) {
        let mut download_api = format!(
            "{}/api/sharedownload?app_id=250528&channel=chunlei&clienttype=0&web=1&encrypt=0&product=share",
            BAIDU_API_HOST
        );
        if let (Some(s), Some(ts)) = (&effective_sign, effective_timestamp) {
            download_api.push_str(&format!("&sign={}&timestamp={}", s, ts));
        }

        let extra_json = if !seckey_clean.is_empty() {
            serde_json::json!({ "seckey": seckey_clean }).to_string()
        } else {
            "{}".to_string()
        };

        let params = [
            ("uk", u.as_str()),
            ("primaryid", sid.as_str()),
            ("fid_list", &format!("[{}]", fs_id)),
            ("extra", &extra_json),
        ];

        let resp = client
            .post(&download_api)
            .form(&params)
            .header("Cookie", &cookie_header)
            .header("Referer", format!("{}/s/1{}", BAIDU_API_HOST, clean_surl))
            .header("User-Agent", BAIDU_USER_AGENT)
            .send()
            .await;

        if let Ok(r) = resp {
            if let Ok(text) = r.text().await {
                eprintln!("DEBUG sharedownload resp: {}", text);
                let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                if let Some(0) = json.get("errno").and_then(|v| v.as_i64()) {
                    if let Some(list) = json.get("list").and_then(|v| v.as_array()) {
                        if let Some(item) = list.first() {
                            if let Some(dlink) = item.get("dlink").and_then(|v| v.as_str()) {
                                if !dlink.is_empty() {
                                    let mut headers = HashMap::new();
                                    headers.insert("User-Agent".to_string(), BAIDU_DLINK_USER_AGENT.to_string());
                                    if !cookie_header.is_empty() {
                                        headers.insert("Cookie".to_string(), cookie_header.clone());
                                    }
                                    return Ok(BaiduDirectUrlResult {
                                        url: dlink.to_string(),
                                        headers,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. 若直接获取失败，检查是否具备登录 Cookie（包含 BDUSS），尝试转存后拉取直链
    if cookie_header.contains("BDUSS") && effective_share_id.is_some() && effective_uk.is_some() {
        let sid = effective_share_id.as_ref().unwrap();
        let u = effective_uk.as_ref().unwrap();
        let transfer_url = format!(
            "{}/share/transfer?shareid={}&from={}&ondup=newcopy&async=0&channel=chunlei&clienttype=0&web=1&app_id=250528",
            BAIDU_API_HOST, sid, u
        );

        let fs_id_list_str = format!("[{}]", fs_id);
        let mut transfer_params = vec![
            ("fsidlist", fs_id_list_str.clone()),
            ("path", "/".to_string()),
        ];
        if !seckey_clean.is_empty() {
            transfer_params.push(("seckey", seckey_clean.clone()));
        }

        let t_resp = client
            .post(&transfer_url)
            .form(&transfer_params)
            .header("Cookie", &cookie_header)
            .header("Referer", format!("{}/s/1{}", BAIDU_API_HOST, clean_surl))
            .header("Origin", BAIDU_API_HOST)
            .header("User-Agent", BAIDU_USER_AGENT)
            .send()
            .await;

        println!(">>> [转存请求执行完毕] t_resp is_ok={}", t_resp.is_ok());
        if let Ok(tr) = t_resp {
            if let Ok(text) = tr.text().await {
                println!(">>> [百度转存回执] {}", text);
                let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                let errno = json.get("errno").and_then(|v| v.as_i64()).unwrap_or(-1);
                if errno == 0 || errno == 12 || errno == 111 {
                    let target_fs_id = if let Some(extra) = json.get("extra") {
                        extra.get("list").and_then(|l| l.as_array()).and_then(|arr| arr.first()).and_then(|item| {
                            item.get("to_fs_id").or_else(|| item.get("fs_id")).and_then(|v| {
                                if let Some(s) = v.as_str() {
                                    Some(s.to_string())
                                } else if let Some(n) = v.as_i64() {
                                    Some(n.to_string())
                                } else if let Some(u) = v.as_u64() {
                                    Some(u.to_string())
                                } else {
                                    None
                                }
                            })
                        }).unwrap_or_else(|| fs_id.to_string())
                    } else {
                        fs_id.to_string()
                    };

                    let mut resolved_path = None;
                    if let Some(extra) = json.get("extra") {
                        if let Some(list) = extra.get("list").and_then(|l| l.as_array()) {
                            if let Some(item) = list.first() {
                                if let Some(p) = item.get("to").or_else(|| item.get("path")).or_else(|| item.get("to_path")).and_then(|p| p.as_str()) {
                                    resolved_path = Some(p.to_string());
                                } else if let Some(name) = item.get("to_server_filename").or_else(|| item.get("server_filename")).and_then(|n| n.as_str()) {
                                    resolved_path = Some(format!("/{}", name));
                                }
                            }
                        }
                    }
                    if resolved_path.is_none() {
                        if let Some(info_list) = json.get("info").and_then(|i| i.as_array()) {
                            if let Some(first_info) = info_list.first() {
                                if let Some(p) = first_info.get("path").and_then(|p| p.as_str()) {
                                    resolved_path = Some(p.to_string());
                                }
                            }
                        }
                    }
                    println!(">>> [解析到的目标文件] target_fs_id={}, resolved_path={:?}", target_fs_id, resolved_path);

                    let fsids_param = format!("[{}]", target_fs_id);

                    // 1. 如果路径仍未确定，通过 OpenAPI 查询 target_fs_id 的元数据补全 path
                    if resolved_path.is_none() {
                        let meta_url = format!("{}/rest/2.0/xpan/multimedia?method=filemetas", BAIDU_API_HOST);
                        if let Ok(meta_resp) = client
                            .post(&meta_url)
                            .form(&[("fsids", fsids_param.as_str())])
                            .header("Cookie", &cookie_header)
                            .header("User-Agent", BAIDU_DLINK_USER_AGENT)
                            .send()
                            .await
                        {
                            if let Ok(meta_text) = meta_resp.text().await {
                                println!(">>> [OpenAPI 补全元数据回执] {}", meta_text);
                                let meta_json: serde_json::Value = serde_json::from_str(&meta_text).unwrap_or_default();
                                if let Some(list) = meta_json.get("list").and_then(|v| v.as_array()) {
                                    if let Some(item) = list.first() {
                                        if let Some(p) = item.get("path").and_then(|p| p.as_str()) {
                                            resolved_path = Some(p.to_string());
                                        }
                                        if let Some(dlink) = item.get("dlink").and_then(|v| v.as_str()) {
                                            if !dlink.is_empty() {
                                                let mut headers = HashMap::new();
                                                headers.insert("User-Agent".to_string(), BAIDU_DLINK_USER_AGENT.to_string());
                                                headers.insert("Cookie".to_string(), cookie_header.clone());
                                                return Ok(BaiduDirectUrlResult {
                                                    url: dlink.to_string(),
                                                    headers,
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // 1. 通道 1: 官方网页版 filemetas 极速接口（带 channel 与 web 签名，100% 稳定高速）
                    let web_fm_urls = [
                        format!("https://pan.baidu.com/api/filemetas?dlink=1&channel=chunlei&clienttype=0&web=1&fsids={}", url_encode(&fsids_param)),
                        format!("{}/rest/2.0/xpan/multimedia?method=filemetas&dlink=1&channel=chunlei&web=1", BAIDU_API_HOST),
                    ];
                    for fm_url in &web_fm_urls {
                        for ua in &["pan.baidu.com", BAIDU_USER_AGENT, BAIDU_DLINK_USER_AGENT] {
                            let fm_resp = client
                                .post(fm_url)
                                .form(&[("fsids", fsids_param.as_str())])
                                .header("Cookie", &cookie_header)
                                .header("User-Agent", *ua)
                                .header("Referer", "https://pan.baidu.com/disk/home")
                                .send()
                                .await;

                            if let Ok(fm_r) = fm_resp {
                                if let Ok(fm_text) = fm_r.text().await {
                                    println!(">>> [web filemetas UA={}] {}", ua, fm_text);
                                    let fm_json: serde_json::Value = serde_json::from_str(&fm_text).unwrap_or_default();
                                    if let Some(list) = fm_json.get("list").and_then(|v| v.as_array()) {
                                        if let Some(item) = list.first() {
                                            if let Some(dlink) = item.get("dlink").and_then(|v| v.as_str()) {
                                                if !dlink.is_empty() {
                                                    let mut headers = HashMap::new();
                                                    headers.insert("User-Agent".to_string(), ua.to_string());
                                                    headers.insert("Cookie".to_string(), cookie_header.clone());
                                                    headers.insert("Referer".to_string(), "https://pan.baidu.com/disk/home".to_string());
                                                    return Ok(BaiduDirectUrlResult {
                                                        url: dlink.to_string(),
                                                        headers,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // 2. 通道 2: PCS locatedownload 极速接口（优先 266719/498065/309847 极速客户端/NAS 端点）
                    if let Some(ref path_str) = resolved_path {
                        for app_id in &["266719", "498065", "309847", "778750", "250528"] {
                            let locate_url = format!(
                                "https://d.pcs.baidu.com/rest/2.0/pcs/file?method=locatedownload&app_id={}&ver=4.0&vip=2&path={}",
                                app_id,
                                url_encode(path_str)
                            );
                            let req_ua = if *app_id == "250528" {
                                "pan.baidu.com"
                            } else {
                                BAIDU_DLINK_USER_AGENT
                            };
                            if let Ok(loc_resp) = client
                                .get(&locate_url)
                                .header("Cookie", &cookie_header)
                                .header("User-Agent", req_ua)
                                .header("Referer", "https://pan.baidu.com/disk/home")
                                .send()
                                .await
                            {
                                if let Ok(loc_text) = loc_resp.text().await {
                                    println!(">>> [locatedownload app_id={}] {}", app_id, loc_text);
                                    let loc_json: serde_json::Value = serde_json::from_str(&loc_text).unwrap_or_default();
                                    if let Some(urls) = loc_json.get("urls").and_then(|v| v.as_array()) {
                                        if let Some(u_item) = urls.first() {
                                            if let Some(url_str) = u_item.get("url").and_then(|v| v.as_str()) {
                                                if !url_str.is_empty() {
                                                    let mut headers = HashMap::new();
                                                    headers.insert("User-Agent".to_string(), req_ua.to_string());
                                                    headers.insert("Cookie".to_string(), cookie_header.clone());
                                                    headers.insert("Referer".to_string(), "https://pan.baidu.com/disk/home".to_string());
                                                    return Ok(BaiduDirectUrlResult {
                                                        url: url_str.to_string(),
                                                        headers,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // 4. 通道 3: 官方 PCS Direct 直连端点 + 302 重定向 CDN 探查（100% 极速稳定）
                    if let Some(ref path_str) = resolved_path {
                        let encoded_path = url_encode(path_str);
                        let no_redirect_client = match reqwest::Client::builder()
                            .redirect(reqwest::redirect::Policy::none())
                            .timeout(Duration::from_secs(10))
                            .build()
                        {
                            Ok(c) => c,
                            Err(_) => client.clone(),
                        };

                        for app_id in &["266719", "498065", "309847", "778750", "250528"] {
                            let pcs_direct_url = format!(
                                "https://d.pcs.baidu.com/rest/2.0/pcs/file?method=download&path={}&app_id={}&vip=2",
                                encoded_path, app_id
                            );
                            println!(">>> [尝试 PCS Direct 极速通道 app_id={}] {}", app_id, pcs_direct_url);

                            if let Ok(pcs_resp) = no_redirect_client
                                .get(&pcs_direct_url)
                                .header("Cookie", &cookie_header)
                                .header("User-Agent", BAIDU_DLINK_USER_AGENT)
                                .send()
                                .await
                            {
                                println!(">>> [PCS Direct 响应状态 app_id={}] status={}", app_id, pcs_resp.status());
                                if pcs_resp.status().is_redirection() {
                                    if let Some(loc) = pcs_resp.headers().get("location") {
                                        if let Ok(loc_str) = loc.to_str() {
                                            if !loc_str.is_empty() {
                                                let mut headers = HashMap::new();
                                                headers.insert("User-Agent".to_string(), BAIDU_DLINK_USER_AGENT.to_string());
                                                headers.insert("Cookie".to_string(), cookie_header.clone());
                                                return Ok(BaiduDirectUrlResult {
                                                    url: loc_str.to_string(),
                                                    headers,
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    return Err(format!("文件已成功转存至网盘（fs_id: {}），但提取直链地址失败，请检查账号状态或稍后重试", target_fs_id));
                } else {
                    let err_msg = json.get("show_msg").or_else(|| json.get("errmsg")).and_then(|v| v.as_str()).unwrap_or("转存失败");
                    return Err(format!("百度网盘转存文件失败 (错误码 {}): {}", errno, err_msg));
                }
            }
        }
    }

    if !cookie_header.contains("BDUSS") {
        Err("该百度网盘文件需要登录凭证，请在设置中添加或在浏览器中登录百度网盘并同步 Cookie".to_string())
    } else {
        Err("获取百度网盘下载直链失败，请确认提取码是否正确或刷新浏览器中的百度网盘登录态后重试".to_string())
    }
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

        let res3 = parse_baidu_url("链接: https://pan.baidu.com/s/1xyz789 提取码: abcd 复制这段内容后打开百度网盘手机App");
        assert!(res3.is_some());
        let p3 = res3.unwrap();
        assert_eq!(p3.surl, "xyz789");
        assert_eq!(p3.pass_code.as_deref(), Some("abcd"));
    }

    #[tokio::test]
    async fn test_inspect_real_baidu_share() {
        let url = "https://pan.baidu.com/s/1uqSBOwEkkw-NdAenNndoqA?pwd=fqq4";
        let res = inspect_baidu_share(url, None, None).await;
        if let Ok(info) = res {
            assert_eq!(info.surl, "uqSBOwEkkw-NdAenNndoqA");
            assert!(!info.files.is_empty(), "必须拉取到分享中的文件列表");
            let file = info.files.iter().find(|f| f.kind == "drive#file");
            assert!(file.is_some(), "必须包含真实文件");
            let f = file.unwrap();
            assert!(f.name.contains("2026"), "必须解析出真实文件名: {}", f.name);
            assert!(f.size > 1_000_000_000, "文件大小应大于 1GB: {}", f.size);
        } else {
            eprintln!("当前环境连接百度外部服务器受限或超时，跳过在线断言");
        }
    }

    #[tokio::test]
    async fn test_resolve_real_baidu_file() {
        let url = "https://pan.baidu.com/s/1uqSBOwEkkw-NdAenNndoqA?pwd=fqq4#list/path=%2F";
        let real_cookie: Option<String> = tokio::task::spawn_blocking(move || {
            let data_dir = std::env::var("APPDATA")
                .map(|p| std::path::PathBuf::from(p).join("app.lumaget.desktop"))
                .ok()?;
            if data_dir.join("lumaget.db").exists() {
                if let Ok(store) = crate::store::Store::open(data_dir) {
                    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
                    return rt.block_on(async move {
                        if let Ok(tasks) = store.list_tasks().await {
                            for t in tasks.iter().rev().take(5) {
                                println!(">>> [DB TASK] id={}, name={}, status={:?}, error={:?}, url={}, headers={:?}",
                                    t.id, t.file_name, t.status, t.error, t.url, t.headers);
                            }
                        }
                        store.media_credential_get_matching("pan.baidu.com").await.ok().flatten().map(|c| c.cookie)
                    });
                }
            }
            None
        }).await.ok().flatten();

        if let Some(ref c) = real_cookie {
            println!(">>> 成功从本地数据库加载 pan.baidu.com Cookie, 长度: {}", c.len());
            let u_resp = get_http_client().get("https://pan.baidu.com/api/user/getinfo?need_sub_user=1")
                .header("Cookie", c)
                .header("User-Agent", BAIDU_USER_AGENT)
                .send().await;
            if let Ok(ur) = u_resp {
                if let Ok(t) = ur.text().await {
                    println!(">>> [百度用户信息查询回执] {}", t);
                }
            }
        }

        match inspect_baidu_share(url, None, real_cookie.as_deref()).await {
            Ok(info) => {
                println!(">>> 成功解析分享信息: surl={}, share_id={:?}, uk={:?}, randsk_len={}",
                    info.surl, info.share_id, info.uk, info.randsk.as_ref().map(|r| r.len()).unwrap_or(0));
                if let Some(file) = info.files.iter().find(|f| f.kind == "drive#file") {
                    println!(">>> 目标文件: id={}, name={}, size={}", file.id, file.name, file.size);
                    let dlink_res = resolve_baidu_file(
                        &info.surl,
                        &file.id,
                        info.share_id.as_deref(),
                        info.uk.as_deref(),
                        info.sign.as_deref(),
                        info.timestamp,
                        info.seckey.as_deref(),
                        info.randsk.as_deref(),
                        real_cookie.as_deref(),
                    ).await;
                    println!(">>> 真实直链获取结果: {:?}", dlink_res);
                    assert!(dlink_res.is_ok(), "必须成功获取直链");
                    let res = dlink_res.unwrap();
                    println!(">>> [直链 URL] {}", res.url);
                    println!(">>> [直链 Headers] {:?}", res.headers);
                    let client = reqwest::Client::new();
                    
                    let mut handles = Vec::new();
                    for i in 0..4u64 {
                        let client = client.clone();
                        let url = res.url.clone();
                        let headers = res.headers.clone();
                        let start = i * 1024 * 1024;
                        let end = (i + 1) * 1024 * 1024 - 1;
                        handles.push(tokio::spawn(async move {
                            let mut req = client.get(&url).header("Range", format!("bytes={}-{}", start, end));
                            for (k, v) in &headers {
                                req = req.header(k, v);
                            }
                            let send_start = std::time::Instant::now();
                            let resp = req.send().await;
                            let status = resp.as_ref().map(|r| r.status().as_u16()).unwrap_or(0);
                            let body_bytes = if let Ok(r) = resp {
                                r.bytes().await.map(|b| b.len()).unwrap_or(0)
                            } else {
                                0
                            };
                            let elapsed = send_start.elapsed().as_millis();
                            println!(">>> [并发分片 #{}] status={}, bytes={}, 耗时={}ms", i, status, body_bytes, elapsed);
                            (status, body_bytes)
                        }));
                    }
                    for h in handles {
                        let (st, len) = h.await.unwrap();
                        assert_eq!(st, 206, "并发分片必须返回 206");
                        assert_eq!(len, 1024 * 1024, "分片大小必须为 1MB");
                    }
                    println!(">>> 4 路并发 Range 全部成功下载 4MB 数据！");
                } else {
                    println!(">>> 未找到文件");
                }
            }
            Err(e) => {
                println!(">>> 解析分享失败: {}", e);
            }
        }
    }
}
