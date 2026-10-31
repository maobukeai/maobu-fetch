use super::super::*;
use super::{harness::Harness, recording_fixture::RecordingFixture};
use crate::media::execution_fixtures::{self, Tools};

pub(super) async fn run(app: &AppHandle) {
    if std::env::var("MAOBU_TEST_EXTERNAL_MEDIA").as_deref() != Ok("1") {
        eprintln!("native live recording: real TS oracle requires explicit external-media opt-in");
        return;
    }
    let source = tempfile::tempdir().unwrap();
    let generated = execution_fixtures::create(source.path())
        .await
        .unwrap()
        .expect("opt-in must have actual media tools");
    let fixture =
        RecordingFixture::new(fs::read(generated.root.join("fixture.ts")).await.unwrap()).await;
    let harness = Harness::new(app, None).await;
    configure_tools(&harness, &generated.tools, false).await;
    let settings = harness.manager.settings().await;
    assert!(crate::media_tools::resolve_ffmpeg(app, &settings).is_none());
    assert!(crate::media_tools::resolve_yt_dlp(app, &settings).is_none());
    pause_resume_finish(&harness, &fixture, &generated.tools).await;
    no_media_yet_is_paused(&harness, &fixture).await;
    cancel_remove_exit(&harness, &fixture).await;
    spoofed_vod_flag(app).await;
    tool_publication_and_vod(app, &fixture, &generated.tools).await;
    eprintln!("NATIVE_RECORDING pause_preserved=true resumed_without_tools=true explicit_finish_completed=true silent_first_segment_paused=true cancel_delete_exit_not_published=true forged_vod_rejected=true tool_decode=true");
}

async fn no_media_yet_is_paused(h: &Harness, fixture: &RecordingFixture) {
    let task = create(
        h,
        &fixture.waiting_url,
        "waiting-recording.mp4",
        serde_json::Value::Null,
    )
    .await;
    h.resume(&task.id).await;
    wait_live(h, &task.id, 0).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.silent_segments.load(Ordering::Acquire) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("actual first segment GET must be pending with zero media bytes");
    assert_eq!(h.row(&task.id).await.downloaded_bytes, 0);
    let error = h
        .manager
        .action(&task.id, "finish-recording")
        .await
        .unwrap_err();
    assert!(error.contains("恢复录制"));
    let stopped = h.row(&task.id).await;
    assert_eq!(stopped.status, TaskStatus::Paused);
    assert_eq!(stopped.active_connections, 0);
    assert!(stopped.error.unwrap().contains("完整媒体片段"));
    assert!(!h.manager.controls.lock().await.contains_key(&task.id));
    assert_no_final(h, "waiting-recording").await;
}

async fn configure_tools(h: &Harness, tools: &Tools, available: bool) {
    let mut settings = h.manager.settings().await;
    settings.yt_dlp_path = h
        .data
        .join("not-installed-yt-dlp.exe")
        .to_string_lossy()
        .into_owned();
    settings.ffmpeg_path = if available {
        tools.ffmpeg.clone()
    } else {
        h.data.join("not-installed-ffmpeg.exe")
    }
    .to_string_lossy()
    .into_owned();
    settings.ffprobe_path = if available {
        tools.ffprobe.clone()
    } else {
        h.data.join("not-installed-ffprobe.exe")
    }
    .to_string_lossy()
    .into_owned();
    // Fixture-only absent paths model missing tools without changing PATH or user settings.
    *h.manager.settings.write().await = settings;
}
async fn create(h: &Harness, url: &str, name: &str, media: serde_json::Value) -> DownloadTask {
    let request:NewTaskRequest=serde_json::from_value(serde_json::json!({
        "url":url,"file_name":name,"destination":h.output,"headers":{},"scheduled_at":null,"priority":0,
        "expected_checksum":null,"source":"desktop","per_task_speed_limit":0,"collision_policy":"rename",
        "completion_action":"none","media":media,"connection_count":1,"start_paused":true,
        "user_edited_file_name":true,"cloud_refresh":null,"method":null,"body":null
    })).unwrap();
    h.manager.add(request).await.unwrap()
}
async fn start_live(h: &Harness, fixture: &RecordingFixture, name: &str) -> DownloadTask {
    let task = create(h, &fixture.url, name, serde_json::Value::Null).await;
    h.resume(&task.id).await;
    wait_live(h, &task.id, fixture.bytes.len() as u64).await;
    task
}
async fn wait_live(h: &Harness, id: &str, minimum: u64) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let row = h.row(id).await;
            assert_ne!(row.status, TaskStatus::Failed, "{:?}", row.error);
            let observed = h
                .manager
                .controls
                .lock()
                .await
                .get(id)
                .is_some_and(|control| control.observed_live.load(Ordering::Acquire));
            if row.status == TaskStatus::Downloading
                && row.media.as_ref().is_some_and(|media| media.is_live)
                && row.downloaded_bytes >= minimum
                && observed
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("actual no-ENDLIST playlist and completed TS segment must be observed");
}
async fn pause_resume_finish(h: &Harness, fixture: &RecordingFixture, tools: &Tools) {
    let task = start_live(h, fixture, "live-recording.mp4").await;
    h.pause(&task.id).await;
    assert_no_final(h, "live-recording").await;
    let before = h.fingerprints(&task).await;
    assert!(before
        .iter()
        .any(|(_, length, hash)| *length == fixture.bytes.len() as u64
            && *hash == hex::encode(Sha256::digest(&fixture.bytes))));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(before, h.fingerprints(&task).await);
    h.resume(&task.id).await;
    wait_live(h, &task.id, fixture.bytes.len() as u64).await;
    let active = h.row(&task.id).await;
    assert!(live_recording::is_native_hls(&active));
    assert_eq!(media_task_tool_requirements(&active), (false, false));
    h.manager
        .action(&task.id, "finish-recording")
        .await
        .unwrap();
    let completed = h.wait(&task.id, TaskStatus::Completed).await;
    assert!(completed.file_name.ends_with(".ts"));
    let path = h.output.join(completed.file_name);
    assert_eq!(fs::read(&path).await.unwrap(), fixture.bytes);
    decode(tools, &path).await;
}
async fn assert_no_final(h: &Harness, base: &str) {
    for extension in ["mp4", "ts", "flv"] {
        assert!(!h.output.join(format!("{base}.{extension}")).exists());
    }
}
async fn cancel_remove_exit(h: &Harness, fixture: &RecordingFixture) {
    for action in ["cancel", "remove", "shutdown"] {
        let base = format!("live-{action}");
        let task = start_live(h, fixture, &format!("{base}.mp4")).await;
        let control = h
            .manager
            .controls
            .lock()
            .await
            .get(&task.id)
            .unwrap()
            .clone();
        if action == "cancel" {
            h.manager.action(&task.id, "cancel").await.unwrap();
            assert_eq!(h.row(&task.id).await.status, TaskStatus::Cancelled);
        } else if action == "remove" {
            h.manager.remove(&task.id, false).await.unwrap();
            assert!(h.manager.store.get_task(&task.id).await.unwrap().is_none());
        } else {
            h.manager.shutdown_bt().await.unwrap();
        }
        assert!(!control.finalize_live.load(Ordering::Acquire));
        assert_no_final(h, &base).await;
        assert!(
            !h.fingerprints(&task).await.is_empty(),
            "ordinary stop keeps resumable fragments"
        );
    }
}
async fn spoofed_vod_flag(app: &AppHandle) {
    let h = Harness::new(app, None).await;
    let fixture = super::fixture::Fixture::new(super::fixture::Mode::SilentBody).await;
    let media = serde_json::json!({"is_live":true,"extractor":"native-hls","format_id":null,"format_label":null,"subtitles":[],"thumbnail":null,"requires_ffmpeg":false,"url":null});
    let task = create(&h, &fixture.url, "not-live.bin", media).await;
    h.resume(&task.id).await;
    super::support::await_started(&fixture, 0).await;
    assert!(h
        .manager
        .action(&task.id, "finish-recording")
        .await
        .unwrap_err()
        .contains("尚未确认"));
    assert_eq!(h.row(&task.id).await.status, TaskStatus::Downloading);
    h.pause(&task.id).await;
    assert!(!h.output.join("not-live.bin").exists());
    h.manager.shutdown_bt().await.unwrap();
}
async fn tool_publication_and_vod(app: &AppHandle, fixture: &RecordingFixture, tools: &Tools) {
    let h = Harness::new(app, None).await;
    configure_tools(&h, tools, true).await;
    let task = start_live(&h, fixture, "tool-recording.mp4").await;
    h.manager
        .action(&task.id, "finish-recording")
        .await
        .unwrap();
    let completed = h.wait(&task.id, TaskStatus::Completed).await;
    assert!(completed.file_name.ends_with(".mp4"));
    decode(tools, &h.output.join(completed.file_name)).await;
    let vod = create(&h, &fixture.vod_url, "finite.mp4", serde_json::Value::Null).await;
    h.resume(&vod.id).await;
    let completed = h.wait(&vod.id, TaskStatus::Completed).await;
    assert!(!completed.media.as_ref().is_some_and(|media| media.is_live));
    let old = serde_json::to_value(&completed).unwrap();
    assert!(h.manager.action(&vod.id, "finish-recording").await.is_err());
    assert_eq!(serde_json::to_value(h.row(&vod.id).await).unwrap(), old);
    h.manager.shutdown_bt().await.unwrap();
}
async fn decode(tools: &Tools, path: &Path) {
    let mut command = crate::media_tools::create_hidden_tokio_command(&tools.ffmpeg);
    command
        .args(["-hide_banner", "-nostdin", "-v", "error", "-xerror", "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-map", "0:a:0", "-f", "null", "-"]);
    execution_fixtures::checked(&mut command).await.unwrap();
}
