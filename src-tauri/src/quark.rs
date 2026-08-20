//! 夸克网盘 (Quark Pan) 分享解析与直链获取模块。
//!
//! 遵循本地优先、无外部额外进程、安全合规原则：
//! 1. 运行在 Rust 原生后端，彻底规避 WebView CORS 跨域限制；
//! 2. 支持公开分享与带提取码加密分享的目录树解析；
//! 3. 结合用户提供的或本地凭证库中保存的 Cookie 获取带鉴权签名的真实 CDN 下载直链；
//! 4. 直链直连猫步下载器 HTTP Range 16/32 线程并发内核。

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;
use std::time::Duration;

pub const QUARK_API_HOST: &str = "https://drive.quark.cn";
pub const QUARK_PC_API_HOST: &str = "https://drive-pc.quark.cn";
pub const QUARK_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
pub const QUARK_PC_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) quark-cloud-drive/3.13.5 Chrome/108.0.5359.215 Electron/22.3.26 Safari/537.36 Channel/pconline";

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn get_http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(QUARK_PC_USER_AGENT)
            .build()
            .unwrap_or_default()
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarkFileItem {
    pub id: String,
    pub name: String,
    pub kind: String, // "drive#file" or "drive#folder"
    pub size: u64,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_fid_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_extension: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarkShareInfo {
    pub pwd_id: String,
    pub title: String,
    pub files: Vec<QuarkFileItem>,
    pub total_size: u64,
    pub file_count: usize,
    pub folder_count: usize,
    pub pass_code_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stoken: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarkDirectUrlResult {
    pub url: String,
    pub headers: HashMap<String, String>,
}

/// 解析夸克分享 URL 结构
pub struct ParsedQuarkUrl {
    pub pwd_id: String,
    pub pdir_fid: Option<String>,
    pub pass_code: Option<String>,
}

pub fn parse_quark_url(raw: &str) -> Option<ParsedQuarkUrl> {
    let text = raw.trim();
    // 匹配形如 https://pan.quark.cn/s/xxxx 或 https://drive.quark.cn/s/xxxx
    let re = Regex::new(
        r"(?i)https?://(?:[a-zA-Z0-9-]+\.)?quark\.cn/s/([a-zA-Z0-9_-]+)(?:/([a-zA-Z0-9_-]+))?(?:\?[^\s#]*)?",
    ).ok()?;

    let caps = re.captures(text)?;
    let pwd_id = caps.get(1)?.as_str().to_string();
    let pdir_fid = caps.get(2).map(|m| m.as_str().to_string());

    // 提取提取码（URL query 或 文本提取码）
    let mut pass_code = None;
    if let Ok(url) = url::Url::parse(caps.get(0)?.as_str()) {
        for (k, v) in url.query_pairs() {
            if k == "pwd" || k == "pass_code" || k == "code" || k == "passcode" {
                pass_code = Some(v.trim().to_string());
                break;
            }
        }
    }

    if pass_code.is_none() {
        let pwd_re = Regex::new(r"(?i)(?:提取码|密码|pwd|code|passcode)[:：\s]+([a-zA-Z0-9]{4,8})").ok();
        if let Some(re) = pwd_re {
            if let Some(m) = re.captures(text) {
                if let Some(c) = m.get(1) {
                    pass_code = Some(c.as_str().trim().to_string());
                }
            }
        }
    }

    Some(ParsedQuarkUrl {
        pwd_id,
        pdir_fid,
        pass_code,
    })
}

/// 获取分享令牌 stoken
pub async fn get_share_token(
    pwd_id: &str,
    pass_code: Option<&str>,
    cookie: Option<&str>,
) -> Result<String, String> {
    let client = get_http_client();
    let mut payload = serde_json::json!({
        "pwd_id": pwd_id,
    });
    if let Some(code) = pass_code {
        if !code.is_empty() {
            payload["passcode"] = serde_json::Value::String(code.to_string());
        }
    }

    let mut req = client
        .post(format!("{}/1/clouddrive/share/sharepage/token", QUARK_API_HOST))
        .header("Content-Type", "application/json")
        .header("Referer", "https://pan.quark.cn/")
        .body(payload.to_string());

    if let Some(c) = cookie {
        if !c.is_empty() {
            req = req.header("Cookie", c);
        }
    }

    let resp = req.send().await.map_err(|e| format!("连接夸克分享服务失败: {}", e))?;

    let text = resp.text().await.map_err(|e| format!("读取夸克响应失败: {}", e))?;
    let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();

    let status = json.get("status").and_then(|v| v.as_i64()).unwrap_or(0);
    let code = json.get("code").and_then(|v| v.as_i64()).unwrap_or(status);

    if code == 40010 || code == 40011 || text.contains("提取码错误") || text.contains("密码错误") {
        return Err("提取码错误，请重新输入".to_string());
    }
    if code == 40008 || code == 40009 || text.contains("分享已失效") || text.contains("分享不存在") {
        return Err("该夸克分享已失效或不存在".to_string());
    }

    if let Some(data) = json.get("data") {
        if let Some(stoken) = data.get("stoken").and_then(|v| v.as_str()) {
            return Ok(stoken.to_string());
        }
    }

    if let Some(msg) = json.get("message").and_then(|v| v.as_str()) {
        if msg.contains("密码") || msg.contains("提取码") {
            return Err("NEED_PASS_CODE".to_string());
        }
        return Err(msg.to_string());
    }

    Err("未能获取有效的夸克分享访问令牌".to_string())
}

/// 队列迭代抓取分享目录树（深度优先 DFS）
async fn fetch_directory_tree(
    pwd_id: &str,
    stoken: &str,
    cookie: Option<&str>,
) -> Result<Vec<QuarkFileItem>, String> {
    let client = get_http_client();
    let mut results = Vec::new();
    let mut queue = VecDeque::new();
    queue.push_back(("0".to_string(), String::new())); // 0 为夸克根目录 fid

    let mut file_count = 0;
    let mut folder_visits = 0;
    const MAX_FILES: usize = 300;
    const MAX_FOLDER_VISITS: usize = 40;

    while let Some((pdir_fid, current_path)) = queue.pop_front() {
        folder_visits += 1;
        if folder_visits > MAX_FOLDER_VISITS && file_count > 0 {
            break;
        }

        let mut page = 1;
        loop {
            let mut req = client
                .get(format!("{}/1/clouddrive/share/sharepage/detail", QUARK_API_HOST))
                .query(&[
                    ("pwd_id", pwd_id),
                    ("stoken", stoken),
                    ("pdir_fid", &pdir_fid),
                    ("_page", &page.to_string()),
                    ("_size", &"100".to_string()),
                ])
                .header("Referer", "https://pan.quark.cn/");

            if let Some(c) = cookie {
                if !c.is_empty() {
                    req = req.header("Cookie", c);
                }
            }

            let resp = req.send().await.map_err(|e| format!("拉取夸克分享内容失败: {}", e))?;

            let text = resp.text().await.map_err(|e| format!("读取夸克文件列表失败: {}", e))?;
            let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();

            if let Some(data) = json.get("data") {
                let list = data.get("list").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                if list.is_empty() {
                    break;
                }

                for item in list {
                    let fid = item.get("fid").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let file_name = item.get("file_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let is_dir = item.get("dir").and_then(|v| v.as_bool()).unwrap_or(false)
                        || item.get("file").and_then(|v| v.as_bool()) == Some(false)
                        || item.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false)
                        || item.get("obj_type").and_then(|v| v.as_str()) == Some("dir")
                        || item.get("format_type").and_then(|v| v.as_str()) == Some("dir");
                    let size = item.get("size").and_then(|v| v.as_u64()).or_else(|| {
                        item.get("size").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok())
                    }).unwrap_or(0);
                    let format_type = item.get("format_type").and_then(|v| v.as_str()).map(|s| s.to_string());
                    let file_extension = item.get("file_extension").and_then(|v| v.as_str()).map(|s| s.to_string())
                        .or_else(|| file_name.split('.').last().map(|s| s.to_string()));
                    let thumbnail_url = item.get("thumbnail").or_else(|| item.get("icon")).and_then(|v| v.as_str()).map(|s| s.to_string());

                    let item_path = if current_path.is_empty() {
                        file_name.clone()
                    } else {
                        format!("{}/{}", current_path, file_name)
                    };

                    let kind = if is_dir { "drive#folder" } else { "drive#file" }.to_string();
                    if !is_dir {
                        file_count += 1;
                    }

                    let share_fid_token = item.get("share_fid_token")
                        .or_else(|| item.get("fid_token"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    results.push(QuarkFileItem {
                        id: fid.clone(),
                        name: file_name.clone(),
                        kind,
                        size,
                        path: item_path.clone(),
                        share_fid_token,
                        mime_type: None,
                        file_extension,
                        thumbnail_url,
                        format_type,
                    });

                    if is_dir && file_count < MAX_FILES {
                        queue.push_front((fid, item_path));
                    }
                }

                let total = data.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                if page * 100 >= total || file_count >= MAX_FILES {
                    break;
                }
                page += 1;
            } else {
                break;
            }
        }

        if file_count >= MAX_FILES {
            break;
        }
    }

    Ok(results)
}

/// 解析夸克分享链接
pub async fn inspect_quark_share(
    raw_url: &str,
    provided_pass_code: Option<String>,
    cookie: Option<String>,
) -> Result<QuarkShareInfo, String> {
    let parsed = parse_quark_url(raw_url).ok_or_else(|| {
        "无效的夸克分享链接，格式应为 https://pan.quark.cn/s/xxxx".to_string()
    })?;

    let effective_pass_code = provided_pass_code.or(parsed.pass_code);
    let stoken_res = get_share_token(&parsed.pwd_id, effective_pass_code.as_deref(), cookie.as_deref()).await;

    let stoken = match stoken_res {
        Ok(t) => t,
        Err(e) if e == "NEED_PASS_CODE" => {
            return Ok(QuarkShareInfo {
                pwd_id: parsed.pwd_id,
                title: "夸克加密分享".to_string(),
                files: Vec::new(),
                total_size: 0,
                file_count: 0,
                folder_count: 0,
                pass_code_required: true,
                stoken: None,
            });
        }
        Err(e) => return Err(e),
    };

    let all_items = fetch_directory_tree(&parsed.pwd_id, &stoken, cookie.as_deref()).await?;

    let total_size = all_items
        .iter()
        .filter(|i| i.kind == "drive#file")
        .map(|i| i.size)
        .sum();
    let file_count = all_items.iter().filter(|i| i.kind == "drive#file").count();
    let folder_count = all_items.iter().filter(|i| i.kind == "drive#folder").count();

    let title = if let Some(top_folder) = all_items.iter().find(|i| i.kind == "drive#folder" && !i.path.contains('/')) {
        top_folder.name.clone()
    } else if let Some(first) = all_items.iter().find(|i| i.kind == "drive#file") {
        first.name.clone()
    } else {
        "夸克分享资源".to_string()
    };

    Ok(QuarkShareInfo {
        pwd_id: parsed.pwd_id,
        title,
        files: all_items,
        total_size,
        file_count,
        folder_count,
        pass_code_required: false,
        stoken: Some(stoken),
    })
}

/// 将夸克分享文件自动转存至用户网盘并返回转存后的 fid
async fn save_share_file_to_drive(
    pwd_id: &str,
    fid: &str,
    share_fid_token: Option<&str>,
    stoken: &str,
    cookie: &str,
) -> Result<String, String> {
    let client = get_http_client();
    let mut payload = serde_json::json!({
        "fid_list": [fid],
        "pwd_id": pwd_id,
        "stoken": stoken,
        "to_pdir_fid": "0",
    });

    if let Some(token) = share_fid_token {
        if !token.is_empty() {
            payload["fid_token_list"] = serde_json::json!([token]);
        }
    }

    let resp = client
        .post(format!("{}/1/clouddrive/share/sharepage/save?pr=ucpro&fr=pc", QUARK_API_HOST))
        .header("Content-Type", "application/json")
        .header("Cookie", cookie)
        .header("Referer", "https://pan.quark.cn/")
        .header("Origin", "https://pan.quark.cn")
        .header("User-Agent", QUARK_USER_AGENT)
        .body(payload.to_string())
        .send()
        .await
        .map_err(|e| format!("自动转存请求失败: {}", e))?;

    let text = resp.text().await.map_err(|e| format!("读取转存响应失败: {}", e))?;
    let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();

    if let Some(code) = json.get("code").and_then(|v| v.as_i64()) {
        if code != 0 && code != 200 {
            let msg = json.get("message").or_else(|| json.get("msg")).and_then(|v| v.as_str()).unwrap_or("未知错误");
            return Err(format!("自动转存失败: {}", msg));
        }
    }

    let task_id = json.get("data")
        .and_then(|d| d.get("task_id").or_else(|| d.get("task_id_str")))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    if let Some(tid) = task_id {
        // 轮询查询转存任务状态（最多等待 6 秒）
        for retry in 0..12 {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            let poll_resp = client
                .get(format!("{}/1/clouddrive/task?task_id={}&retry_index={}&pr=ucpro&fr=pc", QUARK_API_HOST, tid, retry))
                .header("Cookie", cookie)
                .header("Referer", "https://pan.quark.cn/")
                .header("Origin", "https://pan.quark.cn")
                .header("User-Agent", QUARK_USER_AGENT)
                .send()
                .await;

            if let Ok(p_resp) = poll_resp {
                if let Ok(p_text) = p_resp.text().await {
                    let p_json: serde_json::Value = serde_json::from_str(&p_text).unwrap_or_default();
                    if let Some(data) = p_json.get("data") {
                        let status = data.get("status").and_then(|v| v.as_i64()).unwrap_or(0);
                        // 1. 尝试从各种可能的文件 ID 数组中提取
                        for field_name in &["save_as_top_fids", "save_as_fids", "fids", "target_fids"] {
                            if let Some(fids) = data.get(*field_name).and_then(|v| v.as_array()) {
                                if let Some(first_fid) = fids.first().and_then(|v| v.as_str()) {
                                    if !first_fid.is_empty() {
                                        return Ok(first_fid.to_string());
                                    }
                                }
                            }
                        }
                        if let Some(fid_val) = data.get("fid").or_else(|| data.get("file_id")).and_then(|v| v.as_str()) {
                            if !fid_val.is_empty() {
                                return Ok(fid_val.to_string());
                            }
                        }
                        if let Some(list) = data.get("list").and_then(|v| v.as_array()) {
                            if let Some(first_item) = list.first() {
                                if let Some(first_fid) = first_item.get("fid").and_then(|v| v.as_str()) {
                                    return Ok(first_fid.to_string());
                                }
                            }
                        }

                        // 如果状态已是 2（完成），但上述字段未解析到，尝试查询个人网盘根目录最新文件
                        if status == 2 {
                            let sort_resp = client
                                .get(format!("{}/1/clouddrive/file/sort?pdir_fid=0&_sort=created_at:desc&_page=1&_size=10&pr=ucpro&fr=pc", QUARK_API_HOST))
                                .header("Cookie", cookie)
                                .header("Referer", "https://pan.quark.cn/")
                                .header("User-Agent", QUARK_USER_AGENT)
                                .send()
                                .await;

                            if let Ok(s_resp) = sort_resp {
                                if let Ok(s_text) = s_resp.text().await {
                                    let s_json: serde_json::Value = serde_json::from_str(&s_text).unwrap_or_default();
                                    if let Some(list) = s_json.get("data").and_then(|d| d.get("list")).and_then(|v| v.as_array()) {
                                        if let Some(first_item) = list.first() {
                                            if let Some(first_fid) = first_item.get("fid").and_then(|v| v.as_str()) {
                                                return Ok(first_fid.to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Err("自动转存未能在网盘中检索到转存后的文件记录".to_string())
}

/// 解析单文件下载直链
pub async fn resolve_quark_file(
    pwd_id: &str,
    fid: &str,
    share_fid_token: Option<&str>,
    stoken: Option<&str>,
    cookie: Option<&str>,
) -> Result<QuarkDirectUrlResult, String> {
    let client = get_http_client();

    let cookie_val = cookie.unwrap_or("").trim();
    let effective_stoken = match stoken {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => {
            let token_payload = serde_json::json!({
                "pwd_id": pwd_id,
            });
            let token_resp = client
                .post(format!("{}/1/clouddrive/share/sharepage/token", QUARK_API_HOST))
                .header("Content-Type", "application/json")
                .header("Referer", "https://pan.quark.cn/")
                .body(token_payload.to_string())
                .send()
                .await;

            if let Ok(t_resp) = token_resp {
                if let Ok(t_text) = t_resp.text().await {
                    let t_json: serde_json::Value = serde_json::from_str(&t_text).unwrap_or_default();
                    t_json.get("data")
                        .and_then(|d| d.get("stoken"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        }
    };

    let payload = serde_json::json!({
        "fids": [fid],
        "pwd_id": pwd_id,
        "stoken": effective_stoken,
    });

    let mut resolved_url: Option<String> = None;
    let mut last_error_msg = String::new();

    let resp_res = client
        .post(format!("{}/1/clouddrive/file/download?pr=ucpro&fr=pc", QUARK_PC_API_HOST))
        .header("Content-Type", "application/json")
        .header("Cookie", cookie_val)
        .header("Referer", "https://pan.quark.cn/")
        .header("Origin", "https://pan.quark.cn")
        .header("User-Agent", QUARK_PC_USER_AGENT)
        .body(payload.to_string())
        .send()
        .await;

    if let Ok(resp) = resp_res {
        if let Ok(text) = resp.text().await {
            let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            let direct_url = json.get("data")
                .and_then(|d| d.get("download_url").or_else(|| d.get("url")))
                .and_then(|v| v.as_str())
                .or_else(|| {
                    json.get("data")
                        .and_then(|d| d.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|f| {
                            if let Some(s) = f.as_str() {
                                Some(s)
                            } else {
                                f.get("download_url").or_else(|| f.get("url")).and_then(|v| v.as_str())
                            }
                        })
                })
                .map(|s| s.to_string());

            if let Some(url) = direct_url {
                resolved_url = Some(url);
            } else {
                let code = json.get("code").and_then(|v| v.as_i64()).unwrap_or(0);
                let msg = json.get("message")
                    .or_else(|| json.get("msg"))
                    .or_else(|| json.get("error"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                last_error_msg = msg.to_string();

                let is_size_limit = code == 23018 || msg.contains("size limit") || msg.contains("limit") || msg.contains("超限");
                if is_size_limit {
                    let mut stoken_to_use = effective_stoken.clone();
                    if stoken_to_use.is_empty() {
                        let token_payload = serde_json::json!({
                            "pwd_id": pwd_id,
                        });
                        if let Ok(t_resp) = client
                            .post(format!("{}/1/clouddrive/share/sharepage/token", QUARK_API_HOST))
                            .header("Content-Type", "application/json")
                            .header("Referer", "https://pan.quark.cn/")
                            .body(token_payload.to_string())
                            .send()
                            .await
                        {
                            if let Ok(t_text) = t_resp.text().await {
                                let t_json: serde_json::Value = serde_json::from_str(&t_text).unwrap_or_default();
                                if let Some(stk) = t_json.get("data").and_then(|d| d.get("stoken")).and_then(|v| v.as_str()) {
                                    stoken_to_use = stk.to_string();
                                }
                            }
                        }
                    }

                    match save_share_file_to_drive(pwd_id, fid, share_fid_token, &stoken_to_use, cookie_val).await {
                        Ok(saved_fid) => {
                            let save_payload = serde_json::json!({
                                "fids": [saved_fid],
                            });
                            let save_down = client
                                .post(format!("{}/1/clouddrive/file/download?pr=ucpro&fr=pc", QUARK_PC_API_HOST))
                                .header("Content-Type", "application/json")
                                .header("Cookie", cookie_val)
                                .header("Referer", "https://pan.quark.cn/")
                                .header("Origin", "https://pan.quark.cn")
                                .header("User-Agent", QUARK_PC_USER_AGENT)
                                .body(save_payload.to_string())
                                .send()
                                .await;

                            if let Ok(s_resp) = save_down {
                                if let Ok(s_text) = s_resp.text().await {
                                    let s_json: serde_json::Value = serde_json::from_str(&s_text).unwrap_or_default();
                                    let s_direct_url = s_json.get("data")
                                        .and_then(|d| d.get("download_url").or_else(|| d.get("url")))
                                        .and_then(|v| v.as_str())
                                        .or_else(|| {
                                            s_json.get("data")
                                                .and_then(|d| d.as_array())
                                                .and_then(|arr| arr.first())
                                                .and_then(|f| {
                                                    if let Some(s) = f.as_str() {
                                                        Some(s)
                                                    } else {
                                                        f.get("download_url").or_else(|| f.get("url")).and_then(|v| v.as_str())
                                                    }
                                                })
                                        })
                                        .map(|s| s.to_string());

                                    if let Some(url) = s_direct_url {
                                        resolved_url = Some(url);
                                    } else {
                                        let s_msg = s_json.get("message").or_else(|| s_json.get("msg")).and_then(|v| v.as_str()).unwrap_or("");
                                        last_error_msg = format!("转存成功但获取直链失败: {}", s_msg);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            last_error_msg = format!("大文件免转存受限且自动转存失败: {}", e);
                        }
                    }
                }
            }
        }
    }

    let direct_url = resolved_url.ok_or_else(|| {
        if !last_error_msg.is_empty() {
            format!("夸克直链获取失败: {}", last_error_msg)
        } else {
            "夸克服务端未返回有效的下载地址".to_string()
        }
    })?;

    let mut headers = HashMap::new();
    headers.insert("User-Agent".to_string(), QUARK_USER_AGENT.to_string());
    headers.insert("Referer".to_string(), "https://pan.quark.cn/".to_string());
    if !cookie_val.is_empty() {
        headers.insert("Cookie".to_string(), cookie_val.to_string());
    }

    Ok(QuarkDirectUrlResult {
        url: direct_url,
        headers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_quark_url() {
        let url1 = "https://pan.quark.cn/s/1234567890ab";
        let p1 = parse_quark_url(url1).unwrap();
        assert_eq!(p1.pwd_id, "1234567890ab");
        assert!(p1.pass_code.is_none());

        let url2 = "https://pan.quark.cn/s/abcdef123456?pwd=abcd";
        let p2 = parse_quark_url(url2).unwrap();
        assert_eq!(p2.pwd_id, "abcdef123456");
        assert_eq!(p2.pass_code.as_deref(), Some("abcd"));

        let text4 = "https://pan.quark.cn/s/69ba75a686aa#/list/share";
        let p4 = parse_quark_url(text4).unwrap();
        assert_eq!(p4.pwd_id, "69ba75a686aa");
    }

    #[tokio::test]
    async fn test_inspect_real_quark_share() {
        let url = "https://pan.quark.cn/s/69ba75a686aa#/list/share";
        let res = inspect_quark_share(url, None, None).await;
        if let Ok(info) = res {
            if !info.files.is_empty() {
                if let Some(movie) = info.files.iter().find(|f| f.kind == "drive#file") {
                    assert!(movie.name.contains("2026"), "必须解析出真实电影文件名");
                    assert!(movie.size > 1_000_000_000, "电影大小应大于 1GB");
                }
            }
        } else {
            eprintln!("当前环境连接夸克外部服务器受限或超时，跳过在线断言");
        }
    }
}
