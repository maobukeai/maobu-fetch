use super::*;

/// 增量清理历史错误中的凭据；不触及下载认证设置、任务内容或下载文件。
pub(super) fn redact_persisted_errors(connection: &Connection) -> Result<(), String> {
    for (table, column) in [("tasks", "error"), ("error_history", "message")] {
        let records = {
            let mut statement = connection.prepare(&format!("SELECT id,{column} FROM {table} WHERE {column} IS NOT NULL"))
                .map_err(|e| format!("读取历史错误以脱敏失败：{e}"))?;
            let records = statement.query_map([], |row| Ok((row.get::<_,rusqlite::types::Value>(0)?,row.get::<_,String>(1)?)))
                .map_err(|e| format!("读取历史错误失败：{e}"))?;
            records.collect::<Result<Vec<_>,_>>().map_err(|e| format!("读取历史错误失败：{e}"))?
        };
        for (id, original) in records {
            let redacted = crate::manager::redact_sensitive(&original);
            if redacted != original {
                connection.execute(&format!("UPDATE {table} SET {column}=?1 WHERE id=?2"), params![redacted,id])
                    .map_err(|e| format!("历史错误脱敏失败，数据库迁移已回滚：{e}"))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn reopening_redacts_old_errors_without_changing_authentication_or_progress() {
        let directory = tempfile::tempdir().unwrap();
        let data = directory.path().to_owned();
        let store = tokio::task::spawn_blocking({ let data=data.clone();move || Store::open(data) }).await.unwrap().unwrap();
        let mut task = crate::bt::tests::bt_task_for_test();
        task.downloaded_bytes = 123;
        task.headers.insert("Cookie".into(),"live-download-credential".into());
        store.upsert_task(&task).await.unwrap();
        store.connection.lock().await.execute("UPDATE tasks SET error='proxy=https://user:old-secret@proxy.invalid' WHERE id=?1",[&task.id]).unwrap();
        store.connection.lock().await.execute("INSERT INTO error_history(task_id,occurred_at,message) VALUES(?1,1,'token=history-secret')",[&task.id]).unwrap();
        drop(store);
        let store = tokio::task::spawn_blocking(move || Store::open(data)).await.unwrap().unwrap();
        let restored = store.get_task(&task.id).await.unwrap().unwrap();
        assert!(!restored.error.unwrap().contains("old-secret"));
        assert_eq!(restored.downloaded_bytes,123);
        assert_eq!(restored.headers.get("Cookie").unwrap(),"live-download-credential");
        let error:String=store.connection.lock().await.query_row("SELECT message FROM error_history",[],|row|row.get(0)).unwrap();
        assert_eq!(error,"token=***");
    }
}
