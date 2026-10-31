use super::super::work_stealing::WindowTransferHandle;
use super::*;

pub(super) struct Step {
    pub result: Result<(), String>,
    pub bytes: u64,
    pub responded: bool,
}

struct ActiveRequest<'a> {
    _permit: AdaptiveConnectionPermit,
    runtime: &'a SegmentRuntime,
}
impl Drop for ActiveRequest<'_> {
    fn drop(&mut self) {
        let active = self
            .runtime
            .active_windows
            .fetch_sub(1, Ordering::Relaxed)
            .saturating_sub(1);
        if active == 0 && self.runtime.status.load(Ordering::Relaxed) != SEGMENT_FAILED {
            self.runtime
                .status
                .store(SEGMENT_PENDING, Ordering::Relaxed);
        }
    }
}

pub(super) async fn receive(
    session: &Session,
    handle: &WindowTransferHandle,
    runtime: &SegmentRuntime,
    file: &mut BufWriter<fs::File>,
    next_start: &mut u64,
) -> Step {
    let mut step = Step {
        result: Ok(()),
        bytes: 0,
        responded: false,
    };
    let permit = tokio::select! {
        _ = session.worker_cancel.cancelled() => { step.result=Err("任务已暂停".into()); return step; },
        permit = session.adaptive.clone().acquire() => permit,
    };
    runtime.active_windows.fetch_add(1, Ordering::Relaxed);
    runtime.status.store(SEGMENT_DOWNLOADING, Ordering::Relaxed);
    let active = ActiveRequest {
        _permit: permit,
        runtime,
    };
    let end = handle.current_end();
    if *next_start > end {
        return step;
    }
    step.result = async {
        let response = send(session, *next_start, end).await?;
        let status = response.status();
        validate_resource(session, &response)?;
        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            // An in-range 416 is evidence for the bounded expired-link circuit breaker, never completion.
            step.responded = *next_start < session.total;
            return Ok(());
        }
        validate_range(&response, *next_start, end, session.total, runtime.index)?;
        step.responded = true;
        copy_body(
            response,
            session,
            handle,
            runtime,
            file,
            next_start,
            &active._permit,
            &mut step.bytes,
        )
        .await
    }
    .await;
    step
}

async fn send(session: &Session, start: u64, end: u64) -> Result<reqwest::Response, String> {
    let mut request = session.config.client.get(&session.config.url);
    request = request
        .header(ACCEPT_ENCODING, "identity")
        .header(RANGE, format!("bytes={start}-{end}"));
    if let Some(value) = &session.config.if_range {
        request = request.header(IF_RANGE, value);
    }
    http_redirect::send(
        &session.config.client,
        request,
        &session.config.headers,
        &session.config.url,
        &session.worker_cancel,
    )
    .await
}

fn validate_resource(session: &Session, response: &reqwest::Response) -> Result<(), String> {
    cloud_content::validate_binary_response(session.config.cloud_binary_expected, response)?;
    if response.status().is_success()
        && remote_resource_changed(
            session.config.etag.as_deref(),
            header_string(response, ETAG).as_deref(),
            session.config.modified.as_deref(),
            header_string(response, LAST_MODIFIED).as_deref(),
        )
    {
        return Err(format!(
            "{REMOTE_CHANGED_PREFIX}分片响应的资源验证器已变化，已保留旧分片"
        ));
    }
    Ok(())
}

fn validate_range(
    response: &reqwest::Response,
    start: u64,
    end: u64,
    total: u64,
    index: u8,
) -> Result<(), String> {
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(format!(
            "{INVALID_RANGE_PREFIX}服务器返回 HTTP {}，无法安全续传分片 #{}",
            response.status(),
            index + 1
        ));
    }
    match parse_content_range(response) {
        Some((actual_start, actual_end, actual_total))
            if actual_start == start
                && actual_end >= actual_start
                && actual_end == end
                && (actual_total == total || total == 0) => {}
        _ => {
            return Err(format!(
                "{INVALID_RANGE_PREFIX}服务器返回了不匹配的 Content-Range"
            ))
        }
    };
    if response
        .content_length()
        .is_some_and(|bytes| bytes != end - start + 1)
    {
        return Err(format!(
            "{INVALID_RANGE_PREFIX}Content-Length 与请求分片范围不一致"
        ));
    }
    Ok(())
}

async fn copy_body(
    response: reqwest::Response,
    session: &Session,
    handle: &WindowTransferHandle,
    runtime: &SegmentRuntime,
    file: &mut BufWriter<fs::File>,
    cursor: &mut u64,
    permit: &AdaptiveConnectionPermit,
    step_bytes: &mut u64,
) -> Result<(), String> {
    let request_end = handle.current_end();
    let mut stream = response.bytes_stream();
    let mut idle_seconds = 0u8;
    loop {
        let next = tokio::select! {
            _ = session.worker_cancel.cancelled() => return Err("任务已暂停".into()),
            result = tokio::time::timeout(Duration::from_secs(1),stream.next()) => result,
        };
        let chunk = match next {
            Ok(Some(chunk)) => {
                idle_seconds = 0;
                chunk.map_err(friendly_body_error)?
            }
            Ok(None) => break,
            Err(_) if session.adaptive.should_yield(permit) => return Err(ADAPTIVE_YIELD.into()),
            Err(_) if idle_seconds >= 6 => {
                return Err(format!(
                    "分片 #{} 连续 6 秒未收到数据，自动断点续连",
                    runtime.index + 1
                ))
            }
            Err(_) => {
                idle_seconds = idle_seconds.saturating_add(1);
                continue;
            }
        };
        if session.adaptive.should_yield(permit) {
            return Err(ADAPTIVE_YIELD.into());
        }
        if *cursor > handle.current_end() {
            return Err(format!(
                "{INVALID_RANGE_PREFIX}分片响应含有范围结束后的额外数据"
            ));
        }
        if chunk.len() as u64
            > handle
                .current_end()
                .saturating_sub(*cursor)
                .saturating_add(1)
        {
            return Err(format!(
                "{INVALID_RANGE_PREFIX}分片响应数据超出请求范围，已停止写入"
            ));
        }
        if chunk.is_empty() {
            break;
        }
        write_chunk(session, handle, runtime, file, &chunk).await?;
        *cursor += chunk.len() as u64;
        *step_bytes += chunk.len() as u64;
    }
    if *cursor <= request_end {
        return Err("分片响应提前结束，已保留数据供有界重试".into());
    }
    Ok(())
}

async fn write_chunk(
    session: &Session,
    handle: &WindowTransferHandle,
    runtime: &SegmentRuntime,
    file: &mut BufWriter<fs::File>,
    chunk: &[u8],
) -> Result<(), String> {
    let bytes = chunk.len() as u64;
    session
        .config
        .bandwidth
        .acquire(
            &session.config.task_id,
            bytes,
            session.options.priority.load(Ordering::Relaxed),
            &session.worker_cancel,
        )
        .await;
    if session.worker_cancel.is_cancelled() {
        return Err("任务已暂停".into());
    }
    session
        .config
        .limiter
        .acquire_with_cancel(
            bytes,
            session.options.speed_limit.load(Ordering::Relaxed),
            &session.worker_cancel,
        )
        .await;
    if session.worker_cancel.is_cancelled() {
        return Err("任务已暂停".into());
    }
    file.write_all(chunk)
        .await
        .map_err(|error| error.to_string())?;
    handle.downloaded_bytes.fetch_add(bytes, Ordering::Relaxed);
    runtime.downloaded_bytes.fetch_add(bytes, Ordering::Relaxed);
    session.progress.fetch_add(bytes, Ordering::Relaxed);
    Ok(())
}
