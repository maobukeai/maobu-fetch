use super::*;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

pub(super) fn guard_size(
    task: &DownloadTask,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let directory = output::work_directory(task);
    tokio::spawn(async move {
        loop {
            if let Ok(mut entries) = tokio::fs::read_dir(&directory).await {
                let mut bytes = 0u64;
                while let Ok(Some(entry)) = entries.next_entry().await {
                    if let Ok(metadata) = entry.metadata().await {
                        bytes = bytes.saturating_add(metadata.len());
                    }
                }
                if bytes >= 1024 * 1024 * 1024 {
                    cancel.cancel();
                    return;
                }
            }
            tokio::select! {_=cancel.cancelled()=>return,_=tokio::time::sleep(std::time::Duration::from_millis(50))=>{}}
        }
    })
}

pub(super) async fn inspect(
    ffprobe: &Path,
    path: &Path,
    cancel: &CancellationToken,
) -> Result<Value, String> {
    let mut command = create_hidden_tokio_command(ffprobe);
    command
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-show_entries",
            "format=duration,size:stream=codec_type,codec_name,width,height,duration",
            "-of",
            "json",
            "-i",
        ])
        .arg(path);
    let result = process::capture(
        &mut command,
        cancel,
        std::time::Duration::from_secs(30),
        1024 * 1024,
    )
    .await?;
    if !result.status.success() {
        return Err("实际FFprobe校验失败".into());
    }
    let value: Value = serde_json::from_slice(&result.stdout).map_err(|_| "FFprobe轨道JSON无效")?;
    let streams = value
        .get("streams")
        .and_then(Value::as_array)
        .ok_or("实际输出没有轨道")?;
    for kind in ["video", "audio"] {
        if !streams.iter().any(|stream| stream["codec_type"] == kind) {
            return Err(format!("实际输出缺少{kind}轨道"));
        }
    }
    Ok(value)
}

pub(super) async fn fingerprint(
    path: &Path,
    cancel: &CancellationToken,
) -> Result<(u64, String), String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|_| "样本文件缺失")?;
    let mut buffer = vec![0; 256 * 1024];
    let mut hash = Sha256::new();
    let mut bytes = 0;
    loop {
        if cancel.is_cancelled() {
            return Err("样本验证已停止".into());
        }
        let read = file.read(&mut buffer).await.map_err(|_| "读取样本失败")?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        bytes += read as u64;
    }
    Ok((bytes, hex::encode(hash.finalize())))
}

pub(super) async fn decode(
    ffmpeg: &Path,
    path: &Path,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let mut command = create_hidden_tokio_command(ffmpeg);
    command
        .args([
            "-hide_banner",
            "-nostdin",
            "-v",
            "error",
            "-xerror",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(path)
        .args(["-f", "null", "-"]);
    let result = process::capture(
        &mut command,
        cancel,
        std::time::Duration::from_secs(300),
        1024 * 1024,
    )
    .await?;
    if !result.status.success() {
        return Err("完整解码未通过，不能声明完整媒体验收成功".into());
    }
    Ok(())
}
