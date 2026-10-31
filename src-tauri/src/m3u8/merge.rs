//! Stream merge verified segments; never expose a partial target or discard recovery data.
use super::{
    request::PAUSED,
    resume::{replace_owned_file, segment_part_path, SegmentRecord},
};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt, BufWriter},
};
use tokio_util::sync::CancellationToken;

pub async fn merge_segments(
    temp: &Path,
    entries: &[SegmentRecord],
    cancel: &CancellationToken,
) -> Result<u64, String> {
    merge_with_policy(temp, entries, cancel, None).await
}

pub async fn merge_recording(
    temp: &Path,
    entries: &[SegmentRecord],
    permission: &AtomicBool,
) -> Result<u64, String> {
    merge_with_policy(temp, entries, &CancellationToken::new(), Some(permission)).await
}

async fn merge_with_policy(
    temp: &Path,
    entries: &[SegmentRecord],
    cancel: &CancellationToken,
    live_permission: Option<&AtomicBool>,
) -> Result<u64, String> {
    if entries.is_empty() || !entries.iter().any(|entry| entry.sequence.is_some()) {
        return Err("没有已完成的媒体切片，无法保存 M3U8 文件".into());
    }
    let merge = PathBuf::from(format!("{}.merge", temp.to_string_lossy()));
    if fs::symlink_metadata(&merge)
        .await
        .is_ok_and(|meta| !meta.is_file() || meta.file_type().is_symlink())
    {
        return Err("M3U8 合并临时路径不安全".into());
    }
    let file = fs::File::create(&merge)
        .await
        .map_err(|error| format!("创建 M3U8 合并文件失败：{error}"))?;
    let mut writer = BufWriter::with_capacity(512 * 1024, file);
    let mut total = 0u64;
    for entry in entries {
        check_running(cancel, live_permission)?;
        total = total
            .checked_add(
                copy_verified_part(temp, entry, &mut writer, cancel, live_permission).await?,
            )
            .ok_or("M3U8 合并长度溢出")?;
    }
    writer
        .flush()
        .await
        .map_err(|error| format!("刷新 M3U8 合并文件失败：{error}"))?;
    writer
        .get_ref()
        .sync_all()
        .await
        .map_err(|error| format!("保存 M3U8 合并文件失败：{error}"))?;
    drop(writer);
    check_running(cancel, live_permission)?;
    if fs::metadata(&merge)
        .await
        .map_err(|error| error.to_string())?
        .len()
        != total
    {
        return Err("M3U8 合并长度与已校验分片不符，原分片已保留".into());
    }
    replace_owned_file(&merge, temp).await?;
    let metadata = fs::metadata(temp)
        .await
        .map_err(|error| format!("核验 M3U8 输出失败：{error}"))?;
    if !metadata.is_file() || metadata.len() != total || total == 0 {
        return Err("M3U8 输出校验失败，不能标为完成".into());
    }
    Ok(total)
}

async fn copy_verified_part(
    temp: &Path,
    entry: &SegmentRecord,
    writer: &mut BufWriter<fs::File>,
    cancel: &CancellationToken,
    live_permission: Option<&AtomicBool>,
) -> Result<u64, String> {
    if !entry.complete() {
        return Err("M3U8 尚有未完成切片，不能合并".into());
    }
    let path = segment_part_path(temp, entry.index);
    let metadata = fs::symlink_metadata(&path)
        .await
        .map_err(|error| format!("读取 M3U8 分片失败：{error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != entry.length {
        return Err("M3U8 分片长度或路径校验失败，已保留恢复数据".into());
    }
    let mut source = fs::File::open(path)
        .await
        .map_err(|error| format!("打开 M3U8 分片失败：{error}"))?;
    let mut hash = Sha256::new();
    let mut length = 0u64;
    let mut buffer = vec![0; 256 * 1024];
    loop {
        check_running(cancel, live_permission)?;
        let read = source
            .read(&mut buffer)
            .await
            .map_err(|error| format!("读取 M3U8 分片数据失败：{error}"))?;
        if read == 0 {
            break;
        }
        length += read as u64;
        hash.update(&buffer[..read]);
        writer
            .write_all(&buffer[..read])
            .await
            .map_err(|error| format!("写入 M3U8 合并文件失败：{error}"))?;
    }
    if length != entry.length || hex::encode(hash.finalize()) != entry.sha256 {
        return Err("M3U8 分片哈希校验失败，已停止合并并保留数据".into());
    }
    Ok(length)
}

fn check_running(
    cancel: &CancellationToken,
    live_permission: Option<&AtomicBool>,
) -> Result<(), String> {
    if cancel.is_cancelled()
        || live_permission.is_some_and(|permission| !permission.load(Ordering::Acquire))
    {
        return Err(PAUSED.into());
    }
    Ok(())
}
