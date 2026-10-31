use super::super::work_stealing::WindowTransferHandle;
use super::*;

pub(super) async fn run(session: Arc<Session>, children: &TaskTracker) -> Option<String> {
    let mut workers = tokio::task::JoinSet::new();
    for index in 0..session.connections {
        workers.spawn(children.track_future(worker(index, session.clone())));
    }
    let mut error: Option<String> = None;
    while let Some(result) = workers.join_next().await {
        let cause = match result {
            Ok(Ok(())) => continue,
            Ok(Err(cause)) => cause,
            Err(cause) => format!("Worker 异常终止: {cause}"),
        };
        // Preserve a refreshable expired-link sentinel over secondary cancellation/retry errors.
        if error.as_ref().is_none_or(|previous| {
            !previous.starts_with(CLOUD_LINK_DEAD_PREFIX)
                && cause.starts_with(CLOUD_LINK_DEAD_PREFIX)
        }) {
            error = Some(cause);
        }
        session.worker_cancel.cancel();
    }
    error
}

async fn stagger(index: u8, session: &Session) {
    let delay = if session.config.url.contains("baidupcs.com")
        || session.config.url.contains("pan.baidu.com")
    {
        60
    } else if session.config.url.contains("mypikpak.com") {
        40
    } else {
        0
    };
    if index > 0 && delay > 0 {
        tokio::select! {
            _ = session.worker_cancel.cancelled() => {},
            _ = tokio::time::sleep(Duration::from_millis(index as u64 * delay)) => {},
        }
    }
}

async fn worker(index: u8, session: Arc<Session>) -> Result<(), String> {
    stagger(index, &session).await;
    loop {
        if session.worker_cancel.is_cancelled() {
            return Err("任务已暂停".into());
        }
        let work = session.plan.coordinator.claim_pending_work().await;
        let Some((window, handle)) = work else {
            if session.plan.coordinator.is_all_completed().await {
                return Ok(());
            }
            tokio::select! {
                _ = session.worker_cancel.cancelled() => return Err("任务已暂停".into()),
                _ = tokio::time::sleep(Duration::from_millis(50)) => {},
            }
            continue;
        };
        execute_window(&session, &window, &handle).await?;
    }
}

async fn execute_window(
    session: &Session,
    window: &RangeWindow,
    handle: &Arc<WindowTransferHandle>,
) -> Result<(), String> {
    let Some(runtime) = session
        .plan
        .runtimes
        .iter()
        .find(|r| r.index == window.segment_index)
    else {
        session
            .plan
            .coordinator
            .finish_window(window.id, false, window.existing_bytes)
            .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        return Ok(());
    };
    let result = window::transfer(session, window, handle, runtime).await;
    session
        .plan
        .coordinator
        .finish_window(window.id, result.is_ok(), handle.current_downloaded())
        .await;
    let expected = runtime.end_byte - runtime.start_byte + 1;
    let status = if runtime.downloaded_bytes.load(Ordering::Relaxed) >= expected {
        SEGMENT_COMPLETED
    } else if runtime.active_windows.load(Ordering::Relaxed) > 0 {
        SEGMENT_DOWNLOADING
    } else {
        SEGMENT_PENDING
    };
    runtime.status.store(status, Ordering::Relaxed);
    if result.is_err() {
        session.worker_cancel.cancel();
    }
    result
}
