use super::*;

#[derive(Default)]
pub(super) struct DiskState {
    pub paused: AtomicBool,
    pub available: AtomicU64,
    pub required: AtomicU64,
}

/// Observation only: the main flow persists the final low-disk state after every writer has stopped.
pub(super) async fn monitor(destination: String, session: Arc<Session>) {
    let mut last_check_at = Instant::now()
        .checked_sub(DISK_CHECK_TIME_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut last_check_bytes = session.plan.initial;
    loop {
        tokio::select! {
            _ = session.reporter_stop.cancelled() => break,
            _ = session.token.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
        if session.disk.paused.load(Ordering::Relaxed) {
            break;
        }
        let progress = session.progress.load(Ordering::Relaxed);
        if progress.saturating_sub(last_check_bytes) < DISK_CHECK_BYTES_INTERVAL
            && last_check_at.elapsed() < DISK_CHECK_TIME_INTERVAL
        {
            continue;
        }
        match check_disk_space_once(&destination, session.total, progress) {
            Ok(()) => {
                last_check_at = Instant::now();
                last_check_bytes = progress;
            }
            Err((available, required)) => {
                session.disk.paused.store(true, Ordering::Relaxed);
                session.disk.available.store(available, Ordering::Relaxed);
                session.disk.required.store(required, Ordering::Relaxed);
                session.token.cancel();
                break;
            }
        }
    }
}
