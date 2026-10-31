//! 纯 Rust BT/磁力下载引擎（基于 librqbit）
//!
//! 零外部进程依赖；原生会话、校验、速率限制与持久恢复通过 Tokio 异步调用。
//!
//! 强约束（AGENTS.md §3 BT/磁力内核）：做种默认关闭；
//! 元数据获取前不伪造文件名/大小；暂停/退出保持可恢复。

pub mod magnet;
pub mod status;

mod add;
mod control;
mod removal;
mod preparation;
pub use removal::BtRemovalGuard;
mod engine;
mod inspect;
mod limits;
mod metainfo;
mod paths;
mod seed_policy;
mod seeding;
mod selection;
mod snapshot;
mod worker;

pub use engine::BtEngine;
pub use metainfo::{
    inspect_torrent_bytes, inspect_torrent_file, validate_torrent_bytes, validate_torrent_file,
    validate_torrent_file_async,
};
pub use paths::{delete_task_files, prepare_task_output, task_content_path, task_output_folder};
pub use seed_policy::{request_seeding, sync_seeding_policy};
use std::time::Duration;
pub use worker::run_task;

/// BT 任务终态错误前缀：worker 收到后直接结束，不进入退避重试
pub const BT_TERMINAL_PREFIX: &str = "BT_TERMINAL: ";

/// 轮询间隔：真实状态 → 前端增量更新，1 秒与 HTTP 进度节奏一致
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// 默认公共 Trackers（精选全球与国内高可用 HTTP/HTTPS/UDP Public Trackers）
pub const DEFAULT_PUBLIC_TRACKERS: &[&str] = &[
    // HTTP / HTTPS Trackers（完美兼容代理与 Fake-IP 环境）
    "http://tracker.opentrackr.org:1337/announce",
    "http://tracker.openbittorrent.com:80/announce",
    "http://open.acgnxtracker.com:80/announce",
    "http://tracker.renapp.cn:6969/announce",
    "http://tracker.ipv6tracker.ru:80/announce",
    "http://tracker.files.fm:6969/announce",
    "https://tracker.tamersunion.org:443/announce",
    "https://tracker.lilithraws.org:443/announce",
    "https://tr.ready4.icu:29986/announce",
    "https://tracker.gbitt.info:443/announce",
    "https://trackers.mlsub.net:443/announce",
    "https://tracker.loligirl.cn:443/announce",
    "https://tracker.imgoingto.icu:443/announce",
    "https://1337.abcvg.info:443/announce",
    "https://tracker.kuroy.me:443/announce",
    "https://tracker.renapp.cn:443/announce",
    "https://tracker.leeching.top:443/announce",
    "https://tracker.nanoha.org:443/announce",
    "https://tracker.pmman.tech:443/announce",
    // Linux 发行版与系统镜像官方 Trackers
    "https://torrent.ubuntu.com/announce",
    "http://torrent.ubuntu.com:6969/announce",
    "udp://torrent.ubuntu.com:6969/announce",
    "udp://ipv6.torrent.ubuntu.com:6969/announce",
    // UDP Trackers
    "udp://tracker.opentrackr.org:1337/announce",
    "udp://open.demonii.com:1337/announce",
    "udp://tracker.openbittorrent.com:6969/announce",
    "udp://tracker.torrent.eu.org:451/announce",
    "udp://explodie.org:6969/announce",
    "udp://open.stealth.si:80/announce",
    "udp://tracker.dler.org:6969/announce",
    "udp://p4p.arenabg.com:1337/announce",
];

#[cfg(test)]
mod crash_tests;
#[cfg(test)]
pub(crate) mod bandwidth_fixture;
#[cfg(test)]
pub(crate) mod history_tests;
#[cfg(test)]
mod public_probe_tests;
#[cfg(test)]
pub(crate) mod lifecycle_tests;
#[cfg(test)]
mod metainfo_tests;
#[cfg(test)]
mod peer_order_tests;
#[cfg(test)]
mod regression_tests;
#[cfg(test)]
mod swarm_tests;
#[cfg(test)]
pub(crate) mod tests;
