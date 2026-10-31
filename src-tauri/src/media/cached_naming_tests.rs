use super::*;

async fn store(root: &Path) -> crate::store::Store {
    let path = root.join("database");
    tokio::task::spawn_blocking(move || crate::store::Store::open(path))
        .await
        .unwrap()
        .unwrap()
}
fn templates() -> Vec<PlatformNamingTemplate> {
    vec![PlatformNamingTemplate {
        id: "fixture".into(),
        platform: "youtube".into(),
        template: "{author}-{date}-{title}".into(),
        enabled: true,
        is_builtin: false,
    }]
}

#[tokio::test]
async fn explicit_ui_probe_cache_keeps_author_date_when_native_task_skips_reprobe() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path()).await;
    let mut task = test_support::task();
    task.url = "https://www.youtube.com/watch?v=fixture".into();
    task.file_name = "actual.mp4".into();
    store
        .create_task_with_name_intent(&task, false)
        .await
        .unwrap();
    assert!(
        cached_naming::need_native_naming_probe(&task, &templates(), &store)
            .await
            .unwrap()
    );
    task.destination = directory
        .path()
        .join("output")
        .to_string_lossy()
        .into_owned();
    task.headers.insert("Cookie".into(), "session=one".into());
    let metadata = probe_cache::minimal(
        &serde_json::json!({"title":"actual","uploader":"author","upload_date":"20261031","id":"fixture"}),
        MediaPlatform::YouTube,
    );
    probe_cache::save(store.data_dir(), &task.url, Some("session=one"), &metadata)
        .await
        .unwrap();
    cached_naming::prepare_native_output_name(&mut task, &templates(), &store)
        .await
        .unwrap();
    assert_eq!(task.file_name, "author-20261031-actual.mp4");
    // After the concrete CDN URL replaces the page URL, the private task metadata remains sufficient.
    task.url = "https://cdn.example.test/opaque?signature=synthetic".into();
    cached_naming::prepare_native_output_name(&mut task, &templates(), &store)
        .await
        .unwrap();
    assert_eq!(task.file_name, "author-20261031-actual.mp4");
}

#[tokio::test]
async fn cache_miss_or_other_identity_never_silently_loses_a_required_template() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path()).await;
    let mut task = test_support::task();
    task.url = "https://www.youtube.com/watch?v=fixture".into();
    task.destination = directory
        .path()
        .join("output")
        .to_string_lossy()
        .into_owned();
    task.file_name = "actual.mp4".into();
    store
        .create_task_with_name_intent(&task, false)
        .await
        .unwrap();
    assert!(
        cached_naming::prepare_native_output_name(&mut task, &templates(), &store)
            .await
            .unwrap_err()
            .contains("重新分析")
    );
    let metadata = probe_cache::minimal(
        &serde_json::json!({"title":"actual","uploader":"private-author","upload_date":"20261031"}),
        MediaPlatform::YouTube,
    );
    probe_cache::save(
        store.data_dir(),
        &task.url,
        Some("session=other"),
        &metadata,
    )
    .await
    .unwrap();
    assert!(
        cached_naming::prepare_native_output_name(&mut task, &templates(), &store)
            .await
            .is_err()
    );
    assert_eq!(task.file_name, "actual.mp4");
    assert!(!output::work_directory(&task).join("runtime.json").exists());
}

#[tokio::test]
async fn explicit_names_and_unknown_legacy_names_survive_missing_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let store = store(directory.path()).await;
    let mut task = test_support::task();
    task.url = "https://www.youtube.com/watch?v=fixture".into();
    task.file_name = "my chosen name.mp4".into();
    task.destination = directory
        .path()
        .join("output")
        .to_string_lossy()
        .into_owned();
    for intent in [None, Some(true)] {
        if let Some(intent) = intent {
            store
                .create_task_with_name_intent(&task, intent)
                .await
                .unwrap()
        }
        assert!(
            !cached_naming::need_native_naming_probe(&task, &templates(), &store)
                .await
                .unwrap()
        );
        cached_naming::prepare_native_output_name(&mut task, &templates(), &store)
            .await
            .unwrap();
        assert_eq!(task.file_name, "my chosen name.mp4");
    }
}
