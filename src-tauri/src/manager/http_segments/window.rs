use super::super::work_stealing::WindowTransferHandle;
use super::*;

pub(super) async fn transfer(
    session: &Session,
    window: &RangeWindow,
    handle: &WindowTransferHandle,
    runtime: &SegmentRuntime,
) -> Result<(), String> {
    pending::begin(&session.config.task_id, window).await?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&window.path)
        .await
        .map_err(|error| format!("无法打开分片临时文件：{error}"))?;
    let mut file = BufWriter::with_capacity(session.config.buffer_size, file);
    let result = transfer_steps(session, window, handle, runtime, &mut file).await;
    // Every exit, including a remote-validator failure, must preserve buffered bytes or report the flush error.
    let flush = file
        .flush()
        .await
        .map_err(|error| format!("保存分片临时文件失败：{error}"));
    let sync = if result.is_ok() && flush.is_ok() {
        file.get_ref()
            .sync_all()
            .await
            .map_err(|error| format!("同步分片临时文件失败：{error}"))
    } else {
        Ok(())
    };
    drop(file);
    let full = handle.current_downloaded() >= window.end_byte - window.start_byte + 1;
    if result
        .as_ref()
        .is_err_and(|error| error.starts_with(INVALID_RANGE_PREFIX))
        || (result.is_err() || flush.is_err() || sync.is_err()) && full
    {
        quarantine::isolate(session, window, handle, runtime).await?;
    }
    flush?;
    sync?;
    if result.is_ok() {
        pending::remove(&window.path).await?;
    }
    result
}

async fn transfer_steps(
    session: &Session,
    window: &RangeWindow,
    handle: &WindowTransferHandle,
    runtime: &SegmentRuntime,
    file: &mut BufWriter<fs::File>,
) -> Result<(), String> {
    let mut cursor = window.start_byte.saturating_add(window.existing_bytes);
    let mut retries = retry::RetryState::new(window.existing_bytes);
    loop {
        if session.worker_cancel.is_cancelled() {
            return Err("任务已暂停".into());
        }
        if cursor > handle.current_end() {
            return Ok(());
        }
        let step = response::receive(session, handle, runtime, file, &mut cursor).await;
        retries.check_progress(session, handle, runtime, cursor, &step)?;
        // Flush before sleeping/reconnecting, and never replace a file error with an apparent retry success.
        file.flush().await.map_err(|error| error.to_string())?;
        let successful = step.result.is_ok();
        if !successful && cursor > handle.current_end() {
            return step.result;
        }
        retries.after_step(session, runtime, step.result).await?;
        if cursor > handle.current_end() {
            return Ok(());
        }
        if successful {
            retry::wait(
                session,
                Duration::from_millis(8 + (runtime.index as u64 % 8) * 7),
            )
            .await?;
        }
    }
}
