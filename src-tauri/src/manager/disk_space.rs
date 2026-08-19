use std::{path::Path, time::Duration};

pub const LOW_DISK_PREFIX: &str = "LOW_DISK:";
pub const DISK_CHECK_BYTES_INTERVAL: u64 = 10 * 1024 * 1024;
pub const DISK_CHECK_TIME_INTERVAL: Duration = Duration::from_secs(5);
pub const LOW_DISK_SAFETY_MARGIN_BYTES: u64 = 50 * 1024 * 1024;

/// 计算下载中途周期性检查所需磁盘空间：`remaining + remaining/2 + 50MB`。
pub fn compute_low_disk_required_space(total_bytes: u64, downloaded_bytes: u64) -> u64 {
    let remaining = total_bytes.saturating_sub(downloaded_bytes);
    remaining
        .saturating_add(remaining / 2)
        .saturating_add(LOW_DISK_SAFETY_MARGIN_BYTES)
}

/// 查询单个已存在目录的可用空间。
pub fn query_destination_available_space(path: &Path) -> Option<u64> {
    if !path.exists() {
        return None;
    }
    fs2::available_space(path).ok()
}

/// 查询目标目录所在磁盘的可用空间。
pub fn query_available_space_for_destination(destination: &str) -> u64 {
    let path = Path::new(destination);
    if let Some(space) = query_destination_available_space(path) {
        return space;
    }
    let mut current = path;
    while let Some(parent) = current.parent() {
        if parent.as_os_str().is_empty() {
            break;
        }
        if let Some(space) = query_destination_available_space(parent) {
            return space;
        }
        current = parent;
    }
    0
}

/// 下载循环中执行一次磁盘空间检查。
pub fn check_disk_space_once(
    destination: &str,
    total_bytes: u64,
    downloaded_bytes: u64,
) -> Result<(), (u64, u64)> {
    let required = compute_low_disk_required_space(total_bytes, downloaded_bytes);
    let available = query_available_space_for_destination(destination);
    if available < required {
        Err((available, required))
    } else {
        Ok(())
    }
}
