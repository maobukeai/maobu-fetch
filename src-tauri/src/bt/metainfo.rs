//! 种子元数据检查与预览。
use crate::models::BtFileEntry;
use std::path::Path;
/// 校验种子文件格式与大小限制。
pub fn validate_torrent_bytes(bytes: &[u8]) -> Result<(), String> {
    inspect_torrent_bytes(bytes).map(|_| ())
}

fn validate_size_and_header(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > 20 * 1024 * 1024 {
        return Err("种子文件超过 20MiB 限制".into());
    }
    if bytes.len() < 10 {
        return Err("种子文件过小，非有效 .torrent 格式".into());
    }
    if !bytes.starts_with(b"d") {
        return Err("种子文件头非法，非有效 Bencode 字典".into());
    }
    Ok(())
}

pub fn validate_torrent_file(path: &Path) -> Result<(), String> {
    let metadata = std::fs::metadata(path).map_err(|e| format!("无法读取种子文件属性：{e}"))?;
    if metadata.len() > 20 * 1024 * 1024 {
        return Err("种子文件超过 20MB 限制".into());
    }
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("打开种子失败：{e}"))?;
    let mut buf = Vec::new();
    file.take(20 * 1024 * 1024 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("读取种子头部失败：{e}"))?;
    validate_torrent_bytes(&buf)
}

pub async fn validate_torrent_file_async(path: &Path) -> Result<(), String> {
    read_torrent_file_async(path).await.map(|_| ())
}

pub(super) async fn read_torrent_file_async(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|e| format!("无法读取种子文件属性：{e}"))?;
    if metadata.len() > 20 * 1024 * 1024 {
        return Err("种子文件超过 20MB 限制".into());
    }
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| format!("打开种子失败：{e}"))?;
    let mut buf = Vec::new();
    file.take(20 * 1024 * 1024 + 1)
        .read_to_end(&mut buf)
        .await
        .map_err(|e| format!("读取种子头部失败：{e}"))?;
    validate_torrent_bytes(&buf)?;
    Ok(buf)
}

/// 解析种子文件字节内容，提取文件名、总大小与文件列表（用于新建任务前预览与勾选）。
pub fn inspect_torrent_bytes(
    bytes: &[u8],
) -> Result<crate::models::BtTorrentInspectResult, String> {
    validate_size_and_header(bytes)?;
    let meta = librqbit::torrent_from_bytes(bytes)
        .map_err(|e| format!("未能从种子数据解析出有效元数据: {e}"))?;
    let info = &meta.info.data;
    let name = info
        .name
        .as_ref()
        .map(|n| n.to_string())
        .unwrap_or_else(|| "未命名种子".into());
    let mut files = Vec::new();
    let mut total_bytes = 0u64;
    if info.files.is_some() && info.length.is_some() {
        return Err("种子不能同时声明单文件和多文件布局".into());
    }

    if let Some(meta_files) = &info.files {
        for (idx, f) in meta_files.iter().enumerate() {
            let path_str = f
                .path
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join("/");
            super::paths::relative_path(&path_str)?;
            total_bytes = total_bytes
                .checked_add(f.length)
                .ok_or("种子文件总长度超出支持范围")?;
            files.push(BtFileEntry {
                index: (idx + 1) as u32,
                path: path_str,
                length_bytes: f.length,
                selected: true,
                downloaded_bytes: 0,
            });
        }
    } else if let Some(length) = info.length {
        super::paths::relative_path(&name)?;
        total_bytes = length;
        files.push(BtFileEntry {
            index: 1,
            path: name.clone(),
            length_bytes: length,
            selected: true,
            downloaded_bytes: 0,
        });
    }
    if info.piece_length == 0 {
        return Err("种子的分片长度必须大于零".into());
    }
    let expected_pieces = total_bytes.div_ceil(u64::from(info.piece_length));
    let hash_bytes: &[u8] = info.pieces.as_ref();
    if expected_pieces > u64::from(u32::MAX)
        || expected_pieces.saturating_mul(u64::from(info.piece_length).div_ceil(16 * 1024))
            > u64::from(u32::MAX)
    {
        return Err("种子的分片或数据块数量超出内核支持范围".into());
    }
    if hash_bytes.len() % 20 != 0 || hash_bytes.len() as u64 / 20 != expected_pieces {
        return Err("种子分片哈希清单与声明大小不一致".into());
    }
    // 先检查总长度溢出和除数，再调用上游结构校验，避免恶意输入触发 panic。
    meta.info
        .data
        .clone()
        .validate()
        .map_err(|e| format!("种子结构或分片清单无效：{e}"))?;
    let mut unique = std::collections::HashSet::new();
    for file in &files {
        if !unique.insert(file.path.to_lowercase()) {
            return Err("种子文件路径在 Windows 下重名，已停止操作".into());
        }
    }

    Ok(crate::models::BtTorrentInspectResult {
        info_hash: meta.info_hash.as_string(),
        name,
        total_bytes,
        files,
    })
}

/// 从本地文件路径解析种子元数据。
pub async fn inspect_torrent_file(
    path: &Path,
) -> Result<crate::models::BtTorrentInspectResult, String> {
    let bytes = read_torrent_file_async(path).await?;
    inspect_torrent_bytes(&bytes)
}
