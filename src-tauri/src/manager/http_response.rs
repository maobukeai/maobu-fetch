use super::{header_string, parse_content_range, remote_resource_changed, DownloadTask};
use reqwest::{header::{ETAG, LAST_MODIFIED}, Response, StatusCode};
use tokio_util::sync::CancellationToken;

pub(super) async fn send_with_cancel(request: reqwest::RequestBuilder, token: &CancellationToken) -> Result<Response, String> {
    tokio::select! {
        biased;
        _ = token.cancelled() => Err("任务已暂停".into()),
        response = request.send() => response.map_err(super::friendly_reqwest),
    }
}

#[derive(Debug)]
pub(super) struct ResponsePlan {
    pub append: bool,
    pub initial_bytes: u64,
    pub response_bytes: Option<u64>,
    pub total_bytes: u64,
}

pub(super) fn if_range_validator(task: &DownloadTask) -> Option<&str> {
    task.etag.as_deref()
        .filter(|value| value.starts_with('"') && value.ends_with('"'))
        .or(task.last_modified.as_deref())
}

/// 在打开/截断本地文件之前验证实际数据响应，而非仅相信 HEAD 探针。
pub(super) fn response_plan(task: &DownloadTask, response: &Response, requested_start: u64) -> Result<ResponsePlan, String> {
    if response.headers().get(reqwest::header::CONTENT_ENCODING).is_some_and(|value| value != "identity") {
        return Err("服务器对下载数据使用了非 identity 内容编码，不能安全校验字节范围，请更换下载源".into());
    }
    if !response.status().is_success() {
        return Err(format!("服务器返回 HTTP {}，已保留已有临时文件", response.status()));
    }
    super::cloud_content::validate_response(task, response)?;
    if requested_start > 0 && remote_resource_changed(
        task.etag.as_deref(), header_string(response, ETAG).as_deref(),
        task.last_modified.as_deref(), header_string(response, LAST_MODIFIED).as_deref(),
    ) {
        return Err("REMOTE_CHANGED:实际下载响应的资源验证器已变化，已保留旧分片，请确认后重新下载".into());
    }
    if response.status() == StatusCode::PARTIAL_CONTENT {
        let (start, end, total) = parse_content_range(response)
            .ok_or("续传响应缺少或包含无效 Content-Range，已保留旧分片")?;
        if start != requested_start || (task.total_bytes > 0 && total != task.total_bytes) {
            return Err("续传响应的起始偏移或资源总长度不匹配，已保留旧分片".into());
        }
        let length = end - start + 1;
        if response.content_length().is_some_and(|value| value != length) {
            return Err("续传响应长度与 Content-Range 不匹配，已保留旧分片".into());
        }
        return Ok(ResponsePlan { append: requested_start > 0, initial_bytes: start,
            response_bytes: Some(length), total_bytes: total });
    }
    if response.status() != StatusCode::OK {
        return Err(format!("不支持的数据响应 HTTP {}", response.status()));
    }
    let total = response.content_length().unwrap_or(task.total_bytes);
    if task.total_bytes > 0 && total > 0 && total != task.total_bytes {
        return Err("REMOTE_CHANGED:实际响应的资源长度已变化，请确认后重新下载".into());
    }
    // 服务器忽略 Range 时只允许从零重写完整实体，绝不追加 200 body。
    Ok(ResponsePlan { append: false, initial_bytes: 0,
        response_bytes: response.content_length(), total_bytes: total })
}
