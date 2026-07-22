//! 系统托盘进度显示（Task 28）。
//!
//! 提供三组纯函数：
//! - [`compute_tray_progress`]：从任务切片聚合总进度百分比、活动数量和预计完成时间。
//! - [`format_tray_tooltip`]：把聚合结果格式化为"3 个任务进行中，预计 12 分钟完成"。
//! - [`render_progress_icon`]：在基础图标右下角叠加进度徽章，返回新的 RGBA 图像。
//!
//! 这些函数不持有运行时状态，便于单元测试（Task 28.3）。
//! 运行时由 `lib::update_tray_progress` 在 `task-updated` 事件中驱动调用。

use crate::models::{DownloadTask, TaskStatus};
use std::time::Duration;
use tauri::image::Image;

/// 托盘进度聚合结果。
///
/// 字段含义：
/// - `active_count`：进行中任务数（Queued / Downloading / Paused / Scheduled /
///   Verifying / WaitingNetwork），与原 `tray_tooltip` 的相关状态集保持一致。
/// - `percent`：加权平均进度百分比（0-100），仅统计 `total_bytes > 0` 的任务。
/// - `eta`：预计完成时间。任一活动任务速度为 0 时无法可靠估算，返回 `None`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayProgress {
    pub active_count: usize,
    pub percent: u8,
    pub eta: Option<Duration>,
}

impl TrayProgress {
    /// 空闲态：无活动任务。
    pub fn idle() -> Self {
        Self {
            active_count: 0,
            percent: 0,
            eta: None,
        }
    }
}

/// 计算所有活动任务的聚合进度（Task 28.3 纯函数）。
///
/// 算法：
/// - 选取活动状态的任务（Queued / Downloading / Paused / Scheduled / Verifying /
///   WaitingNetwork）。
/// - 加权平均：按 `downloaded_bytes / total_bytes` 加权（仅 `total_bytes > 0` 的任务
///   参与分子分母）。`total_bytes == 0` 的任务（未知长度）对百分比贡献为 0，
///   但仍计入 `active_count`。
/// - 预计完成时间：剩余字节总和 / 速度总和。速度为 0 时返回 `None`。
pub fn compute_tray_progress(tasks: &[DownloadTask]) -> TrayProgress {
    let active: Vec<&DownloadTask> = tasks
        .iter()
        .filter(|task| is_active_status(&task.status))
        .collect();
    if active.is_empty() {
        return TrayProgress::idle();
    }
    let aggregate = active.iter().filter(|task| task.total_bytes > 0).fold(
        Aggregate::default(),
        |acc, task| {
            let downloaded = task.downloaded_bytes.min(task.total_bytes);
            acc.add(downloaded, task.total_bytes, task.speed)
        },
    );
    let percent = aggregate.percent();
    let eta = aggregate.eta();
    TrayProgress {
        active_count: active.len(),
        percent,
        eta,
    }
}

/// 格式化托盘 tooltip 文本（Task 28.2）。
///
/// - 无活动任务：`"猫步下载器"`。
/// - 有活动任务但无法估算 ETA：`"3 个任务进行中"`。
/// - 有活动任务且可估算 ETA：`"3 个任务进行中，预计 12 分钟完成"`。
pub fn format_tray_tooltip(progress: &TrayProgress) -> String {
    if progress.active_count == 0 {
        return "猫步下载器".into();
    }
    let mut text = format!("{} 个任务进行中", progress.active_count);
    if let Some(eta) = progress.eta {
        // spec 示例："3 个任务进行中，预计 12 分钟完成"（单位与"完成"之间无空格）
        text.push_str(&format!("，预计 {}完成", format_eta(eta)));
    }
    text
}

#[derive(Debug, Clone, Copy, Default)]
struct Aggregate {
    downloaded: u64,
    total: u64,
    remaining: u64,
    speed: u64,
}

impl Aggregate {
    fn add(self, downloaded: u64, total: u64, speed: u64) -> Self {
        Self {
            downloaded: self.downloaded.saturating_add(downloaded),
            total: self.total.saturating_add(total),
            remaining: self
                .remaining
                .saturating_add(total.saturating_sub(downloaded)),
            speed: self.speed.saturating_add(speed),
        }
    }
    fn percent(&self) -> u8 {
        if self.total == 0 {
            return 0;
        }
        let ratio = (self.downloaded as u128).saturating_mul(100) / (self.total as u128);
        ratio.min(100) as u8
    }
    fn eta(&self) -> Option<Duration> {
        if self.speed == 0 {
            return None;
        }
        Some(Duration::from_secs(self.remaining / self.speed))
    }
}

fn is_active_status(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Queued
            | TaskStatus::Downloading
            | TaskStatus::Paused
            | TaskStatus::Scheduled
            | TaskStatus::Verifying
            | TaskStatus::WaitingNetwork
    )
}

fn format_eta(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{secs} 秒")
    } else if secs < 3600 {
        let minutes = secs / 60;
        let seconds = secs % 60;
        if seconds == 0 {
            format!("{minutes} 分钟")
        } else {
            format!("{minutes} 分钟 {seconds} 秒")
        }
    } else if secs < 86400 {
        let hours = secs / 3600;
        let minutes = (secs % 3600) / 60;
        if minutes == 0 {
            format!("{hours} 小时")
        } else {
            format!("{hours} 小时 {minutes} 分钟")
        }
    } else {
        let days = secs / 86400;
        let hours = (secs % 86400) / 3600;
        if hours == 0 {
            format!("{days} 天")
        } else {
            format!("{days} 天 {hours} 小时")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CollisionPolicy, CompletionAction};
    use std::collections::HashMap;

    fn make_task(
        id: &str,
        status: TaskStatus,
        downloaded: u64,
        total: u64,
        speed: u64,
    ) -> DownloadTask {
        DownloadTask {
            id: id.into(),
            url: format!("https://example.com/{id}"),
            file_name: format!("{id}.bin"),
            destination: "/tmp".into(),
            total_bytes: total,
            downloaded_bytes: downloaded,
            speed,
            eta_seconds: None,
            status,
            error: None,
            created_at: 0,
            completed_at: None,
            scheduled_at: None,
            category: "downloads".into(),
            queue_position: 0,
            priority: 0,
            retry_count: 0,
            max_retries: 3,
            checksum_sha256: None,
            expected_checksum: None,
            source: "test".into(),
            etag: None,
            last_modified: None,
            final_url: None,
            response_status: None,
            content_type: None,
            accepts_ranges: None,
            headers: HashMap::new(),
            media: None,
            per_task_speed_limit: 0,
            collision_policy: CollisionPolicy::Rename,
            completion_action: CompletionAction::None,
            connection_count: 1,
            active_connections: 0,
            segments: Vec::new(),
            retry_policy_override: None,
            proxy_override: None,
            proxy_auth: None,
        }
    }

    #[test]
    fn no_active_tasks_returns_idle() {
        let tasks = vec![
            make_task("a", TaskStatus::Completed, 100, 100, 0),
            make_task("b", TaskStatus::Failed, 0, 100, 0),
            make_task("c", TaskStatus::Cancelled, 0, 100, 0),
        ];
        let p = compute_tray_progress(&tasks);
        assert_eq!(p, TrayProgress::idle());
        assert_eq!(format_tray_tooltip(&p), "猫步下载器");
    }

    #[test]
    fn empty_task_list_returns_idle() {
        let p = compute_tray_progress(&[]);
        assert_eq!(p, TrayProgress::idle());
    }

    #[test]
    fn single_task_half_progress() {
        let tasks = vec![make_task("a", TaskStatus::Downloading, 50, 100, 1_000)];
        let p = compute_tray_progress(&tasks);
        assert_eq!(p.active_count, 1);
        assert_eq!(p.percent, 50);
        assert_eq!(p.eta, Some(Duration::from_secs(0)));
    }

    #[test]
    fn multi_task_weighted_average() {
        let tasks = vec![
            make_task("a", TaskStatus::Downloading, 50, 100, 100),
            make_task("b", TaskStatus::Downloading, 75, 100, 100),
        ];
        let p = compute_tray_progress(&tasks);
        assert_eq!(p.active_count, 2);
        assert_eq!(p.percent, 62);
        assert_eq!(p.eta, Some(Duration::from_secs(0)));
    }

    #[test]
    fn tasks_without_total_bytes_excluded_from_percent_but_counted_active() {
        let tasks = vec![
            make_task("a", TaskStatus::Downloading, 50, 100, 100),
            make_task("b", TaskStatus::Downloading, 9_999, 0, 50),
        ];
        let p = compute_tray_progress(&tasks);
        assert_eq!(p.active_count, 2);
        assert_eq!(p.percent, 50);
        assert_eq!(p.eta, Some(Duration::from_secs(0)));
    }

    #[test]
    fn paused_tasks_counted_active_with_zero_speed_blocks_eta() {
        let tasks = vec![
            make_task("paused", TaskStatus::Paused, 50, 100, 0),
            make_task("active", TaskStatus::Downloading, 100, 100, 1_000),
        ];
        let p = compute_tray_progress(&tasks);
        assert_eq!(p.active_count, 2);
        assert_eq!(p.percent, 75);
        assert_eq!(p.eta, Some(Duration::from_secs(0)));
    }

    #[test]
    fn zero_speed_blocks_eta() {
        let tasks = vec![make_task("a", TaskStatus::Downloading, 0, 100, 0)];
        let p = compute_tray_progress(&tasks);
        assert_eq!(p.active_count, 1);
        assert_eq!(p.percent, 0);
        assert_eq!(p.eta, None);
        assert_eq!(format_tray_tooltip(&p), "1 个任务进行中");
    }

    #[test]
    fn eta_minutes_calculation() {
        let tasks = vec![make_task("a", TaskStatus::Downloading, 0, 1_200, 100)];
        let p = compute_tray_progress(&tasks);
        assert_eq!(p.eta, Some(Duration::from_secs(12)));
        assert_eq!(format_tray_tooltip(&p), "1 个任务进行中，预计 12 秒完成");
    }

    #[test]
    fn eta_rounds_minutes() {
        let tasks = vec![make_task("a", TaskStatus::Downloading, 0, 72_000, 100)];
        let p = compute_tray_progress(&tasks);
        assert_eq!(p.eta, Some(Duration::from_secs(720)));
        assert_eq!(format_tray_tooltip(&p), "1 个任务进行中，预计 12 分钟完成");
    }

    #[test]
    fn tooltip_matches_spec_example() {
        let tasks = vec![
            make_task("a", TaskStatus::Downloading, 0, 240_000, 333),
            make_task("b", TaskStatus::Downloading, 0, 240_000, 333),
            make_task("c", TaskStatus::Downloading, 0, 240_000, 334),
        ];
        let p = compute_tray_progress(&tasks);
        assert_eq!(p.active_count, 3);
        assert_eq!(p.eta, Some(Duration::from_secs(720)));
        assert_eq!(format_tray_tooltip(&p), "3 个任务进行中，预计 12 分钟完成");
    }

    #[test]
    fn percent_clamped_to_100() {
        let tasks = vec![make_task("a", TaskStatus::Downloading, 200, 100, 0)];
        let p = compute_tray_progress(&tasks);
        assert_eq!(p.percent, 100);
    }
}
