//! One task's proxy, proxy authentication and cancellation across cloud phases.
#[cfg(test)]
#[path = "cloud_request/tests.rs"]
mod tests;
use futures_util::StreamExt;
use reqwest::{Client, Method};
use std::future::Future;
use tokio_util::sync::CancellationToken;
#[path = "cloud_request/cookies.rs"]
mod cookies;
pub(crate) use cookies::{cookie_headers, with_cookie_pair};
#[path = "cloud_request/request.rs"]
mod request;
pub(crate) use request::CloudRequest;

#[derive(Clone)]
pub struct CloudRequestContext {
    client: Client,
    no_redirect: Client,
    cancel: CancellationToken,
    user_agent: Option<&'static str>,
    cookie_source: Option<String>,
    follow_redirects: bool,
    #[cfg(test)]
    mock_endpoint: Option<String>,
}
impl CloudRequestContext {
    pub fn defaults(
        user_agent: &'static str,
        timeout_secs: u64,
        cancel: CancellationToken,
    ) -> Result<Self, String> {
        let build = |policy| {
            Client::builder()
                .user_agent(user_agent)
                .timeout(std::time::Duration::from_secs(timeout_secs))
                .redirect(policy)
                .build()
                .map_err(|error| format!("创建网盘请求客户端失败：{}", error.without_url()))
        };
        Ok(Self::new(
            build(reqwest::redirect::Policy::limited(10))?,
            build(reqwest::redirect::Policy::none())?,
            cancel,
        ))
    }
    pub fn new(client: Client, no_redirect: Client, cancel: CancellationToken) -> Self {
        Self {
            client,
            no_redirect,
            cancel,
            user_agent: None,
            cookie_source: None,
            follow_redirects: true,
            #[cfg(test)]
            mock_endpoint: None,
        }
    }
    pub fn cancel(&self) -> &CancellationToken {
        &self.cancel
    }
    pub fn client(&self) -> Client {
        self.client.clone()
    }
    pub fn user_agent(&self) -> Option<&str> {
        self.user_agent
    }
    pub fn without_redirects(&self) -> Self {
        let mut context = self.clone();
        context.client = self.no_redirect.clone();
        context.follow_redirects = false;
        context
    }
    pub fn with_cookie_source(&self, source: &str) -> Self {
        let mut context = self.clone();
        context.cookie_source = Some(source.into());
        context
    }
    pub fn with_user_agent(&self, user_agent: &'static str) -> Self {
        let mut context = self.clone();
        context.user_agent = Some(user_agent);
        context
    }
    pub fn endpoint(&self, original: &str) -> String {
        #[cfg(test)]
        if let Some(base) = &self.mock_endpoint {
            if let Ok(url) = url::Url::parse(original) {
                let mut result = format!(
                    "{}/{}/{}",
                    base.trim_end_matches('/'),
                    url.host_str().unwrap_or("invalid"),
                    url.path().trim_start_matches('/')
                );
                if let Some(query) = url.query() {
                    result.push('?');
                    result.push_str(query);
                }
                return result;
            }
        }
        original.to_owned()
    }
    fn request(&self, method: Method, url: &str) -> CloudRequest {
        let mut request = CloudRequest::new(self.clone(), method, url);
        if let Some(agent) = self.user_agent {
            request = request.header(reqwest::header::USER_AGENT, agent);
        }
        request
    }
    pub fn get(&self, url: impl AsRef<str>) -> CloudRequest {
        self.request(Method::GET, url.as_ref())
    }
    pub fn post(&self, url: impl AsRef<str>) -> CloudRequest {
        self.request(Method::POST, url.as_ref())
    }
    pub fn no_redirect_get(&self, url: impl AsRef<str>) -> CloudRequest {
        self.without_redirects().request(Method::GET, url.as_ref())
    }
    pub async fn text(&self, response: reqwest::Response) -> Result<String, String> {
        if !response.status().is_success() {
            return Err(format!(
                "网盘接口返回 HTTP {}，请检查网络、代理和访问权限",
                response.status().as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > 4 * 1024 * 1024)
        {
            return Err("网盘接口响应超过安全大小，请缩小目录后重试".into());
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|error| format!("读取网盘响应失败：{}", error.without_url()))?;
            if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
                return Err("网盘接口响应超过安全大小".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).map_err(|_| "网盘接口响应不是有效的 UTF-8 文本".into())
    }
    pub async fn run<T>(
        &self,
        platform: &str,
        future: impl Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        crate::cloud_pagination::cancellable(&self.cancel, platform, future)
            .await
            .map_err(|error| {
                let mut safe = crate::manager::redact_sensitive(&error);
                for scheme in ["https://", "http://"] {
                    while let Some(start) = safe.find(scheme) {
                        let length = safe[start..]
                            .find(|character: char| {
                                character.is_whitespace()
                                    || matches!(character, '\'' | '"' | '<' | '>' | ')')
                            })
                            .unwrap_or(safe.len() - start);
                        safe.replace_range(start..start + length, "[已隐藏网盘地址]");
                    }
                }
                safe
            })
    }
    #[cfg(test)]
    pub(crate) fn with_test_endpoint(mut self, base: &str) -> Self {
        self.mock_endpoint = Some(base.to_owned());
        self
    }
}
