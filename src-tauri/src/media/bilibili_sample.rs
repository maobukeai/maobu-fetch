//! Explicitly opted-in, user-authorized public sample. Never runs in the default test suite.
use super::*;

const SAMPLE: &str = "https://www.bilibili.com/zh-Hans/video/BV1VeHQ6tEaS/";
const CANONICAL: &str = "https://www.bilibili.com/video/BV1VeHQ6tEaS/";
const MAX_BYTES: u64 = 1024 * 1024 * 1024;

pub(crate) async fn run(
    app: &AppHandle,
    manager: &crate::manager::SharedManager,
    output: &Path,
) -> Result<Value, String> {
    let cancel = CancellationToken::new();
    let mut operation = Box::pin(run_inner(app, manager, output, &cancel));
    tokio::select! {
        result=operation.as_mut()=>result,
        _=tokio::time::sleep(std::time::Duration::from_secs(600))=>{cancel.cancel();let _=tokio::time::timeout(std::time::Duration::from_secs(3),operation.as_mut()).await;Err("用户样本测试超过10分钟，已停止且不声明完整下载通过".into())}
    }
}

async fn run_inner(
    app: &AppHandle,
    manager: &crate::manager::SharedManager,
    output: &Path,
    cancel: &CancellationToken,
) -> Result<Value, String> {
    let find = |name| {
        execution_fixtures::find_tool(name)
            .ok_or_else(|| format!("真实B站样本需要本机已有{name}，未安装或下载任何工具"))
    };
    let mut settings = manager.settings().await;
    settings.yt_dlp_path = find("yt-dlp")?.to_string_lossy().into_owned();
    settings.ffmpeg_path = find("ffmpeg")?.to_string_lossy().into_owned();
    settings.ffprobe_path = find("ffprobe")?.to_string_lossy().into_owned();
    settings.proxy_mode = "none".into();
    settings.default_retry_policy.task_timeout_secs = Some(600);
    let mut task = test_support::task();
    task.id = uuid::Uuid::new_v4().to_string();
    task.url = SAMPLE.into();
    task.destination = output.to_string_lossy().into_owned();
    task.file_name = "bilibili-user-sample-BV1VeHQ6tEaS.mp4".into();
    task.headers.clear();
    task.proxy_override = Some(String::new());
    manager
        .store
        .create_task_with_name_intent(&task, true)
        .await?;
    let probe = probe_for_task(app, &settings, &task, cancel, manager).await?;
    if probe.drm {
        return Err("用户样本声明DRM，已拒绝".into());
    }
    let duration = probe.duration.ok_or("样本没有返回时长，未盲目下载")?;
    if !duration.is_finite() || duration <= 0.0 {
        return Err("样本时长无效，未盲目下载".into());
    }
    let video = probe
        .formats
        .iter()
        .filter(|format| {
            format.has_video
                && !format.has_audio
                && !format.audio_unknown
                && !format.requires_ffmpeg
                && format.height.is_some()
                && format.file_size.is_some()
        })
        .min_by_key(|format| (format.height, format.file_size))
        .ok_or("样本没有可明确计量的低清视频轨")?;
    let audio = probe
        .formats
        .iter()
        .filter(|format| {
            format.has_audio
                && !format.has_video
                && !format.audio_unknown
                && format.file_size.is_some()
        })
        .min_by_key(|format| format.file_size)
        .ok_or("样本没有可明确计量的已确认音轨")?;
    let declared = video
        .file_size
        .zip(audio.file_size)
        .and_then(|(video, audio)| video.checked_add(audio))
        .ok_or("样本体积未知")?;
    if declared > MAX_BYTES {
        return Err("最低有声组合超过1GiB，本次未下载且不声明通过".into());
    }
    let format = format!("{}+{}", video.id, audio.id);
    eprintln!(
        "user Bilibili sample metadata: {}",
        serde_json::json!({"url":CANONICAL,"title":probe.title,"duration":duration,"format":format,"declared_bytes":declared,"height":video.height})
    );
    task.media = Some(crate::models::MediaSelection {
        format_id: Some(format.clone()),
        requires_ffmpeg: true,
        ..Default::default()
    });
    manager.store.upsert_task(&task).await?;
    let monitor = super::sample_validation::guard_size(&task, cancel.clone());
    let downloaded = download::download_with_manager(
        app,
        &settings,
        task.clone(),
        cancel.clone(),
        Vec::new(),
        manager,
    )
    .await;
    monitor.abort();
    let _ = monitor.await;
    let completed = downloaded?;
    let path = output.join(&completed.file_name);
    let streams =
        super::sample_validation::inspect(Path::new(&settings.ffprobe_path), &path, cancel).await?;
    let actual_duration = streams
        .pointer("/format/duration")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<f64>().ok())
        .ok_or("输出没有实际时长")?;
    if (actual_duration - duration).abs() > 2.0 {
        return Err("输出时长与完整样本不符，未声明完整下载成功".into());
    }
    let workspace = output::work_directory(&task);
    let mut source_hashes = Vec::new();
    for selected in [video, audio] {
        if !selected.id.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("样本格式ID不是预期的B站数值ID".into());
        }
        let ext = selected
            .extension
            .as_deref()
            .ok_or("样本轨道没有真实扩展名")?;
        let file = workspace.join(format!("payload.f{}.{ext}", selected.id));
        let (bytes, sha) = super::sample_validation::fingerprint(&file, cancel).await?;
        // The official extractor provides filesize_approx for this sample, not a trusted length.
        if bytes == 0 || bytes > MAX_BYTES {
            return Err("样本原始轨道为空或超过授权大小".into());
        }
        source_hashes.push(serde_json::json!({"format":selected.id,"bytes":bytes,"declared_bytes_approx":selected.file_size,"sha256":sha}));
    }
    let (bytes, sha) = super::sample_validation::fingerprint(&path, cancel).await?;
    super::sample_validation::decode(Path::new(&settings.ffmpeg_path), &path, cancel).await?;
    Ok(
        serde_json::json!({"url":CANONICAL,"title":probe.title,"duration":duration,"format":format,"declared_bytes":declared,"output_bytes":bytes,"output_sha256":sha,"source_tracks":source_hashes,"ffprobe":streams,"complete_decode":true,"anonymous":true,"used_personal_cookie":false}),
    )
}
