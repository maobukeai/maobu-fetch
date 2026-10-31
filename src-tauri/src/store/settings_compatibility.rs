//! Settings updates preserve explicit thresholds and report corrupted data instead of resetting it.
use super::*;
async fn fixture() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_owned();
    let store = tokio::task::spawn_blocking(move || Store::open(path))
        .await
        .unwrap()
        .unwrap();
    (directory, store)
}
#[tokio::test]
async fn explicitly_chosen_one_megabyte_threshold_survives_reads_and_restart() {
    let (directory, store) = fixture().await;
    let settings = AppSettings {
        min_file_size_mb: 1,
        download_dir: directory
            .path()
            .join("custom-downloads")
            .to_string_lossy()
            .into_owned(),
        ..Default::default()
    };
    store.save_settings(&settings).await.unwrap();
    assert_eq!(store.get_settings().await.unwrap().min_file_size_mb, 1);
    assert_eq!(store.get_settings().await.unwrap().min_file_size_mb, 1);
    let path = directory.path().to_owned();
    drop(store);
    let reopened = tokio::task::spawn_blocking(move || Store::open(path))
        .await
        .unwrap()
        .unwrap();
    let saved = reopened.get_settings().await.unwrap();
    assert_eq!(saved.min_file_size_mb, 1);
    assert_eq!(saved.download_dir, settings.download_dir);
}
#[tokio::test]
async fn partial_old_json_defaults_missing_fields_without_losing_explicit_preferences() {
    let (directory, store) = fixture().await;
    let raw=serde_json::json!({"min_file_size_mb":1,"theme":"dark","download_dir":directory.path().join("explicit").to_string_lossy()}).to_string();
    store
        .connection
        .lock()
        .await
        .execute(
            "INSERT INTO app_state(key,value)VALUES('settings',?1)",
            [&raw],
        )
        .unwrap();
    let saved = store.get_settings().await.unwrap();
    assert_eq!(saved.min_file_size_mb, 1);
    assert_eq!(saved.theme, "dark");
    assert!(
        !saved.frosted_glass,
        "old per-field defaults remain authoritative"
    );
    assert_eq!(saved.window_width, None);
    assert_eq!(saved.window_height, None);
    assert_eq!(saved.auto_scale_ui, None);
    assert_eq!(
        saved.connections_per_download,
        AppSettings::default().connections_per_download
    );
}
#[tokio::test]
async fn invalid_json_or_field_types_fail_without_replacing_the_stored_value() {
    let (_directory, store) = fixture().await;
    for raw in ["{", r#"{"min_file_size_mb":"broken"}"#] {
        store.connection.lock().await.execute("INSERT INTO app_state(key,value)VALUES('settings',?1)ON CONFLICT(key)DO UPDATE SET value=excluded.value",[raw]).unwrap();
        assert!(store.get_settings().await.unwrap_err().contains("未重置"));
        let retained: String = store
            .connection
            .lock()
            .await
            .query_row(
                "SELECT value FROM app_state WHERE key='settings'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained, raw);
    }
}
