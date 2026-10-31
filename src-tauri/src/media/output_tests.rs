use super::*;

#[tokio::test]
async fn subtitles_complete_before_main_publication_and_never_replace_its_journal() {
    let root = tempfile::tempdir().unwrap();
    let mut task = test_support::task();
    task.destination = root.path().to_string_lossy().into_owned();
    task.collision_policy = crate::models::CollisionPolicy::Rename;
    let workspace = output::Workspace::open(&task, "best").await.unwrap();
    let source = workspace.root.join("payload.mp4");
    let subtitle = workspace.root.join("payload.zh.vtt");
    tokio::fs::write(&source, b"verified main media")
        .await
        .unwrap();
    tokio::fs::write(&subtitle, b"WEBVTT\n\n").await.unwrap();
    tokio::fs::write(root.path().join("video.mp4"), b"existing user video")
        .await
        .unwrap();
    let manifest = output::OutputManifest {
        subtitles: vec![("zh".into(), subtitle)],
        ..Default::default()
    };
    workspace
        .publish_with_subtitles(&source, &mut task, &CancellationToken::new(), &manifest)
        .await
        .unwrap();
    assert_eq!(task.file_name, "video (1).mp4");
    assert!(root.path().join("video (1).zh.vtt").exists());
    let journal: Value = serde_json::from_slice(
        &tokio::fs::read(
            crate::manager::task_temp_dir(&task.destination, &task.id)
                .join("task-publication.json"),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(journal["file_name"], "video (1).mp4");
    assert_eq!(
        tokio::fs::read(root.path().join("video.mp4"))
            .await
            .unwrap(),
        b"existing user video"
    );
}

#[tokio::test]
async fn failed_subtitle_publish_keeps_main_staging_and_does_not_record_false_completion() {
    let root = tempfile::tempdir().unwrap();
    let mut task = test_support::task();
    task.destination = root.path().to_string_lossy().into_owned();
    task.collision_policy = crate::models::CollisionPolicy::Skip;
    let workspace = output::Workspace::open(&task, "best").await.unwrap();
    let source = workspace.root.join("payload.mp4");
    let subtitle = workspace.root.join("payload.zh.vtt");
    tokio::fs::write(&source, b"verified main media")
        .await
        .unwrap();
    tokio::fs::write(&subtitle, b"WEBVTT\n\n").await.unwrap();
    tokio::fs::write(root.path().join("video.zh.vtt"), b"user subtitle")
        .await
        .unwrap();
    let manifest = output::OutputManifest {
        subtitles: vec![("zh".into(), subtitle)],
        ..Default::default()
    };
    assert!(workspace
        .publish_with_subtitles(&source, &mut task, &CancellationToken::new(), &manifest)
        .await
        .is_err());
    assert!(source.exists());
    assert!(!root.path().join("video.mp4").exists());
    assert!(!crate::manager::task_temp_dir(&task.destination, &task.id)
        .join("task-publication.json")
        .exists());
}

#[tokio::test]
async fn private_output_rejects_foreign_paths_and_does_not_overwrite_a_racing_target() {
    let root = tempfile::tempdir().unwrap();
    let mut task = test_support::task();
    task.destination = root.path().join("downloads").to_string_lossy().into_owned();
    let workspace = output::Workspace::open(&task, "best").await.unwrap();
    let unrelated = root.path().join("unrelated.mp4");
    tokio::fs::write(&unrelated, b"user file").await.unwrap();
    assert!(workspace.validate_file(&unrelated).await.is_err());
    let staged = workspace.root.join("payload.mp4");
    tokio::fs::write(&staged, b"complete media").await.unwrap();
    let target = Path::new(&task.destination).join(&task.file_name);
    tokio::fs::write(&target, b"existing user file")
        .await
        .unwrap();
    task.collision_policy = crate::models::CollisionPolicy::Skip;
    assert!(workspace
        .publish(&staged, &mut task, &CancellationToken::new())
        .await
        .is_err());
    assert_eq!(
        tokio::fs::read(&target).await.unwrap(),
        b"existing user file"
    );
    assert!(staged.exists());
    task.collision_policy = crate::models::CollisionPolicy::Rename;
    assert_eq!(
        workspace
            .publish(&staged, &mut task, &CancellationToken::new())
            .await
            .unwrap(),
        14
    );
    assert_eq!(task.file_name, "video (1).mp4");
    assert_eq!(tokio::fs::read(&unrelated).await.unwrap(), b"user file");
}

#[tokio::test]
async fn vod_m3u8_names_do_not_authorize_live_finalization_without_actual_metadata() {
    let root = tempfile::tempdir().unwrap();
    let mut task = test_support::task();
    task.url = "https://example.test/vod.m3u8".into();
    task.destination = root.path().join("downloads").to_string_lossy().into_owned();
    let workspace = output::Workspace::open(&task, "best").await.unwrap();
    assert!(!task_output_is_live(&task));
    workspace
        .record_metadata(&serde_json::json!({"is_live":false,"live_status":"was_live"}))
        .await
        .unwrap();
    assert!(!task_output_is_live(&task));
    workspace
        .record_metadata(&serde_json::json!({"is_live":true,"live_status":"is_live"}))
        .await
        .unwrap();
    assert!(task_output_is_live(&task));
    assert!(output::Workspace::open(&task, "different-format")
        .await
        .is_err());
}
