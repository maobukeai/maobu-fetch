use crate::models::{
    AppSettings, CollisionPolicy, CompletionAction, DownloadSegment, DownloadTask, MediaSelection,
    TaskStatus,
};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::{collections::HashMap, path::PathBuf};
use tokio::sync::Mutex;

pub struct Store {
    connection: Mutex<Connection>,
    data_dir: PathBuf,
}

impl Store {
    pub fn open(data_dir: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
        let connection =
            Connection::open(data_dir.join("lumaget.db")).map_err(|e| e.to_string())?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| e.to_string())?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| e.to_string())?;
        connection
            .execute_batch(SCHEMA)
            .map_err(|e| e.to_string())?;
        ensure_task_column(
            &connection,
            "connection_count",
            "INTEGER NOT NULL DEFAULT 8",
        )?;
        ensure_task_column(&connection, "segments_json", "TEXT NOT NULL DEFAULT '[]'")?;
        ensure_task_column(
            &connection,
            "completion_action",
            "TEXT NOT NULL DEFAULT '\"none\"'",
        )?;
        ensure_task_column(&connection, "final_url", "TEXT")?;
        ensure_task_column(&connection, "response_status", "INTEGER")?;
        ensure_task_column(&connection, "content_type", "TEXT")?;
        ensure_task_column(&connection, "accepts_ranges", "INTEGER")?;
        let store = Self {
            connection: Mutex::new(connection),
            data_dir,
        };
        store.migrate_legacy_json()?;
        Ok(store)
    }

    fn migrate_legacy_json(&self) -> Result<(), String> {
        let connection = self.connection.blocking_lock();
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if count > 0 {
            return Ok(());
        }
        let legacy = self.data_dir.join("downloads.json");
        if !legacy.exists() {
            return Ok(());
        }
        let bytes = std::fs::read(&legacy).map_err(|e| e.to_string())?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let Some(map) = value.as_object() else {
            return Ok(());
        };
        for raw in map.values() {
            let id = raw.get("id").and_then(|v| v.as_str()).unwrap_or_default();
            let url = raw.get("url").and_then(|v| v.as_str()).unwrap_or_default();
            if id.is_empty() || url.is_empty() {
                continue;
            }
            let task = DownloadTask {
                id: id.into(),
                url: url.into(),
                file_name: raw
                    .get("file_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("download")
                    .into(),
                destination: raw
                    .get("destination")
                    .and_then(|v| v.as_str())
                    .unwrap_or(".")
                    .into(),
                total_bytes: raw.get("total_bytes").and_then(|v| v.as_u64()).unwrap_or(0),
                downloaded_bytes: raw
                    .get("downloaded_bytes")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                speed: 0,
                eta_seconds: None,
                status: match raw.get("status").and_then(|v| v.as_str()) {
                    Some("completed") => TaskStatus::Completed,
                    Some("failed") => TaskStatus::Failed,
                    _ => TaskStatus::Paused,
                },
                error: raw.get("error").and_then(|v| v.as_str()).map(str::to_owned),
                created_at: raw.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
                completed_at: raw.get("completed_at").and_then(|v| v.as_u64()),
                scheduled_at: None,
                category: raw
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("other")
                    .into(),
                queue_position: 0,
                priority: 0,
                retry_count: 0,
                max_retries: 3,
                checksum_sha256: None,
                expected_checksum: None,
                source: "migration".into(),
                etag: None,
                last_modified: None,
                final_url: None,
                response_status: None,
                content_type: None,
                accepts_ranges: None,
                headers: HashMap::new(),
                media: None,
                per_task_speed_limit: 0,
                collision_policy: CollisionPolicy::Rename,
                completion_action: CompletionAction::None,
                connection_count: 8,
                active_connections: 0,
                segments: Vec::new(),
            };
            Self::upsert_with(&connection, &task)?;
        }
        let _ = std::fs::rename(&legacy, self.data_dir.join("downloads.migrated.json"));
        Ok(())
    }

    pub async fn list_tasks(&self) -> Result<Vec<DownloadTask>, String> {
        let connection = self.connection.lock().await;
        let mut stmt = connection
            .prepare("SELECT * FROM tasks ORDER BY queue_position ASC, created_at DESC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], task_from_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub async fn get_task(&self, id: &str) -> Result<Option<DownloadTask>, String> {
        let connection = self.connection.lock().await;
        connection
            .query_row("SELECT * FROM tasks WHERE id=?1", [id], task_from_row)
            .optional()
            .map_err(|e| e.to_string())
    }

    pub async fn upsert_task(&self, task: &DownloadTask) -> Result<(), String> {
        let connection = self.connection.lock().await;
        Self::upsert_with(&connection, task)
    }

    fn upsert_with(connection: &Connection, task: &DownloadTask) -> Result<(), String> {
        connection
            .execute(
                UPSERT_TASK,
                params![
                    task.id,
                    task.url,
                    task.file_name,
                    task.destination,
                    task.total_bytes as i64,
                    task.downloaded_bytes as i64,
                    task.speed as i64,
                    task.eta_seconds.map(|v| v as i64),
                    task.status.as_str(),
                    task.error,
                    task.created_at as i64,
                    task.completed_at.map(|v| v as i64),
                    task.scheduled_at.map(|v| v as i64),
                    task.category,
                    task.queue_position,
                    task.priority,
                    task.retry_count,
                    task.max_retries,
                    task.checksum_sha256,
                    task.expected_checksum,
                    task.source,
                    task.etag,
                    task.last_modified,
                    task.final_url,
                    task.response_status.map(i64::from),
                    task.content_type,
                    task.accepts_ranges.map(i64::from),
                    serde_json::to_string(&task.headers).unwrap_or_else(|_| "{}".into()),
                    serde_json::to_string(&task.media).unwrap_or_else(|_| "null".into()),
                    task.per_task_speed_limit as i64,
                    serde_json::to_string(&task.collision_policy)
                        .unwrap_or_else(|_| "\"rename\"".into()),
                    task.connection_count as i64,
                    serde_json::to_string(&task.segments).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&task.completion_action)
                        .unwrap_or_else(|_| "\"none\"".into())
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn remove_task(&self, id: &str) -> Result<(), String> {
        self.connection
            .lock()
            .await
            .execute("DELETE FROM tasks WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn next_queue_position(&self) -> Result<i64, String> {
        self.connection
            .lock()
            .await
            .query_row(
                "SELECT COALESCE(MAX(queue_position),0)+1 FROM tasks",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
    }

    pub async fn reorder(&self, ids: &[String]) -> Result<(), String> {
        let mut connection = self.connection.lock().await;
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        for (index, id) in ids.iter().enumerate() {
            transaction
                .execute(
                    "UPDATE tasks SET queue_position=?1 WHERE id=?2",
                    params![index as i64, id],
                )
                .map_err(|e| e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())
    }

    pub async fn get_settings(&self) -> Result<AppSettings, String> {
        let connection = self.connection.lock().await;
        let json: Option<String> = connection
            .query_row(
                "SELECT value FROM app_state WHERE key='settings'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(json
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default())
    }

    pub async fn save_settings(&self, settings: &AppSettings) -> Result<(), String> {
        let json = serde_json::to_string(settings).map_err(|e| e.to_string())?;
        self.connection.lock().await.execute("INSERT INTO app_state(key,value) VALUES('settings',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json]).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn save_pairing(&self, extension_id: &str, token_hash: &str) -> Result<(), String> {
        self.connection.lock().await.execute(
            "INSERT INTO bridge_pairing(id,extension_id,token_hash,paired_at) VALUES(1,?1,?2,strftime('%s','now')*1000) ON CONFLICT(id) DO UPDATE SET extension_id=excluded.extension_id, token_hash=excluded.token_hash, paired_at=excluded.paired_at",
            params![extension_id, token_hash],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn get_pairing(&self) -> Result<Option<(String, String)>, String> {
        self.connection
            .lock()
            .await
            .query_row(
                "SELECT extension_id,token_hash FROM bridge_pairing WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub async fn clear_pairing(&self) -> Result<(), String> {
        self.connection
            .lock()
            .await
            .execute("DELETE FROM bridge_pairing", [])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn clear_history(&self, delete_completed: bool) -> Result<(), String> {
        let sql = if delete_completed {
            "DELETE FROM tasks WHERE status IN ('completed','cancelled')"
        } else {
            "DELETE FROM tasks WHERE status='cancelled'"
        };
        self.connection
            .lock()
            .await
            .execute(sql, [])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn task_from_row(row: &Row<'_>) -> rusqlite::Result<DownloadTask> {
    let headers_json: String = row.get("headers_json")?;
    let media_json: String = row.get("media_json")?;
    let collision_json: String = row.get("collision_policy")?;
    let segments_json: String = row.get("segments_json")?;
    let completion_action_json: String = row.get("completion_action")?;
    Ok(DownloadTask {
        id: row.get("id")?,
        url: row.get("url")?,
        file_name: row.get("file_name")?,
        destination: row.get("destination")?,
        total_bytes: row.get::<_, i64>("total_bytes")? as u64,
        downloaded_bytes: row.get::<_, i64>("downloaded_bytes")? as u64,
        speed: row.get::<_, i64>("speed")? as u64,
        eta_seconds: row.get::<_, Option<i64>>("eta_seconds")?.map(|v| v as u64),
        status: TaskStatus::from_db(&row.get::<_, String>("status")?),
        error: row.get("error")?,
        created_at: row.get::<_, i64>("created_at")? as u64,
        completed_at: row.get::<_, Option<i64>>("completed_at")?.map(|v| v as u64),
        scheduled_at: row.get::<_, Option<i64>>("scheduled_at")?.map(|v| v as u64),
        category: row.get("category")?,
        queue_position: row.get("queue_position")?,
        priority: row.get("priority")?,
        retry_count: row.get::<_, i64>("retry_count")? as u32,
        max_retries: row.get::<_, i64>("max_retries")? as u32,
        checksum_sha256: row.get("checksum_sha256")?,
        expected_checksum: row.get("expected_checksum")?,
        source: row.get("source")?,
        etag: row.get("etag")?,
        last_modified: row.get("last_modified")?,
        final_url: row.get("final_url")?,
        response_status: row
            .get::<_, Option<i64>>("response_status")?
            .map(|value| value as u16),
        content_type: row.get("content_type")?,
        accepts_ranges: row
            .get::<_, Option<i64>>("accepts_ranges")?
            .map(|value| value != 0),
        headers: serde_json::from_str(&headers_json).unwrap_or_default(),
        media: serde_json::from_str::<Option<MediaSelection>>(&media_json).unwrap_or_default(),
        per_task_speed_limit: row.get::<_, i64>("per_task_speed_limit")? as u64,
        collision_policy: serde_json::from_str(&collision_json).unwrap_or_default(),
        completion_action: serde_json::from_str(&completion_action_json).unwrap_or_default(),
        connection_count: row.get::<_, i64>("connection_count")? as u8,
        active_connections: 0,
        segments: serde_json::from_str::<Vec<DownloadSegment>>(&segments_json).unwrap_or_default(),
    })
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS tasks (
  id TEXT PRIMARY KEY, url TEXT NOT NULL, file_name TEXT NOT NULL, destination TEXT NOT NULL,
  total_bytes INTEGER NOT NULL DEFAULT 0, downloaded_bytes INTEGER NOT NULL DEFAULT 0, speed INTEGER NOT NULL DEFAULT 0,
  eta_seconds INTEGER, status TEXT NOT NULL, error TEXT, created_at INTEGER NOT NULL, completed_at INTEGER, scheduled_at INTEGER,
  category TEXT NOT NULL DEFAULT 'other', queue_position INTEGER NOT NULL DEFAULT 0, priority INTEGER NOT NULL DEFAULT 0,
  retry_count INTEGER NOT NULL DEFAULT 0, max_retries INTEGER NOT NULL DEFAULT 3, checksum_sha256 TEXT, expected_checksum TEXT,
  source TEXT NOT NULL DEFAULT 'desktop', etag TEXT, last_modified TEXT, final_url TEXT, response_status INTEGER,
  content_type TEXT, accepts_ranges INTEGER, headers_json TEXT NOT NULL DEFAULT '{}',
  media_json TEXT NOT NULL DEFAULT 'null', per_task_speed_limit INTEGER NOT NULL DEFAULT 0, collision_policy TEXT NOT NULL DEFAULT '"rename"',
  connection_count INTEGER NOT NULL DEFAULT 8, segments_json TEXT NOT NULL DEFAULT '[]',
  completion_action TEXT NOT NULL DEFAULT '"none"'
);
CREATE INDEX IF NOT EXISTS idx_tasks_status_queue ON tasks(status, priority DESC, queue_position ASC);
CREATE TABLE IF NOT EXISTS segments (
  task_id TEXT NOT NULL, segment_index INTEGER NOT NULL, start_byte INTEGER NOT NULL, end_byte INTEGER NOT NULL,
  downloaded_bytes INTEGER NOT NULL DEFAULT 0, path TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'pending',
  PRIMARY KEY(task_id, segment_index), FOREIGN KEY(task_id) REFERENCES tasks(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS app_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS bridge_pairing (id INTEGER PRIMARY KEY CHECK(id=1), extension_id TEXT NOT NULL, token_hash TEXT NOT NULL, paired_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS error_history (id INTEGER PRIMARY KEY AUTOINCREMENT, task_id TEXT, occurred_at INTEGER NOT NULL, code TEXT, message TEXT NOT NULL);
"#;

const UPSERT_TASK: &str = r#"
INSERT INTO tasks(id,url,file_name,destination,total_bytes,downloaded_bytes,speed,eta_seconds,status,error,created_at,completed_at,scheduled_at,category,queue_position,priority,retry_count,max_retries,checksum_sha256,expected_checksum,source,etag,last_modified,final_url,response_status,content_type,accepts_ranges,headers_json,media_json,per_task_speed_limit,collision_policy,connection_count,segments_json,completion_action)
VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33,?34)
ON CONFLICT(id) DO UPDATE SET url=excluded.url,file_name=excluded.file_name,destination=excluded.destination,total_bytes=excluded.total_bytes,downloaded_bytes=excluded.downloaded_bytes,speed=excluded.speed,eta_seconds=excluded.eta_seconds,status=excluded.status,error=excluded.error,completed_at=excluded.completed_at,scheduled_at=excluded.scheduled_at,category=excluded.category,queue_position=excluded.queue_position,priority=excluded.priority,retry_count=excluded.retry_count,max_retries=excluded.max_retries,checksum_sha256=excluded.checksum_sha256,expected_checksum=excluded.expected_checksum,source=excluded.source,etag=excluded.etag,last_modified=excluded.last_modified,final_url=excluded.final_url,response_status=excluded.response_status,content_type=excluded.content_type,accepts_ranges=excluded.accepts_ranges,headers_json=excluded.headers_json,media_json=excluded.media_json,per_task_speed_limit=excluded.per_task_speed_limit,collision_policy=excluded.collision_policy,connection_count=excluded.connection_count,segments_json=excluded.segments_json,completion_action=excluded.completion_action
"#;

fn ensure_task_column(connection: &Connection, name: &str, definition: &str) -> Result<(), String> {
    let mut statement = connection
        .prepare("PRAGMA table_info(tasks)")
        .map_err(|e| e.to_string())?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if !columns.iter().any(|column| column == name) {
        connection
            .execute_batch(&format!("ALTER TABLE tasks ADD COLUMN {name} {definition}"))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_task(directory: &PathBuf) -> DownloadTask {
        DownloadTask {
            id: "store-upsert".into(),
            url: "https://example.com/file.zip".into(),
            file_name: "file.zip".into(),
            destination: directory.to_string_lossy().into_owned(),
            total_bytes: 100,
            downloaded_bytes: 25,
            speed: 0,
            eta_seconds: None,
            status: TaskStatus::Paused,
            error: None,
            created_at: 1,
            completed_at: None,
            scheduled_at: None,
            category: "archives".into(),
            queue_position: 0,
            priority: 0,
            retry_count: 1,
            max_retries: 4,
            checksum_sha256: None,
            expected_checksum: None,
            source: "desktop".into(),
            etag: Some("etag-1".into()),
            last_modified: None,
            final_url: Some("https://cdn.example.com/file.zip".into()),
            response_status: Some(206),
            content_type: Some("application/zip".into()),
            accepts_ranges: Some(true),
            headers: HashMap::new(),
            media: None,
            per_task_speed_limit: 0,
            collision_policy: CollisionPolicy::Rename,
            completion_action: CompletionAction::None,
            connection_count: 8,
            active_connections: 0,
            segments: Vec::new(),
        }
    }

    #[test]
    fn inserts_and_updates_tasks_with_network_details() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut task = test_task(&directory.path().to_path_buf());
            store.upsert_task(&task).await.unwrap();
            task.max_retries = 7;
            task.response_status = Some(200);
            store.upsert_task(&task).await.unwrap();
            let restored = store.get_task(&task.id).await.unwrap().unwrap();
            assert_eq!(restored.max_retries, 7);
            assert_eq!(restored.response_status, Some(200));
            assert_eq!(restored.content_type.as_deref(), Some("application/zip"));
            assert_eq!(restored.accepts_ranges, Some(true));
        });
    }

    #[test]
    fn persists_settings_in_sqlite() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut settings = AppSettings::default();
            settings.concurrent_downloads = 7;
            settings.speed_limit_kbps = 2048;
            settings.low_memory_mode = true;
            settings.frosted_glass = true;
            store.save_settings(&settings).await.unwrap();
            let restored = store.get_settings().await.unwrap();
            assert_eq!(restored.concurrent_downloads, 7);
            assert_eq!(restored.speed_limit_kbps, 2048);
            assert!(restored.low_memory_mode);
            assert!(restored.frosted_glass);
        });
        assert!(directory.path().join("lumaget.db").exists());
    }

    #[test]
    fn migrates_old_tasks_with_a_safe_completion_action_default() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("lumaget.db");
        let connection = Connection::open(&database).unwrap();
        connection.execute_batch(
            r#"CREATE TABLE tasks (
              id TEXT PRIMARY KEY, url TEXT NOT NULL, file_name TEXT NOT NULL, destination TEXT NOT NULL,
              total_bytes INTEGER NOT NULL DEFAULT 0, downloaded_bytes INTEGER NOT NULL DEFAULT 0, speed INTEGER NOT NULL DEFAULT 0,
              eta_seconds INTEGER, status TEXT NOT NULL, error TEXT, created_at INTEGER NOT NULL, completed_at INTEGER, scheduled_at INTEGER,
              category TEXT NOT NULL DEFAULT 'other', queue_position INTEGER NOT NULL DEFAULT 0, priority INTEGER NOT NULL DEFAULT 0,
              retry_count INTEGER NOT NULL DEFAULT 0, max_retries INTEGER NOT NULL DEFAULT 3, checksum_sha256 TEXT, expected_checksum TEXT,
              source TEXT NOT NULL DEFAULT 'desktop', etag TEXT, last_modified TEXT, headers_json TEXT NOT NULL DEFAULT '{}',
              media_json TEXT NOT NULL DEFAULT 'null', per_task_speed_limit INTEGER NOT NULL DEFAULT 0,
              collision_policy TEXT NOT NULL DEFAULT '"rename"', connection_count INTEGER NOT NULL DEFAULT 8,
              segments_json TEXT NOT NULL DEFAULT '[]'
            );"#,
        ).unwrap();
        drop(connection);

        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let connection = store.connection.blocking_lock();
        let columns = connection
            .prepare("PRAGMA table_info(tasks)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for expected in [
            "completion_action",
            "final_url",
            "response_status",
            "content_type",
            "accepts_ranges",
        ] {
            assert!(columns.iter().any(|column| column == expected));
        }
    }
}
