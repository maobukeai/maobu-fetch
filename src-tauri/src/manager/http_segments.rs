//! Concurrent Range execution; planning, observation, transfer and merge have separate lifetimes.
use super::*;
use tokio_util::task::TaskTracker;
mod disk;
mod merge;
mod outcome;
mod pending;
mod plan;
mod quarantine;
mod reporter;
mod response;
mod retry;
mod runtime;
mod window;
mod workers;
use runtime::Session;
const INVALID_RANGE_PREFIX: &str = "HTTP_RANGE_INVALID:";

pub(super) async fn download(
    manager: &DownloadManager,
    mut task: DownloadTask,
    client: &reqwest::Client,
    temp: &Path,
    total: u64,
    connections: u8,
    token: CancellationToken,
    limiter: Arc<RateLimiter>,
) -> Result<DownloadTask, String> {
    let session = Arc::new(
        Session::new(
            manager,
            &task,
            client,
            temp,
            total,
            connections,
            token,
            limiter,
        )
        .await?,
    );
    task.downloaded_bytes = session.plan.initial;
    task.segments = snapshot_segments(&session.plan.runtimes);
    task.active_connections = 0;
    manager.store.upsert_task(&task).await?;
    manager.emit_task("updated", &task);

    let children = manager.child_tasks(&task.id).await;
    let _reporter_guard = session.reporter_stop.clone().drop_guard();
    let monitors = reporter::Monitors::spawn(manager, &task, session.clone(), &children);
    let error = workers::run(session.clone(), &children).await;
    let error = monitors.finish(error).await;
    outcome::settle(manager, &mut task, &session, error).await?;
    merge::complete(manager, &mut task, temp, &session).await?;

    task.downloaded_bytes = total;
    task.segments = snapshot_segments(&session.plan.runtimes);
    Ok(task)
}
