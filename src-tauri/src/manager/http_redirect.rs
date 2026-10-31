//! HTTP clients used here must have Policy::none: rebuild credentials for every actual target.
use super::*;
use reqwest::{Client, Method, Request, RequestBuilder, Response, StatusCode};
#[cfg(test)]
#[path = "http_redirect_tests.rs"]
mod tests;

pub(super) async fn send(
    client: &Client,
    builder: RequestBuilder,
    raw_headers: &HashMap<String, String>,
    source_url: &str,
    cancel: &CancellationToken,
) -> Result<Response, String> {
    let template = builder
        .build()
        .map_err(|error| format!("无法构建下载请求：{}", error.without_url()))?;
    if template.try_clone().is_none() {
        return Err("下载请求包含不能安全重放的流式正文，无法处理重定向".into());
    }
    let mut current = template.url().clone();
    let mut method = template.method().clone();
    let mut drop_body = false;
    let mut visited = HashSet::new();
    for hop in 0..=10 {
        validate_url(&current, &mut visited)?;
        if cancel.is_cancelled() {
            return Err("任务已暂停".into());
        }
        let mut request = prepare(
            &template,
            &current,
            &method,
            drop_body,
            raw_headers,
            source_url,
        )?;
        // The supplied client controls proxy/auth/TLS; no independent client or fallback exit is created.
        request.headers_mut().remove(reqwest::header::HOST);
        let response = tokio::select! {
            biased;
            _ = cancel.cancelled()=>return Err("任务已暂停".into()),
            result=client.execute(request)=>result.map_err(|error|format!("下载请求失败：{}",error.without_url()))?,
        };
        if !matches!(
            response.status(),
            StatusCode::MOVED_PERMANENTLY
                | StatusCode::FOUND
                | StatusCode::SEE_OTHER
                | StatusCode::TEMPORARY_REDIRECT
                | StatusCode::PERMANENT_REDIRECT
        ) {
            return Ok(response);
        }
        if hop == 10 {
            return Err("下载重定向超过 10 跳，已停止请求".into());
        }
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or("下载重定向缺少有效 Location，已停止请求")?;
        let next = current
            .join(location)
            .map_err(|_| "下载重定向地址无效，已停止请求")?;
        if response.status() == StatusCode::SEE_OTHER && method != Method::HEAD
            || matches!(
                response.status(),
                StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND
            ) && method == Method::POST
        {
            method = Method::GET;
            drop_body = true;
        }
        current = next;
    }
    Err("下载重定向超过保护上限".into())
}

fn validate_url(current: &Url, visited: &mut HashSet<String>) -> Result<(), String> {
    if !matches!(current.scheme(), "http" | "https")
        || current.host_str().is_none()
        || !current.username().is_empty()
        || current.password().is_some()
    {
        return Err("下载重定向地址协议或认证信息无效，已停止请求".into());
    }
    if !visited.insert(current.to_string()) {
        return Err("下载重定向存在循环，已停止请求".into());
    }
    Ok(())
}

fn prepare(
    template: &Request,
    target: &Url,
    method: &Method,
    drop_body: bool,
    raw: &HashMap<String, String>,
    source: &str,
) -> Result<Request, String> {
    let mut request = template
        .try_clone()
        .ok_or("下载请求正文不能安全重放，已停止重定向")?;
    *request.url_mut() = target.clone();
    *request.method_mut() = method.clone();
    // Preserve explicit builder/basic auth at the source, but scope it exactly like raw credentials.
    let mut scoped_source = raw.clone();
    for (name, value) in template
        .headers()
        .iter()
        .filter(|(name, _)| sensitive_name(name.as_str()))
    {
        if !scoped_source
            .keys()
            .any(|key| key.eq_ignore_ascii_case(name.as_str()))
        {
            scoped_source.insert(
                name.as_str().into(),
                value.to_str().map_err(|_| "下载认证请求头值无效")?.into(),
            );
        }
    }
    // Credentials from a builder must never bypass source/path checks on a later hop.
    let sensitive: Vec<_> = request
        .headers()
        .keys()
        .filter(|name| sensitive_name(name.as_str()))
        .cloned()
        .collect();
    for name in sensitive {
        request.headers_mut().remove(name);
    }
    for (name, value) in request_headers::for_direct_url(&scoped_source, source, target.as_str()) {
        if forbidden_header(&name) || request.headers().contains_key(name.as_str()) {
            continue;
        }
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| "下载请求头名称无效")?;
        let value = reqwest::header::HeaderValue::from_str(&value)
            .map_err(|_| "下载请求头值无效，已停止请求")?;
        request.headers_mut().insert(name, value);
    }
    if drop_body {
        *request.body_mut() = None;
        for name in [
            reqwest::header::CONTENT_TYPE,
            reqwest::header::CONTENT_LENGTH,
            reqwest::header::TRANSFER_ENCODING,
        ] {
            request.headers_mut().remove(name);
        }
    }
    Ok(request)
}

fn sensitive_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "cookie"
            | "authorization"
            | "proxy-authorization"
            | "x-api-key"
            | "x-auth-token"
            | "x-access-token"
            | "x-csrf-token"
            | "x-luma-token"
            | "x-luma-signature"
    )
}
fn forbidden_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "host" | "content-length" | "transfer-encoding" | "connection"
    )
}
