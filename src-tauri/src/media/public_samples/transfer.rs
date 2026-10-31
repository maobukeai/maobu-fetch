use super::*;

pub(super) async fn run(
    manager: &crate::manager::SharedManager,
    settings: &AppSettings,
    mut task: DownloadTask,
    probe: &MediaProbeResult,
    sample: &Sample,
    cancel: &CancellationToken,
    metadata: Value,
) -> Result<Value, String> {
    let duration = probe
        .duration
        .or(sample.expected_duration)
        .filter(|n| n.is_finite() && *n > 0.0 && *n <= 1800.0)
        .ok_or("没有明确的30分钟内点播时长")?;
    let format = probe
        .formats
        .iter()
        .filter(|f| {
            sample.format_id.as_ref().is_none_or(|id| id == &f.id)
                && ((f.has_video && f.has_audio && !f.audio_unknown)
                    || (sample.allow_unknown_audio
                        && f.audio_unknown
                        && (f.has_video || sample.format_id.as_ref() == Some(&f.id))))
        })
        .min_by_key(|f| {
            (
                f.audio_unknown,
                f.height.unwrap_or(u64::MAX),
                f.file_size.unwrap_or(u64::MAX),
            )
        })
        .ok_or("没有有声候选；未知音轨需要在测试清单中明确选择")?;
    if format.file_size.is_some_and(|n| n > 256 * 1024 * 1024) {
        return Err("最低有声候选超过256MiB，本次未下载".into());
    }
    task.media = Some(crate::models::MediaSelection {
        format_id: Some(format.id.clone()),
        requires_ffmpeg: format.requires_ffmpeg,
        ..Default::default()
    });
    let monitor = guard(&task, cancel.clone());
    let downloaded = manager.run_public_audit_task(task, cancel).await;
    monitor.abort();
    let _ = monitor.await;
    let completed = downloaded.map_err(|e| format!("传输阶段：{e}"))?;
    let path = Path::new(&completed.destination).join(&completed.file_name);
    let streams =
        sample_validation::inspect(Path::new(&settings.ffprobe_path), &path, cancel).await?;
    let actual = streams
        .pointer("/format/duration")
        .and_then(Value::as_str)
        .and_then(|n| n.parse::<f64>().ok())
        .ok_or("最终文件无时长")?;
    if !actual.is_finite() || actual <= 0.0 || (actual - duration).abs() > 2.0 {
        return Err("最终时长不符，不声明整片通过".into());
    }
    let (bytes, sha) = sample_validation::fingerprint(&path, cancel).await?;
    if bytes == 0 || bytes > 256 * 1024 * 1024 {
        return Err("输出为空或超出256MiB边界".into());
    }
    sample_validation::decode(Path::new(&settings.ffmpeg_path), &path, cancel).await?;
    Ok(
        serde_json::json!({"passed":true,"complete_download":true,"complete_decode":true,"metadata":metadata,"format":format.id,"output_bytes":bytes,"output_sha256":sha,"ffprobe":streams,"expected_duration":duration,"duration_evidence":if probe.duration.is_some(){"production-metadata"}else{"explicit-independent-oracle"}}),
    )
}

fn guard(task: &DownloadTask, cancel: CancellationToken) -> tokio::task::JoinHandle<()> {
    let directory = output::work_directory(task);
    tokio::spawn(async move {
        loop {
            let mut size = 0u64;
            if let Ok(mut entries) = tokio::fs::read_dir(&directory).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    if let Ok(meta) = entry.metadata().await {
                        size = size.saturating_add(meta.len());
                    }
                }
            }
            if size >= 256 * 1024 * 1024 {
                cancel.cancel();
                return;
            }
            tokio::select! {_=cancel.cancelled()=>return,_=tokio::time::sleep(std::time::Duration::from_millis(50))=>{}}
        }
    })
}
