use super::CloudRequestContext;
use reqwest::{
    header::{HeaderName, COOKIE},
    Method, Request, RequestBuilder, Response, StatusCode,
};
use serde::Serialize;
use std::{collections::HashSet, time::Duration};

/// Holds logical URLs and raw Cookie scope separately from the transport endpoint used in local tests.
pub struct CloudRequest {
    context: CloudRequestContext,
    builder: RequestBuilder,
    cookie: Option<String>,
}
impl CloudRequest {
    pub(super) fn new(context: CloudRequestContext, method: Method, url: &str) -> Self {
        let builder = context.no_redirect.request(method, url);
        Self {
            context,
            builder,
            cookie: None,
        }
    }
    pub fn header(mut self, name: impl AsRef<str>, value: impl ToString) -> Self {
        if name.as_ref().eq_ignore_ascii_case("cookie") {
            self.cookie = Some(value.to_string());
        } else {
            self.builder = self.builder.header(name.as_ref(), value.to_string());
        }
        self
    }
    pub fn form<T: Serialize + ?Sized>(mut self, value: &T) -> Self {
        self.builder = self.builder.form(value);
        self
    }
    pub fn json<T: Serialize + ?Sized>(mut self, value: &T) -> Self {
        self.builder = self.builder.json(value);
        self
    }
    pub fn query<T: Serialize + ?Sized>(mut self, value: &T) -> Self {
        self.builder = self.builder.query(value);
        self
    }
    pub fn body(mut self, value: impl Into<reqwest::Body>) -> Self {
        self.builder = self.builder.body(value);
        self
    }
    pub fn timeout(mut self, value: Duration) -> Self {
        self.builder = self.builder.timeout(value);
        self
    }

    pub async fn send(self) -> Result<Response, String> {
        let template = self
            .builder
            .build()
            .map_err(|error| format!("构建网盘请求失败：{}", error.without_url()))?;
        if template.try_clone().is_none() {
            return Err("网盘请求正文不能安全重放，已停止请求".into());
        }
        let original = template.url().clone();
        let source = self
            .context
            .cookie_source
            .as_deref()
            .unwrap_or(original.as_str());
        let mut current = original.clone();
        let mut method = template.method().clone();
        let mut drop_body = false;
        let mut visited = HashSet::new();
        for hop in 0..=10 {
            validate(&current, &mut visited)?;
            let mut request = prepare(
                &template,
                &original,
                &current,
                &method,
                drop_body,
                self.cookie.as_deref(),
                source,
            )?;
            *request.url_mut() = url::Url::parse(&self.context.endpoint(current.as_str()))
                .map_err(|_| "网盘请求地址无效")?;
            let response = tokio::select! {biased;
                _=self.context.cancel.cancelled()=>return Err("网盘操作已取消".into()),
                response=self.context.no_redirect.execute(request)=>response.map_err(|error|format!("网盘请求失败：{}",error.without_url()))?,
            };
            if !self.context.follow_redirects || !redirect(response.status()) {
                return Ok(response);
            }
            if hop == 10 {
                return Err("网盘重定向超过 10 跳，已停止请求".into());
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or("网盘重定向缺少有效地址")?;
            current = current.join(location).map_err(|_| "网盘重定向地址无效")?;
            if response.status() == StatusCode::SEE_OTHER && method != Method::HEAD
                || matches!(
                    response.status(),
                    StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND
                ) && method == Method::POST
            {
                method = Method::GET;
                drop_body = true;
            }
        }
        Err("网盘重定向达到保护上限".into())
    }
}
fn redirect(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}
fn validate(url: &url::Url, visited: &mut HashSet<String>) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("网盘重定向协议或认证地址无效".into());
    }
    if !visited.insert(url.to_string()) {
        return Err("网盘重定向存在循环".into());
    }
    Ok(())
}
fn prepare(
    template: &Request,
    original: &url::Url,
    target: &url::Url,
    method: &Method,
    drop_body: bool,
    cookie: Option<&str>,
    source: &str,
) -> Result<Request, String> {
    let mut request = template.try_clone().ok_or("网盘请求正文不能安全重放")?;
    *request.url_mut() = target.clone();
    *request.method_mut() = method.clone();
    request.headers_mut().remove(reqwest::header::HOST);
    request.headers_mut().remove(COOKIE);
    if original.origin() != target.origin() {
        let names: Vec<HeaderName> = request
            .headers()
            .keys()
            .filter(|name| !safe_cross_origin(name.as_str()))
            .cloned()
            .collect();
        for name in names {
            request.headers_mut().remove(name);
        }
    }
    if let Some(cookie) = cookie {
        request
            .headers_mut()
            .extend(super::cookie_headers(cookie, source, target.as_str())?);
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
fn safe_cross_origin(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "accept"
            | "accept-language"
            | "user-agent"
            | "origin"
            | "referer"
            | "content-type"
            | "content-length"
            | "accept-encoding"
            | "range"
            | "if-range"
    )
}
