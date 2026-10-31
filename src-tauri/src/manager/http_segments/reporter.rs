use super::*;
use tokio::task::JoinHandle;

pub(super) struct Monitors {
    stop: CancellationToken,
    reporter: JoinHandle<Result<(), String>>,
    disk: JoinHandle<()>,
}
impl Monitors {
    pub fn spawn(
        manager: &DownloadManager,
        task: &DownloadTask,
        session: Arc<Session>,
        children: &TaskTracker,
    ) -> Self {
        let stop = session.reporter_stop.clone();
        let reporter = children.spawn(report(
            manager.store.clone(),
            manager.app.clone(),
            task.clone(),
            session.clone(),
        ));
        let disk = children.spawn(disk::monitor(task.destination.clone(), session));
        Self {
            stop,
            reporter,
            disk,
        }
    }
    pub async fn finish(self, mut error: Option<String>) -> Option<String> {
        self.stop.cancel();
        match self.reporter.await {
            Ok(Ok(())) => {}
            Ok(Err(cause)) => error = Some(cause),
            Err(cause) => error = Some(format!("进度工作线程异常结束：{cause}")),
        }
        if let Err(cause) = self.disk.await {
            error = Some(format!("磁盘检查工作线程异常结束：{cause}"));
        }
        error
    }
}

struct Samples {
    progress: ProgressSample,
    connection_at: Instant,
    connection_bytes: Vec<u64>,
    persisted_at: Instant,
    persisted_bytes: u64,
    persisted_statuses: Vec<u8>,
}
impl Samples {
    fn new(session: &Session) -> Self {
        Self {
            progress: ProgressSample::new(session.plan.initial),
            connection_at: Instant::now(),
            connection_bytes: connection_bytes(session),
            persisted_at: Instant::now(),
            persisted_bytes: session.plan.initial,
            persisted_statuses: Vec::new(),
        }
    }
    async fn persist(
        &mut self,
        store: &Store,
        snapshot: &mut DownloadTask,
        session: &Session,
    ) -> Result<(), String> {
        let statuses: Vec<u8> = session
            .plan
            .runtimes
            .iter()
            .map(|r| r.status.load(Ordering::Relaxed))
            .collect();
        if (snapshot.downloaded_bytes != self.persisted_bytes
            || statuses != self.persisted_statuses)
            && self.persisted_at.elapsed() >= Duration::from_secs(1)
        {
            match store.update_http_progress(snapshot).await {
                Ok(Some(current)) => *snapshot = current,
                Ok(None) => {
                    session.worker_cancel.cancel();
                    return Err("任务已停止或被删除，已保留分片".into());
                }
                Err(error) => {
                    session.worker_cancel.cancel();
                    return Err(format!("保存分片进度失败，已停止连接并保留分片：{error}"));
                }
            }
            self.persisted_at = Instant::now();
            self.persisted_bytes = snapshot.downloaded_bytes;
            self.persisted_statuses = statuses;
        }
        Ok(())
    }
    fn emit_connections(&mut self, app: &AppHandle, task_id: &str, session: &Session) {
        if self.connection_at.elapsed() < Duration::from_secs(1) {
            return;
        }
        let segments = snapshot_segment_statuses(
            &session.plan.runtimes,
            &self.connection_bytes,
            self.connection_at.elapsed().as_secs_f64(),
            false,
        );
        self.connection_at = Instant::now();
        self.connection_bytes = connection_bytes(session);
        let _ = app.emit(
            "task-connections",
            TaskConnectionsEvent {
                task_id: task_id.into(),
                segments,
                timestamp: now_millis(),
            },
        );
    }
}

fn connection_bytes(session: &Session) -> Vec<u64> {
    session
        .plan
        .runtimes
        .iter()
        .map(|r| r.downloaded_bytes.load(Ordering::Relaxed))
        .collect()
}

/// UI updates remain 250 ms; SQLite and connection snapshots are throttled to one second.
async fn report(
    store: Arc<Store>,
    app: AppHandle,
    mut snapshot: DownloadTask,
    session: Arc<Session>,
) -> Result<(), String> {
    let mut samples = Samples::new(&session);
    loop {
        tokio::select! {
            _ = session.reporter_stop.cancelled() => break,
            _ = session.token.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
        snapshot.downloaded_bytes = session.progress.load(Ordering::Relaxed);
        snapshot.segments = snapshot_segments(&session.plan.runtimes);
        session.options.apply(&mut snapshot).await;
        samples.progress.apply(&mut snapshot);
        session.adaptive.observe(snapshot.speed);
        let active: u32 = session
            .plan
            .runtimes
            .iter()
            .map(|r| r.active_windows.load(Ordering::Relaxed) as u32)
            .sum();
        snapshot.active_connections = active.min(session.connections as u32) as u8;
        samples.persist(&store, &mut snapshot, &session).await?;
        let _ = app.emit(
            "task-updated",
            TaskProgressEvent {
                task: snapshot.clone(),
                event: "updated".into(),
            },
        );
        samples.emit_connections(&app, &snapshot.id, &session);
    }
    Ok(())
}
