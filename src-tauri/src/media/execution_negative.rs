use super::*;
use std::sync::{atomic::Ordering, Arc};

pub(super) async fn run(
    app: &AppHandle,
    manager: &crate::manager::SharedManager,
    fixtures: &execution_fixtures::Fixtures,
    settings: &AppSettings,
    output: &Path,
) -> Result<Vec<Value>, String> {
    let mut task = make_task(fixtures, output, "task-probe", "fixture.mp4", "best");
    manager.store.upsert_task(&task).await?;
    let before = fixtures.cookie_hits.load(Ordering::Relaxed);
    let result = probe_for_task(app, settings, &task, &CancellationToken::new(), manager).await?;
    assert!(!result.formats.is_empty());
    assert!(fixtures.cookie_hits.load(Ordering::Relaxed) > before);
    assert!(output::read_metadata(&task).is_some());

    task = make_task(
        fixtures,
        output,
        "unsupported-edts",
        "fixture.mpd",
        "bestvideo+bestaudio/best",
    );
    manager.store.upsert_task(&task).await?;
    let mut without = settings.clone();
    without.ffmpeg_path = output
        .join("not-installed-ffmpeg.exe")
        .to_string_lossy()
        .into_owned();
    without.ffprobe_path = output
        .join("not-installed-ffprobe.exe")
        .to_string_lossy()
        .into_owned();
    let error = download::download_with_manager(
        app,
        &without,
        task.clone(),
        CancellationToken::new(),
        Vec::new(),
        manager,
    )
    .await
    .unwrap_err();
    assert!(error.contains("FFmpeg"));
    assert!(!output.join(&task.file_name).exists());
    let mut entries = tokio::fs::read_dir(output::work_directory(&task))
        .await
        .map_err(|error| error.to_string())?;
    let mut tracks = 0;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| error.to_string())?
    {
        if entry.file_name().to_string_lossy().starts_with("payload.")
            && matches!(
                entry.path().extension().and_then(|value| value.to_str()),
                Some("mp4" | "m4a")
            )
        {
            tracks += 1;
        }
    }
    assert_eq!(
        tracks, 2,
        "unsupported edits retain both actual source tracks"
    );

    cancel_actual_download(app, manager, fixtures, settings, output).await?;
    Ok(vec![
        serde_json::json!({"case":"task-probe","synthetic_cookie_and_private_cache":true}),
        serde_json::json!({"case":"builtin-edts-rejected","retained_tracks":tracks,"no_final_file":true}),
        serde_json::json!({"case":"actual-tool-cancel","no_final_file":true}),
    ])
}

pub(super) fn make_task(
    fixtures: &execution_fixtures::Fixtures,
    output: &Path,
    name: &str,
    path: &str,
    format: &str,
) -> DownloadTask {
    let mut task = test_support::task();
    task.id = uuid::Uuid::new_v4().to_string();
    task.destination = output.to_string_lossy().into_owned();
    task.file_name = format!("{name}.mp4");
    task.url = format!("{}/{path}", fixtures.base_url);
    task.proxy_override = Some(String::new());
    task.headers
        .insert("Cookie".into(), "session=synthetic".into());
    task.media = Some(crate::models::MediaSelection {
        format_id: Some(format.into()),
        requires_ffmpeg: format.contains('+'),
        ..Default::default()
    });
    task
}

async fn cancel_actual_download(
    app: &AppHandle,
    manager: &crate::manager::SharedManager,
    fixtures: &execution_fixtures::Fixtures,
    settings: &AppSettings,
    output: &Path,
) -> Result<(), String> {
    let mut task = make_task(fixtures, output, "cancelled", "fixture.mp4", "best");
    task.per_task_speed_limit = 1024;
    manager.store.upsert_task(&task).await?;
    manager.task_runtime.write().await.insert(
        task.id.clone(),
        Arc::new(crate::manager::RuntimeTaskOptions::new(&task)),
    );
    let token = CancellationToken::new();
    let cancelled = token.clone();
    let hits = fixtures.cookie_hits.clone();
    let before = hits.load(Ordering::Relaxed);
    let stop = tokio::spawn(async move {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while hits.load(Ordering::Relaxed) == before {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("actual tool must reach local source before cancellation");
        cancelled.cancel();
    });
    let result =
        download::download_with_manager(app, settings, task.clone(), token, Vec::new(), manager)
            .await;
    stop.await.map_err(|error| error.to_string())?;
    assert!(result.is_err());
    assert!(!output.join(&task.file_name).exists());
    Ok(())
}
