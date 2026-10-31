//! Patch only the progress preference, serialized with full settings saves/restores.
use super::*;

async fn persist_pinned(
    store: &Store,
    current: &mut AppSettings,
    pinned: bool,
) -> Result<AppSettings, String> {
    let mut next = current.clone();
    next.progress_window_always_on_top = pinned;
    store
        .save_settings(&next)
        .await
        .map_err(|error| format!("无法保存进度悬浮窗置顶设置：{}", redact_sensitive(&error)))?;
    *current = next.clone();
    Ok(next)
}

impl DownloadManager {
    pub(crate) async fn set_progress_window_pinned(
        self: &SharedManager,
        pinned: bool,
    ) -> Result<AppSettings, String> {
        let _operation = self.settings_operation.lock().await;
        let mut current = self.settings.write().await;
        let next = persist_pinned(&self.store, &mut current, pinned).await?;
        drop(current);
        // Only the main window needs the complete settings object. The progress
        // window receives the restricted snapshot from progress_window instead.
        if let Err(error) = self.app.emit_to("main", "settings-changed", next.clone()) {
            tracing::warn!(reason=%redact_sensitive(&error.to_string()), "置顶设置已保存，但主窗口未能刷新");
        }
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pinned_patch_preserves_other_preferences_and_reopens() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().to_path_buf();
        let store = tokio::task::spawn_blocking(move || Store::open(data))
            .await
            .unwrap()
            .unwrap();
        let mut settings = AppSettings {
            download_dir: "C:/synthetic/keep-downloads".into(),
            proxy_url: "http://proxy.invalid:8080".into(),
            proxy_username: "synthetic-user".into(),
            proxy_password: "synthetic-secret".into(),
            progress_window_enabled: false,
            ..Default::default()
        };
        let before = settings.clone();
        persist_pinned(&store, &mut settings, false).await.unwrap();
        let mut expected = before;
        expected.progress_window_always_on_top = false;
        assert_eq!(settings, expected);
        drop(store);
        let data = root.path().to_path_buf();
        let reopened = tokio::task::spawn_blocking(move || Store::open(data))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reopened.get_settings().await.unwrap(), expected);
    }

    #[tokio::test]
    async fn failed_pinned_save_keeps_memory_and_original_database_settings() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().to_path_buf();
        let store = tokio::task::spawn_blocking(move || Store::open(data))
            .await
            .unwrap()
            .unwrap();
        let mut settings = AppSettings::default();
        store.save_settings(&settings).await.unwrap();
        let before = settings.clone();
        let path = root.path().join("lumaget.db");
        tokio::task::spawn_blocking(move || {
            rusqlite::Connection::open(path).unwrap().execute_batch(
                "CREATE TRIGGER reject_progress_settings BEFORE INSERT ON app_state BEGIN SELECT RAISE(ABORT, 'synthetic settings failure'); END;"
            ).unwrap();
        }).await.unwrap();
        assert!(persist_pinned(&store, &mut settings, false).await.is_err());
        assert_eq!(settings, before);
        assert_eq!(store.get_settings().await.unwrap(), before);
    }
}
