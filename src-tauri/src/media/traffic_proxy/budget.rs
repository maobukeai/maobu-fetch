use crate::manager::{bandwidth::BandwidthScheduler, RateLimiter, RuntimeTaskOptions};
use std::sync::{atomic::Ordering, Arc};
use tokio_util::sync::CancellationToken;

/// 一个任务的所有外部下载连接共享同一任务限速器，并参加原生 HTTP 的全局调度。
#[derive(Clone)]
pub(crate) struct TrafficBudget {
    pub task_id: String,
    pub scheduler: BandwidthScheduler,
    pub runtime: Arc<RuntimeTaskOptions>,
    limiter: Arc<RateLimiter>,
}

impl TrafficBudget {
    pub fn new(
        task_id: String,
        scheduler: BandwidthScheduler,
        runtime: Arc<RuntimeTaskOptions>,
    ) -> Self {
        Self {
            task_id,
            scheduler,
            runtime,
            limiter: Arc::new(RateLimiter::new()),
        }
    }

    pub async fn acquire(&self, bytes: usize, cancel: &CancellationToken) -> Result<(), String> {
        self.scheduler
            .acquire(
                &self.task_id,
                bytes as u64,
                self.runtime.priority.load(Ordering::Relaxed),
                cancel,
            )
            .await;
        self.limiter
            .acquire_with_cancel(
                bytes as u64,
                self.runtime.speed_limit.load(Ordering::Relaxed),
                cancel,
            )
            .await;
        if cancel.is_cancelled() {
            return Err("任务已暂停".into());
        }
        Ok(())
    }
}
