use super::*;

#[derive(Debug, PartialEq, Eq)]
struct Shares { http: u64, bt: u64 }

fn shares(total: u64, http: usize, bt: usize) -> Shares {
    if total == 0 { return Shares { http: 0, bt: 0 }; }
    match (http, bt) {
        (0, _) => Shares { http: 0, bt: total },
        (_, 0) => Shares { http: total, bt: 0 },
        _ => {
            let count = (http + bt) as u128;
            let assigned = ((total as u128 * http as u128) / count) as u64;
            // 设置以KiB/s输入，最多16个活动下载；正份额不能误变成“0=无限”。
            let assigned = assigned.max(1).min(total.saturating_sub(1));
            Shares { http: assigned, bt: total - assigned }
        }
    }
}

impl DownloadManager {
    /// 仅在任务启动/结束和设置变化时重分配；不增加任务全量轮询。
    pub(super) async fn rebalance_bandwidth(&self) -> Result<(), String> {
        let _guard = self.bandwidth_allocation.lock().await;
        let settings = self.settings().await;
        let total = bandwidth::effective_global_limit_kbps(settings.speed_limit_kbps,
            settings.scheduled_limit.as_ref(), bandwidth::local_minute_of_day()).saturating_mul(1024);
        let (http, bt) = {
            let controls = self.controls.lock().await;
            let bt = controls.values().filter(|control| control.kind == TaskKind::Bt).count();
            (controls.len() - bt, bt)
        };
        let allocation = shares(total, http, bt);
        // 先减小两个传输引擎的额度，再增加目标份额，防止切换时同时各拿全局额度。
        if total > 0 {
            self.bandwidth_scheduler.set_limit(1);
            self.bt.set_global_download_budget(1).await?;
        }
        self.bandwidth_scheduler.set_limit(if http == 0 { total } else { allocation.http });
        self.bt.set_global_budgets(if bt == 0 { total } else { allocation.bt }, total).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn positive_limits_are_shared_instead_of_duplicated_or_rounded_to_unlimited() {
        for http in 1..16 {
            for bt in 1..16 {
                let allocation = shares(1024, http, bt);
                assert!(allocation.http > 0 && allocation.bt > 0);
                assert_eq!(allocation.http + allocation.bt, 1024);
            }
        }
        assert_eq!(shares(0, 2, 3), Shares { http: 0, bt: 0 });
        assert_eq!(shares(1024, 0, 4), Shares { http: 0, bt: 1024 });
        assert_eq!(shares(1024, 4, 0), Shares { http: 1024, bt: 0 });
    }
}
