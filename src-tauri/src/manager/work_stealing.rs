// 猫步下载器 - HTTP 多连接动态分片工作窃取与慢连接重分流协调器（End-Game 模式）。
//
// 核心职责：
//   1. 管理任务各逻辑分片内的所有活动/待分配切片窗口（RangeWindow）；
//   2. 初始阶段派发静态规划的窗口；
//   3. 当快速连接完成自身分片且无待分配窗口时，探查当前正在运行中的最慢/剩余字节最多的分片；
//   4. 若剩余未下载字节 >= 4MB，执行原子二分切分（Work-Stealing）：
//      - 原 Worker 动态收缩其结束边界至 split_point 并于完成后退出；
//      - 空闲 Worker 认领 [split_point + 1, original_end] 新窗口并建立 HTTP Range 连接协同下载；
//   5. 保证任意切分与重组过程 0 字节重叠、0 字节丢失，并兼容断点续传与合并顺序校验。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

/// 触发工作窃取的最小剩余字节阈值（8MB）。
/// 低于此阈值时，新建 HTTP 连接和握手的开销大于并发收益，不再执行切分。
pub const MIN_STEAL_REMAINING_BYTES: u64 = 8 * 1024 * 1024;

/// 窗口状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowStatus {
    Pending,
    Claimed,
    Completed,
    Failed,
}

/// 切片窗口元数据
#[derive(Debug, Clone)]
pub struct RangeWindow {
    pub id: u64,
    pub segment_index: u8,
    pub ordinal: u32,
    pub start_byte: u64,
    pub end_byte: u64,
    pub existing_bytes: u64,
    pub path: PathBuf,
    pub status: WindowStatus,
}

/// 正在执行中的窗口传输控制句柄（供 Worker 协程与 Coordinator 共享）
#[derive(Debug)]
pub struct WindowTransferHandle {
    pub window_id: u64,
    pub segment_index: u8,
    pub start_byte: u64,
    /// 包含已有的 existing_bytes 以及本次连接下载的增量字节
    pub downloaded_bytes: Arc<AtomicU64>,
    /// 动态生效的结束边界（若被窃取切分，Coordinator 会原子缩减此值）
    pub effective_end_byte: Arc<AtomicU64>,
    /// 标记窗口是否已完成或被窃取提前收尾
    pub is_completed: Arc<AtomicBool>,
}

impl WindowTransferHandle {
    pub fn new(window_id: u64, segment_index: u8, start_byte: u64, end_byte: u64, existing_bytes: u64) -> Self {
        Self {
            window_id,
            segment_index,
            start_byte,
            downloaded_bytes: Arc::new(AtomicU64::new(existing_bytes)),
            effective_end_byte: Arc::new(AtomicU64::new(end_byte)),
            is_completed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 当前已下载的总字节（相对该窗口的 start_byte）
    pub fn current_downloaded(&self) -> u64 {
        self.downloaded_bytes.load(Ordering::Relaxed)
    }

    /// 当前有效结束偏移
    pub fn current_end(&self) -> u64 {
        self.effective_end_byte.load(Ordering::Relaxed)
    }

    /// 标记该窗口下载完成
    pub fn mark_completed(&self) {
        self.is_completed.store(true, Ordering::Relaxed);
    }
}

/// 工作窃取协调器内部状态
struct CoordinatorState {
    next_window_id: u64,
    windows: Vec<RangeWindow>,
    active_transfers: HashMap<u64, Arc<WindowTransferHandle>>,
}

/// 工作窃取协调器
#[derive(Clone)]
pub struct WorkStealingCoordinator {
    state: Arc<TokioMutex<CoordinatorState>>,
    temp_path: PathBuf,
}

impl WorkStealingCoordinator {
    /// 基于初始规划的窗口集合构建协调器
    pub fn new(temp_path: &Path, initial_windows: Vec<RangeWindow>) -> Self {
        let max_id = initial_windows.iter().map(|w| w.id).max().unwrap_or(0);
        Self {
            state: Arc::new(TokioMutex::new(CoordinatorState {
                next_window_id: max_id + 1,
                windows: initial_windows,
                active_transfers: HashMap::new(),
            })),
            temp_path: temp_path.to_path_buf(),
        }
    }

    /// 领取下一个可执行的工作：
    /// 1. 优先领取未认领的 Pending 窗口；
    /// 2. 若无 Pending 窗口，则尝试从当前运行中最慢/剩余最大的活跃窗口执行工作窃取（Work-Stealing）；
    /// 3. 若无可窃取的任务且无活跃任务，返回 None。
    pub async fn claim_or_steal_work(&self) -> Option<(RangeWindow, Arc<WindowTransferHandle>)> {
        let mut state = self.state.lock().await;

        // 1. 优先领取 Pending 窗口（自动过滤并标记已写满的无效窗口）
        while let Some(pos) = state.windows.iter().position(|w| w.status == WindowStatus::Pending) {
            let window_len = state.windows[pos].end_byte.saturating_sub(state.windows[pos].start_byte).saturating_add(1);
            if state.windows[pos].existing_bytes >= window_len
                || state.windows[pos].start_byte.saturating_add(state.windows[pos].existing_bytes) > state.windows[pos].end_byte
            {
                state.windows[pos].status = WindowStatus::Completed;
                continue;
            }
            state.windows[pos].status = WindowStatus::Claimed;
            let window = state.windows[pos].clone();
            let handle = Arc::new(WindowTransferHandle::new(
                window.id,
                window.segment_index,
                window.start_byte,
                window.end_byte,
                window.existing_bytes,
            ));
            state.active_transfers.insert(window.id, handle.clone());
            return Some((window, handle));
        }

        // 2. 尝试从活跃传输中窃取最长剩余分片的后半段（按 window_id 稳定遍历，保证确定性）
        let mut best_target: Option<(u64, u64, u64, u64, u8, u32)> = None; // (window_id, current_cursor, current_end, remaining, segment_index, ordinal)

        let mut active_list: Vec<(u64, Arc<WindowTransferHandle>)> = state
            .active_transfers
            .iter()
            .map(|(id, h)| (*id, h.clone()))
            .collect();
        active_list.sort_by_key(|(id, _)| *id);

        for (window_id, handle) in active_list {
            if handle.is_completed.load(Ordering::Relaxed) {
                continue;
            }
            let downloaded = handle.current_downloaded();
            // 防御性工程：仅当连接已处于实际传输中（已下载 >= 512KB）且该分片窗口数 < 4 时才允许窃取
            if downloaded < 512 * 1024 {
                continue;
            }
            let sub_count = state.windows.iter().filter(|w| w.segment_index == handle.segment_index).count();
            if sub_count >= 4 {
                continue;
            }

            let start = handle.start_byte;
            let current_end = handle.current_end();
            let current_cursor = start.saturating_add(downloaded);

            if current_end > current_cursor {
                let remaining = current_end - current_cursor;
                if remaining >= MIN_STEAL_REMAINING_BYTES {
                    if let Some((_, _, _, max_rem, _, _)) = best_target {
                        if remaining > max_rem {
                            best_target = Some((window_id, current_cursor, current_end, remaining, handle.segment_index, 0));
                        }
                    } else {
                        best_target = Some((window_id, current_cursor, current_end, remaining, handle.segment_index, 0));
                    }
                }
            }
        }

        if let Some((victim_id, current_cursor, current_end, remaining, segment_index, _)) = best_target {
            // 计算切分点：在当前游标与结束边界之间二分切分
            // 确保切出的两半均有合理大小
            let split_length = remaining / 2;
            let split_point = current_cursor.saturating_add(split_length);

            if split_point > current_cursor && split_point < current_end {
                // 1. 原子缩减被窃取窗口的结束边界
                if let Some(victim_handle) = state.active_transfers.get(&victim_id) {
                    victim_handle.effective_end_byte.store(split_point, Ordering::SeqCst);
                }
                if let Some(victim_window) = state.windows.iter_mut().find(|w| w.id == victim_id) {
                    victim_window.end_byte = split_point;
                }

                // 2. 创建新被窃取出的子窗口
                let new_id = state.next_window_id;
                state.next_window_id += 1;
                let new_start = split_point + 1;
                let new_end = current_end;
                let new_ordinal = state
                    .windows
                    .iter()
                    .filter(|w| w.segment_index == segment_index)
                    .map(|w| w.ordinal)
                    .max()
                    .unwrap_or(0)
                    + 1;
                let new_path = window_part_path(&self.temp_path, segment_index, new_start);

                let new_window = RangeWindow {
                    id: new_id,
                    segment_index,
                    ordinal: new_ordinal,
                    start_byte: new_start,
                    end_byte: new_end,
                    existing_bytes: 0,
                    path: new_path,
                    status: WindowStatus::Claimed,
                };

                state.windows.push(new_window.clone());
                let new_handle = Arc::new(WindowTransferHandle::new(
                    new_id,
                    segment_index,
                    new_start,
                    new_end,
                    0,
                ));
                state.active_transfers.insert(new_id, new_handle.clone());

                return Some((new_window, new_handle));
            }
        }

        None
    }

    /// 标记窗口传输结束。只有实际下载字节数完整覆盖该窗口区间时才标记为 Completed，否则重置为 Pending 供续传
    pub async fn finish_window(&self, window_id: u64, success: bool, downloaded_bytes: u64) {
        let mut state = self.state.lock().await;
        if let Some(handle) = state.active_transfers.remove(&window_id) {
            handle.mark_completed();
        }
        if let Some(window) = state.windows.iter_mut().find(|w| w.id == window_id) {
            window.existing_bytes = downloaded_bytes;
            let window_len = window.end_byte.saturating_sub(window.start_byte).saturating_add(1);
            if success && window.existing_bytes >= window_len {
                window.status = WindowStatus::Completed;
            } else {
                // 未下载完整，重置为 Pending 允许 Worker 立即自动重试认领
                window.status = WindowStatus::Pending;
            }
        }
    }

    /// 检查是否所有窗口均已 100% 成功完成（全部为 Completed，无任何活跃或待下载窗口）
    pub async fn is_all_completed(&self) -> bool {
        let state = self.state.lock().await;
        state.active_transfers.is_empty()
            && !state.windows.is_empty()
            && state.windows.iter().all(|w| w.status == WindowStatus::Completed)
    }

    /// 获取当前活跃传输的窗口数
    pub async fn active_transfers_count(&self) -> usize {
        let state = self.state.lock().await;
        state.active_transfers.len()
    }

    /// 获取指定逻辑分片下所有已完成的切片窗口，并按 start_byte 严格升序排序
    pub async fn get_ordered_windows_for_segment(&self, segment_index: u8) -> Vec<RangeWindow> {
        let state = self.state.lock().await;
        let mut list: Vec<RangeWindow> = state
            .windows
            .iter()
            .filter(|w| w.segment_index == segment_index && w.status == WindowStatus::Completed)
            .cloned()
            .collect();
        list.sort_by_key(|w| w.start_byte);
        list
    }

    /// 获取所有窗口快照（供诊断与合并校验）
    pub async fn snapshot_windows(&self) -> Vec<RangeWindow> {
        let state = self.state.lock().await;
        state.windows.clone()
    }
}

/// 生成切片窗口文件路径
pub fn window_part_path(temp: &Path, segment_index: u8, start: u64) -> PathBuf {
    PathBuf::from(format!(
        "{}.part{segment_index}.w{start}",
        temp.to_string_lossy()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_work_stealing_coordinator_initial_claim() {
        let temp = Path::new("test_file.tmp");
        let initial = vec![
            RangeWindow {
                id: 1,
                segment_index: 0,
                ordinal: 0,
                start_byte: 0,
                end_byte: 10_000_000,
                existing_bytes: 0,
                path: window_part_path(temp, 0, 0),
                status: WindowStatus::Pending,
            },
            RangeWindow {
                id: 2,
                segment_index: 1,
                ordinal: 0,
                start_byte: 10_000_001,
                end_byte: 20_000_000,
                existing_bytes: 0,
                path: window_part_path(temp, 1, 10_000_001),
                status: WindowStatus::Pending,
            },
        ];

        let coordinator = WorkStealingCoordinator::new(temp, initial);

        // Worker 1 认领窗口 1
        let (w1, h1) = coordinator.claim_or_steal_work().await.expect("should claim w1");
        assert_eq!(w1.id, 1);
        assert_eq!(h1.start_byte, 0);
        assert_eq!(h1.current_end(), 10_000_000);

        // Worker 2 认领窗口 2
        let (w2, h2) = coordinator.claim_or_steal_work().await.expect("should claim w2");
        assert_eq!(w2.id, 2);
        assert_eq!(h2.start_byte, 10_000_001);

        // 窗口 2 完成
        coordinator.finish_window(w2.id, true, 10_000_000).await;

        // 此时窗口 1 正在下载，已下载 1MB（>= 512KB 传输中阈值），
        // 剩余 9MB >= 8MB 窃取阈值（MIN_STEAL_REMAINING_BYTES）
        h1.downloaded_bytes.store(1_000_000, Ordering::Relaxed);

        // Worker 2 再次请求工作，应成功从窗口 1 窃取后半段！
        let (w3, h3) = coordinator.claim_or_steal_work().await.expect("should steal from w1");
        assert_eq!(w3.segment_index, 0);
        // 原窗口 1 游标在 1_000_000，剩余 9_000_000，切分点在 1_000_000 + 4_500_000 = 5_500_000
        assert_eq!(h1.current_end(), 5_500_000);
        assert_eq!(w3.start_byte, 5_500_001);
        assert_eq!(w3.end_byte, 10_000_000);
        assert_eq!(h3.current_end(), 10_000_000);

        // 原 Worker 1 完成其前半段（实际覆盖 [0, 5_500_000] 共 5_500_001 字节）
        coordinator.finish_window(w1.id, true, 5_500_001).await;
        // Worker 2 完成被窃取的后半段（[5_500_001, 10_000_000] 共 4_500_000 字节）
        coordinator.finish_window(w3.id, true, 4_500_000).await;

        assert!(coordinator.is_all_completed().await);

        // 检查 segment 0 的有序窗口：必须无缝覆盖 [0 .. 10_000_000]
        let ordered = coordinator.get_ordered_windows_for_segment(0).await;
        assert_eq!(ordered.len(), 2);
        assert_eq!(ordered[0].start_byte, 0);
        assert_eq!(ordered[0].end_byte, 5_500_000);
        assert_eq!(ordered[1].start_byte, 5_500_001);
        assert_eq!(ordered[1].end_byte, 10_000_000);
    }

    #[tokio::test]
    async fn test_work_stealing_rejects_under_threshold() {
        let temp = Path::new("test_file.tmp");
        let initial = vec![RangeWindow {
            id: 1,
            segment_index: 0,
            ordinal: 0,
            start_byte: 0,
            end_byte: 3_000_000, // 总共仅 3MB (< 4MB 阈值)
            existing_bytes: 0,
            path: window_part_path(temp, 0, 0),
            status: WindowStatus::Pending,
        }];

        let coordinator = WorkStealingCoordinator::new(temp, initial);
        let (w1, _) = coordinator.claim_or_steal_work().await.unwrap();

        // 尝试窃取，应该返回 None
        let steal_res = coordinator.claim_or_steal_work().await;
        assert!(steal_res.is_none());

        coordinator.finish_window(w1.id, true, 3_000_001).await;
        assert!(coordinator.is_all_completed().await);
    }

    #[tokio::test]
    async fn test_work_stealing_failed_window_resumes_as_pending() {
        let temp = Path::new("test_resume.tmp");
        let initial = vec![RangeWindow {
            id: 1,
            segment_index: 0,
            ordinal: 0,
            start_byte: 0,
            end_byte: 10_000_000,
            existing_bytes: 0,
            path: window_part_path(temp, 0, 0),
            status: WindowStatus::Pending,
        }];

        let coordinator = WorkStealingCoordinator::new(temp, initial);
        let (w1, _) = coordinator.claim_or_steal_work().await.unwrap();

        // 模拟下载了 3MB 后失败
        coordinator.finish_window(w1.id, false, 3_000_000).await;
        assert!(!coordinator.is_all_completed().await);

        // 重新认领，必须拿到断点 3MB 续传
        let (w1_retry, h1_retry) = coordinator.claim_or_steal_work().await.unwrap();
        assert_eq!(w1_retry.id, 1);
        assert_eq!(w1_retry.existing_bytes, 3_000_000);
        assert_eq!(h1_retry.current_downloaded(), 3_000_000);

        // 完成剩余 7MB
        coordinator.finish_window(w1_retry.id, true, 10_000_001).await;
        assert!(coordinator.is_all_completed().await);
    }

    #[tokio::test]
    async fn test_work_stealing_cascading_splits_gapless_coverage() {
        let temp = Path::new("cascade_test.tmp");
        // 单个 100MB 大分片
        let initial = vec![RangeWindow {
            id: 1,
            segment_index: 0,
            ordinal: 0,
            start_byte: 0,
            end_byte: 99_999_999, // 100MB
            existing_bytes: 0,
            path: window_part_path(temp, 0, 0),
            status: WindowStatus::Pending,
        }];

        let coordinator = WorkStealingCoordinator::new(temp, initial);

        // Worker 1 认领原始 100MB 窗口
        let (w1, h1) = coordinator.claim_or_steal_work().await.unwrap();
        assert_eq!(w1.id, 1);
        assert_eq!(h1.current_end(), 99_999_999);

        // Worker 1 下载了 10MB，当前游标在 10MB，剩余 90MB
        h1.downloaded_bytes.store(10_000_000, Ordering::Relaxed);

        // Worker 2 窃取：切分 [10MB .. 100MB]，剩余 89_999_999，半长 44_999_999
        // w1 结束边界变为 54_999_999；w2 认领 [55_000_000 .. 99_999_999]
        let (w2, h2) = coordinator.claim_or_steal_work().await.unwrap();
        assert_eq!(h1.current_end(), 54_999_999);
        assert_eq!(w2.start_byte, 55_000_000);
        assert_eq!(h2.current_end(), 99_999_999);

        // Worker 3 此时也空闲并请求工作：
        // w1 (id=1) 剩余 54_999_999 - 10MB = 44_999_999；w2 (id=2) 剩余 99_999_999 - 55_000_000 = 44_999_999
        // 稳定判定选择 id 较小的 w1 再次二分：中点 10_000_000 + 22_499_999 = 32_499_999
        let (w3, h3) = coordinator.claim_or_steal_work().await.unwrap();
        assert_eq!(h1.current_end(), 32_499_999);
        assert_eq!(w3.start_byte, 32_500_000);
        assert_eq!(h3.current_end(), 54_999_999);
        assert_eq!(h2.current_end(), 99_999_999);

        // 模拟所有 worker 完成各自负责的区间
        coordinator.finish_window(w1.id, true, 32_500_000).await;
        coordinator.finish_window(w3.id, true, 22_500_000).await;
        coordinator.finish_window(w2.id, true, 45_000_000).await;

        assert!(coordinator.is_all_completed().await);

        // 检查 segment 0 的最终有序切片
        let ordered = coordinator.get_ordered_windows_for_segment(0).await;
        assert_eq!(ordered.len(), 3);
        assert_eq!(ordered[0].start_byte, 0);
        assert_eq!(ordered[0].end_byte, 32_499_999);

        assert_eq!(ordered[1].start_byte, 32_500_000);
        assert_eq!(ordered[1].end_byte, 54_999_999);

        assert_eq!(ordered[2].start_byte, 55_000_000);
        assert_eq!(ordered[2].end_byte, 99_999_999);

        // 验证整体连续性无重叠、无间隙
        let mut cursor = 0u64;
        let mut total_bytes = 0u64;
        for w in ordered {
            assert_eq!(w.start_byte, cursor);
            let len = w.end_byte - w.start_byte + 1;
            total_bytes += len;
            cursor = w.end_byte + 1;
        }
        assert_eq!(total_bytes, 100_000_000);
        assert_eq!(cursor, 100_000_000);
    }
}
