//! 蓝奏云 (Lanzou) 分享解析与直链获取模块。
//!
//! 支持各类官方与镜像域名（`lanzoux.com` / `lanzoui.com` / `lanzouy.com` / `lanzouv.com` / `lanzoup.com` 等），
//! 支持单文件、文件夹（目录树）及用户主页（`/u/xxx`）解析，支持带密码（提取码）公开分享与阿里云 WAF (`acw_sc__v2`) 自动计算。

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

pub const LANZOU_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

static RE_LANZOU_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)https?://(?:[a-zA-Z0-9-]+\.)*(?:lanzou[a-z]?|baidupan)\.(?:com|net|cn|org|xyz|cc|icu|vip)(?:/(?:u|tp|b)/|/)?([a-zA-Z0-9_-]+)(?:\?[^\s#]*)?")
        .expect("RE_LANZOU_URL regex compile")
});

static RE_IFRAME_SRC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<iframe[^>]+src="(/fn\?[^"]+)""#).expect("RE_IFRAME_SRC regex compile")
});

static RE_FILE_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<div class="n_box_3fn">([^<]+)</div>|<div class="filethetext">([^<]+)</div>|<div style="font-size: 30px;font-weight: bold;padding: 30px;">([^<]+)</div>|<div style="font-size: 30px;text-align: center;padding: 56px 0px 20px 0px;">([^<]+)</div>|<title>([^<-]+)"#)
        .expect("RE_FILE_NAME regex compile")
});

static RE_FILE_SIZE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<span class="p7">大小：</span>([^<]+)|<span class="p7">文件大小：</span>([^<]+)|<span class="p7">大小：([^<]+)</span>|<div class="n_filesize">大小：([^<]+)</div>"#)
        .expect("RE_FILE_SIZE regex compile")
});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanzouFileItem {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub size_formatted: String,
    pub time: String,
    pub kind: String, // "file" | "folder"
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanzouShareInfo {
    pub share_id: String,
    pub share_url: String,
    pub title: String,
    pub is_folder: bool,
    pub requires_password: bool,
    pub files: Vec<LanzouFileItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanzouDirectUrlResult {
    pub url: String,
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLanzouUrl {
    pub host: String,
    pub share_id: String,
    pub pass_code: Option<String>,
}

pub fn parse_lanzou_url(raw: &str) -> Option<ParsedLanzouUrl> {
    let parsed = url::Url::parse(raw).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    if !is_lanzou_host(&host) {
        return None;
    }

    let path_segments: Vec<&str> = parsed.path_segments()?.filter(|s| !s.is_empty()).collect();
    let share_id = path_segments.last()?.to_string();
    if share_id.is_empty() {
        return None;
    }

    let mut pass_code = None;
    for (k, v) in parsed.query_pairs() {
        if (k == "pwd" || k == "p" || k == "passcode") && !v.is_empty() {
            pass_code = Some(v.to_string());
            break;
        }
    }

    Some(ParsedLanzouUrl {
        host,
        share_id,
        pass_code,
    })
}

pub fn is_lanzou_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host.contains("lanzou") || host.contains("lanzo") || host.contains("baidupan.com.lanzou")
}

/// 计算阿里云 WAF 安全验证 Cookie (`acw_sc__v2`)
fn compute_acw_sc_v2(arg1: &str) -> String {
    let m = [
        0xf, 0x23, 0x1d, 0x18, 0x21, 0x10, 0x1, 0x26, 0xa, 0x9, 0x13, 0x1f, 0x28, 0x1b, 0x16,
        0x17, 0x19, 0xd, 0x6, 0xb, 0x27, 0x12, 0x14, 0x8, 0xe, 0x15, 0x20, 0x1a, 0x2, 0x1e,
        0x7, 0x4, 0x11, 0x5, 0x3, 0x1c, 0x22, 0x25, 0xc, 0x24,
    ];
    let p = "3000176000856006061501533003690027800375";
    let arg1_chars: Vec<char> = arg1.chars().collect();
    let mut q: Vec<char> = vec![' '; m.len()];

    for (x, &ch) in arg1_chars.iter().enumerate() {
        for (z, &pos) in m.iter().enumerate() {
            if pos == x + 1 && z < q.len() {
                q[z] = ch;
            }
        }
    }

    let u: String = q.into_iter().collect();
    let u_bytes = u.as_bytes();
    let p_bytes = p.as_bytes();
    let mut v = String::new();

    let max_len = u_bytes.len().min(p_bytes.len());
    let mut x = 0;
    while x + 2 <= max_len {
        let u_sub = &u[x..x + 2];
        let p_sub = &p[x..x + 2];
        let u_val = u32::from_str_radix(u_sub, 16).unwrap_or(0);
        let p_val = u32::from_str_radix(p_sub, 16).unwrap_or(0);
        let xor_val = u_val ^ p_val;
        v.push_str(&format!("{:02x}", xor_val));
        x += 2;
    }

    v
}

/// 发送请求并自动处理 WAF Cookie 挑战
/// 发送请求并自动处理 WAF Cookie 挑战，返回 (页面内容, 有效Cookie)
async fn fetch_lanzou_page(
    client: &reqwest::Client,
    url: &str,
    referer: Option<&str>,
    cookie: Option<&str>,
) -> Result<(String, Option<String>), String> {
    let mut req = client
        .get(url)
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("User-Agent", LANZOU_USER_AGENT);

    if let Some(ref_) = referer {
        req = req.header("Referer", ref_);
    }
    if let Some(ck) = cookie {
        req = req.header("Cookie", ck);
    }

    let resp = req.send().await.map_err(|e| format!("请求蓝奏云页面失败：{e}"))?;
    let html = resp.text().await.unwrap_or_default();

    if html.contains("var arg1") {
        let re_arg1 = Regex::new(r#"var\s+arg1\s*=\s*'([^']+)'"#).unwrap();
        if let Some(cap) = re_arg1.captures(&html).and_then(|c| c.get(1)) {
            let acw = compute_acw_sc_v2(cap.as_str());
            let cookie_val = format!("acw_sc__v2={acw}");

            let mut req2 = client
                .get(url)
                .header("Accept-Language", "zh-CN,zh;q=0.9")
                .header("User-Agent", LANZOU_USER_AGENT)
                .header("Cookie", &cookie_val);

            if let Some(ref_) = referer {
                req2 = req2.header("Referer", ref_);
            }

            let resp2 = req2.send().await.map_err(|e| format!("挑战验证后重新请求失败：{e}"))?;
            return Ok((resp2.text().await.unwrap_or_default(), Some(cookie_val)));
        }
    }

    Ok((html, cookie.map(|s| s.to_string())))
}

pub async fn inspect_lanzou_share(
    share_url: &str,
    pass_code: Option<&str>,
) -> Result<LanzouShareInfo, String> {
    let parsed = parse_lanzou_url(share_url)
        .ok_or_else(|| "无法识别的蓝奏云分享链接".to_string())?;
    let effective_pwd = pass_code.map(|s| s.to_string()).or(parsed.pass_code.clone());

    let client = reqwest::Client::builder()
        .user_agent(LANZOU_USER_AGENT)
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;

    let (html, cookie_header) = fetch_lanzou_page(&client, share_url, None, None).await?;

    let requires_pwd = html.contains("pwdload")
        || html.contains("passwddiv")
        || html.contains("输入密码")
        || html.contains("输入提取码");

    let is_folder = parsed.share_id.starts_with('b')
        || share_url.contains("/u/")
        || html.contains("filemoreajax.php")
        || html.contains("folder");

    if is_folder {
        let mut files = Vec::new();
        let mut title = format!("蓝奏云分享 - {}", parsed.share_id);

        if let Some(caps) = RE_FILE_NAME.captures(&html) {
            for idx in 1..=5 {
                if let Some(m) = caps.get(idx) {
                    let name = m.as_str().trim().to_string();
                    if !name.is_empty() {
                        title = name;
                        break;
                    }
                }
            }
        }

        let re_ajax_path = Regex::new(r#"url\s*:\s*['"](/filemoreajax\.php[^'"]*)['"]"#).unwrap();
        let ajax_path = re_ajax_path
            .captures(&html)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str())
            .unwrap_or("/filemoreajax.php");

        let mut var_map: HashMap<String, String> = HashMap::new();
        let re_vars = Regex::new(r#"var\s+([a-zA-Z0-9_]+)\s*=\s*['"]([^'"]+)['"]"#).unwrap();
        for cap in re_vars.captures_iter(&html) {
            if let (Some(k), Some(v)) = (cap.get(1), cap.get(2)) {
                var_map.insert(k.as_str().to_string(), v.as_str().to_string());
            }
        }

        let mut params: HashMap<String, String> = HashMap::new();
        let re_data_block = Regex::new(r#"data\s*:\s*\{([^}]+)\}"#).unwrap();
        if let Some(block) = re_data_block.captures(&html).and_then(|c| c.get(1)) {
            let re_pairs = Regex::new(r#"['"]?([a-zA-Z0-9_]+)['"]?\s*:\s*([^,\n\r]+)"#).unwrap();
            for cap in re_pairs.captures_iter(block.as_str()) {
                if let (Some(k), Some(v)) = (cap.get(1), cap.get(2)) {
                    let key = k.as_str().trim().to_string();
                    let raw_val = v.as_str().trim().trim_matches('\'').trim_matches('"');
                    let mut val = var_map.get(raw_val).cloned().unwrap_or_else(|| raw_val.to_string());
                    if val == "pgs" || val == "pg" {
                        val = "1".to_string();
                    }
                    params.insert(key, val);
                }
            }
        }

        if !params.contains_key("lx") {
            params.insert("lx".to_string(), "1".to_string());
        }
        if !params.contains_key("pg") {
            params.insert("pg".to_string(), "1".to_string());
        }
        if !params.contains_key("rep") {
            params.insert("rep".to_string(), "0".to_string());
        }
        if let Some(ref pwd) = effective_pwd {
            params.insert("pwd".to_string(), pwd.clone());
        }

        let api_url = if ajax_path.starts_with("http") {
            ajax_path.to_string()
        } else {
            format!("https://{}{}", parsed.host, ajax_path)
        };

        let mut req_post = client
            .post(&api_url)
            .form(&params)
            .header("Referer", share_url)
            .header("Accept", "application/json, text/javascript, */*");

        if let Some(ref ck) = cookie_header {
            req_post = req_post.header("Cookie", ck);
        }

        if let Ok(f_resp) = req_post.send().await {
            if let Ok(json_text) = f_resp.text().await {
                if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&json_text) {
                    if let Some(text_arr) = json_val.get("text").and_then(|v| v.as_array()) {
                        for item in text_arr {
                            let item_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            if item_id.is_empty() || item_id == "-1" {
                                continue;
                            }
                            let raw_name = item.get("name_all").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let clean_name = raw_name.replace("<span class=\"s_ad\">推广</span>", "");
                            let size_str = item.get("size").and_then(|v| v.as_str()).unwrap_or("0").to_string();
                            let time_str = item.get("time").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let parsed_size = parse_size_to_bytes(&size_str);

                            let item_url = if item_id.starts_with("http") {
                                item_id.clone()
                            } else {
                                format!("https://{}/{}", parsed.host, item_id)
                            };

                            files.push(LanzouFileItem {
                                id: item_id,
                                name: clean_name,
                                size: parsed_size,
                                size_formatted: size_str,
                                time: time_str,
                                kind: "file".to_string(),
                                url: item_url,
                            });
                        }
                    }
                }
            }
        }

        return Ok(LanzouShareInfo {
            share_id: parsed.share_id,
            share_url: share_url.to_string(),
            title,
            is_folder: true,
            requires_password: requires_pwd && files.is_empty(),
            files,
        });
    }

    let mut title = format!("蓝奏云文件 - {}", parsed.share_id);
    let mut file_size = 0u64;
    let mut size_formatted = String::new();

    if let Some(caps) = RE_FILE_NAME.captures(&html) {
        for idx in 1..=5 {
            if let Some(m) = caps.get(idx) {
                let name = m.as_str().trim().to_string();
                if !name.is_empty() {
                    title = name;
                    break;
                }
            }
        }
    }

    if let Some(caps) = RE_FILE_SIZE.captures(&html) {
        for idx in 1..=4 {
            if let Some(m) = caps.get(idx) {
                size_formatted = m.as_str().trim().to_string();
                file_size = parse_size_to_bytes(&size_formatted);
                break;
            }
        }
    }

    let files = vec![LanzouFileItem {
        id: parsed.share_id.clone(),
        name: title.clone(),
        size: file_size,
        size_formatted,
        time: String::new(),
        kind: "file".to_string(),
        url: share_url.to_string(),
    }];

    Ok(LanzouShareInfo {
        share_id: parsed.share_id,
        share_url: share_url.to_string(),
        title,
        is_folder: false,
        requires_password: requires_pwd,
        files,
    })
}

pub async fn resolve_lanzou_file(
    share_url: &str,
    file_id: &str,
    pass_code: Option<&str>,
) -> Result<LanzouDirectUrlResult, String> {
    let parsed = parse_lanzou_url(share_url)
        .ok_or_else(|| "无法识别的蓝奏云分享链接".to_string())?;
    let effective_pwd = pass_code.map(|s| s.to_string()).or(parsed.pass_code.clone());

    let target_url = if file_id.starts_with("http") {
        file_id.to_string()
    } else {
        format!("https://{}/{}", parsed.host, file_id)
    };

    let client = reqwest::Client::builder()
        .user_agent(LANZOU_USER_AGENT)
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;

    let (html, cookie_opt) = fetch_lanzou_page(&client, &target_url, Some(share_url), None).await?;
    let re_find_iframe = Regex::new(r#"(?is)<iframe[^>]*\s+src=['"]?([^'"\s>]+)['"]?"#).unwrap();

    let (sign_html, req_host, iframe_ref) = if let Some(iframe_cap) = re_find_iframe.captures(&html) {
        if let Some(m) = iframe_cap.get(1) {
            let path = m.as_str();
            let iframe_url = if path.starts_with("http") {
                path.to_string()
            } else {
                format!("https://{}{}", parsed.host, path)
            };
            let (if_html, _) = fetch_lanzou_page(&client, &iframe_url, Some(&target_url), cookie_opt.as_deref()).await?;
            (if_html, parsed.host.clone(), iframe_url)
        } else {
            (html.clone(), parsed.host.clone(), target_url.clone())
        }
    } else {
        (html.clone(), parsed.host.clone(), target_url.clone())
    };

    // 1. 提取变量映射
    let mut var_map: HashMap<String, String> = HashMap::new();
    let re_vars = Regex::new(r#"var\s+([a-zA-Z0-9_]+)\s*=\s*['"]([^'"]*)['"]"#).unwrap();
    for cap in re_vars.captures_iter(&sign_html) {
        if let (Some(k), Some(v)) = (cap.get(1), cap.get(2)) {
            var_map.insert(k.as_str().to_string(), v.as_str().to_string());
        }
    }

    // 2. 提取 AJAX 请求的 URL（如 /ajaxfile.php?file=... 或 /ajaxm.php）
    let re_ajax_file = Regex::new(r#"url\s*:\s*['"](/ajax[^'"]*)['"]"#).unwrap();
    let ajax_path = re_ajax_file
        .captures(&sign_html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())
        .unwrap_or("/ajaxm.php");

    // 3. 提取 data 对象参数
    let mut params: HashMap<String, String> = HashMap::new();
    let re_data_block = Regex::new(r#"data\s*:\s*\{([^}]+)\}"#).unwrap();
    if let Some(block) = re_data_block.captures(&sign_html).and_then(|c| c.get(1)) {
        let re_pairs = Regex::new(r#"['"]?([a-zA-Z0-9_]+)['"]?\s*:\s*([^,\n\r]+)"#).unwrap();
        for cap in re_pairs.captures_iter(block.as_str()) {
            if let (Some(k), Some(v)) = (cap.get(1), cap.get(2)) {
                let key = k.as_str().trim().to_string();
                let raw_val = v.as_str().trim().trim_matches('\'').trim_matches('"');
                let mut val = var_map.get(raw_val).cloned().unwrap_or_else(|| raw_val.to_string());
                if raw_val == "kdns" || raw_val == "kd" {
                    val = "1".to_string();
                }
                params.insert(key, val);
            }
        }
    }

    // 兜底补齐常见必须参数
    if !params.contains_key("action") {
        params.insert("action".to_string(), "downprocess".to_string());
    }
    if !params.contains_key("ves") {
        params.insert("ves".to_string(), "1".to_string());
    }
    if let Some(ref pwd) = effective_pwd {
        if !pwd.trim().is_empty() {
            params.insert("pwd".to_string(), pwd.trim().to_string());
            params.insert("p".to_string(), pwd.trim().to_string());
        }
    }

    let ajax_full_url = if ajax_path.starts_with("http") {
        ajax_path.to_string()
    } else {
        format!("https://{}{}", req_host, ajax_path)
    };

    let mut ajax_req = client
        .post(&ajax_full_url)
        .form(&params)
        .header("Referer", &iframe_ref)
        .header("X-Requested-With", "XMLHttpRequest")
        .header("User-Agent", LANZOU_USER_AGENT);

    if let Some(ref ck) = cookie_opt {
        ajax_req = ajax_req.header("Cookie", ck);
    }

    let ajax_resp = ajax_req
        .send()
        .await
        .map_err(|e| format!("请求蓝奏云直链接口失败：{e}"))?;

    let ajax_text = ajax_resp.text().await.unwrap_or_default();
    let ajax_json: serde_json::Value = serde_json::from_str(&ajax_text)
        .map_err(|_| format!("解析直链接口回执失败：{ajax_text}"))?;

    let zt = ajax_json.get("zt").and_then(|v| v.as_i64()).unwrap_or(0);
    if zt != 1 {
        let info = ajax_json
            .get("info")
            .or_else(|| ajax_json.get("inf"))
            .and_then(|v| v.as_str())
            .unwrap_or("获取下载地址失败");
        return Err(format!("蓝奏云解析失败：{info}"));
    }

    let dom = ajax_json.get("dom").and_then(|v| v.as_str()).unwrap_or("");
    let url_part = ajax_json.get("url").and_then(|v| v.as_str()).unwrap_or("");

    let mut direct_url = if url_part.starts_with("http") {
        url_part.to_string()
    } else {
        format!("{}/file/{}", dom, url_part.trim_start_matches('/'))
    };

    // 发送 GET 跟踪 302 得到最底层的真实 CDN 直链
    if let Ok(loc_resp) = client
        .get(&direct_url)
        .header("Referer", &target_url)
        .send()
        .await
    {
        if loc_resp.status().is_redirection() {
            if let Some(loc) = loc_resp.headers().get("location") {
                if let Ok(loc_str) = loc.to_str() {
                    direct_url = loc_str.to_string();
                }
            }
        }
    }

    let mut headers = HashMap::new();
    headers.insert("Referer".to_string(), target_url);
    headers.insert("User-Agent".to_string(), LANZOU_USER_AGENT.to_string());
    headers.insert("Accept-Language".to_string(), "zh-CN,zh;q=0.9".to_string());

    Ok(LanzouDirectUrlResult {
        url: direct_url,
        headers,
    })
}

fn parse_size_to_bytes(size_str: &str) -> u64 {
    let s = size_str.trim().to_uppercase();
    let num_str: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let num: f64 = num_str.parse().unwrap_or(0.0);

    if s.contains("GB") || s.contains('G') {
        (num * 1024.0 * 1024.0 * 1024.0) as u64
    } else if s.contains("MB") || s.contains('M') {
        (num * 1024.0 * 1024.0) as u64
    } else if s.contains("KB") || s.contains('K') {
        (num * 1024.0) as u64
    } else {
        num as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_lanzou_url() {
        let p1 = parse_lanzou_url("https://www.lanzoui.com/u/yoyodadada").unwrap();
        assert_eq!(p1.host, "www.lanzoui.com");
        assert_eq!(p1.share_id, "yoyodadada");

        let p2 = parse_lanzou_url("https://wwx.lanzoux.com/b0xxxxxx?pwd=abcd").unwrap();
        assert_eq!(p2.share_id, "b0xxxxxx");
        assert_eq!(p2.pass_code, Some("abcd".to_string()));
    }

    #[test]
    fn test_compute_acw_sc_v2() {
        let arg1 = "C3597E2A7313215F416A94C769DB83E10CA60CEE";
        let acw = compute_acw_sc_v2(arg1);
        assert!(!acw.is_empty());
        assert_eq!(acw.len(), 40);
    }

    #[tokio::test]
    async fn test_live_lanzou_resolve() {
        let info = inspect_lanzou_share("https://www.lanzoui.com/u/yoyodadada", None).await.unwrap();
        assert!(!info.files.is_empty(), "必须拉取到文件列表");
        let first_file = &info.files[0];
        let direct = resolve_lanzou_file("https://www.lanzoui.com/u/yoyodadada", &first_file.id, None).await.unwrap();
        assert!(direct.url.starts_with("http"), "必须返回以 http 开头的直链: {}", direct.url);
    }
}
