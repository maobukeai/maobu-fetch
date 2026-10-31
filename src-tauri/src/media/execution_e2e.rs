use super::*;
use sha2::{Digest, Sha256};
use std::sync::{atomic::Ordering, Arc};

pub(crate) enum Outcome {
    Executed(Vec<Value>),
    Skipped(String),
}

pub(crate) async fn run(
    app: &AppHandle,
    manager: &crate::manager::SharedManager,
    tempdir: &Path,
) -> Result<Outcome, String> {
    let Some(fixtures) =
        super::execution_fixtures::create(&tempdir.join("external-tool-fixtures")).await?
    else {
        return Ok(Outcome::Skipped(
            "没有同时找到已安装的 yt-dlp、FFmpeg 和 FFprobe；未安装或下载组件，真实工具组未执行"
                .into(),
        ));
    };
    let mut settings = manager.settings().await;
    settings.yt_dlp_path = fixtures.tools.yt_dlp.to_string_lossy().into_owned();
    settings.ffmpeg_path = fixtures.tools.ffmpeg.to_string_lossy().into_owned();
    settings.ffprobe_path = fixtures.tools.ffprobe.to_string_lossy().into_owned();
    settings.proxy_mode = "none".into();
    settings.default_retry_policy.task_timeout_secs = Some(30);
    let output = tempdir.join("external-tool-results");
    tokio::fs::create_dir_all(&output)
        .await
        .map_err(|error| error.to_string())?;
    let sentinel = output.join("unrelated-user-file.txt");
    tokio::fs::write(&sentinel, b"unrelated user data")
        .await
        .map_err(|error| error.to_string())?;
    let mut evidence = Vec::new();
    for (name, path, format, merge, tools_available) in [
        ("progressive", "fixture.mp4", "best", false, true),
        (
            "dash-separated",
            "fixture.mpd",
            "bestvideo+bestaudio/best",
            true,
            true,
        ),
        (
            "dash-builtin",
            "builtin.mpd",
            "bestvideo+bestaudio/best",
            true,
            false,
        ),
    ] {
        eprintln!("external media integration: {name}");
        let mut choice = settings.clone();
        if !tools_available {
            choice.ffmpeg_path = output
                .join("not-installed-ffmpeg.exe")
                .to_string_lossy()
                .into_owned();
            choice.ffprobe_path = output
                .join("not-installed-ffprobe.exe")
                .to_string_lossy()
                .into_owned();
        }
        let mut task = super::test_support::task();
        task.id = uuid::Uuid::new_v4().to_string();
        task.destination = output.to_string_lossy().into_owned();
        task.file_name = format!("{name}.mp4");
        task.url = format!("{}/{}", fixtures.base_url, path);
        task.proxy_override = Some(String::new());
        task.headers
            .insert("Cookie".into(), "session=synthetic".into());
        task.media = Some(crate::models::MediaSelection {
            format_id: Some(format.into()),
            requires_ffmpeg: merge,
            ..Default::default()
        });
        manager.store.upsert_task(&task).await?;
        manager.task_runtime.write().await.insert(
            task.id.clone(),
            Arc::new(crate::manager::RuntimeTaskOptions::new(&task)),
        );
        let mut complete = download::download_with_manager(
            app,
            &choice,
            task,
            CancellationToken::new(),
            Vec::new(),
            manager,
        )
        .await?;
        let saved = output.join(&complete.file_name);
        let bytes = tokio::fs::read(&saved)
            .await
            .map_err(|error| error.to_string())?;
        assert!(!bytes.is_empty());
        assert_eq!(complete.total_bytes, bytes.len() as u64);
        assert_eq!(complete.active_connections, 0);
        if !merge {
            assert_eq!(
                Sha256::digest(&bytes),
                Sha256::digest(
                    tokio::fs::read(fixtures.root.join("fixture.mp4"))
                        .await
                        .map_err(|error| error.to_string())?
                )
            );
        }
        let streams = inspect(&fixtures.tools.ffprobe, &saved).await?;
        assert!(streams.iter().any(|stream| stream["codec_type"] == "video"));
        assert!(streams.iter().any(|stream| stream["codec_type"] == "audio"));
        decode(&fixtures.tools.ffmpeg, &saved).await?;
        assert!(
            !task_output_is_live(&complete),
            "VoD MPD must not become a live recording based on URL"
        );
        complete.completed_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_millis() as u64,
        );
        manager
            .store
            .complete_http_task(&complete)
            .await?
            .ok_or("真实媒体完成状态没有提交")?;
        evidence.push(serde_json::json!({"case":name,"ffmpeg_available":tools_available,"bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(&bytes)),"video_and_audio":true,"full_decode":true}));
    }
    assert!(
        fixtures.cookie_hits.load(Ordering::Relaxed) > 0,
        "actual yt-dlp must use its scoped synthetic Cookie"
    );
    assert!(
        !fixtures.leaked_proxy_header.load(Ordering::Relaxed),
        "local proxy credentials must never reach source"
    );
    assert_eq!(
        tokio::fs::read(&sentinel)
            .await
            .map_err(|error| error.to_string())?,
        b"unrelated user data"
    );
    for tools_available in [false, true] {
        let mut task = super::test_support::task();
        task.destination = output.to_string_lossy().into_owned();
        task.file_name = format!("native-{tools_available}.mp4");
        let mut choice = settings.clone();
        if !tools_available {
            choice.ffmpeg_path = output
                .join("not-installed-ffmpeg.exe")
                .to_string_lossy()
                .into_owned();
            choice.ffprobe_path = output
                .join("not-installed-ffprobe.exe")
                .to_string_lossy()
                .into_owned();
        }
        let input = output.join(format!("native-source-{tools_available}.lumaget"));
        tokio::fs::copy(fixtures.root.join("fixture.ts"), &input)
            .await
            .map_err(|error| error.to_string())?;
        let candidate =
            prepare_native_media_file(app, &choice, &mut task, &input, &CancellationToken::new())
                .await?;
        assert!(task
            .file_name
            .ends_with(if tools_available { ".mp4" } else { ".ts" }));
        let target = output.join(&task.file_name);
        crate::manager::publish_file_atomically(&candidate, &target, false).await?;
        decode(&fixtures.tools.ffmpeg, &target).await?;
        evidence.push(serde_json::json!({"case":"native-ts-container","ffmpeg_available":tools_available,"file_name":task.file_name,"full_decode":true}));
    }
    evidence
        .extend(super::execution_negative::run(app, manager, &fixtures, &settings, &output).await?);
    evidence
        .push(super::execution_proxy_test::run(app, manager, &fixtures, &settings, &output).await?);
    Ok(Outcome::Executed(evidence))
}

async fn inspect(ffprobe: &Path, path: &Path) -> Result<Vec<Value>, String> {
    let mut command = create_hidden_tokio_command(ffprobe);
    command
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,width,height,duration",
            "-of",
            "json",
            "-i",
        ])
        .arg(path);
    let bytes = super::execution_fixtures::checked(&mut command).await?;
    let metadata: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    metadata
        .get("streams")
        .and_then(Value::as_array)
        .cloned()
        .ok_or("真实媒体缺少轨道信息".into())
}

async fn decode(ffmpeg: &Path, path: &Path) -> Result<(), String> {
    let mut command = create_hidden_tokio_command(ffmpeg);
    command
        .args(["-hide_banner", "-nostdin", "-v", "error", "-xerror", "-i"])
        .arg(path)
        .args(["-f", "null", "-"]);
    super::execution_fixtures::checked(&mut command).await?;
    Ok(())
}
