use crate::models::{CompletionAction, DownloadPreset, DownloadTask, TaskStatus};
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

/// 校验下载预设的连接数（Task 12.3）。
///
/// 仅允许 `1 / 2 / 4 / 8 / 16 / 32` 这六个档位，与 §3 下载内核强约束一致。
/// 非法值返回中文错误，便于前端直接展示。
pub fn validate_preset_connections(n: u8) -> Result<(), String> {
    if [1u8, 2, 4, 8, 16, 32].contains(&n) {
        Ok(())
    } else {
        Err("连接数只能是 1 / 2 / 4 / 8 / 16 / 32".into())
    }
}

/// 校验预设的计划时间格式。仅接受 "HH:MM" 24 小时制（HH 00-23，MM 00-59）。
/// `None` 表示立即开始，通过校验。
pub fn validate_preset_scheduled_at(value: Option<&str>) -> Result<(), String> {
    let Some(raw) = value else {
        return Ok(());
    };
    let bytes = raw.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return Err("计划时间格式必须为 HH:MM".into());
    }
    let parse = |start: usize, end: usize| -> Result<u32, String> {
        let slice = &bytes[start..end];
        let value = std::str::from_utf8(slice)
            .map_err(|_| "计划时间格式必须为 HH:MM".to_string())?
            .parse::<u32>()
            .map_err(|_| "计划时间格式必须为 HH:MM".to_string())?;
        Ok(value)
    };
    let hh = parse(0, 2)?;
    let mm = parse(3, 5)?;
    if hh > 23 || mm > 59 {
        return Err("计划时间格式必须为 HH:MM".into());
    }
    Ok(())
}

/// 将 "HH:MM" 转换为下一次该本地时刻的 Unix 毫秒时间戳。
pub fn next_scheduled_timestamp(hhmm: &str) -> Option<u64> {
    if validate_preset_scheduled_at(Some(hhmm)).is_err() {
        return None;
    }
    let hh: u64 = hhmm[0..2].parse().ok()?;
    let mm: u64 = hhmm[3..5].parse().ok()?;
    let now_ms = now();
    const DAY_MS: u64 = 24 * 60 * 60 * 1000;
    let today_ms = now_ms % DAY_MS;
    let target_ms = hh * 60 * 60 * 1000 + mm * 60 * 1000;
    let delta = if target_ms > today_ms {
        target_ms - today_ms
    } else {
        DAY_MS - today_ms + target_ms
    };
    Some(now_ms.saturating_add(delta))
}

/// 把预设字段应用到任务。
pub fn apply_preset_to_task_fields(
    task: &mut DownloadTask,
    preset: &DownloadPreset,
) -> Result<(), String> {
    if !matches!(
        task.status,
        TaskStatus::Queued
            | TaskStatus::Paused
            | TaskStatus::Scheduled
            | TaskStatus::Failed
            | TaskStatus::Cancelled
    ) {
        return Err("当前任务状态不支持应用预设，请先暂停任务".into());
    }

    validate_preset_connections(preset.connections)?;
    validate_preset_scheduled_at(preset.scheduled_at.as_deref())?;

    task.connection_count = preset.connections;
    task.per_task_speed_limit = preset.speed_limit;

    if let Some(action) = &preset.completion_action {
        task.completion_action = action.clone();
    }

    if let Some(hhmm) = preset.scheduled_at.as_deref() {
        if let Some(next_ms) = next_scheduled_timestamp(hhmm) {
            task.scheduled_at = Some(next_ms);
            task.status = TaskStatus::Scheduled;
        }
    }

    Ok(())
}
