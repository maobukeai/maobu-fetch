//! aria2 tellStatus → 任务字段映射（纯函数，AGENTS.md §9 可无网络测试）。
//!
//! 展示约束（§3 BT/磁力内核）：磁力元数据获取阶段（aria2 files[0].path 以
//! `[METADATA]` 开头）不提供真实文件名/大小，映射结果必须保留
//! `metadata_fetching = true`，由 UI 显示“待获取”。

use crate::models::BtFileEntry;
use serde_json::Value;

/// 从 aria2 状态对象提取的进度快照。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BtProgress {
    pub aria2_status: String,
    /// 元数据获取阶段此值可能为 0，UI 不得显示为“0 字节文件”。
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub download_speed: u64,
    pub upload_speed: u64,
    /// 累计上传字节（aria2 `uploadLength`），用于分享率展示。
    pub uploaded_bytes: u64,
    pub num_seeds: u32,
    pub num_peers: u32,
    /// aria2 `seeder`：本机已完成并处于做种上传状态。
    pub seeder: bool,
    pub metadata_fetching: bool,
    pub info_hash: String,
    /// 种子 name（bittorrent.info.name），元数据未就绪时为 None。
    pub display_name: Option<String>,
    pub files: Vec<BtFileEntry>,
    pub error: Option<String>,
}

/// 解析 aria2 `tellStatus` 结果。
///
/// `base_dir` 为任务下载目录：files[].path 剥离该目录前缀与种子根目录，
/// 得到种子内相对路径；传空字符串时仅做最小剥离（引擎轮询路径）。
pub fn parse_status(status: &Value, base_dir: &str) -> BtProgress {
    let mut progress = BtProgress {
        aria2_status: str_field(status, "status").unwrap_or_default(),
        total_bytes: num_field(status, "totalLength"),
        downloaded_bytes: num_field(status, "completedLength"),
        download_speed: num_field(status, "downloadSpeed"),
        upload_speed: num_field(status, "uploadSpeed"),
        uploaded_bytes: num_field(status, "uploadLength"),
        num_seeds: num_field(status, "numSeeders").min(u32::MAX as u64) as u32,
        num_peers: num_field(status, "connections").min(u32::MAX as u64) as u32,
        seeder: bool_field(status, "seeder").unwrap_or(false),
        ..BtProgress::default()
    };
    if let Some(bt) = status.get("bittorrent").filter(|v| v.is_object()) {
        progress.info_hash = str_field(bt, "infoHash").unwrap_or_default();
        progress.display_name = bt
            .get("info")
            .and_then(|info| str_field(info, "name"))
            .filter(|name| !name.is_empty());
    }
    progress.files = parse_files(status.get("files"), base_dir);
    // 元数据标志必须看原始首条目路径（[METADATA] 文件不会进入 files 展示列表）。
    let raw_first_path = status
        .get("files")
        .and_then(Value::as_array)
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.get("path"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    progress.metadata_fetching =
        raw_first_path.starts_with("[METADATA]") && progress.display_name.is_none();
    if progress.aria2_status == "error" {
        progress.error = Some(str_field(status, "errorMessage").unwrap_or_else(|| {
            "aria2 报告下载错误（错误码 ".to_string()
                + &status
                    .get("errorCode")
                    .and_then(Value::as_str)
                    .unwrap_or("未知")
                + "）"
        }));
    }
    progress
}

/// 解析 files 数组为展示条目。
fn parse_files(files: Option<&Value>, base_dir: &str) -> Vec<BtFileEntry> {
    let Some(entries) = files.and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let index = num_field(entry, "index") as u32;
            let path = str_field(entry, "path")?;
            if path.starts_with("[METADATA]") {
                return None;
            }
            Some(BtFileEntry {
                index,
                path: strip_to_torrent_relative(&path, base_dir),
                length_bytes: num_field(entry, "length"),
                // aria2 的 selected 是字符串 "true"/"false"，缺失时默认选中。
                selected: bool_field(entry, "selected").unwrap_or(true),
            })
        })
        .collect()
}

/// aria2 绝对路径 → 种子内相对路径。
///
/// 步骤：分隔符归一 → 剥离任务目录前缀（大小写不敏感，Windows 语义）→
/// 若剩余仍含目录层级，则剥掉首个分量（种子根目录）。
/// 任一步不匹配时按“仅归一分隔符”降级，绝不返回空路径。
pub fn strip_to_torrent_relative(path: &str, base_dir: &str) -> String {
    let normalized = path.replace('\\', "/");
    let mut remainder = normalized.as_str();
    let normalized_base = base_dir.trim().replace('\\', "/");
    if !normalized_base.is_empty() {
        let prefix = normalized_base.trim_end_matches('/');
        let candidate = remainder.to_ascii_lowercase();
        let prefix_lower = prefix.to_ascii_lowercase();
        if let Some(stripped) = candidate
            .strip_prefix(&(prefix_lower + "/"))
            .map(|s| s.to_owned())
        {
            remainder = &normalized[remainder.len() - stripped.len()..];
        }
    }
    // 剩余多段：剥掉种子根目录，保留内部相对结构。
    match remainder.split_once('/') {
        Some((_, rest)) if !rest.is_empty() => rest.to_string(),
        _ => remainder.to_string(),
    }
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

fn str_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn bool_field(value: &Value, key: &str) -> Option<bool> {
    match value.get(key) {
        Some(Value::Bool(flag)) => Some(*flag),
        Some(Value::String(text)) => match text.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn num_field(value: &Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(|v| {
            // aria2 数值字段为字符串；对异常返回的数字也做兼容。
            v.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| v.as_u64())
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_active_download_status() {
        let status = json!({
            "gid": "2089b05ecca3d829",
            "status": "active",
            "totalLength": "1048576",
            "completedLength": "524288",
            "downloadSpeed": "131072",
            "uploadSpeed": "2048",
            "numSeeders": "3",
            "connections": "7",
            "bittorrent": {
                "infoHash": "0123456789abcdef0123456789abcdef01234567",
                "info": { "name": "ubuntu-24.04.iso" }
            },
            "files": [
                { "index": "1", "path": "C:/dl/ubuntu-24.04.iso", "length": "1048576",
                  "completedLength": "524288", "selected": "true", "uris": [] }
            ]
        });
        let progress = parse_status(&status, "C:/dl");
        assert_eq!(progress.total_bytes, 1_048_576);
        assert_eq!(progress.downloaded_bytes, 524_288);
        assert_eq!(progress.num_seeds, 3);
        assert_eq!(progress.num_peers, 7);
        assert_eq!(progress.upload_speed, 2048);
        assert!(!progress.seeder);
        assert!(!progress.metadata_fetching);
        assert_eq!(progress.display_name.as_deref(), Some("ubuntu-24.04.iso"));
        assert_eq!(progress.files.len(), 1);
        assert_eq!(progress.files[0].path, "ubuntu-24.04.iso");
        assert_eq!(estimate_eta(&progress), Some(4));
    }

    #[test]
    fn magnet_metadata_phase_is_flagged() {
        let status = json!({
            "status": "active",
            "totalLength": "0",
            "completedLength": "0",
            "downloadSpeed": "0",
            "files": [
                { "index": "1", "path": "[METADATA]0123456789abcdef.torrent",
                  "length": "0", "selected": "true" }
            ]
        });
        let progress = parse_status(&status, "");
        assert!(progress.metadata_fetching);
        // 元数据阶段不得估算 ETA，也不得暴露 0 字节总长为真实大小。
        assert_eq!(estimate_eta(&progress), None);
        assert!(progress.files.is_empty());
    }

    #[test]
    fn seeding_state_reports_uploaded_bytes_and_seeder_flag() {
        let status = json!({
            "status": "active",
            "seeder": "true",
            "totalLength": "1048576",
            "completedLength": "1048576",
            "downloadSpeed": "0",
            "uploadSpeed": "16384",
            "uploadLength": "2097152",
            "numSeeders": "0",
            "connections": "2",
            "files": [
                { "index": "1", "path": "C:/dl/ubuntu-24.04.iso", "length": "1048576",
                  "completedLength": "1048576", "selected": "true", "uris": [] }
            ]
        });
        let progress = parse_status(&status, "C:/dl");
        assert!(progress.seeder);
        assert_eq!(progress.uploaded_bytes, 2_097_152);
        assert_eq!(progress.upload_speed, 16_384);
        // 做种阶段无剩余下载量，ETA 必须为 None。
        assert_eq!(estimate_eta(&progress), None);
    }

    #[test]
    fn missing_seeder_field_defaults_to_false() {
        let status = json!({
            "status": "active",
            "totalLength": "10", "completedLength": "1", "downloadSpeed": "1"
        });
        let progress = parse_status(&status, "");
        assert!(!progress.seeder);
        assert_eq!(progress.uploaded_bytes, 0);
    }

    #[test]
    fn error_status_carries_message() {
        let status = json!({
            "status": "error",
            "errorCode": "1",
            "errorMessage": "unauthorized",
            "totalLength": "10", "completedLength": "0", "downloadSpeed": "0"
        });
        let progress = parse_status(&status, "");
        assert!(progress.error.is_some());
        assert!(progress.error.unwrap().contains("unauthorized"));
    }

    #[test]
    fn multi_file_torrent_strips_root_component() {
        let status = json!({
            "status": "active",
            "files": [
                { "index": "1", "path": "C:\\dl\\SomeTorrent\\README.txt", "length": "5", "selected": "true" },
                { "index": "2", "path": "C:\\dl\\SomeTorrent\\sub\\big.bin", "length": "7", "selected": "false" }
            ]
        });
        let progress = parse_status(&status, "C:/dl");
        assert_eq!(progress.files[0].path, "README.txt");
        assert_eq!(progress.files[1].path, "sub/big.bin");
        assert!(!progress.files[1].selected);
    }

    #[test]
    fn strip_matches_base_dir_case_insensitive() {
        assert_eq!(
            strip_to_torrent_relative("c:/DL/SomeRoot/file.bin", "C:/dl"),
            "file.bin"
        );
        // base_dir 为空时退化为"剥掉首个分量"。
        assert_eq!(
            strip_to_torrent_relative("SomeRoot/file.bin", ""),
            "file.bin"
        );
        // base_dir 不匹配时降级为剥掉首个分量，不得把文件名剥没。
        assert_eq!(
            strip_to_torrent_relative("E:/other/file.bin", "C:/dl"),
            "other/file.bin"
        );
    }

    #[test]
    fn numeric_fields_accept_bare_numbers() {
        let status = json!({
            "status": "active",
            "totalLength": 100,
            "completedLength": 40,
            "downloadSpeed": 10,
            "numSeeders": 2,
            "connections": 5
        });
        let progress = parse_status(&status, "");
        assert_eq!(progress.total_bytes, 100);
        assert_eq!(progress.downloaded_bytes, 40);
        assert_eq!(progress.download_speed, 10);
    }
}
