use super::*;
use crate::cloud_request::CloudRequestContext;
use serde_json::Value;
use std::collections::HashMap;

fn normalize_cookie_header_for_url(cookie_header: &str, target_url: &str) -> String {
    let trimmed = cookie_header.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if crate::media_cookies::is_netscape_format(trimmed) {
        return crate::media_cookies::header_for_url(trimmed, target_url, target_url);
    }
    crate::media_cookies::parse_cookie_header(trimmed)
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

pub(super) fn has_valid_bduss(cookie_header: &str) -> bool {
    let normalized = normalize_cookie_header_for_url(cookie_header, BAIDU_API_HOST);
    crate::media_cookies::parse_cookie_header(&normalized)
        .into_iter()
        .any(|(name, value)| name == "BDUSS" && !value.trim().is_empty())
}

pub(super) async fn resolve_baidu_file_impl(
    surl: &str,
    fs_id: &str,
    share_id: Option<&str>,
    uk: Option<&str>,
    sign: Option<&str>,
    timestamp: Option<i64>,
    seckey: Option<&str>,
    randsk: Option<&str>,
    cookie: Option<&str>,
    network: &CloudRequestContext,
) -> Result<BaiduDirectUrlResult, String> {
    let clean_surl = surl.strip_prefix('1').unwrap_or(surl);
    let share_meta = resolve_share_meta(
        clean_surl, share_id, uk, sign, timestamp, seckey, randsk, cookie, network,
    )
    .await;

    let cookie_header = match share_meta
        .randsk
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        Some(value) => crate::cloud_request::with_cookie_pair(
            cookie.unwrap_or(""),
            "BDCLND",
            value,
            BAIDU_API_HOST,
        )?,
        None => cookie.unwrap_or("").to_owned(),
    };

    let seckey_clean = share_meta
        .randsk
        .as_deref()
        .map(url_decode)
        .or_else(|| share_meta.seckey.clone())
        .unwrap_or_default();
    let referer = format!("{}/s/1{}", BAIDU_API_HOST, clean_surl);

    let mut sharedownload_hint: Option<String> = None;
    if let (Some(sid), Some(u)) = (&share_meta.share_id, &share_meta.uk) {
        match try_sharedownload(
            network,
            sid,
            u,
            fs_id,
            share_meta.sign.as_deref(),
            share_meta.timestamp,
            &seckey_clean,
            &cookie_header,
            &referer,
        )
        .await
        {
            SharedownloadOutcome::Found(result) => return Ok(result),
            SharedownloadOutcome::RetryTransfer(hint) => sharedownload_hint = hint,
        }
    }

    let has_bduss = has_valid_bduss(&cookie_header);
    if has_bduss {
        if let (Some(sid), Some(u)) = (&share_meta.share_id, &share_meta.uk) {
            let transferred = transfer_to_netdisk(
                network,
                sid,
                u,
                fs_id,
                &seckey_clean,
                &cookie_header,
                &referer,
            )
            .await?;
            return fetch_transfer_dlink(
                network,
                &transferred.fs_id,
                transferred.path,
                &cookie_header,
            )
            .await;
        }
    }

    if let Some(hint) = sharedownload_hint {
        return Err(hint);
    }

    if !has_bduss {
        Err(
            "该百度网盘文件需要登录凭证，请在设置中添加或在浏览器中登录百度网盘并同步 Cookie"
                .to_string(),
        )
    } else {
        Err(
            "获取百度网盘下载直链失败，请确认提取码是否正确或刷新浏览器中的百度网盘登录态后重试"
                .to_string(),
        )
    }
}

struct ShareMeta {
    share_id: Option<String>,
    uk: Option<String>,
    sign: Option<String>,
    timestamp: Option<i64>,
    seckey: Option<String>,
    randsk: Option<String>,
}

async fn resolve_share_meta(
    clean_surl: &str,
    share_id: Option<&str>,
    uk: Option<&str>,
    sign: Option<&str>,
    timestamp: Option<i64>,
    seckey: Option<&str>,
    randsk: Option<&str>,
    cookie: Option<&str>,
    network: &CloudRequestContext,
) -> ShareMeta {
    let mut meta = ShareMeta {
        share_id: share_id.map(str::to_owned),
        uk: uk.map(str::to_owned),
        sign: sign.map(str::to_owned),
        timestamp,
        seckey: seckey.map(str::to_owned),
        randsk: randsk.map(str::to_owned),
    };
    if meta.share_id.is_some() && meta.uk.is_some() {
        return meta;
    }
    if let Ok(info) = inspect_baidu_share_with_context(
        &format!("{}/s/1{}", BAIDU_API_HOST, clean_surl),
        None,
        cookie,
        network,
    )
    .await
    {
        if meta.share_id.is_none() {
            meta.share_id = info.share_id;
        }
        if meta.uk.is_none() {
            meta.uk = info.uk;
        }
        if meta.sign.is_none() {
            meta.sign = info.sign;
        }
        if meta.timestamp.is_none() {
            meta.timestamp = info.timestamp;
        }
        if meta.seckey.is_none() {
            meta.seckey = info.seckey;
        }
        if meta.randsk.is_none() {
            meta.randsk = info.randsk;
        }
    }
    meta
}

enum SharedownloadOutcome {
    Found(BaiduDirectUrlResult),
    RetryTransfer(Option<String>),
}

#[allow(clippy::too_many_arguments)]
async fn try_sharedownload(
    network: &CloudRequestContext,
    share_id: &str,
    uk: &str,
    fs_id: &str,
    sign: Option<&str>,
    timestamp: Option<i64>,
    seckey_clean: &str,
    cookie_header: &str,
    referer: &str,
) -> SharedownloadOutcome {
    let mut download_api = format!(
        "{}/api/sharedownload?app_id=250528&channel=chunlei&clienttype=0&web=1&encrypt=0&product=share",
        BAIDU_API_HOST
    );
    if let (Some(s), Some(ts)) = (sign, timestamp) {
        download_api.push_str(&format!("&sign={}&timestamp={}", s, ts));
    }

    let extra_json = if !seckey_clean.is_empty() {
        serde_json::json!({ "seckey": seckey_clean }).to_string()
    } else {
        "{}".to_string()
    };

    let fid_list = format!("[{}]", fs_id);
    let params = [
        ("uk", uk),
        ("primaryid", share_id),
        ("fid_list", fid_list.as_str()),
        ("extra", extra_json.as_str()),
    ];

    let Ok(resp) = network
        .post(&download_api)
        .form(&params)
        .header("Cookie", cookie_header)
        .header("Referer", referer)
        .header("User-Agent", BAIDU_USER_AGENT)
        .send()
        .await
    else {
        return SharedownloadOutcome::RetryTransfer(None);
    };

    let Ok(text) = network.text(resp).await else {
        return SharedownloadOutcome::RetryTransfer(None);
    };
    let json: Value = serde_json::from_str(&text).unwrap_or_default();
    let errno = json.get("errno").and_then(|v| v.as_i64());
    if errno == Some(0) {
        if let Some(dlink) = json
            .get("list")
            .and_then(|v| v.as_array())
            .and_then(|list| list.first())
            .and_then(|item| item.get("dlink"))
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
        {
            return SharedownloadOutcome::Found(build_dlink_result(
                dlink,
                dlink_user_agent(dlink),
                cookie_header,
                None,
            ));
        }
    }

    SharedownloadOutcome::RetryTransfer(errno.and_then(sharedownload_error_hint))
}

fn sharedownload_error_hint(errno: i64) -> Option<String> {
    match errno {
        -20 => Some(
            "百度网盘免登录直链触发验证码保护（errno=-20），请提供已登录的百度网盘 Cookie 后重试，或在浏览器打开分享页点击下载由扩展接管"
                .into(),
        ),
        113 => Some(
            "百度网盘免登录直链校验未通过（errno=113），请先在浏览器登录 pan.baidu.com 并在扩展弹窗「登录态下载 → 同步到媒体凭证」后重试"
                .into(),
        ),
        112 => Some("百度网盘分享页面签名已过期（errno=112），请重新解析分享链接后再试".into()),
        110 => Some("百度网盘接口访问过于频繁（errno=110），请稍后重试或配置登录 Cookie".into()),
        -9 => Some("百度网盘分享提取码无效（errno=-9），请检查提取码".into()),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub(super) struct TransferredFile {
    pub(super) fs_id: String,
    pub(super) path: Option<String>,
}

async fn transfer_to_netdisk(
    network: &CloudRequestContext,
    share_id: &str,
    uk: &str,
    fs_id: &str,
    seckey_clean: &str,
    cookie_header: &str,
    referer: &str,
) -> Result<TransferredFile, String> {
    let transfer_url = format!(
        "{}/share/transfer?shareid={}&from={}&ondup=newcopy&async=0&channel=chunlei&clienttype=0&web=1&app_id=250528",
        BAIDU_API_HOST, share_id, uk
    );
    let fs_id_list_str = format!("[{}]", fs_id);
    let mut transfer_params = vec![
        ("fsidlist", fs_id_list_str.as_str()),
        ("path", "/"),
    ];
    if !seckey_clean.is_empty() {
        transfer_params.push(("seckey", seckey_clean));
    }

    let resp = network
        .post(&transfer_url)
        .form(&transfer_params)
        .header("Cookie", cookie_header)
        .header("Referer", referer)
        .header("Origin", BAIDU_API_HOST)
        .header("User-Agent", BAIDU_USER_AGENT)
        .send()
        .await
        .map_err(|e| format!("百度网盘转存请求失败: {e}"))?;
    let text = network.text(resp).await?;
    let json: Value = serde_json::from_str(&text).unwrap_or_default();
    parse_transfer_response(&json, fs_id)
}

pub(super) fn parse_transfer_response(
    json: &Value,
    original_fs_id: &str,
) -> Result<TransferredFile, String> {
    let errno = json.get("errno").and_then(|v| v.as_i64()).unwrap_or(-1);
    tracing::debug!(baidu_errno = errno, "百度转存请求已返回");
    if let Some(msg) = transfer_error_message(errno) {
        return Err(msg);
    }

    let extracted_fs_id = extract_transferred_fs_id(json);
    let resolved_path = extract_transferred_path(json);
    if errno == 12 && extracted_fs_id.is_none() && resolved_path.is_none() {
        return Err(
            "网盘根目录可能已存在同名文件（errno=12），未能定位转存目标，请清理同名文件后重试"
                .into(),
        );
    }
    Ok(TransferredFile {
        fs_id: extracted_fs_id.unwrap_or_else(|| original_fs_id.to_owned()),
        path: resolved_path,
    })
}

fn transfer_error_message(errno: i64) -> Option<String> {
    match errno {
        0 | 12 => None,
        111 => Some("百度网盘当前有其他转存任务正在执行（errno=111），请稍后重试".into()),
        -6 => Some("百度网盘登录凭证已失效（errno=-6），请在设置或浏览器中重新同步 Cookie".into()),
        -7 => Some("转存失败：文件名或路径包含不支持的字符（errno=-7）".into()),
        -9 => Some("提取码校验失败（errno=-9），请重新输入 4 位提取码".into()),
        -10 | 110 => Some("网盘可用空间不足或超出单次转存上限，请清理网盘空间后重试".into()),
        -30 => Some("网盘根目录已存在同名文件（errno=-30），请清理或重命名后重试".into()),
        other => Some(format!("百度网盘转存文件失败（代码 {other}），请检查账号权限")),
    }
}

fn extract_transferred_fs_id(json: &Value) -> Option<String> {
    let item = json
        .get("extra")
        .and_then(|extra| extra.get("list"))
        .and_then(|l| l.as_array())
        .and_then(|arr| arr.first())?;
    let value = item.get("to_fs_id").or_else(|| item.get("fs_id"))?;
    if let Some(s) = value.as_str().filter(|s| !s.is_empty()) {
        Some(s.to_string())
    } else if let Some(n) = value.as_i64() {
        Some(n.to_string())
    } else {
        value.as_u64().map(|u| u.to_string())
    }
}

fn extract_transferred_path(json: &Value) -> Option<String> {
    if let Some(item) = json
        .get("extra")
        .and_then(|extra| extra.get("list"))
        .and_then(|l| l.as_array())
        .and_then(|arr| arr.first())
    {
        if let Some(p) = item
            .get("to")
            .or_else(|| item.get("path"))
            .or_else(|| item.get("to_path"))
            .and_then(|p| p.as_str())
            .filter(|s| !s.is_empty())
        {
            return Some(p.to_string());
        }
        if let Some(name) = item
            .get("to_server_filename")
            .or_else(|| item.get("server_filename"))
            .and_then(|n| n.as_str())
            .filter(|s| !s.is_empty())
        {
            return Some(format!("/{}", name));
        }
    }
    json.get("info")
        .and_then(|i| i.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("path"))
        .and_then(|p| p.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

async fn fetch_transfer_dlink(
    network: &CloudRequestContext,
    target_fs_id: &str,
    mut resolved_path: Option<String>,
    cookie_header: &str,
) -> Result<BaiduDirectUrlResult, String> {
    let fsids_param = format!("[{}]", target_fs_id);

    if resolved_path.is_none() {
        if let Some(found) =
            lookup_openapi_metadata(network, &fsids_param, &mut resolved_path, cookie_header).await
        {
            return Ok(found);
        }
    }

    if let Some(found) = lookup_web_filemetas(network, &fsids_param, cookie_header).await {
        return Ok(found);
    }

    if let Some(ref path_str) = resolved_path {
        if let Some(found) = lookup_locatedownload(network, path_str, cookie_header).await {
            return Ok(found);
        }
        if let Some(found) = lookup_pcs_direct(network, path_str, cookie_header).await {
            return Ok(found);
        }
    }

    Err(format!(
        "文件已成功转存至网盘（fs_id: {}），但提取直链地址失败，请检查账号状态或稍后重试",
        target_fs_id
    ))
}

async fn lookup_openapi_metadata(
    network: &CloudRequestContext,
    fsids_param: &str,
    resolved_path: &mut Option<String>,
    cookie_header: &str,
) -> Option<BaiduDirectUrlResult> {
    let meta_url = format!("{}/rest/2.0/xpan/multimedia?method=filemetas&dlink=1", BAIDU_API_HOST);
    let resp = network
        .post(&meta_url)
        .form(&[("fsids", fsids_param)])
        .header("Cookie", cookie_header)
        .header("User-Agent", BAIDU_DLINK_USER_AGENT)
        .send()
        .await
        .ok()?;
    let text = network.text(resp).await.ok()?;
    let json: Value = serde_json::from_str(&text).unwrap_or_default();
    let item = json
        .get("list")
        .and_then(|v| v.as_array())
        .and_then(|v| v.first())?;
    if let Some(p) = item.get("path").and_then(|p| p.as_str()).filter(|s| !s.is_empty()) {
        *resolved_path = Some(p.to_string());
    }
    let dlink = item.get("dlink").and_then(|v| v.as_str()).filter(|s| !s.is_empty())?;
    Some(build_dlink_result(
        dlink,
        dlink_user_agent(dlink),
        cookie_header,
        None,
    ))
}

pub(super) fn extract_filemetas_item(json: &Value) -> Option<&Value> {
    json.get("info")
        .and_then(|v| v.as_array())
        .and_then(|v| v.first())
        .or_else(|| {
            json.get("list")
                .and_then(|v| v.as_array())
                .and_then(|v| v.first())
        })
}

fn is_live_baidu_cdn_url(raw_url: &str) -> bool {
    url::Url::parse(raw_url).is_ok_and(|u| {
        matches!(u.scheme(), "http" | "https")
            && u.host_str().is_some_and(|h| {
                h == "baidu.com"
                    || h.ends_with(".baidu.com")
                    || h == "baidupcs.com"
                    || h.ends_with(".baidupcs.com")
            })
    })
}

async fn probe_baidu_download_candidate(
    network: &CloudRequestContext,
    url: &str,
    ua: &str,
    cookie_header: &str,
    referer: Option<&str>,
) -> bool {
    let target_origin = url::Url::parse(url)
        .ok()
        .map(|u| format!("{}://{}", u.scheme(), u.host_str().unwrap_or_default()))
        .unwrap_or_else(|| BAIDU_API_HOST.to_string());
    let probe_net = if crate::media_cookies::is_netscape_format(cookie_header) {
        network.clone()
    } else {
        network.with_cookie_source(&target_origin)
    };
    let mut req = probe_net
        .get(url)
        .header("User-Agent", ua)
        .header("Accept-Encoding", "identity")
        .header("Range", "bytes=0-0")
        .timeout(std::time::Duration::from_secs(5));
    if !cookie_header.is_empty() {
        req = req.header("Cookie", cookie_header);
    }
    if let Some(r) = referer {
        req = req.header("Referer", r);
    }
    let Ok(resp) = req.send().await else {
        return false;
    };
    let status = resp.status();
    if !(status == reqwest::StatusCode::PARTIAL_CONTENT || status.is_success()) {
        return false;
    }
    resp.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(is_browser_download_content_type)
}

async fn select_working_dlink(
    network: &CloudRequestContext,
    url: &str,
    preferred_ua: &str,
    cookie_header: &str,
    referer: Option<&str>,
) -> Option<BaiduDirectUrlResult> {
    if !is_live_baidu_cdn_url(url) {
        return Some(build_dlink_result(url, preferred_ua, cookie_header, referer));
    }
    let is_cdn_edge = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .is_some_and(|h| h == "baidupcs.com" || h.ends_with(".baidupcs.com"));

    let mut uas = vec![
        preferred_ua,
        "pan.baidu.com",
        BAIDU_USER_AGENT,
        "netdisk;P2SP;2.2.90.43;WindowsBaiduYunGuanJia",
    ];
    uas.dedup();
    let cookie_modes: &[bool] = if is_cdn_edge { &[false, true] } else { &[true] };

    for &with_cookie in cookie_modes {
        let test_cookie = if with_cookie { cookie_header } else { "" };
        for &ua in &uas {
            if probe_baidu_download_candidate(network, url, ua, test_cookie, referer).await {
                return Some(build_dlink_result(url, ua, test_cookie, referer));
            }
        }
    }
    None
}

async fn lookup_web_filemetas(
    network: &CloudRequestContext,
    fsids_param: &str,
    cookie_header: &str,
) -> Option<BaiduDirectUrlResult> {
    let encoded_fsids = url_encode(fsids_param);
    let get_url = format!(
        "https://pan.baidu.com/api/filemetas?dlink=1&app_id=250528&channel=chunlei&clienttype=0&web=1&fsids={}",
        encoded_fsids
    );
    let post_urls = [
        get_url.clone(),
        format!(
            "{}/rest/2.0/xpan/multimedia?method=filemetas&dlink=1&channel=chunlei&web=1",
            BAIDU_API_HOST
        ),
    ];
    for ua in &["pan.baidu.com", BAIDU_USER_AGENT, BAIDU_DLINK_USER_AGENT] {
        if let Ok(resp) = network
            .get(&get_url)
            .header("Cookie", cookie_header)
            .header("User-Agent", *ua)
            .header("Referer", "https://pan.baidu.com/disk/home")
            .send()
            .await
        {
            if let Ok(text) = network.text(resp).await {
                let json: Value = serde_json::from_str(&text).unwrap_or_default();
                if let Some(dlink) = extract_filemetas_item(&json)
                    .and_then(|item| item.get("dlink"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    if let Some(found) = select_working_dlink(
                        network,
                        dlink,
                        dlink_user_agent(dlink),
                        cookie_header,
                        Some("https://pan.baidu.com/disk/home"),
                    )
                    .await
                    {
                        return Some(found);
                    }
                }
            }
        }
    }
    for fm_url in &post_urls {
        for ua in &["pan.baidu.com", BAIDU_DLINK_USER_AGENT, BAIDU_USER_AGENT] {
            let Ok(resp) = network
                .post(fm_url)
                .form(&[("fsids", fsids_param)])
                .header("Cookie", cookie_header)
                .header("User-Agent", *ua)
                .header("Referer", "https://pan.baidu.com/disk/home")
                .send()
                .await
            else {
                continue;
            };
            let Ok(text) = network.text(resp).await else {
                continue;
            };
            let json: Value = serde_json::from_str(&text).unwrap_or_default();
            if let Some(dlink) = extract_filemetas_item(&json)
                .and_then(|item| item.get("dlink"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                if let Some(found) = select_working_dlink(
                    network,
                    dlink,
                    dlink_user_agent(dlink),
                    cookie_header,
                    Some("https://pan.baidu.com/disk/home"),
                )
                .await
                {
                    return Some(found);
                }
            }
        }
    }
    None
}

pub(super) fn select_preferred_locate_url(urls: &[Value]) -> Option<String> {
    let candidates: Vec<&str> = urls
        .iter()
        .filter_map(|item| item.get("url").and_then(|v| v.as_str()))
        .map(str::trim)
        .filter(|s| s.starts_with("https://") || s.starts_with("http://"))
        .collect();
    candidates
        .iter()
        .find(|u| u.starts_with("https://"))
        .or_else(|| candidates.first())
        .map(|s| (*s).to_string())
}

async fn lookup_locatedownload(
    network: &CloudRequestContext,
    path_str: &str,
    cookie_header: &str,
) -> Option<BaiduDirectUrlResult> {
    // Netscape jar 保留原始作用域规则由 CloudRequest 按目标 URL 匹配；
    // 若为浏览器同步的 HTTP 头，则显式绑定 d.pcs.baidu.com 避免跨子域被剥离。
    let pcs_network = if crate::media_cookies::is_netscape_format(cookie_header) {
        network.clone()
    } else {
        network.with_cookie_source("https://d.pcs.baidu.com")
    };
    for app_id in &["250528", "778750", "498065", "309847", "266719"] {
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
        let Ok(resp) = pcs_network
            .get(&locate_url)
            .header("Cookie", cookie_header)
            .header("User-Agent", req_ua)
            .header("Referer", "https://pan.baidu.com/disk/home")
            .send()
            .await
        else {
            continue;
        };
        let Ok(text) = pcs_network.text(resp).await else {
            continue;
        };
        let json: Value = serde_json::from_str(&text).unwrap_or_default();
        if let Some(urls) = json.get("urls").and_then(|v| v.as_array()) {
            let mut ordered: Vec<&str> = urls
                .iter()
                .filter_map(|item| item.get("url").and_then(|v| v.as_str()))
                .map(str::trim)
                .filter(|s| s.starts_with("https://"))
                .collect();
            ordered.extend(
                urls.iter()
                    .filter_map(|item| item.get("url").and_then(|v| v.as_str()))
                    .map(str::trim)
                    .filter(|s| s.starts_with("http://")),
            );
            for url_str in ordered {
                if let Some(found) = select_working_dlink(
                    &pcs_network,
                    url_str,
                    req_ua,
                    cookie_header,
                    Some("https://pan.baidu.com/disk/home"),
                )
                .await
                {
                    return Some(found);
                }
            }
        }
    }
    None
}

async fn lookup_pcs_direct(
    network: &CloudRequestContext,
    path_str: &str,
    cookie_header: &str,
) -> Option<BaiduDirectUrlResult> {
    let pcs_network = if crate::media_cookies::is_netscape_format(cookie_header) {
        network.clone()
    } else {
        network.with_cookie_source("https://d.pcs.baidu.com")
    };
    let encoded_path = url_encode(path_str);
    for app_id in &["250528", "778750", "498065", "309847", "266719"] {
        let pcs_direct_url = format!(
            "https://d.pcs.baidu.com/rest/2.0/pcs/file?method=download&path={}&app_id={}&vip=2",
            encoded_path, app_id
        );
        let req_ua = if *app_id == "250528" {
            "pan.baidu.com"
        } else {
            BAIDU_DLINK_USER_AGENT
        };
        let Ok(pcs_resp) = pcs_network
            .no_redirect_get(&pcs_direct_url)
            .header("Cookie", cookie_header)
            .header("User-Agent", req_ua)
            .send()
            .await
        else {
            continue;
        };
        tracing::debug!(pcs_app_id = app_id, status = %pcs_resp.status(), "PCS Direct 响应状态");
        if pcs_resp.status().is_redirection() {
            if let Some(loc_str) = pcs_resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .filter(|s| !s.is_empty())
            {
                if let Some(found) =
                    select_working_dlink(&pcs_network, loc_str, req_ua, cookie_header, None).await
                {
                    return Some(found);
                }
            }
        } else if pcs_resp.status().is_success()
            && pcs_resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(is_browser_download_content_type)
        {
            return Some(build_dlink_result(
                &pcs_direct_url,
                req_ua,
                cookie_header,
                None,
            ));
        }
    }
    None
}

pub(super) fn build_dlink_result(
    url: &str,
    user_agent: &str,
    cookie_header: &str,
    referer: Option<&str>,
) -> BaiduDirectUrlResult {
    let normalized_cookie = normalize_cookie_header_for_url(cookie_header, url);
    let mut headers = HashMap::new();
    headers.insert("User-Agent".to_string(), user_agent.to_string());
    if !normalized_cookie.is_empty() {
        headers.insert("Cookie".to_string(), normalized_cookie);
    }
    if let Some(ref_url) = referer {
        headers.insert("Referer".to_string(), ref_url.to_string());
    }
    BaiduDirectUrlResult {
        url: url.to_string(),
        headers,
    }
}
