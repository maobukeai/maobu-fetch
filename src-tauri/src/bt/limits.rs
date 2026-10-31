//! 共享预算与原生上下行限速，状态切换保留同一 limiter。
use super::BtEngine;
use crate::models::{AppSettings, DownloadTask};
use librqbit::Session;

impl BtEngine {
    pub fn apply_ratelimits(session: &Session, settings: &AppSettings) {
        let dl_bps = if settings.speed_limit_kbps > 0 {
            let bytes_per_sec = settings.speed_limit_kbps.saturating_mul(1024);
            std::num::NonZeroU32::new(bytes_per_sec.min(u32::MAX as u64) as u32)
        } else {
            None
        };
        let ul_bps = if settings.bt_upload_limit_kbps > 0 {
            let bytes_per_sec = settings.bt_upload_limit_kbps.saturating_mul(1024);
            std::num::NonZeroU32::new(bytes_per_sec.min(u32::MAX as u64) as u32)
        } else {
            None
        };
        session.ratelimits.set_download_bps(dl_bps);
        session.ratelimits.set_upload_bps(ul_bps);
    }

    pub(super) fn apply_session_limits(&self, session: &Session, settings: &AppSettings) {
        Self::apply_ratelimits(session, settings);
        let budget = self
            .download_budget
            .load(std::sync::atomic::Ordering::Relaxed);
        if budget > 0 {
            let configured = settings.speed_limit_kbps.saturating_mul(1024);
            session
                .ratelimits
                .set_download_bps(as_bps(if configured == 0 {
                    budget
                } else {
                    budget.min(configured)
                }));
        }
        let upload_budget = self
            .upload_budget
            .load(std::sync::atomic::Ordering::Relaxed);
        let configured_upload = session
            .ratelimits
            .get_upload_bps()
            .map(|b| b.get() as u64)
            .unwrap_or(0);
        let global_upload = settings.speed_limit_kbps.saturating_mul(1024);
        let upload = [configured_upload, global_upload, upload_budget]
            .into_iter()
            .filter(|value| *value > 0)
            .min()
            .unwrap_or(0);
        session.ratelimits.set_upload_bps(as_bps(upload));
    }

    pub async fn set_global_download_budget(&self, bytes_per_second: u64) -> Result<(), String> {
        self.download_budget
            .store(bytes_per_second, std::sync::atomic::Ordering::Relaxed);
        self.refresh_budget().await;
        Ok(())
    }

    pub async fn set_global_upload_budget(&self, bytes_per_second: u64) -> Result<(), String> {
        self.upload_budget
            .store(bytes_per_second, std::sync::atomic::Ordering::Relaxed);
        self.refresh_budget().await;
        Ok(())
    }

    pub async fn set_global_budgets(
        &self,
        download_bps: u64,
        upload_bps: u64,
    ) -> Result<(), String> {
        self.download_budget
            .store(download_bps, std::sync::atomic::Ordering::Relaxed);
        self.upload_budget
            .store(upload_bps, std::sync::atomic::Ordering::Relaxed);
        self.refresh_budget().await;
        Ok(())
    }

    pub(super) async fn refresh_budget(&self) {
        let settings = self.last_settings.lock().await.clone().unwrap_or_default();
        if let Some(session) = self.session.lock().await.as_ref() {
            self.apply_session_limits(session, &settings);
        }
    }

    pub async fn set_task_limits(
        &self,
        task_id: &str,
        download_bps: u64,
        upload_bps: u64,
    ) -> Result<(), String> {
        let handle = self.bindings.lock().await.get(task_id).cloned();
        // 尚在获取元数据的任务没有数据连接；持久化设置会在绑定和轮询时应用。
        let Some(handle) = handle else {
            return Ok(());
        };
        handle.ratelimits().set_download_bps(as_bps(download_bps));
        handle.ratelimits().set_upload_bps(as_bps(upload_bps));
        Ok(())
    }

    pub async fn active_downloads(&self) -> usize {
        self.bindings
            .lock()
            .await
            .values()
            .filter(|handle| {
                let stats = handle.stats();
                !stats.finished
                    && matches!(
                        stats.state,
                        librqbit::TorrentStatsState::Live
                            | librqbit::TorrentStatsState::Initializing { paused: false }
                    )
            })
            .count()
    }

    pub async fn apply_settings(&self, settings: &AppSettings) -> Result<(), String> {
        {
            let mut last_settings = self.last_settings.lock().await;
            *last_settings = Some(settings.clone());
        }
        let session = self.session.lock().await.clone();
        if let Some(session) = session.as_ref() {
            self.apply_session_limits(session, settings);
        }
        if !settings.bt_seed_enabled {
            self.cancel_seed_preparation(None);
            let completed = self
                .bindings
                .lock()
                .await
                .iter()
                .filter(|(_, handle)| handle.stats().finished)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            for id in completed {
                self.pause_task(&id).await?;
            }
        }
        Ok(())
    }
}

pub(super) fn as_bps(bytes_per_second: u64) -> Option<std::num::NonZeroU32> {
    std::num::NonZeroU32::new(bytes_per_second.min(u32::MAX as u64) as u32)
}

pub(super) fn task_limits(task: &DownloadTask) -> librqbit::limits::LimitsConfig {
    let bytes = task.per_task_speed_limit;
    librqbit::limits::LimitsConfig {
        download_bps: as_bps(bytes),
        upload_bps: as_bps(bytes),
    }
}
