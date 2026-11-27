use super::{authorize, BridgeState};
use crate::{
    baidupan::{is_browser_download_content_type, is_browser_download_target},
    manager::{build_client_with_redirect_policy, parse_content_range_value},
};
use axum::{
    body::Bytes,
    extract::{rejection::BytesRejection, State},
};
use reqwest::{
    header::{
        HeaderMap, HeaderName, HeaderValue, ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE,
        CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, COOKIE, RANGE, REFERER,
        USER_AGENT,
    },
    Client, Response, StatusCode,
};
use serde::Deserialize;
use std::{collections::HashMap, time::Duration};
use url::Url;

const MAX_BROWSER_FILE_SIZE: u64 = 9_007_199_254_740_991;
const TIMEOUT: Duration = Duration::from_secs(4);

type BridgeError = (StatusCode, &'static str);
const INVALID_REQUEST: BridgeError = (StatusCode::BAD_REQUEST, "百度网盘下载验证参数无效");
const UNSUPPORTED_TARGET: BridgeError = (StatusCode::BAD_REQUEST, "百度网盘下载地址不在接管范围内");
const NETWORK_ERROR: BridgeError = (
    StatusCode::BAD_GATEWAY,
    "无法验证百度网盘下载，请继续使用浏览器下载",
);
const UNVERIFIED_FILE: BridgeError = (
    StatusCode::UNPROCESSABLE_ENTITY,
    "百度网盘文件无法确认，请继续使用浏览器下载",
);

#[derive(Deserialize)]
struct Request {
    url: String,
    #[serde(default)]
    headers: HashMap<String, String>,
    total_bytes: u64,
}

impl Request {
    fn url(&self) -> Result<Url, BridgeError> {
        // URL parsers may discard empty userinfo, so also reject a literal @ in the authority.
        let authority = self
            .url
            .split_once("://")
            .and_then(|(_, rest)| rest.split(['/', '?', '#']).next());
        if !authority.is_some_and(|part| !part.is_empty() && !part.contains(['@', '\\'])) {
            return Err(UNSUPPORTED_TARGET);
        }
        let url = Url::parse(&self.url).map_err(|_| UNSUPPORTED_TARGET)?;
        if !is_browser_download_target(&url) {
            return Err(UNSUPPORTED_TARGET);
        }
        Ok(url)
    }

    fn headers(&self) -> Result<HeaderMap, BridgeError> {
        let mut headers = HeaderMap::new();
        for (name, value) in &self.headers {
            let name = match name.to_ascii_lowercase().as_str() {
                "accept" => ACCEPT,
                "accept-language" => ACCEPT_LANGUAGE,
                "cookie" => COOKIE,
                "referer"
                    if Url::parse(value).is_ok_and(|url| is_browser_download_target(&url)) =>
                {
                    REFERER
                }
                "user-agent" => USER_AGENT,
                _ => continue,
            };
            let mut value = HeaderValue::from_str(value).map_err(|_| INVALID_REQUEST)?;
            value.set_sensitive(name == COOKIE || name == REFERER);
            if headers.insert(name, value).is_some() {
                return Err(INVALID_REQUEST);
            }
        }
        Ok(headers)
    }
}

pub(super) async fn verify(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<StatusCode, (StatusCode, String)> {
    let body = body.map_err(|_| to_bridge_response(INVALID_REQUEST))?;
    authorize(&state, &headers, &body).await.map_err(|error| {
        if error.0.is_server_error() {
            to_bridge_response((
                StatusCode::INTERNAL_SERVER_ERROR,
                "无法验证扩展配对，请稍后重试",
            ))
        } else {
            error
        }
    })?;
    let settings = state.manager.settings().await;
    if !settings.intercept_browser_downloads {
        return Err(to_bridge_response((
            StatusCode::FORBIDDEN,
            "桌面端已关闭浏览器下载接管",
        )));
    }
    verify_download(&settings, &body)
        .await
        .map_err(to_bridge_response)
}

fn to_bridge_response((status, message): BridgeError) -> (StatusCode, String) {
    (status, message.to_owned())
}

async fn verify_download(
    settings: &crate::models::AppSettings,
    body: &[u8],
) -> Result<StatusCode, BridgeError> {
    let request: Request = serde_json::from_slice(body).map_err(|_| INVALID_REQUEST)?;
    if !(1..=MAX_BROWSER_FILE_SIZE).contains(&request.total_bytes) {
        return Err(INVALID_REQUEST);
    }
    let url = request.url()?;
    let headers = request.headers()?;
    let client = build_client_with_redirect_policy(settings, reqwest::redirect::Policy::none())
        .map_err(|_| NETWORK_ERROR)?;
    probe(&client, url, headers, request.total_bytes).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn probe(
    client: &Client,
    url: Url,
    headers: HeaderMap,
    total_bytes: u64,
) -> Result<(), BridgeError> {
    let mut response = client
        .get(url)
        .headers(headers)
        .header(ACCEPT_ENCODING, "identity")
        .header(RANGE, "bytes=0-0")
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|_| NETWORK_ERROR)?;
    validate_response_headers(&response, total_bytes)?;
    let first = response.chunk().await.map_err(|_| UNVERIFIED_FILE)?;
    if !first.is_some_and(|bytes| bytes.len() == 1)
        || response
            .chunk()
            .await
            .map_err(|_| UNVERIFIED_FILE)?
            .is_some()
    {
        return Err(UNVERIFIED_FILE);
    }
    Ok(())
}

fn validate_response_headers(response: &Response, total_bytes: u64) -> Result<(), BridgeError> {
    // A complete response or redirect does not prove the final URL can be replayed safely.
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(UNVERIFIED_FILE);
    }
    let headers = response.headers();
    if single_header(headers, CONTENT_RANGE)?.and_then(parse_content_range_value)
        != Some((0, 0, total_bytes))
        || !single_header(headers, CONTENT_TYPE)?.is_some_and(is_browser_download_content_type)
        || single_header(headers, CONTENT_LENGTH)?.is_some_and(|value| value != "1")
        || single_header(headers, CONTENT_ENCODING)?
            .is_some_and(|value| !value.eq_ignore_ascii_case("identity"))
    {
        return Err(UNVERIFIED_FILE);
    }
    Ok(())
}

fn single_header(headers: &HeaderMap, name: HeaderName) -> Result<Option<&str>, BridgeError> {
    let fields = headers.get_all(name);
    let mut values = fields.iter();
    let value = values.next();
    if values.next().is_some() {
        return Err(UNVERIFIED_FILE);
    }
    value
        .map(|value| value.to_str().map_err(|_| UNVERIFIED_FILE))
        .transpose()
}

#[cfg(test)]
mod tests;
