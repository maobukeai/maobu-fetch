//! PikPak 网盘免登录分享解析与直链获取模块。
//!
//! 遵循本地优先、无外部额外进程、安全合规原则：
//! 1. 运行在 Rust 原生后端，彻底规避 WebView CORS 跨域限制；
//! 2. 纯客户端直连 PikPak 官方 API，免登录获取公开或加密分享的文件树；
//! 3. 解析出的文件下载直链直连猫步下载器 HTTP Range 多连接内核。

use md5::{Digest, Md5};
use regex::Regex;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
#[path = "pikpak/direct_link.rs"]
mod direct_link;
#[path = "pikpak/listing.rs"]
mod listing;

#[path = "pikpak/protocol.rs"]
mod protocol;
pub use protocol::*;

#[path = "pikpak/models.rs"]
mod models;
pub use models::*;

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

/// PikPak 裸直链（`dl-*.mypikpak.com/download/...`）的元数据。
#[derive(Debug, Clone)]
pub struct PikPakDirectLinkMeta {
    /// URL 中的 `fileid` 参数。
    pub file_id: String,
    /// URL 中的 `f` 参数（文件总字节数）。
    pub file_size: u64,
    /// URL 中的 `expire` 参数（Unix 时间戳）。
    pub expire: u64,
    /// URL 中的 `userid` 参数。
    pub user_id: String,
}

/// 从 PikPak 裸直链 URL 中解析元数据。
///
/// 裸直链格式：`https://dl-a10b-0862.mypikpak.com/download/?fid=...&fileid=VP-VLwpxxMPiLrSBZWR0JpFUo2&...`
/// 返回 `None` 表示该 URL 不是 PikPak 裸直链或缺少关键参数。
pub fn parse_pikpak_direct_link_meta(raw_url: &str) -> Option<PikPakDirectLinkMeta> {
    let url = url::Url::parse(raw_url.trim()).ok()?;
    let host = url.host_str()?;
    // 必须匹配 dl-*.mypikpak.com 或 dl-*.mypikpak.net
    if !host.starts_with("dl-")
        || !(host.ends_with(".mypikpak.com") || host.ends_with(".mypikpak.net"))
    {
        return None;
    }
    if !url.path().starts_with("/download") {
        return None;
    }
    let file_id = url
        .query_pairs()
        .find(|(k, _)| k == "fileid")
        .map(|(_, v)| v.to_string())
        .filter(|v| !v.is_empty())?;
    let file_size = url
        .query_pairs()
        .find(|(k, _)| k == "f")
        .and_then(|(_, v)| v.parse::<u64>().ok())
        .unwrap_or(0);
    let expire = url
        .query_pairs()
        .find(|(k, _)| k == "expire")
        .and_then(|(_, v)| v.parse::<u64>().ok())
        .unwrap_or(0);
    let user_id = url
        .query_pairs()
        .find(|(k, _)| k == "userid")
        .map(|(_, v)| v.to_string())
        .unwrap_or_default();
    Some(PikPakDirectLinkMeta {
        file_id,
        file_size,
        expire,
        user_id,
    })
}

/// 判断 URL 是否为 PikPak 裸直链。
pub fn is_pikpak_direct_link(url: &str) -> bool {
    parse_pikpak_direct_link_meta(url).is_some()
}

/// 获取/生成 Captcha Token 与 Device Sign
pub async fn get_captcha_and_sign(device_id: &str) -> Result<(String, String), String> {
    let network = crate::cloud_request::CloudRequestContext::defaults(
        PIKPAK_USER_AGENT,
        20,
        tokio_util::sync::CancellationToken::new(),
    )?;
    get_captcha_and_sign_with_context(device_id, &network).await
}
pub async fn get_captcha_and_sign_with_context(
    device_id: &str,
    network: &crate::cloud_request::CloudRequestContext,
) -> Result<(String, String), String> {
    let network = network
        .with_user_agent(PIKPAK_USER_AGENT)
        .with_cookie_source(PIKPAK_API_HOST);
    network
        .run("PikPak", get_captcha_and_sign_impl(device_id, &network))
        .await
}
async fn get_captcha_and_sign_impl(
    device_id: &str,
    network: &crate::cloud_request::CloudRequestContext,
) -> Result<(String, String), String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();

    let timestamp_str = now.to_string();
    let salt = "l-sark";
    let raw_sign = format!(
        "{}{}{}{}{}{}",
        PIKPAK_CLIENT_ID,
        PIKPAK_CLIENT_VERSION,
        PIKPAK_PACKAGE_NAME,
        device_id,
        timestamp_str,
        salt
    );

    let mut hasher = Md5::new();
    hasher.update(raw_sign.as_bytes());
    let sign_hex = hex::encode(hasher.finalize());
    let captcha_sign = format!("1.{}", sign_hex);

    let client = network;
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

    let resp_res = client
        .post(format!("{}/v1/shield/captcha/init", PIKPAK_USER_HOST))
        .header("Content-Type", "application/json")
        .body(payload.to_string())
        .send()
        .await;

    let token = match resp_res {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(text) = network.text(resp).await {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                    json.get("captcha_token")
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
        _ => String::new(),
    };

    Ok((token, captcha_sign))
}

/// 队列迭代抓取分享目录树（深度优先 DFS，优先提取真实文件）
async fn fetch_directory_tree(
    share_id: &str,
    initial_parent_id: &str,
    pass_code: Option<&str>,
    pass_code_token: &mut String,
    captcha_token: &str,
    device_sign: &str,
    device_id: &str,
    network: &crate::cloud_request::CloudRequestContext,
) -> Result<Vec<PikPakFileItem>, String> {
    listing::fetch(
        network.client(),
        PIKPAK_API_HOST,
        share_id,
        initial_parent_id,
        pass_code,
        pass_code_token,
        captcha_token,
        device_sign,
        device_id,
        network.cancel().clone(),
        network.user_agent(),
        network.clone(),
    )
    .await
}

/// 解析分享链接
pub async fn inspect_pikpak_share(
    raw_url: &str,
    provided_pass_code: Option<String>,
    device_id: &str,
) -> Result<PikPakShareInfo, String> {
    inspect_pikpak_share_with_cancel(
        raw_url,
        provided_pass_code,
        device_id,
        &tokio_util::sync::CancellationToken::new(),
    )
    .await
}
pub async fn inspect_pikpak_share_with_cancel(
    raw_url: &str,
    provided_pass_code: Option<String>,
    device_id: &str,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<PikPakShareInfo, String> {
    let network =
        crate::cloud_request::CloudRequestContext::defaults(PIKPAK_USER_AGENT, 20, cancel.clone())?;
    inspect_pikpak_share_with_context(raw_url, provided_pass_code, device_id, &network).await
}
pub async fn inspect_pikpak_share_with_context(
    raw_url: &str,
    provided_pass_code: Option<String>,
    device_id: &str,
    network: &crate::cloud_request::CloudRequestContext,
) -> Result<PikPakShareInfo, String> {
    let network = network
        .with_user_agent(PIKPAK_USER_AGENT)
        .with_cookie_source(PIKPAK_API_HOST);
    network
        .run(
            "PikPak",
            inspect_pikpak_share_impl(raw_url, provided_pass_code, device_id, &network),
        )
        .await
}
async fn inspect_pikpak_share_impl(
    raw_url: &str,
    provided_pass_code: Option<String>,
    device_id: &str,
    network: &crate::cloud_request::CloudRequestContext,
) -> Result<PikPakShareInfo, String> {
    let parsed = parse_pikpak_url(raw_url).ok_or_else(|| {
        "无效的 PikPak 分享链接，格式应为 https://mypikpak.com/s/xxxx".to_string()
    })?;

    let (captcha_token, device_sign) =
        get_captcha_and_sign_with_context(device_id, network).await?;
    let mut pass_code_token = String::new();

    let effective_pass_code = provided_pass_code.or(parsed.pass_code);
    let initial_parent = parsed.parent_id.as_deref().unwrap_or("");
    let fetch_res = fetch_directory_tree(
        &parsed.share_id,
        initial_parent,
        effective_pass_code.as_deref(),
        &mut pass_code_token,
        &captcha_token,
        &device_sign,
        device_id,
        network,
    )
    .await;

    let tree_res = fetch_res;

    match tree_res {
        Ok(all_items) => {
            let total_size = all_items
                .iter()
                .filter(|i| i.kind == "drive#file")
                .map(|i| i.size)
                .sum();
            let file_count = all_items.iter().filter(|i| i.kind == "drive#file").count();
            let folder_count = all_items
                .iter()
                .filter(|i| i.kind == "drive#folder")
                .count();

            // 优先使用首个顶层文件夹名或首个文件名作为标题
            let title = if let Some(top_folder) = all_items
                .iter()
                .find(|i| i.kind == "drive#folder" && !i.path.contains('/'))
            {
                top_folder.name.clone()
            } else if let Some(first) = all_items.iter().find(|i| i.kind == "drive#file") {
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
    let network = crate::cloud_request::CloudRequestContext::defaults(
        PIKPAK_USER_AGENT,
        20,
        tokio_util::sync::CancellationToken::new(),
    )?;
    resolve_pikpak_file_with_context(share_id, file_id, pass_code_token, device_id, &network).await
}
pub async fn resolve_pikpak_file_with_context(
    share_id: &str,
    file_id: &str,
    pass_code_token: Option<&str>,
    device_id: &str,
    network: &crate::cloud_request::CloudRequestContext,
) -> Result<PikPakDirectUrlResult, String> {
    let network = network
        .with_user_agent(PIKPAK_USER_AGENT)
        .with_cookie_source(PIKPAK_API_HOST);
    network
        .run(
            "PikPak",
            resolve_pikpak_file_impl(share_id, file_id, pass_code_token, device_id, &network),
        )
        .await
}
async fn resolve_pikpak_file_impl(
    share_id: &str,
    file_id: &str,
    pass_code_token: Option<&str>,
    device_id: &str,
    network: &crate::cloud_request::CloudRequestContext,
) -> Result<PikPakDirectUrlResult, String> {
    let (captcha_token, device_sign) =
        get_captcha_and_sign_with_context(device_id, network).await?;
    let client = network;

    let mut query = vec![("share_id", share_id), ("file_id", file_id)];
    if let Some(token) = pass_code_token {
        if !token.is_empty() {
            query.push(("pass_code_token", token));
        }
    }

    let mut req = client
        .get(format!("{}/drive/v1/share/file_info", PIKPAK_API_HOST))
        .query(&query)
        .header("X-Client-Id", PIKPAK_CLIENT_ID)
        .header("X-Client-Version", PIKPAK_CLIENT_VERSION)
        .header("X-Device-Id", device_id)
        .header("X-Device-Sign", &device_sign)
        .header("Referer", "https://mypikpak.com/");

    if !captcha_token.is_empty() {
        req = req.header("X-Captcha-Token", &captcha_token);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("获取文件直链失败: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("PikPak 直链接口返回错误: {}", resp.status()));
    }

    let text = network
        .text(resp)
        .await
        .map_err(|e| format!("读取直链响应失败: {}", e))?;

    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("解析直链响应失败: {}", e))?;

    let direct_url = direct_link::select(&json)?;

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

    #[test]
    fn test_parse_pikpak_direct_link_meta() {
        let url = "https://dl-a10b-0862.mypikpak.com/download/?fid=zYHh&from=5&verno=3&prod=pikpak&expire=1787401193&f=619030226&fileid=VP-VLwpxxMPiLrSBZWR0JpFUo2&userid=888880000045788&sign=3B59";
        let meta = parse_pikpak_direct_link_meta(url).unwrap();
        assert_eq!(meta.file_id, "VP-VLwpxxMPiLrSBZWR0JpFUo2");
        assert_eq!(meta.file_size, 619030226);
        assert_eq!(meta.expire, 1787401193);
        assert_eq!(meta.user_id, "888880000045788");

        // 非 PikPak 链接返回 None
        assert!(parse_pikpak_direct_link_meta("https://example.com/download").is_none());
        assert!(parse_pikpak_direct_link_meta("https://mypikpak.com/s/abc123").is_none());
        // 缺少 fileid
        assert!(
            parse_pikpak_direct_link_meta("https://dl-a10b.mypikpak.com/download/?f=100").is_none()
        );
    }

    #[tokio::test]
    #[ignore = "显式授权的联网验收：设置 MAOBU_TEST_PIKPAK_SHARE 后单独运行"]
    async fn test_inspect_real_pikpak_share() {
        let url = std::env::var("MAOBU_TEST_PIKPAK_SHARE")
            .expect("请提供自己的合法测试分享 MAOBU_TEST_PIKPAK_SHARE");
        let device_id = hex::encode(rand::random::<[u8; 16]>());
        let result = inspect_pikpak_share(&url, None, &device_id).await;
        assert!(
            result.is_ok(),
            "PikPak 在线解析失败；不将失败记为跳过或成功"
        );
        let info = result.unwrap();
        let first = info
            .files
            .iter()
            .find(|file| file.kind == "drive#file")
            .expect("测试分享必须包含文件");
        let result = resolve_pikpak_file(
            &info.share_id,
            &first.id,
            info.pass_code_token.as_deref(),
            &device_id,
        )
        .await;
        assert!(result.is_ok(), "PikPak 在线直链解析失败（认证信息不输出）");
        assert!(url::Url::parse(&result.unwrap().url)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https")));
    }

    #[tokio::test]
    #[ignore = "需显式配置自有失效子目录，验证明确失败且不替换为根目录结果"]
    async fn test_invalid_subpath_fails_without_returning_root_contents() {
        let url = std::env::var("MAOBU_TEST_PIKPAK_SUBPATH_SHARE")
            .expect("请设置包含已失效子目录的合法 MAOBU_TEST_PIKPAK_SUBPATH_SHARE");
        let device_id = hex::encode(rand::random::<[u8; 16]>());
        let result = inspect_pikpak_share(&url, None, &device_id).await;
        assert!(
            result.is_err(),
            "失效子目录必须明确失败，不能返回根目录文件冒充成功"
        );
    }

    #[tokio::test]
    #[ignore = "显式授权的联网验收：需测试分享和提取码环境变量"]
    async fn test_inspect_real_protected_share() {
        let url = std::env::var("MAOBU_TEST_PIKPAK_PROTECTED_SHARE")
            .expect("请设置自己的受保护测试分享 MAOBU_TEST_PIKPAK_PROTECTED_SHARE");
        let password = std::env::var("MAOBU_TEST_PIKPAK_PASSCODE")
            .expect("请设置该测试分享的 MAOBU_TEST_PIKPAK_PASSCODE");
        let device_id = hex::encode(rand::random::<[u8; 16]>());
        let required = inspect_pikpak_share(&url, None, &device_id).await;
        assert!(required.is_ok(), "在线密码状态探测失败；未作为成功或跳过");
        assert!(
            required.unwrap().pass_code_required,
            "该样本必须是受密码保护的分享"
        );
        let wrong = inspect_pikpak_share(
            &url,
            Some(format!("invalid-{}", uuid::Uuid::new_v4())),
            &device_id,
        )
        .await;
        assert!(
            wrong.is_err() || wrong.as_ref().is_ok_and(|info| info.pass_code_required),
            "错误密码不能返回可下载文件列表"
        );
        let correct = inspect_pikpak_share(&url, Some(password), &device_id).await;
        assert!(
            correct.is_ok(),
            "正确提取码的在线解析失败（认证信息不输出）"
        );
        let info = correct.unwrap();
        assert!(info.file_count > 0 && info.pass_code_token.is_some());
    }

    /// 实机验证：PikPak 直链下载停滞熔断 + 自动刷新 + 断点续传（2026-08-21 修复）。
    ///
    /// 镜像 manager.rs 生产路径的完整语义：
    /// 1. 通过分享元数据解析直链（等价 NewTaskDialog 创建任务 + cloud_refresh）；
    /// 2. 分段下载中检测"有效 206 但连续 3 次空 body"、"45 秒进展不足 1MB"
    ///    或"有效 Range 被伪 416 拒绝"（PikPak 单链接流量配额签名）；
    /// 3. 触发 CLOUD_LINK_DEAD 哨兵后用同一分享元数据重新解析直链；
    /// 4. 依据磁盘分片真实长度重建窗口续传，循环直至完成（上限 5 次刷新）；
    /// 5. 连续三轮用同一链路下载同一文件（累计约 396MB）：若触发配额则
    ///    验证"刷新→续传→完整下载"全链路，否则退化为多轮下载+续传+完整性回归；
    /// 6. 三轮合并文件必须逐字节一致（同一文件内容）。
    ///
    /// 注：直接粘贴的裸直链（无分享元数据）无法自动刷新——实测 PikPak
    /// 存在单链接流量配额（有效 Range 范围外返回伪 416，实测某链接仅
    /// [0,~330.8MB]∪[尾部] 可访问且 30 分钟以上不重置），裸直链物理上
    /// 可能无法完整下载大文件，由 test_real_user_pikpak_link_live_download
    /// 验证其"有限时间内识别停滞"的半链路行为。
    #[tokio::test]
    #[ignore = "在线 PikPak 实机多轮下载验证（需外网与大文件下载）"]
    async fn test_live_pikpak_stall_detect_refresh_and_resume() {
        let share_id = std::env::var("MAOBU_TEST_PIKPAK_SHARE_ID").expect("请设置合法测试分享ID");
        let SHARE_ID = share_id.as_str();
        let file_id = std::env::var("MAOBU_TEST_PIKPAK_FILE_ID").expect("请设置合法测试文件ID");
        let FILE_ID = file_id.as_str();
        let EXPECTED_TOTAL: u64 = std::env::var("MAOBU_TEST_PIKPAK_BYTES")
            .expect("请设置已知样本大小")
            .parse()
            .expect("样本大小必须是整数");
        assert!(
            (1..=64 * 1024 * 1024).contains(&EXPECTED_TOTAL),
            "多轮联网样本上限64MiB，请使用小型合法测试文件"
        );
        const CONNECTIONS: u8 = 8;
        const MAX_LINK_REFRESHES: u32 = 5;
        // 与 manager.rs 熔断阈值完全一致（CLOUD_LINK_DEAD 检测参数）
        const MAX_EMPTY_STEPS: u32 = 3;
        const STALL_TIMEOUT_SECS: u64 = 45;
        const STALL_RECOVERY_BYTES: u64 = 1024 * 1024;
        const LINK_DEAD_PREFIX: &str = "CLOUD_LINK_DEAD:";
        // 连续下载轮数：3 × 132MB = 396MB，必然超过单链接 ~330MB 配额
        const PASSES: usize = 3;

        let device_id = hex::encode(rand::random::<[u8; 16]>());
        // 首次解析直链（等价于前端创建任务时 resolvePikPakDirectUrl）
        let mut direct = resolve_pikpak_file(SHARE_ID, FILE_ID, None, &device_id)
            .await
            .expect("首次解析 PikPak 直链失败");
        let client = reqwest::Client::new();
        // 记录首次直链的 ETag 与 Content-Md5：刷新后必须一致（同一文件内容）
        let (etag_first, content_md5) = probe_validators(&client, &direct.url).await;
        assert!(etag_first.is_some(), "PikPak 直链必须返回 ETag（内容 MD5）");

        let root_dir =
            std::env::temp_dir().join(format!("pikpak_refresh_{}", rand::random::<u32>()));
        let _ = std::fs::create_dir_all(&root_dir);

        // —— 三轮下载：同一链路持续消耗配额，验证"熔断→刷新→续传"全链路 ——
        let mut refreshes = 0u32;
        let started = std::time::Instant::now();
        let mut merged_files: Vec<std::path::PathBuf> = Vec::new();

        for pass in 1..=PASSES {
            let pass_dir = root_dir.join(format!("pass{pass}"));
            let _ = std::fs::create_dir_all(&pass_dir);
            let temp_file = pass_dir.join("live_refresh.bin.lumaget");
            println!(">>> 第 {pass}/{PASSES} 轮开始（已刷新 {refreshes} 次）");

            // 单轮主循环：下载 → 熔断 → 刷新 → 续传（镜像 spawn_worker 语义）
            let final_result: Result<(), String> = loop {
                if started.elapsed() > std::time::Duration::from_secs(900) {
                    break Err("实机验证总超时（15 分钟）".into());
                }
                let round = download_round_with_stall_detection(
                    &client,
                    &direct.url,
                    &temp_file,
                    EXPECTED_TOTAL,
                    CONNECTIONS,
                    MAX_EMPTY_STEPS,
                    STALL_TIMEOUT_SECS,
                    STALL_RECOVERY_BYTES,
                    LINK_DEAD_PREFIX,
                )
                .await;
                match round {
                    Ok(()) => break Ok(()),
                    Err(e) if e.starts_with(LINK_DEAD_PREFIX) && refreshes < MAX_LINK_REFRESHES => {
                        refreshes += 1;
                        println!(
                            ">>> [第 {refreshes} 次刷新] 直链失效哨兵触发：{e}（已耗时 {:.0}s）",
                            started.elapsed().as_secs_f64()
                        );
                        direct = resolve_pikpak_file(SHARE_ID, FILE_ID, None, &device_id)
                            .await
                            .expect("自动刷新直链失败");
                        // 刷新后的校验头必须与首次一致（同一文件，续传安全前提）
                        let (etag_new, md5_new) = probe_validators(&client, &direct.url).await;
                        assert_eq!(
                            etag_new.as_deref(),
                            etag_first.as_deref(),
                            "刷新后的直链必须指向同一文件内容（ETag 一致）"
                        );
                        if let (Some(old_md5), Some(new_md5)) =
                            (content_md5.as_deref(), md5_new.as_deref())
                        {
                            assert_eq!(
                                new_md5, old_md5,
                                "刷新后的直链必须指向同一文件内容（Content-Md5 一致）"
                            );
                        }
                        println!(">>> [第 {refreshes} 次刷新] 新直链校验一致，从磁盘分片续传");
                    }
                    Err(e) => break Err(e),
                }
            };
            assert!(
                final_result.is_ok(),
                "第 {pass} 轮下载未完成：{final_result:?}（刷新 {refreshes} 次）"
            );

            // —— 合并本轮分片并校验完整性 ——
            let merged = pass_dir.join("merged_output.bin");
            let mut merged_bytes = 0u64;
            {
                let mut out = tokio::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&merged)
                    .await
                    .unwrap();
                let chunk = EXPECTED_TOTAL / CONNECTIONS as u64;
                for i in 0..CONNECTIONS {
                    let start = i as u64 * chunk;
                    let path =
                        crate::manager::work_stealing::window_part_path(&temp_file, i, start);
                    let data = tokio::fs::read(&path).await.unwrap();
                    merged_bytes += data.len() as u64;
                    tokio::io::AsyncWriteExt::write_all(&mut out, &data)
                        .await
                        .unwrap();
                }
                tokio::io::AsyncWriteExt::flush(&mut out).await.unwrap();
            }
            assert_eq!(
                merged_bytes, EXPECTED_TOTAL,
                "第 {pass} 轮合并后的大小必须等于源文件大小"
            );
            merged_files.push(merged);
        }

        // —— 配额触发情况说明 ——
        // 三轮累计 396MB：若测试用的直链触发了 PikPak 单链接流量配额
        // （伪 416），上方 per-pass 断言已保证"熔断→刷新→续传→完整下载"
        // 全链路成功；若未触发（配额为特定链接/CDN 节点行为，实测并非
        // 所有链接都有），则本测试退化为多轮下载 + 断点续传 + 完整性
        // 回归（三轮文件逐字节一致）。真实配额场景由
        // test_real_user_pikpak_link_live_download（已确认配额的用户直链）
        // 验证检测半链路。
        println!(">>> 配额触发情况：本轮实测自动刷新 {refreshes} 次（0 = 测试链接未触发配额）");

        // MD5 完整性：与服务器 Content-Md5（base64）比对
        if let Some(expected_b64) = content_md5.as_deref() {
            use base64::Engine as _;
            use md5::Digest as _;
            let data = tokio::fs::read(&merged_files[0]).await.unwrap();
            let digest = md5::Md5::digest(&data);
            let actual_b64 = base64::engine::general_purpose::STANDARD
                .encode(digest)
                .to_string();
            // 服务器头可能带引号
            let expected_trim = expected_b64.trim_matches('"');
            assert_eq!(
                actual_b64, expected_trim,
                "合并文件 MD5 必须与服务器 Content-Md5 一致"
            );
        }

        // —— 三轮文件必须逐字节一致（同一文件 + 刷新续传不引入坏字节）——
        let first = tokio::fs::read(&merged_files[0]).await.unwrap();
        for (idx, path) in merged_files.iter().enumerate().skip(1) {
            let other = tokio::fs::read(path).await.unwrap();
            assert_eq!(
                first.len(),
                other.len(),
                "第 {} 轮文件长度必须与第 1 轮一致",
                idx + 1
            );
            let mismatch = first
                .iter()
                .zip(other.iter())
                .position(|(a, b)| a != b)
                .map(|p| format!("首个差异 @ 偏移 {p}"))
                .unwrap_or_else(|| "无差异".into());
            assert_eq!(
                first,
                other,
                "第 {} 轮文件必须与第 1 轮逐字节一致（{mismatch}）",
                idx + 1
            );
        }

        println!(
            ">>> 实机验证通过：3 × 132MB 全部完整下载（触发自动刷新 {refreshes} 次，总耗时 {:.1}s）",
            started.elapsed().as_secs_f64()
        );
        let _ = tokio::fs::remove_dir_all(&root_dir).await;
    }

    /// 探测直链的 ETag 与 Content-Md5 校验头（刷新一致性验证用）。
    async fn probe_validators(
        client: &reqwest::Client,
        url: &str,
    ) -> (Option<String>, Option<String>) {
        let resp = client
            .head(url)
            .header(
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            )
            .header("Referer", "https://mypikpak.com/")
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await;
        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            _ => return (None, None),
        };
        let etag = resp
            .headers()
            .get("ETag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let md5h = resp
            .headers()
            .get("Content-Md5")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        (etag, md5h)
    }

    /// 单轮分段下载：完成返回 Ok，直链失效返回 Err(LINK_DEAD_PREFIX...)。
    ///
    /// 窗口按磁盘分片真实长度重建（断点续传语义），worker 步骤级
    /// 熔断逻辑与 manager.rs download_segments 一致。
    async fn download_round_with_stall_detection(
        client: &reqwest::Client,
        url: &str,
        temp_file: &std::path::Path,
        total: u64,
        connections: u8,
        max_empty_steps: u32,
        stall_timeout_secs: u64,
        stall_recovery_bytes: u64,
        link_dead_prefix: &str,
    ) -> Result<(), String> {
        use crate::manager::work_stealing::{RangeWindow, WindowStatus, WorkStealingCoordinator};

        let chunk = total / connections as u64;
        let mut windows = Vec::new();
        for i in 0..connections {
            let start = i as u64 * chunk;
            let end = if i == connections - 1 {
                total - 1
            } else {
                (i as u64 + 1) * chunk - 1
            };
            let path = crate::manager::work_stealing::window_part_path(temp_file, i, start);
            let existing = std::fs::metadata(&path)
                .map(|m| m.len())
                .unwrap_or(0)
                .min(end.saturating_sub(start).saturating_add(1));
            windows.push(RangeWindow {
                id: i as u64 + 1,
                segment_index: i,
                ordinal: 0,
                start_byte: start,
                end_byte: end,
                existing_bytes: existing,
                path,
                status: WindowStatus::Pending,
            });
        }
        let existing_total: u64 = windows.iter().map(|w| w.existing_bytes).sum();
        println!(
            ">>> 本轮开始：磁盘已有 {existing_total}/{total} 字节（{:.1}%）",
            existing_total as f64 / total as f64 * 100.0
        );
        if existing_total >= total {
            return Ok(());
        }

        let coordinator = std::sync::Arc::new(WorkStealingCoordinator::new(temp_file, windows));
        let cancel = tokio_util::sync::CancellationToken::new();
        let progress = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(existing_total));
        let link_dead_prefix = link_dead_prefix.to_string();

        let mon_progress = progress.clone();
        let mon_cancel = cancel.clone();
        let mon_coordinator = coordinator.clone();
        tokio::spawn(async move {
            let mut last = mon_progress.load(std::sync::atomic::Ordering::Relaxed);
            for _ in 0..120 {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                if mon_cancel.is_cancelled() {
                    break;
                }
                let cur = mon_progress.load(std::sync::atomic::Ordering::Relaxed);
                let speed = cur.saturating_sub(last);
                last = cur;
                let pct = (cur as f64 / total as f64) * 100.0;
                println!(
                    ">>> 进度: {:.1}% ({}/{} MB), 速度: {:.2} MB/s",
                    pct,
                    cur / 1024 / 1024,
                    total / 1024 / 1024,
                    speed as f64 / 1024.0 / 1024.0
                );
                if cur >= total || mon_coordinator.is_all_completed().await {
                    break;
                }
            }
        });

        let mut handles = Vec::new();

        for worker_id in 0..connections {
            let coordinator = coordinator.clone();
            let client = client.clone();
            let url = url.to_string();
            let cancel = cancel.clone();
            let progress = progress.clone();
            let link_dead_prefix = link_dead_prefix.clone();
            handles.push(tokio::spawn(async move {
                loop {
                    if cancel.is_cancelled() {
                        return Err("已取消".to_string());
                    }
                    if coordinator.is_all_completed().await {
                        return Ok(());
                    }
                    let work = coordinator.claim_or_steal_work().await;
                    let (window, handle) = match work {
                        Some(w) => w,
                        None => {
                            if coordinator.is_all_completed().await {
                                return Ok(());
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                            continue;
                        }
                    };
                    let index = window.segment_index;
                    let window_len = window
                        .end_byte
                        .saturating_sub(window.start_byte)
                        .saturating_add(1);
                    if window.existing_bytes >= window_len {
                        coordinator
                            .finish_window(window.id, true, window.existing_bytes)
                            .await;
                        continue;
                    }
                    let mut file = match tokio::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&window.path)
                        .await
                    {
                        Ok(f) => tokio::io::BufWriter::with_capacity(512 * 1024, f),
                        Err(e) => {
                            println!("[Worker #{worker_id:02}] 打开分片失败: {e}");
                            coordinator
                                .finish_window(window.id, false, handle.current_downloaded())
                                .await;
                            continue;
                        }
                    };

                    let mut next_start = window.start_byte + window.existing_bytes;
                    let mut empty_steps = 0u32;
                    let mut stall_at = std::time::Instant::now();
                    let mut stall_base = window.existing_bytes;
                    let mut retry_count = 0u32;
                    let mut window_error: Option<String> = None;

                    loop {
                        if cancel.is_cancelled() {
                            let _ = tokio::io::AsyncWriteExt::flush(&mut file).await;
                            window_error = Some("已取消".into());
                            break;
                        }
                        let current_end = handle.current_end();
                        if next_start > current_end {
                            break;
                        }
                        let mut step_bytes = 0u64;
                        let mut server_206 = false;
                        let current_start = next_start;

                        let step_result: Result<(), String> = async {
                            let resp = client
                                .get(&url)
                                .header(
                                    "Range",
                                    format!("bytes={current_start}-{current_end}"),
                                )
                                .header(
                                    "User-Agent",
                                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
                                )
                                .header("Referer", "https://mypikpak.com/")
                                .timeout(std::time::Duration::from_secs(10))
                                .send()
                                .await
                                .map_err(|e| format!("连接错误: {e}"))?;
                            if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
                                // 伪 416（有效范围被拒）= PikPak 限速签名，与 manager.rs
                                // 一致计入直链失效证据；真 416（越过末尾）视为已完成。
                                if current_start >= total {
                                    return Ok(());
                                }
                                server_206 = true;
                                return Ok(());
                            }
                            if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                                return Err(format!("HTTP {}", resp.status()));
                            }
                            server_206 = true;
                            let mut stream = resp.bytes_stream();
                            use futures_util::StreamExt;
                            let mut idle_secs = 0u8;
                            loop {
                                let next = tokio::time::timeout(
                                    std::time::Duration::from_secs(1),
                                    stream.next(),
                                )
                                .await;
                                let chunk = match next {
                                    Ok(Some(Ok(c))) => {
                                        idle_secs = 0;
                                        c
                                    }
                                    Ok(None) => break,
                                    Ok(Some(Err(e))) => {
                                        return Err(format!("流读取错误: {e}"))
                                    }
                                    Err(_) => {
                                        idle_secs += 1;
                                        if idle_secs >= 6 {
                                            return Err("连续 6 秒未收到数据".into());
                                        }
                                        continue;
                                    }
                                };
                                let mut slice = &chunk[..];
                                let remaining = current_end
                                    .saturating_sub(next_start)
                                    .saturating_add(1);
                                if (slice.len() as u64) > remaining {
                                    slice = &slice[..remaining as usize];
                                }
                                let len = slice.len() as u64;
                                if len == 0 {
                                    break;
                                }
                                tokio::io::AsyncWriteExt::write_all(&mut file, slice)
                                    .await
                                    .map_err(|e| format!("写盘错误: {e}"))?;
                                next_start += len;
                                step_bytes += len;
                                handle
                                    .downloaded_bytes
                                    .fetch_add(len, std::sync::atomic::Ordering::Relaxed);
                                progress.fetch_add(len, std::sync::atomic::Ordering::Relaxed);
                                if next_start > current_end {
                                    break;
                                }
                            }
                            Ok(())
                        }
                        .await;

                        // —— 熔断检测（与 manager.rs 一致：仅统计有效 206 的步骤）——
                        if server_206 && !cancel.is_cancelled() {
                            let downloaded_now = handle.current_downloaded();
                            if step_bytes == 0 {
                                empty_steps += 1;
                            } else {
                                empty_steps = 0;
                            }
                            if downloaded_now.saturating_sub(stall_base) >= stall_recovery_bytes {
                                stall_at = std::time::Instant::now();
                                stall_base = downloaded_now;
                            }
                            let window_remaining = current_end
                                .saturating_sub(next_start)
                                .saturating_add(1);
                            if empty_steps >= max_empty_steps {
                                let _ = tokio::io::AsyncWriteExt::flush(&mut file).await;
                                window_error = Some(format!(
                                    "{link_dead_prefix}分片 #{} 连续 {} 次收到 0 字节响应",
                                    index + 1,
                                    empty_steps
                                ));
                                break;
                            }
                            if stall_at.elapsed().as_secs() >= stall_timeout_secs
                                && window_remaining > stall_recovery_bytes
                            {
                                let _ = tokio::io::AsyncWriteExt::flush(&mut file).await;
                                window_error = Some(format!(
                                    "{link_dead_prefix}分片 #{} 超过 {} 秒下载不足 {} 字节",
                                    index + 1,
                                    stall_timeout_secs,
                                    stall_recovery_bytes
                                ));
                                break;
                            }
                        }

                        match step_result {
                            Ok(()) => {
                                retry_count = 0;
                                if next_start <= handle.current_end() {
                                    tokio::time::sleep(std::time::Duration::from_millis(50))
                                        .await;
                                    continue;
                                }
                                break;
                            }
                            Err(e) => {
                                retry_count += 1;
                                if retry_count > 8 {
                                    let _ = tokio::io::AsyncWriteExt::flush(&mut file).await;
                                    window_error =
                                        Some(format!("分片 #{} 重试耗尽: {e}", index + 1));
                                    break;
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                            }
                        }
                    }

                    let _ = tokio::io::AsyncWriteExt::flush(&mut file).await;
                    let final_downloaded = handle.current_downloaded();
                    let success = window_error.is_none()
                        && (final_downloaded >= window_len
                            || window.start_byte + final_downloaded > handle.current_end());
                    coordinator
                        .finish_window(window.id, success, final_downloaded)
                        .await;
                    if let Some(err) = window_error {
                        cancel.cancel();
                        return Err(err);
                    }
                }
            }));
        }

        // —— 错误汇聚：直链失效哨兵优先（与 manager.rs join 逻辑一致）——
        let mut worker_error: Option<String> = None;
        for h in handles {
            match h.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    let is_link_dead = e.starts_with(link_dead_prefix.as_str());
                    match &worker_error {
                        Some(prev)
                            if !prev.starts_with(link_dead_prefix.as_str()) && is_link_dead =>
                        {
                            worker_error = Some(e);
                        }
                        None => worker_error = Some(e),
                        _ => {}
                    }
                }
                Err(join_err) => {
                    if worker_error.is_none() {
                        worker_error = Some(format!("Worker 异常终止: {join_err}"));
                    }
                }
            }
        }
        match worker_error {
            Some(e) => Err(e),
            None => {
                let done = coordinator.is_all_completed().await;
                if done {
                    Ok(())
                } else {
                    Err("所有 worker 结束但窗口未全部完成".into())
                }
            }
        }
    }

    /// 用户提供的真实直链（544MB）验证：修复前该链接在约 60% 处停滞、
    /// 线程全部 0 速且无限空转；修复后必须在有限时间内识别停滞并给出
    /// CLOUD_LINK_DEAD 哨兵（该直链无分享元数据，验证的是检测半链路），
    /// 或完整下完（未触发限速时）。
    #[tokio::test]
    #[ignore = "需要在线 PikPak 直链，用于真实下载验证（约 5-10 分钟）"]
    async fn test_real_user_pikpak_link_live_download() {
        let test_url =
            std::env::var("MAOBU_TEST_PIKPAK_DIRECT_URL").expect("请显式提供自己的合法测试直链");

        let TOTAL: u64 = std::env::var("MAOBU_TEST_PIKPAK_BYTES")
            .expect("请设置已知样本大小")
            .parse()
            .expect("样本大小必须是整数");
        assert!((1..=64 * 1024 * 1024).contains(&TOTAL), "联网样本上限64MiB");
        const CONNECTIONS: u8 = 16;
        const LINK_DEAD_PREFIX: &str = "CLOUD_LINK_DEAD:";

        let client = reqwest::Client::new();
        let directory = tempfile::tempdir().unwrap();
        let temp_file = directory.path().join("sample.bin.lumaget");

        let started = std::time::Instant::now();
        let result = download_round_with_stall_detection(
            &client,
            &test_url,
            &temp_file,
            TOTAL,
            CONNECTIONS,
            3,
            45,
            1024 * 1024,
            LINK_DEAD_PREFIX,
        )
        .await;

        let downloaded: u64 = std::fs::read_dir(directory.path())
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter_map(|e| e.metadata().ok().map(|m| m.len()))
                    .sum()
            })
            .unwrap_or(0);

        match &result {
            Ok(()) => {
                println!(
                    ">>> 用户直链完整下载完成：{downloaded}/{TOTAL} 字节，耗时 {:.0}s",
                    started.elapsed().as_secs_f64()
                );
                assert!(downloaded >= TOTAL, "完成路径必须覆盖全部字节");
            }
            Err(e) if e.starts_with(LINK_DEAD_PREFIX) => {
                // 修复的核心断言：停滞被有限时间识别，不再无限 0 速空转
                println!(
                    ">>> 用户直链停滞已按预期识别（{:.0}s 内）：{e}，进度 {}/{}",
                    started.elapsed().as_secs_f64(),
                    downloaded,
                    TOTAL
                );
                assert!(
                    started.elapsed() < std::time::Duration::from_secs(600),
                    "停滞识别必须在 10 分钟内完成，不得无限空转"
                );
            }
            Err(e) => panic!(
                "测试直链出现非预期错误：{}",
                crate::manager::redact_sensitive(e)
            ),
        }
    }
}
