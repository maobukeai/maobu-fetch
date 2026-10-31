//! Bounded, cancellable HLS requests shared by playlists, keys and media.
use futures_util::StreamExt;
use reqwest::{
    header::{
        ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, ETAG, LAST_MODIFIED,
        RANGE,
    },
    Client, Method, Response, StatusCode,
};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, OnceLock,
    },
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

pub type LimitHook =
    Arc<dyn Fn(u64, CancellationToken) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;
pub const PAUSED: &str = "任务已暂停";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BUFFERED_BYTES: usize = 256 * 1024 * 1024;

fn memory_budget() -> Arc<Semaphore> {
    static BUDGET: OnceLock<Arc<Semaphore>> = OnceLock::new();
    BUDGET
        .get_or_init(|| Arc::new(Semaphore::new(MAX_BUFFERED_BYTES)))
        .clone()
}

fn strong_etag(value: &str) -> bool {
    value.len() >= 2
        && value.starts_with('"')
        && value.ends_with('"')
        && value.as_bytes()[1..value.len() - 1]
            .iter()
            .all(|byte| *byte == 0x21 || (0x23..=0x7e).contains(byte))
}

#[derive(Default)]
pub struct TransferProgress {
    /// Set only after parsing a real media playlist without EXT-X-ENDLIST.
    pub is_live: AtomicBool,
    pub network_bytes: AtomicU64,
    pub saved_bytes: AtomicU64,
    pub active_connections: AtomicU64,
}

#[derive(Clone, Debug, Default)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl Validators {
    pub fn usable(&self) -> bool {
        self.etag.as_deref().is_some_and(strong_etag) || self.last_modified.is_some()
    }
    pub fn from_response(response: &Response) -> Self {
        let text = |name| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        Self {
            etag: text(ETAG),
            last_modified: text(LAST_MODIFIED),
        }
    }

    pub fn safely_matches(&self, fresh: &Self) -> bool {
        let etag = self.etag.as_deref().filter(|tag| strong_etag(tag));
        let authoritative = etag.map_or_else(
            || self.last_modified.is_some() && self.last_modified == fresh.last_modified,
            |tag| fresh.etag.as_deref() == Some(tag),
        );
        authoritative && (self.last_modified.is_none() || self.last_modified == fresh.last_modified)
    }
}

#[derive(Clone)]
pub struct HlsClient {
    pub client: Client,
    pub source_url: String,
    pub headers: HashMap<String, String>,
    pub cancel: CancellationToken,
    pub progress: Arc<TransferProgress>,
    pub limit: LimitHook,
    pub retries: u32,
}

pub struct FetchedBytes {
    pub bytes: Vec<u8>,
    pub validators: Validators,
    pub final_url: String,
    _memory: OwnedSemaphorePermit,
}

struct ActiveRequest(Arc<TransferProgress>);
impl ActiveRequest {
    fn new(progress: Arc<TransferProgress>) -> Self {
        progress.active_connections.fetch_add(1, Ordering::Relaxed);
        Self(progress)
    }
}
impl Drop for ActiveRequest {
    fn drop(&mut self) {
        self.0.active_connections.fetch_sub(1, Ordering::Relaxed);
    }
}

impl HlsClient {
    fn request(&self, method: Method, url: &str) -> reqwest::RequestBuilder {
        let same_origin = url::Url::parse(&self.source_url)
            .ok()
            .zip(url::Url::parse(url).ok())
            .is_some_and(|(a, b)| a.origin() == b.origin());
        let mut request = self
            .client
            .request(method, url)
            .header(ACCEPT_ENCODING, "identity");
        for (name, value) in &self.headers {
            if name.eq_ignore_ascii_case("cookie") {
                let cookie = crate::media_cookies::header_for_url(value, &self.source_url, url);
                if !cookie.is_empty() {
                    request = request.header(name, cookie);
                }
                continue;
            }
            if !same_origin {
                if name.eq_ignore_ascii_case("referer") {
                    if let Ok(referer) = url::Url::parse(value) {
                        if matches!(referer.scheme(), "http" | "https") {
                            request = request.header(
                                name,
                                format!("{}/", referer.origin().ascii_serialization()),
                            );
                        }
                    }
                    continue;
                }
                if !["accept", "accept-language", "user-agent", "origin"]
                    .iter()
                    .any(|safe| name.eq_ignore_ascii_case(safe))
                {
                    continue;
                }
            }
            if !name.eq_ignore_ascii_case("range")
                && !name.eq_ignore_ascii_case("if-range")
                && !name.eq_ignore_ascii_case("accept-encoding")
            {
                request = request.header(name, value);
            }
        }
        request
    }

    async fn send_once(&self, request: reqwest::RequestBuilder) -> Result<Response, String> {
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Err(PAUSED.into()),
            result = tokio::time::timeout(REQUEST_TIMEOUT, request.send()) => {
                result.map_err(|_| "M3U8 请求响应超时，请检查网络后重试".to_owned())?
                    .map_err(|error| format!("M3U8 请求失败：{}", error.without_url()))
            }
        }
    }

    // The adapter must supply a Policy::none client: every redirect is rebuilt so
    // custom credentials receive the same origin checks as playlist media URLs.
    async fn send(
        &self,
        method: Method,
        url: &str,
        range: Option<(u64, Option<u64>)>,
    ) -> Result<Response, String> {
        let mut current = url::Url::parse(url).map_err(|_| "M3U8 校验失败：请求地址无效")?;
        let mut visited = std::collections::HashSet::new();
        for hop in 0..=10 {
            if !matches!(current.scheme(), "http" | "https") || !visited.insert(current.to_string())
            {
                return Err("M3U8 校验失败：重定向地址协议无效或存在循环".into());
            }
            let mut request = self.request(method.clone(), current.as_str());
            if let Some((length, offset)) = range {
                let start = offset.unwrap_or(0);
                let end = start
                    .checked_add(length)
                    .and_then(|end| end.checked_sub(1))
                    .ok_or("M3U8 校验失败：字节范围无效")?;
                request = request.header(RANGE, format!("bytes={start}-{end}"));
            }
            let response = self.send_once(request).await?;
            if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                return Ok(response);
            }
            if hop == 10 {
                return Err("M3U8 校验失败：重定向超过 10 次，请更新媒体链接".into());
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or("M3U8 校验失败：重定向缺少有效 Location")?;
            current = current
                .join(location)
                .map_err(|_| "M3U8 校验失败：重定向地址无效")?;
        }
        Err("M3U8 校验失败：重定向次数超限".into())
    }

    pub async fn cached_validators_match(
        &self,
        url: &str,
        saved: &Validators,
    ) -> Result<bool, String> {
        if !saved.usable() {
            return Ok(false);
        }
        let _active = ActiveRequest::new(self.progress.clone());
        let response = match self.send(Method::HEAD, url, None).await {
            Ok(response) => response,
            Err(error) if self.cancel.is_cancelled() => return Err(error),
            Err(_) => return Ok(false),
        };
        if !response.status().is_success() {
            return Ok(false);
        }
        if !saved.safely_matches(&Validators::from_response(&response)) {
            return Err(
                "REMOTE_CHANGED:M3U8 媒体分片的 ETag 或 Last-Modified 已变化，已保留原分片".into(),
            );
        }
        Ok(true)
    }

    pub async fn fetch(
        &self,
        url: &str,
        range: Option<(u64, Option<u64>)>,
        max_bytes: u64,
    ) -> Result<FetchedBytes, String> {
        let mut attempt = 0;
        loop {
            match self.fetch_once(url, range, max_bytes).await {
                Ok(value) => return Ok(value),
                Err(error)
                    if self.cancel.is_cancelled()
                        || error.starts_with("M3U8 校验失败")
                        || attempt >= self.retries =>
                {
                    return Err(error)
                }
                Err(_) => {
                    attempt += 1;
                    tokio::select! {
                        biased;
                        _ = self.cancel.cancelled() => return Err(PAUSED.into()),
                        _ = tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt.min(8)))) => {}
                    }
                }
            }
        }
    }

    async fn fetch_once(
        &self,
        url: &str,
        range: Option<(u64, Option<u64>)>,
        max_bytes: u64,
    ) -> Result<FetchedBytes, String> {
        let _active = ActiveRequest::new(self.progress.clone());
        let response = self.send(Method::GET, url, range).await?;
        let expected = validate_response(&response, range, max_bytes)?;
        let reserved = u32::try_from(expected.unwrap_or(max_bytes))
            .map_err(|_| "M3U8 校验失败：内存预算超限")?;
        if reserved as usize > MAX_BUFFERED_BYTES {
            return Err("M3U8 校验失败：单次缓冲超过共享内存预算".into());
        }
        let memory = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => return Err(PAUSED.into()),
            permit = memory_budget().acquire_many_owned(reserved) => permit.map_err(|_| "M3U8 内存预算已关闭")?,
        };
        let validators = Validators::from_response(&response);
        let final_url = response.url().to_string();
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(reserved as usize)
            .map_err(|_| "M3U8 媒体缓冲内存不足，请减少同时下载任务后重试")?;
        loop {
            let next = tokio::select! {
                biased;
                _ = self.cancel.cancelled() => return Err(PAUSED.into()),
                chunk = tokio::time::timeout(REQUEST_TIMEOUT, stream.next()) =>
                    chunk.map_err(|_| "M3U8 媒体数据长时间无响应，请重试")?,
            };
            let Some(chunk) = next else {
                break;
            };
            let chunk =
                chunk.map_err(|error| format!("读取 M3U8 数据失败：{}", error.without_url()))?;
            let new_length = (bytes.len() as u64)
                .checked_add(chunk.len() as u64)
                .ok_or("M3U8 校验失败：数据长度溢出")?;
            if new_length > max_bytes || expected.is_some_and(|length| new_length > length) {
                return Err("M3U8 校验失败：响应数据超出声明范围或安全内存上限".into());
            }
            (self.limit)(chunk.len() as u64, self.cancel.clone()).await;
            if self.cancel.is_cancelled() {
                return Err(PAUSED.into());
            }
            self.progress
                .network_bytes
                .fetch_add(chunk.len() as u64, Ordering::Relaxed);
            bytes.extend_from_slice(&chunk);
        }
        if expected.is_some_and(|length| bytes.len() as u64 != length) || bytes.is_empty() {
            return Err("M3U8 校验失败：响应长度不完整或为空，已保留恢复数据".into());
        }
        Ok(FetchedBytes {
            bytes,
            validators,
            final_url,
            _memory: memory,
        })
    }
}

fn validate_response(
    response: &Response,
    range: Option<(u64, Option<u64>)>,
    max_bytes: u64,
) -> Result<Option<u64>, String> {
    if let Some((length, offset)) = range {
        if response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(format!(
                "M3U8 校验失败：服务器未正确响应 Range（HTTP {}），不能拼接",
                response.status().as_u16()
            ));
        }
        let start = offset.unwrap_or(0);
        let end = start
            .checked_add(length)
            .and_then(|n| n.checked_sub(1))
            .ok_or("M3U8 校验失败：字节范围溢出")?;
        let content_range = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_content_range)
            .ok_or("M3U8 校验失败：缺少或错误的 Content-Range")?;
        if content_range.0 != start || content_range.1 != end {
            return Err("M3U8 校验失败：Content-Range 起止偏移与请求不一致".into());
        }
    } else if response.status() != StatusCode::OK {
        return Err(format!(
            "获取 M3U8 数据失败：HTTP {}",
            response.status().as_u16()
        ));
    }
    if response
        .headers()
        .get(CONTENT_ENCODING)
        .is_some_and(|value| value != "identity")
    {
        return Err("M3U8 校验失败：服务器返回了不支持的内容编码".into());
    }
    let declared = response
        .headers()
        .get(CONTENT_LENGTH)
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|text| text.parse::<u64>().ok())
                .ok_or("M3U8 校验失败：Content-Length 无效")
        })
        .transpose()?;
    let expected = range.map(|(length, _)| length).or(declared);
    if expected.is_some_and(|length| length > max_bytes || length == 0)
        || range.is_some_and(|(length, _)| declared.is_some_and(|actual| actual != length))
    {
        return Err("M3U8 校验失败：Content-Length 与请求范围不一致或超过上限".into());
    }
    Ok(expected)
}

pub(crate) fn parse_content_range(value: &str) -> Option<(u64, u64)> {
    let (span, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = span.split_once('-')?;
    let (start, end) = (start.parse::<u64>().ok()?, end.parse::<u64>().ok()?);
    if end < start || (total != "*" && total.parse::<u64>().ok()? <= end) {
        return None;
    }
    Some((start, end))
}
