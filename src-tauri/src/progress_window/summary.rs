//! The compact bar describes every current task, including rows outside the list.
use super::state::ProgressTask;

pub(super) fn percent<'a>(tasks: impl Iterator<Item = &'a ProgressTask>) -> Option<f64> {
    let mut total = 0u128;
    let mut downloaded = 0u128;
    for task in tasks {
        if task.fetching_metadata || task.total_bytes == 0 {
            return None;
        }
        total = total.checked_add(u128::from(task.total_bytes))?;
        downloaded =
            downloaded.checked_add(u128::from(task.downloaded_bytes.min(task.total_bytes)))?;
    }
    if total == 0 {
        return None;
    }
    Some((downloaded as f64 / total as f64 * 100.0).clamp(0.0, 100.0))
}
