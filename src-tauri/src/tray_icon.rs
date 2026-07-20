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

/// 在基础图标右下角叠加进度徽章（Task 28.1）。
///
/// 徽章背景色按百分比分段：
/// - 0-25%：红 `#E63946`
/// - 26-50%：橙 `#F4A261`
/// - 51-75%：蓝绿 `#2A9D8F`
/// - 76-100%：绿 `#06A77D`
///
/// 中间显示百分比数字（最多两位），白色像素绘制。`percent == 0` 时不绘制徽章，
/// 直接返回基础图标的副本。
///
/// 失败时返回中文错误信息，不 panic（AGENTS.md §7）。
pub fn render_progress_icon(percent: u8, base: &Image<'_>) -> Result<Image<'static>, String> {
    let width = base.width();
    let height = base.height();
    if width == 0 || height == 0 {
        return Err("基础图标尺寸无效".into());
    }
    let src_rgba = base.rgba();
    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| "基础图标尺寸溢出".to_string())?;
    if src_rgba.len() < expected_len {
        return Err("基础图标 RGBA 数据长度与尺寸不匹配".into());
    }
    let mut rgba = src_rgba.to_vec();
    if percent == 0 {
        return Ok(Image::new_owned(rgba, width, height));
    }
    let Some(layout) = compute_badge_layout(width, height) else {
        return Ok(Image::new_owned(rgba, width, height));
    };
    let origin_x = width.saturating_sub(layout.badge_size);
    let origin_y = height.saturating_sub(layout.badge_size);
    let bg = badge_color(percent);
    draw_badge(
        &mut rgba,
        width,
        height,
        origin_x,
        origin_y,
        layout.badge_size,
        bg,
    );
    draw_percent_text(
        &mut rgba,
        width,
        height,
        origin_x,
        origin_y,
        layout.badge_size,
        layout.font_scale,
        percent,
    );
    Ok(Image::new_owned(rgba, width, height))
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

#[derive(Debug, Clone, Copy)]
struct BadgeLayout {
    badge_size: u32,
    /// 字形像素倍数（1 = 5x7，2 = 10x14 等）。
    font_scale: u32,
}

fn compute_badge_layout(width: u32, height: u32) -> Option<BadgeLayout> {
    let min_side = width.min(height);
    if min_side < 16 {
        return None;
    }
    // 徽章占图标短边的 1/2，钳制到 [12, 24]，避免极端尺寸下徽章过大或不可辨识。
    let badge_size = (min_side / 2).clamp(12, 24);
    // 字形像素倍数：badge >= 24 时使用 2x（10x14），否则 1x（5x7）。
    let font_scale = if badge_size >= 24 { 2 } else { 1 };
    Some(BadgeLayout {
        badge_size,
        font_scale,
    })
}

fn badge_color(percent: u8) -> [u8; 4] {
    match percent {
        0..=25 => [230, 57, 70, 255],   // #E63946 红
        26..=50 => [244, 162, 97, 255], // #F4A261 橙
        51..=75 => [42, 157, 143, 255], // #2A9D8F 蓝绿
        _ => [6, 167, 125, 255],        // #06A77D 绿
    }
}

fn draw_badge(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    origin_x: u32,
    origin_y: u32,
    size: u32,
    color: [u8; 4],
) {
    let corner = (size / 6).max(1);
    for y in 0..size {
        for x in 0..size {
            let px = origin_x.saturating_add(x);
            let py = origin_y.saturating_add(y);
            if px >= width || py >= height {
                continue;
            }
            if is_corner_pixel(x, y, size, corner) {
                continue;
            }
            write_pixel(rgba, width, px, py, color);
        }
    }
}

fn is_corner_pixel(x: u32, y: u32, size: u32, corner: u32) -> bool {
    let near_right = x >= size.saturating_sub(corner);
    let near_bottom = y >= size.saturating_sub(corner);
    (x < corner && y < corner)
        || (near_right && y < corner)
        || (x < corner && near_bottom)
        || (near_right && near_bottom)
}

fn draw_percent_text(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    origin_x: u32,
    origin_y: u32,
    badge_size: u32,
    font_scale: u32,
    percent: u8,
) {
    let display = percent.min(99);
    let tens = display / 10;
    let ones = display % 10;
    let digits: Vec<u8> = if display >= 10 {
        vec![tens, ones]
    } else {
        vec![ones]
    };
    let glyph_w = 5u32;
    let glyph_h = 7u32;
    let spacing = 1u32;
    let scaled_w = glyph_w * font_scale;
    let scaled_h = glyph_h * font_scale;
    let total_w = digits.len() as u32 * scaled_w
        + digits.len().saturating_sub(1) as u32 * spacing * font_scale;
    let start_x = origin_x + (badge_size.saturating_sub(total_w)) / 2;
    let start_y = origin_y + (badge_size.saturating_sub(scaled_h)) / 2;
    for (index, &digit) in digits.iter().enumerate() {
        let glyph = digit_glyph(digit);
        let dx = start_x + index as u32 * (scaled_w + spacing * font_scale);
        draw_glyph(
            rgba,
            width,
            height,
            dx,
            start_y,
            glyph,
            font_scale,
            [255, 255, 255, 255],
        );
    }
}

fn draw_glyph(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    origin_x: u32,
    origin_y: u32,
    glyph: &[u8; 35],
    scale: u32,
    color: [u8; 4],
) {
    for gy in 0..7u32 {
        for gx in 0..5u32 {
            let bit = glyph[(gy * 5 + gx) as usize];
            if bit == 0 {
                continue;
            }
            for dy in 0..scale {
                for dx in 0..scale {
                    let px = origin_x.saturating_add(gx * scale + dx);
                    let py = origin_y.saturating_add(gy * scale + dy);
                    if px >= width || py >= height {
                        continue;
                    }
                    write_pixel(rgba, width, px, py, color);
                }
            }
        }
    }
}

fn write_pixel(rgba: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 4]) {
    let index = match (y as usize)
        .checked_mul(width as usize)
        .and_then(|row| row.checked_add(x as usize))
        .and_then(|i| i.checked_mul(4))
    {
        Some(i) if i + 4 <= rgba.len() => i,
        _ => return,
    };
    rgba[index] = color[0];
    rgba[index + 1] = color[1];
    rgba[index + 2] = color[2];
    rgba[index + 3] = color[3];
}

/// 5x7 像素数字字形（0-9）。每行 5 位，共 7 行，行优先。
fn digit_glyph(d: u8) -> &'static [u8; 35] {
    const GLYPHS: [[u8; 35]; 10] = [
        [
            0, 1, 1, 1, 0, 1, 0, 0, 0, 1, 1, 0, 0, 1, 1, 1, 0, 1, 0, 1, 1, 1, 0, 0, 1, 1, 0, 0, 0,
            1, 0, 1, 1, 1, 0,
        ],
        [
            0, 0, 1, 0, 0, 0, 1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0,
            0, 0, 1, 1, 1, 0,
        ],
        [
            0, 1, 1, 1, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0,
            0, 1, 1, 1, 1, 1,
        ],
        [
            1, 1, 1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 1, 1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0,
            1, 1, 1, 1, 1, 0,
        ],
        [
            0, 0, 0, 1, 0, 0, 0, 1, 1, 0, 0, 1, 0, 1, 0, 1, 0, 0, 1, 0, 1, 1, 1, 1, 1, 0, 0, 0, 1,
            0, 0, 0, 0, 1, 0,
        ],
        [
            1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 1, 0, 0, 0,
            1, 0, 1, 1, 1, 0,
        ],
        [
            0, 0, 1, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 1, 0, 0, 0, 1, 1, 0, 0, 0,
            1, 0, 1, 1, 1, 0,
        ],
        [
            1, 1, 1, 1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0,
            0, 0, 1, 0, 0, 0,
        ],
        [
            0, 1, 1, 1, 0, 1, 0, 0, 0, 1, 1, 0, 0, 0, 1, 0, 1, 1, 1, 0, 1, 0, 0, 0, 1, 1, 0, 0, 0,
            1, 0, 1, 1, 1, 0,
        ],
        [
            0, 1, 1, 1, 0, 1, 0, 0, 0, 1, 1, 0, 0, 0, 1, 0, 1, 1, 1, 1, 0, 0, 0, 0, 1, 0, 0, 0, 1,
            0, 0, 1, 1, 0, 0,
        ],
    ];
    &GLYPHS[d as usize]
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
        // 剩余 50B / 1000B/s = 0 秒
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
        // (50 + 75) / 200 = 62.5% → 62
        assert_eq!(p.percent, 62);
        // 剩余 (50 + 25) = 75B，速度 200B/s = 0 秒
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
        // 只有任务 a 贡献百分比：50/100 = 50%
        assert_eq!(p.percent, 50);
        // 剩余 50B / (100 + 50)B/s = 0 秒
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
        // (50 + 100) / 200 = 75%
        assert_eq!(p.percent, 75);
        // 剩余 50B / 1000B/s = 0 秒
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
        // 剩余 1200B / 100B/s = 12 秒 → "12 秒 完成"
        let tasks = vec![make_task("a", TaskStatus::Downloading, 0, 1_200, 100)];
        let p = compute_tray_progress(&tasks);
        assert_eq!(p.eta, Some(Duration::from_secs(12)));
        assert_eq!(format_tray_tooltip(&p), "1 个任务进行中，预计 12 秒完成");
    }

    #[test]
    fn eta_rounds_minutes() {
        // 720 秒 = 12 分钟
        let tasks = vec![make_task("a", TaskStatus::Downloading, 0, 72_000, 100)];
        let p = compute_tray_progress(&tasks);
        assert_eq!(p.eta, Some(Duration::from_secs(720)));
        assert_eq!(format_tray_tooltip(&p), "1 个任务进行中，预计 12 分钟完成");
    }

    #[test]
    fn tooltip_matches_spec_example() {
        // 复刻 spec 中的 "3 个任务进行中，预计 12 分钟完成"
        let tasks = vec![
            make_task("a", TaskStatus::Downloading, 0, 240_000, 333),
            make_task("b", TaskStatus::Downloading, 0, 240_000, 333),
            make_task("c", TaskStatus::Downloading, 0, 240_000, 334),
        ];
        let p = compute_tray_progress(&tasks);
        assert_eq!(p.active_count, 3);
        // 剩余 720000B / 1000B/s = 720 秒 = 12 分钟
        assert_eq!(p.eta, Some(Duration::from_secs(720)));
        assert_eq!(format_tray_tooltip(&p), "3 个任务进行中，预计 12 分钟完成");
    }

    #[test]
    fn percent_clamped_to_100() {
        // downloaded_bytes 超过 total_bytes 时应被钳制
        let tasks = vec![make_task("a", TaskStatus::Downloading, 200, 100, 0)];
        let p = compute_tray_progress(&tasks);
        assert_eq!(p.percent, 100);
    }

    #[test]
    fn render_progress_icon_zero_percent_returns_base_unchanged() {
        let base_rgba = vec![10u8; 32 * 32 * 4];
        let base = Image::new_owned(base_rgba.clone(), 32, 32);
        let out = render_progress_icon(0, &base).unwrap();
        assert_eq!(out.rgba(), base_rgba.as_slice());
        assert_eq!(out.width(), 32);
        assert_eq!(out.height(), 32);
    }

    #[test]
    fn render_progress_icon_draws_badge_in_corner() {
        let base_rgba = vec![0u8; 32 * 32 * 4];
        let base = Image::new_owned(base_rgba, 32, 32);
        let out = render_progress_icon(50, &base).unwrap();
        let rgba = out.rgba();
        // 16x16 徽章应位于右下角，检查徽章中心 (24, 24) 是否有非零像素
        let center_idx = ((24 * 32 + 24) * 4) as usize;
        assert!(rgba[center_idx] != 0 || rgba[center_idx + 3] != 0);
    }

    #[test]
    fn render_progress_icon_invalid_dimensions_returns_error() {
        let base = Image::new_owned(Vec::new(), 0, 0);
        assert!(render_progress_icon(50, &base).is_err());
    }

    #[test]
    fn render_progress_icon_rgba_length_mismatch_returns_error() {
        // 声明 32x32 但实际只提供 16 字节
        let base = Image::new_owned(vec![0u8; 16], 32, 32);
        assert!(render_progress_icon(50, &base).is_err());
    }

    #[test]
    fn render_progress_icon_skips_badge_when_icon_too_small() {
        let base_rgba = vec![0u8; 12 * 12 * 4];
        let base = Image::new_owned(base_rgba.clone(), 12, 12);
        let out = render_progress_icon(50, &base).unwrap();
        // 小于 16x16 不绘制徽章，应与基础一致
        assert_eq!(out.rgba(), base_rgba.as_slice());
    }

    #[test]
    fn render_progress_icon_100_percent_uses_green_badge() {
        let base_rgba = vec![0u8; 32 * 32 * 4];
        let base = Image::new_owned(base_rgba, 32, 32);
        let out = render_progress_icon(100, &base).unwrap();
        let rgba = out.rgba();
        // 绿色徽章 #06A77D 应出现在徽章中心区域
        let center_idx = ((24 * 32 + 24) * 4) as usize;
        assert_eq!(rgba[center_idx], 6); // R
        assert_eq!(rgba[center_idx + 1], 167); // G
        assert_eq!(rgba[center_idx + 2], 125); // B
        assert_eq!(rgba[center_idx + 3], 255); // A
    }
}
