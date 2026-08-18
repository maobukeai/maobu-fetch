//! BT 进度与状态模型（纯函数，AGENTS.md §9 可无网络测试）。
//!
//! 展示约束（§3 BT/磁力内核）：磁力元数据获取阶段不提供真实文件名/大小，
//! 映射结果保留 `metadata_fetching = true`，由 UI 显示“待获取”。

use crate::models::BtFileEntry;

/// BT 任务进度与运行时状态快照。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BtProgress {
    /// 运行状态："active", "complete", "paused", "error", "removed"
    pub state: String,
    /// 元数据获取阶段此值可能为 0，UI 不得显示为“0 字节文件”。
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub download_speed: u64,
    pub upload_speed: u64,
    /// 累计上传字节，用于分享率展示。
    pub uploaded_bytes: u64,
    pub num_seeds: u32,
    pub num_peers: u32,
    /// 是否处于做种状态。
    pub seeder: bool,
    pub metadata_fetching: bool,
    pub info_hash: String,
    /// 种子显示名称，元数据未就绪时为 None。
    pub display_name: Option<String>,
    pub files: Vec<BtFileEntry>,
    pub error: Option<String>,
}

/// 由已下载字节与速度估算剩余秒数；速度为 0 或元数据阶段返回 None。
pub fn estimate_eta(progress: &BtProgress) -> Option<u64> {
    if progress.metadata_fetching || progress.download_speed == 0 {
        return None;
    }
    progress
        .total_bytes
        .checked_sub(progress.downloaded_bytes)
        .map(|remaining| remaining / progress.download_speed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_eta_calculates_remaining_seconds() {
        let progress = BtProgress {
            total_bytes: 1000,
            downloaded_bytes: 400,
            download_speed: 100,
            metadata_fetching: false,
            ..Default::default()
        };
        assert_eq!(estimate_eta(&progress), Some(6));
    }

    #[test]
    fn estimate_eta_zero_speed_returns_none() {
        let progress = BtProgress {
            total_bytes: 1000,
            downloaded_bytes: 400,
            download_speed: 0,
            metadata_fetching: false,
            ..Default::default()
        };
        assert_eq!(estimate_eta(&progress), None);
    }

    #[test]
    fn estimate_eta_metadata_fetching_returns_none() {
        let progress = BtProgress {
            total_bytes: 1000,
            downloaded_bytes: 0,
            download_speed: 100,
            metadata_fetching: true,
            ..Default::default()
        };
        assert_eq!(estimate_eta(&progress), None);
    }
}
