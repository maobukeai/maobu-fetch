use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub fn task_output_is_live(task: &DownloadTask) -> bool {
    output::read_metadata(task).is_some_and(|value| super::live_marker::from_metadata(&value))
}

/// Only task-owned fixed output names are considered; never scan unrelated user downloads.
pub async fn find_live_task_output(task: &DownloadTask) -> Option<PathBuf> {
    let root = output::work_directory(task);
    let metadata = tokio::fs::symlink_metadata(&root).await.ok()?;
    if !metadata.is_dir() || super::path_safety::is_link(&metadata) {
        return None;
    }
    let mut found = None;
    for extension in ["mp4", "ts", "flv", "mkv", "webm", "m4a", "aac"] {
        for suffix in ["", ".part"] {
            let path = root.join(format!("payload.{extension}{suffix}"));
            if tokio::fs::symlink_metadata(&path)
                .await
                .is_ok_and(|metadata| {
                    metadata.is_file()
                        && !super::path_safety::is_link(&metadata)
                        && metadata.len() > 0
                })
            {
                if found.is_some() {
                    return None;
                }
                found = Some(path);
            }
        }
    }
    found
}

pub async fn finalize_live_task_output(
    app: &AppHandle,
    settings: &AppSettings,
    mut task: DownloadTask,
    permission: Arc<AtomicBool>,
) -> Result<DownloadTask, String> {
    allowed(&permission)?;
    let source = find_live_task_output(&task)
        .await
        .ok_or("没有唯一可验证的当前任务录制文件，原数据已保留")?;
    let format = task
        .media
        .as_ref()
        .and_then(|media| media.format_id.as_deref())
        .unwrap_or("best[vcodec!=none][acodec!=none]");
    let workspace = output::Workspace::open(&task, format).await?;
    let source = workspace.validate_file(&source).await?;
    let cancel = CancellationToken::new();
    let operation = async {
        let output = if let Some(tools) = resolve_ffmpeg(app, settings) {
            let output = workspace
                .root
                .join(format!("recording-final-{}.mp4", uuid::Uuid::new_v4()));
            let mut command = create_hidden_tokio_command(&tools.ffmpeg);
            command
                .args([
                    "-hide_banner",
                    "-nostdin",
                    "-v",
                    "error",
                    "-protocol_whitelist",
                    "file,pipe",
                    "-fflags",
                    "+genpts",
                    "-i",
                ])
                .arg(&source)
                .args([
                    "-map",
                    "0:v?",
                    "-map",
                    "0:a?",
                    "-c",
                    "copy",
                    "-avoid_negative_ts",
                    "make_zero",
                    "-movflags",
                    "+faststart",
                    "-f",
                    "mp4",
                    "-n",
                ])
                .arg(&output);
            let result = process::capture(
                &mut command,
                &cancel,
                std::time::Duration::from_secs(300),
                1024 * 1024,
            )
            .await?;
            if !result.status.success() {
                return Err("录制容器修复失败，原始录制已保留；请检查 FFmpeg 或稍后重试".into());
            }
            download::verify_streams(&tools.ffprobe, &output, false, &cancel).await?;
            output
        } else {
            let extension = validated_container(&source, &cancel).await?;
            let output = workspace.root.join(format!(
                "recording-final-{}.{extension}",
                uuid::Uuid::new_v4()
            ));
            copy_recording(&source, &output, &permission).await?;
            output
        };
        allowed(&permission)?;
        let length = workspace.publish(&output, &mut task, &cancel).await?;
        allowed(&permission)?;
        task.total_bytes = length;
        task.downloaded_bytes = length;
        task.active_connections = 0;
        task.speed = 0;
        task.eta_seconds = None;
        task.status = TaskStatus::Completed;
        Ok(task)
    };
    tokio::select! {
        biased;
        _ = permission_revoked(&permission) => { cancel.cancel(); Err("录制保存已撤销，原始数据已保留".into()) },
        result = tokio::time::timeout(std::time::Duration::from_secs(300), operation) => result.unwrap_or_else(|_| Err("录制保存超时，原始数据已保留".into())),
    }
}

fn allowed(permission: &AtomicBool) -> Result<(), String> {
    if permission.load(Ordering::Acquire) {
        Ok(())
    } else {
        Err("录制保存已撤销，原始数据已保留".into())
    }
}

async fn permission_revoked(permission: &AtomicBool) {
    while permission.load(Ordering::Acquire) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn copy_recording(
    source: &Path,
    target: &Path,
    permission: &AtomicBool,
) -> Result<(), String> {
    let mut input = tokio::fs::File::open(source)
        .await
        .map_err(|_| "读取原始录制失败")?;
    let expected = input
        .metadata()
        .await
        .map_err(|_| "读取原始录制长度失败")?
        .len();
    let mut output = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target)
        .await
        .map_err(|_| "创建录制暂存文件失败")?;
    let mut buffer = vec![0; 256 * 1024];
    let mut copied = 0u64;
    loop {
        allowed(permission)?;
        let read = input
            .read(&mut buffer)
            .await
            .map_err(|_| "读取录制数据失败")?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .await
            .map_err(|_| "写入录制暂存文件失败")?;
        copied += read as u64;
    }
    output
        .sync_all()
        .await
        .map_err(|_| "保存录制暂存文件失败")?;
    if copied != expected {
        return Err("录制长度变化，暂不发布以保护原始数据".into());
    }
    Ok(())
}

pub(super) async fn validated_container(
    path: &Path,
    cancel: &CancellationToken,
) -> Result<&'static str, String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|_| "读取录制容器失败")?;
    let length = file
        .metadata()
        .await
        .map_err(|_| "读取录制容器长度失败")?
        .len();
    let mut header = [0; 16];
    let read = file
        .read(&mut header)
        .await
        .map_err(|_| "读取录制容器头失败")?;
    if read >= 8 && &header[4..8] == b"ftyp" {
        crate::media_muxer::validate_fragmented_mp4_with_cancel(path, cancel).await?;
        return Ok("mp4");
    }
    if read >= 3 && &header[..3] == b"FLV" {
        validate_flv(&mut file, length, cancel).await?;
        return Ok("flv");
    }
    if read >= 1 && header[0] == 0x47 && length >= 188 * 3 && length % 188 == 0 {
        file.rewind().await.map_err(|_| "读取 TS 录制失败")?;
        let mut reader = tokio::io::BufReader::with_capacity(256 * 1024, file);
        let mut packet = [0; 188];
        for _ in 0..length / 188 {
            if cancel.is_cancelled() {
                return Err("任务已暂停".into());
            }
            reader
                .read_exact(&mut packet)
                .await
                .map_err(|_| "TS 录制存在截断数据")?;
            if packet[0] != 0x47 {
                return Err("TS 录制同步校验失败，原文件已保留".into());
            }
            if packet[1] & 0x80 != 0 {
                return Err("TS 数据声明传输错误，原文件已保留".into());
            }
            if packet[3] & 0xc0 != 0 {
                return Err("MEDIA_DRM_PROTECTED: TS 数据声明加密，已拒绝处理".into());
            }
            let adaptation = (packet[3] >> 4) & 3;
            if adaptation == 0
                || adaptation == 2 && packet[4] != 183
                || adaptation == 3 && packet[4] > 182
            {
                return Err("TS 适配字段长度无效，原文件已保留".into());
            }
        }
        return Ok("ts");
    }
    Err("录制容器尚未完成或无法验证，请安装 FFmpeg 后修复；原数据已保留".into())
}

async fn validate_flv(
    file: &mut tokio::fs::File,
    length: u64,
    cancel: &CancellationToken,
) -> Result<(), String> {
    file.rewind().await.map_err(|_| "读取 FLV 录制失败")?;
    let mut header = [0; 13];
    file.read_exact(&mut header)
        .await
        .map_err(|_| "FLV 文件头不完整")?;
    let offset = u32::from_be_bytes(header[5..9].try_into().map_err(|_| "FLV 文件头无效")?) as u64;
    if header[3] != 1 || header[4] & !5 != 0 || offset != 9 || header[9..13] != [0, 0, 0, 0] {
        return Err("FLV 文件头不受支持，请使用 FFmpeg 修复".into());
    }
    let mut cursor = 13u64;
    let mut media_tag = false;
    while cursor < length {
        if cancel.is_cancelled() {
            return Err("任务已暂停".into());
        }
        let mut tag = [0; 11];
        file.read_exact(&mut tag)
            .await
            .map_err(|_| "FLV 录制标签截断")?;
        if tag[0] & 0xe0 != 0 || tag[8..11] != [0, 0, 0] || !matches!(tag[0] & 0x1f, 8 | 9 | 18) {
            return Err("FLV 录制含加密或未知标签，已拒绝发布".into());
        }
        let size = ((tag[1] as u32) << 16 | (tag[2] as u32) << 8 | tag[3] as u32) as u64;
        media_tag |= matches!(tag[0] & 0x1f, 8 | 9) && size > 0;
        cursor = cursor.checked_add(11 + size + 4).ok_or("FLV 长度溢出")?;
        if cursor > length {
            return Err("FLV 录制末尾截断，请使用 FFmpeg 修复".into());
        }
        file.seek(std::io::SeekFrom::Current(size as i64))
            .await
            .map_err(|_| "读取 FLV 标签失败")?;
        if file.read_u32().await.map_err(|_| "FLV 标签长度缺失")? as u64 != size + 11 {
            return Err("FLV 标签长度不匹配".into());
        }
    }
    if cursor <= 13 || !media_tag {
        return Err("FLV 录制没有媒体数据".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_container_rejects_truncation_encryption_transport_errors_and_cancel() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("input.lumaget");
        let mut packet = [0xff; 188];
        packet[..4].copy_from_slice(&[0x47, 0x1f, 0xff, 0x10]);
        let valid = packet.repeat(3);
        tokio::fs::write(&path, &valid).await.unwrap();
        assert_eq!(
            validated_container(&path, &CancellationToken::new())
                .await
                .unwrap(),
            "ts"
        );
        for (offset, value) in [(188, 0), (3, 0x90), (1, 0x9f), (3, 0), (3, 0x20)] {
            let mut invalid = valid.clone();
            invalid[offset] = value;
            tokio::fs::write(&path, invalid).await.unwrap();
            assert!(validated_container(&path, &CancellationToken::new())
                .await
                .is_err());
        }
        tokio::fs::write(&path, &valid[..valid.len() - 1])
            .await
            .unwrap();
        assert!(validated_container(&path, &CancellationToken::new())
            .await
            .is_err());
        tokio::fs::write(&path, &valid).await.unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(validated_container(&path, &cancel).await.is_err());
    }
}
