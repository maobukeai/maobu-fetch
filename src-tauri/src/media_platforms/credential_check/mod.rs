use crate::{
    media_platforms::{detect_platform, MediaPlatform},
    models::MediaCredentialCheckResult,
};
use futures_util::StreamExt;
use std::time::Duration;
mod response;
#[cfg(test)]
mod tests;

pub fn target(domain: &str) -> Result<String, String> {
    let parsed =
        url::Url::parse(&format!("https://{}/", domain.trim())).map_err(|_| "凭证域名无效")?;
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("凭证域名无效".into());
    }
    Ok(match detect_platform(parsed.as_str()) {
        MediaPlatform::Bilibili => "https://api.bilibili.com/x/web-interface/nav".into(),
        MediaPlatform::Douyin => "https://www.douyin.com/passport/web/user/info/".into(),
        MediaPlatform::Twitter => "https://x.com/i/api/1.1/account/verify_credentials.json".into(),
        MediaPlatform::YouTube => "https://www.youtube.com/".into(),
        MediaPlatform::BaiduPan => {
            "https://pan.baidu.com/api/quota?checkfree=1&checkexpire=1".into()
        }
        _ => parsed.to_string(),
    })
}

/// The injected client must disable redirects and apply the application's proxy policy.
pub async fn check_media_credential_with_client(
    domain: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
    client: reqwest::Client,
) -> Result<MediaCredentialCheckResult, String> {
    if [cookie, referer, user_agent]
        .iter()
        .all(|value| value.is_none_or(|value| value.trim().is_empty()))
    {
        return Ok(result(
            domain,
            false,
            "未设置凭证内容（Cookie/Referer/User-Agent均为空）".into(),
        ));
    }
    let target = target(domain)?;
    check_target(domain, cookie, referer, user_agent, &client, &target).await
}

pub async fn check_media_credential(
    domain: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
) -> Result<MediaCredentialCheckResult, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "创建凭证检查连接失败")?;
    check_media_credential_with_client(domain, cookie, referer, user_agent, client).await
}

async fn check_target(
    domain: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
    client: &reqwest::Client,
    target: &str,
) -> Result<MediaCredentialCheckResult, String> {
    let platform = detect_platform(&format!("https://{domain}/"));
    let cookie = crate::media_cookies::header_for_url(cookie.unwrap_or(""), target, target);
    let ua=user_agent.filter(|value|!value.trim().is_empty()).unwrap_or("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/131.0.0.0 Safari/537.36");
    if ua.chars().any(char::is_control)
        || referer.is_some_and(|value| value.chars().any(char::is_control))
    {
        return Err("凭证请求头含无效控制字符".into());
    }
    let mut request = client
        .get(target)
        .timeout(Duration::from_secs(10))
        .header(reqwest::header::USER_AGENT, ua);
    if !cookie.is_empty() {
        request = request.header(reqwest::header::COOKIE, &cookie);
    }
    if let Some(referer) = referer.filter(|value| !value.is_empty()) {
        let parsed = url::Url::parse(referer).map_err(|_| "凭证Referer地址无效")?;
        if !matches!(parsed.scheme(), "http" | "https")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err("凭证Referer地址无效".into());
        }
        request = request.header(reqwest::header::REFERER, referer);
    }
    if platform == MediaPlatform::Twitter {
        // Public web-client bearer identifies the API application, never a user's secret.
        request = request.header(reqwest::header::AUTHORIZATION, response::TWITTER_WEB_BEARER);
        if let Some((_, csrf)) = crate::media_cookies::parse_cookie_header(&cookie)
            .into_iter()
            .find(|(name, _)| name == "ct0")
        {
            request = request.header("x-csrf-token", csrf);
        }
    }
    let response = request.send().await.map_err(|_| {
        "网络不可达：凭证接口连接失败，请检查应用代理、网络与站点可用性；尚未判定凭证有效性"
    })?;
    let status = response.status();
    if !status.is_success() {
        return Ok(result(
            domain,
            false,
            if status.is_redirection() {
                "站点要求重定向，未转发凭证；请检查登录域名".into()
            } else {
                format!(
                    "站点返回HTTP {}，认证未通过或访问受限；请在浏览器确认登录后重试",
                    status.as_u16()
                )
            },
        ));
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "网络不可达：凭证接口响应中断，尚未判定凭证有效性")?;
        if chunk.len() > 1024 * 1024usize - body.len() {
            return Err("凭证接口响应超过1MB安全上限，未判定凭证有效性".into());
        }
        body.extend_from_slice(&chunk);
    }
    let (valid, message) = response::classify(platform, &body)?;
    Ok(result(domain, valid, message))
}

fn result(domain: &str, valid: bool, message: String) -> MediaCredentialCheckResult {
    MediaCredentialCheckResult {
        domain: domain.to_owned(),
        valid,
        message,
        tested_at: crate::now_iso8601_utc(),
    }
}
