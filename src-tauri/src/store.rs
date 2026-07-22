use crate::models::{
    AppSettings, BackupBundle, CategoryRule, CategoryRuleType, CollisionPolicy, CompletionAction,
    DownloadPreset, DownloadSegment, DownloadTask, FilenameCleanupRule, MediaCredential,
    MediaSelection, PlatformCompatibility, PlatformNamingTemplate, RestoreStats, SupportLevel, Tag,
    TaskStatus, TaskTemplate, UrlHistoryEntry,
};
use crate::secure_storage::{decrypt_password, encrypt_password};
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
        // Task 14: 任务级重试策略覆盖。NULL 表示使用全局默认。
        // 旧数据库迁移后所有现有任务的该字段均为 NULL，反序列化为 None。
        ensure_task_column(&connection, "retry_policy_override", "TEXT")?;
        // Task 31: 任务级代理覆盖与代理认证。
        // proxy_override: NULL 表示使用全局；空字符串表示显式禁用代理。
        // proxy_auth_json: JSON 字符串，含 username 和 DPAPI 加密的 password。
        // 旧数据库迁移后所有现有任务的这两个字段均为 NULL，反序列化为 None。
        ensure_task_column(&connection, "proxy_override", "TEXT")?;
        ensure_task_column(&connection, "proxy_auth_json", "TEXT")?;
        seed_builtin_download_presets(&connection)?;
        // Task 20: 文件名清理规则。新表通过 SCHEMA 中 CREATE TABLE IF NOT EXISTS 创建；
        // 此处仅插入内置默认规则（INSERT OR IGNORE 不覆盖用户改动）。
        seed_builtin_filename_cleanup_rules(&connection)?;
        // Task 43: 平台命名模板。表通过 SCHEMA 中 CREATE TABLE IF NOT EXISTS 创建；
        // 此处仅插入内置默认模板（INSERT OR IGNORE 不覆盖用户改动）。
        seed_builtin_platform_naming_templates(&connection)?;
        // Task 44: 平台兼容性矩阵。表通过 SCHEMA 中 CREATE TABLE IF NOT EXISTS 创建；
        // 此处仅插入内置 6 条默认记录（INSERT OR IGNORE 不覆盖用户改动）。
        seed_builtin_platform_compatibility(&connection)?;
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
                retry_policy_override: None,
                proxy_override: None,
                proxy_auth: None,
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
                        .unwrap_or_else(|_| "\"none\"".into()),
                    task.retry_policy_override.as_ref().and_then(|policy| {
                        serde_json::to_string(policy).ok()
                    }),
                    task.proxy_override.as_deref(),
                    task.proxy_auth.as_ref().and_then(|auth| {
                        serde_json::to_string(auth).ok()
                    })
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
        let mut settings: AppSettings = json
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        // Task 31.3：全局代理密码以 DPAPI 密文形式落库。
        // 读取后尝试解密；解密失败说明是旧版本明文（或跨用户迁移），保留原值
        // 让下一次 save_settings 重新加密。空密码跳过。
        if !settings.proxy_password.is_empty() {
            if let Ok(plain) = decrypt_password(&settings.proxy_password) {
                settings.proxy_password = plain;
            }
        }
        Ok(settings)
    }

    pub async fn save_settings(&self, settings: &AppSettings) -> Result<(), String> {
        // Task 31.3：保存前对全局代理密码做 DPAPI 加密，避免明文落库。
        // 加密在 clone 上进行，传入的 settings 保持明文不变，便于调用方继续 emit 事件。
        // 空密码或加密失败时保留原值（空密码不影响功能；加密失败时回退到明文存储，
        // 用户下次保存仍会重试）。
        let mut clone = settings.clone();
        if !clone.proxy_password.is_empty() {
            if let Ok(cipher) = encrypt_password(&clone.proxy_password) {
                clone.proxy_password = cipher;
            }
        }
        let json = serde_json::to_string(&clone).map_err(|e| e.to_string())?;
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

    /// 新增分类规则。`target_directory` 在调用前应已规范化。
    pub async fn category_rule_add(&self, rule: CategoryRule) -> Result<CategoryRule, String> {
        let connection = self.connection.lock().await;
        connection
            .execute(
                "INSERT INTO category_rules(id,name,rule_type,pattern,target_directory,enabled,priority) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    rule.id,
                    rule.name,
                    rule.rule_type.as_str(),
                    rule.pattern,
                    rule.target_directory,
                    i64::from(rule.enabled as i32),
                    rule.priority,
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(rule)
    }

    /// 更新分类规则。所有字段都会被覆盖。
    pub async fn category_rule_update(&self, rule: CategoryRule) -> Result<(), String> {
        let connection = self.connection.lock().await;
        let affected = connection
            .execute(
                "UPDATE category_rules SET name=?1, rule_type=?2, pattern=?3, \
                 target_directory=?4, enabled=?5, priority=?6 WHERE id=?7",
                params![
                    rule.name,
                    rule.rule_type.as_str(),
                    rule.pattern,
                    rule.target_directory,
                    i64::from(rule.enabled as i32),
                    rule.priority,
                    rule.id,
                ],
            )
            .map_err(|e| e.to_string())?;
        if affected == 0 {
            return Err("分类规则不存在".into());
        }
        Ok(())
    }

    /// 删除分类规则。
    pub async fn category_rule_delete(&self, id: &str) -> Result<(), String> {
        let connection = self.connection.lock().await;
        let affected = connection
            .execute("DELETE FROM category_rules WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        if affected == 0 {
            return Err("分类规则不存在".into());
        }
        Ok(())
    }

    /// 列出全部分类规则，按 priority 升序、name 升序排列。
    pub async fn category_rule_list(&self) -> Result<Vec<CategoryRule>, String> {
        let connection = self.connection.lock().await;
        let mut stmt = connection
            .prepare(
                "SELECT id,name,rule_type,pattern,target_directory,enabled,priority \
                 FROM category_rules ORDER BY priority ASC, name ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], category_rule_from_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// 新增文件名清理规则（Task 20）。
    pub async fn filename_cleanup_rule_add(
        &self,
        rule: FilenameCleanupRule,
    ) -> Result<FilenameCleanupRule, String> {
        let connection = self.connection.lock().await;
        connection
            .execute(
                "INSERT INTO filename_cleanup_rules(id,name,pattern,replacement,enabled,priority) \
                 VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    rule.id,
                    rule.name,
                    rule.pattern,
                    rule.replacement,
                    i64::from(rule.enabled as i32),
                    rule.priority,
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(rule)
    }

    /// 更新文件名清理规则（Task 20）。所有字段都会被覆盖。
    pub async fn filename_cleanup_rule_update(
        &self,
        rule: FilenameCleanupRule,
    ) -> Result<(), String> {
        let connection = self.connection.lock().await;
        let affected = connection
            .execute(
                "UPDATE filename_cleanup_rules SET name=?1, pattern=?2, \
                 replacement=?3, enabled=?4, priority=?5 WHERE id=?6",
                params![
                    rule.name,
                    rule.pattern,
                    rule.replacement,
                    i64::from(rule.enabled as i32),
                    rule.priority,
                    rule.id,
                ],
            )
            .map_err(|e| e.to_string())?;
        if affected == 0 {
            return Err("文件名清理规则不存在".into());
        }
        Ok(())
    }

    /// 删除文件名清理规则（Task 20）。
    pub async fn filename_cleanup_rule_delete(&self, id: &str) -> Result<(), String> {
        let connection = self.connection.lock().await;
        let affected = connection
            .execute("DELETE FROM filename_cleanup_rules WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        if affected == 0 {
            return Err("文件名清理规则不存在".into());
        }
        Ok(())
    }

    /// 列出全部文件名清理规则，按 priority 升序、name 升序排列（Task 20）。
    pub async fn filename_cleanup_rule_list(&self) -> Result<Vec<FilenameCleanupRule>, String> {
        let connection = self.connection.lock().await;
        let mut stmt = connection
            .prepare(
                "SELECT id,name,pattern,replacement,enabled,priority \
                 FROM filename_cleanup_rules ORDER BY priority ASC, name ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], filename_cleanup_rule_from_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// 新增下载预设。`is_builtin` 必须由调用方决定，自定义预设应为 `false`。
    /// 调用前应由 manager 校验 `connections` 是 1/2/4/8/16/32 之一。
    pub async fn download_preset_add(
        &self,
        preset: DownloadPreset,
    ) -> Result<DownloadPreset, String> {
        let connection = self.connection.lock().await;
        connection
            .execute(
                "INSERT INTO download_presets(id,name,connections,speed_limit,completion_action,verify_checksum,scheduled_at,is_builtin) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    preset.id,
                    preset.name,
                    i64::from(preset.connections),
                    preset.speed_limit.map(|v| v as i64),
                    preset
                        .completion_action
                        .as_ref()
                        .map(|action| serde_json::to_string(action).unwrap_or_else(|_| "\"none\"".into())),
                    i64::from(preset.verify_checksum as i32),
                    preset.scheduled_at,
                    i64::from(preset.is_builtin as i32),
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(preset)
    }

    /// 更新下载预设。`is_builtin` 以数据库中既有值为准，由调用方在传入前保证逻辑正确。
    /// 不存在的预设会返回中文错误。
    pub async fn download_preset_update(
        &self,
        preset: DownloadPreset,
    ) -> Result<(), String> {
        let connection = self.connection.lock().await;
        let affected = connection
            .execute(
                "UPDATE download_presets SET name=?1, connections=?2, speed_limit=?3, \
                 completion_action=?4, verify_checksum=?5, scheduled_at=?6, is_builtin=?7 \
                 WHERE id=?8",
                params![
                    preset.name,
                    i64::from(preset.connections),
                    preset.speed_limit.map(|v| v as i64),
                    preset
                        .completion_action
                        .as_ref()
                        .map(|action| serde_json::to_string(action).unwrap_or_else(|_| "\"none\"".into())),
                    i64::from(preset.verify_checksum as i32),
                    preset.scheduled_at,
                    i64::from(preset.is_builtin as i32),
                    preset.id,
                ],
            )
            .map_err(|e| e.to_string())?;
        if affected == 0 {
            return Err("预设不存在".into());
        }
        Ok(())
    }

    /// 删除下载预设。不区分内置/自定义；内置预设的删除保护由 manager 层负责。
    pub async fn download_preset_delete(&self, id: &str) -> Result<(), String> {
        let connection = self.connection.lock().await;
        let affected = connection
            .execute("DELETE FROM download_presets WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        if affected == 0 {
            return Err("预设不存在".into());
        }
        Ok(())
    }

    /// 列出全部下载预设，内置预设排在前面，然后按 name 升序。
    pub async fn download_preset_list(&self) -> Result<Vec<DownloadPreset>, String> {
        let connection = self.connection.lock().await;
        let mut stmt = connection
            .prepare(
                "SELECT id,name,connections,speed_limit,completion_action,verify_checksum,scheduled_at,is_builtin \
                 FROM download_presets ORDER BY is_builtin DESC, name ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], download_preset_from_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// 按 id 查询单个预设。
    pub async fn download_preset_get(
        &self,
        id: &str,
    ) -> Result<Option<DownloadPreset>, String> {
        let connection = self.connection.lock().await;
        connection
            .query_row(
                "SELECT id,name,connections,speed_limit,completion_action,verify_checksum,scheduled_at,is_builtin \
                 FROM download_presets WHERE id=?1",
                [id],
                download_preset_from_row,
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    /// URL 历史记录容量上限（Task 19）。
    /// 超过此数量时按 `last_used` 升序删除最旧的，保持 LRU 语义。
    pub const URL_HISTORY_MAX: i64 = 20;

    /// 新增或更新一条 URL 历史（Task 19）。
    ///
    /// - URL 在表内唯一：重复添加时仅更新 `last_used`（LRU 语义）。
    /// - 容量限制为 [`URL_HISTORY_MAX`]：插入后若超出上限，按 `last_used`
    ///   升序删除多余的旧记录。
    /// - 调用方负责对 URL 做基本校验（如非空、http/https 协议）；
    ///   此函数仅做持久化，不做协议过滤。
    pub async fn url_history_add(&self, url: &str) -> Result<(), String> {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return Err("URL 不能为空".into());
        }
        let now_ms = now_unix_millis();
        let mut connection = self.connection.lock().await;
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        transaction
            .execute(
                "INSERT INTO url_history(url,last_used) VALUES(?1,?2) \
                 ON CONFLICT(url) DO UPDATE SET last_used=excluded.last_used",
                params![trimmed, now_ms],
            )
            .map_err(|e| e.to_string())?;
        // 删除超出容量的最旧记录。子查询按 last_used 升序取出多余的 id。
        let overflow_sql = format!(
            "DELETE FROM url_history WHERE id IN (\
               SELECT id FROM url_history ORDER BY last_used ASC \
               LIMIT max(0, (SELECT COUNT(*) FROM url_history) - {max})\
             )",
            max = Self::URL_HISTORY_MAX
        );
        transaction.execute_batch(&overflow_sql).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())
    }

    /// 列出全部 URL 历史，按 `last_used` 降序（最近使用在前）。
    /// 最多返回 [`URL_HISTORY_MAX`] 条。
    pub async fn url_history_list(&self) -> Result<Vec<UrlHistoryEntry>, String> {
        let connection = self.connection.lock().await;
        let mut stmt = connection
            .prepare(
                "SELECT url,last_used FROM url_history \
                 ORDER BY last_used DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![Self::URL_HISTORY_MAX], url_history_from_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// 清空全部 URL 历史（Task 19）。
    pub async fn url_history_clear(&self) -> Result<(), String> {
        self.connection
            .lock()
            .await
            .execute("DELETE FROM url_history", [])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ===== Task 25: 标签 CRUD 与任务-标签关联操作 =====

    /// Task 25: 新增用户标签。`name` 在表内唯一，重复添加返回中文错误。
    /// `color` 为 `#RRGGBB` 十六进制颜色字符串，由调用方校验格式。
    pub async fn tag_add(&self, tag: Tag) -> Result<Tag, String> {
        let connection = self.connection.lock().await;
        connection
            .execute(
                "INSERT INTO tags(id,name,color) VALUES(?1,?2,?3)",
                params![tag.id, tag.name, tag.color],
            )
            .map_err(|e| {
                let msg = e.to_string();
                if msg.to_lowercase().contains("unique") {
                    "标签名称已存在".to_string()
                } else {
                    msg
                }
            })?;
        Ok(tag)
    }

    /// Task 25: 更新标签。所有字段都会被覆盖。
    /// `name` 重复时返回中文错误。
    pub async fn tag_update(&self, tag: Tag) -> Result<(), String> {
        let connection = self.connection.lock().await;
        let affected = connection
            .execute(
                "UPDATE tags SET name=?1, color=?2 WHERE id=?3",
                params![tag.name, tag.color, tag.id],
            )
            .map_err(|e| {
                let msg = e.to_string();
                if msg.to_lowercase().contains("unique") {
                    "标签名称已存在".to_string()
                } else {
                    msg
                }
            })?;
        if affected == 0 {
            return Err("标签不存在".into());
        }
        Ok(())
    }

    /// Task 25: 删除标签。`task_tags` 关联由外键 ON DELETE CASCADE 自动清理。
    pub async fn tag_delete(&self, id: &str) -> Result<(), String> {
        let connection = self.connection.lock().await;
        let affected = connection
            .execute("DELETE FROM tags WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        if affected == 0 {
            return Err("标签不存在".into());
        }
        Ok(())
    }

    /// Task 25: 列出全部标签，按 name 升序排列。
    pub async fn tag_list(&self) -> Result<Vec<Tag>, String> {
        let connection = self.connection.lock().await;
        let mut stmt = connection
            .prepare("SELECT id,name,color FROM tags ORDER BY name ASC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], tag_from_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// Task 25: 替换任务的全部标签关联。先删除现有关联，再批量插入新关联。
    /// 在事务中执行以保证原子性。`tag_ids` 中不存在的 tag_id 会因外键约束失败。
    pub async fn task_tags_set(
        &self,
        task_id: &str,
        tag_ids: Vec<String>,
    ) -> Result<(), String> {
        let mut connection = self.connection.lock().await;
        let tx = connection
            .transaction()
            .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM task_tags WHERE task_id=?1",
            [task_id],
        )
        .map_err(|e| e.to_string())?;
        for tag_id in &tag_ids {
            tx.execute(
                "INSERT INTO task_tags(task_id,tag_id) VALUES(?1,?2)",
                params![task_id, tag_id],
            )
            .map_err(|e| {
                let msg = e.to_string();
                if msg.to_lowercase().contains("foreign") {
                    "标签或任务不存在".to_string()
                } else {
                    msg
                }
            })?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Task 25: 获取单个任务的标签列表，按 name 升序排列。
    pub async fn task_tags_get(&self, task_id: &str) -> Result<Vec<Tag>, String> {
        let connection = self.connection.lock().await;
        let mut stmt = connection
            .prepare(
                "SELECT t.id,t.name,t.color FROM tags t \
                 INNER JOIN task_tags tt ON tt.tag_id=t.id \
                 WHERE tt.task_id=?1 ORDER BY t.name ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([task_id], tag_from_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// Task 25: 列出全部任务-标签关联，按 task_id 分组返回 HashMap。
    /// 用于前端一次性加载所有任务的标签，避免逐任务查询。
    pub async fn task_tags_list_all(&self) -> Result<HashMap<String, Vec<Tag>>, String> {
        let connection = self.connection.lock().await;
        let mut stmt = connection
            .prepare(
                "SELECT t.id,t.name,t.color,tt.task_id FROM tags t \
                 INNER JOIN task_tags tt ON tt.tag_id=t.id \
                 ORDER BY tt.task_id ASC, t.name ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(3)?,
                    tag_from_row(row)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut map: HashMap<String, Vec<Tag>> = HashMap::new();
        for row in rows {
            let (task_id, tag) = row.map_err(|e| e.to_string())?;
            map.entry(task_id).or_default().push(tag);
        }
        Ok(map)
    }

    // ===== Task 36: 任务模板 CRUD =====

    /// Task 36: 新增任务模板。
    /// `domain_pattern` 非空校验由 manager 层负责；此处仅做持久化。
    pub async fn task_template_add(
        &self,
        template: TaskTemplate,
    ) -> Result<TaskTemplate, String> {
        let connection = self.connection.lock().await;
        connection
            .execute(
                "INSERT INTO task_templates(id,name,domain_pattern,connections,speed_limit,headers_json,destination,completion_action,enabled,priority) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    template.id,
                    template.name,
                    template.domain_pattern,
                    template.connections.map(i64::from),
                    template.speed_limit.map(|v| v as i64),
                    template
                        .headers
                        .as_ref()
                        .map(|h| serde_json::to_string(h).unwrap_or_else(|_| "{}".into())),
                    template.destination.as_deref(),
                    template
                        .completion_action
                        .as_ref()
                        .map(|a| serde_json::to_string(a).unwrap_or_else(|_| "\"none\"".into())),
                    i64::from(template.enabled as i32),
                    template.priority,
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(template)
    }

    /// Task 36: 更新任务模板。所有字段都会被覆盖。
    pub async fn task_template_update(&self, template: TaskTemplate) -> Result<(), String> {
        let connection = self.connection.lock().await;
        let affected = connection
            .execute(
                "UPDATE task_templates SET name=?1, domain_pattern=?2, connections=?3, \
                 speed_limit=?4, headers_json=?5, destination=?6, completion_action=?7, \
                 enabled=?8, priority=?9 WHERE id=?10",
                params![
                    template.name,
                    template.domain_pattern,
                    template.connections.map(i64::from),
                    template.speed_limit.map(|v| v as i64),
                    template
                        .headers
                        .as_ref()
                        .map(|h| serde_json::to_string(h).unwrap_or_else(|_| "{}".into())),
                    template.destination.as_deref(),
                    template
                        .completion_action
                        .as_ref()
                        .map(|a| serde_json::to_string(a).unwrap_or_else(|_| "\"none\"".into())),
                    i64::from(template.enabled as i32),
                    template.priority,
                    template.id,
                ],
            )
            .map_err(|e| e.to_string())?;
        if affected == 0 {
            return Err("任务模板不存在".into());
        }
        Ok(())
    }

    /// Task 36: 删除任务模板。
    pub async fn task_template_delete(&self, id: &str) -> Result<(), String> {
        let connection = self.connection.lock().await;
        let affected = connection
            .execute("DELETE FROM task_templates WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        if affected == 0 {
            return Err("任务模板不存在".into());
        }
        Ok(())
    }

    /// Task 36: 列出全部任务模板，按 priority 升序、name 升序排列。
    pub async fn task_template_list(&self) -> Result<Vec<TaskTemplate>, String> {
        let connection = self.connection.lock().await;
        let mut stmt = connection
            .prepare(
                "SELECT id,name,domain_pattern,connections,speed_limit,headers_json,destination,completion_action,enabled,priority \
                 FROM task_templates ORDER BY priority ASC, name ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], task_template_from_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    // ===== Task 46: 媒体凭证 CRUD =====

    /// Task 46: 新增或更新一条媒体凭证（按 domain 主键 upsert）。
    ///
    /// `credential.cookie` 为明文，调用方（命令层）负责传入；
    /// 此处使用 DPAPI 加密后存入 `cookie_encrypted` 列。
    /// 空字符串 Cookie 由调用方决定是否允许（用于"清除 Cookie 但保留域名条目"场景）。
    /// 返回的 `MediaCredential` 与传入一致（明文），前端可直接用于展示。
    pub async fn media_credential_upsert(
        &self,
        credential: MediaCredential,
    ) -> Result<MediaCredential, String> {
        let cookie_encrypted = if credential.cookie.is_empty() {
            String::new()
        } else {
            encrypt_password(&credential.cookie)?
        };
        let connection = self.connection.lock().await;
        connection
            .execute(
                "INSERT INTO media_credentials(domain,cookie_encrypted,referer,user_agent,updated_at) \
                 VALUES(?1,?2,?3,?4,?5) \
                 ON CONFLICT(domain) DO UPDATE SET \
                 cookie_encrypted=excluded.cookie_encrypted, \
                 referer=excluded.referer, \
                 user_agent=excluded.user_agent, \
                 updated_at=excluded.updated_at",
                params![
                    credential.domain,
                    cookie_encrypted,
                    credential.referer,
                    credential.user_agent,
                    credential.updated_at,
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(credential)
    }

    /// Task 46: 按 domain 查询单条凭证。
    ///
    /// 返回的 `MediaCredential.cookie` 为解密后的明文。
    /// 解密失败（换机器/密文损坏）时返回中文错误，调用方应提示用户重新录入。
    /// 不存在时返回 `None`，不视为错误。
  pub async fn media_credential_get(
        &self,
        domain: &str,
    ) -> Result<Option<MediaCredential>, String> {
        let connection = self.connection.lock().await;
        let row: Option<(String, Option<String>, Option<String>, String)> = connection
            .query_row(
                "SELECT cookie_encrypted,referer,user_agent,updated_at \
                 FROM media_credentials WHERE domain=?1",
                [domain],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match row {
            None => Ok(None),
            Some((cookie_encrypted, referer, user_agent, updated_at)) => {
                let cookie = if cookie_encrypted.is_empty() {
                    String::new()
                } else {
                    decrypt_password(&cookie_encrypted)
                        .map_err(|_| "凭证解密失败，请重新录入 Cookie".to_string())?
                };
                Ok(Some(MediaCredential {
                    domain: domain.to_string(),
                    cookie,
                    referer,
                    user_agent,
                    updated_at,
                }))
            }
        }
    }

    /// Query credential matching domain, its parent domains (e.g. v.douyin.com -> douyin.com)
    /// or platform alias domains (e.g. youtu.be -> youtube.com, x.com -> twitter.com).
    pub async fn media_credential_get_matching(
        &self,
        domain: &str,
    ) -> Result<Option<MediaCredential>, String> {
        if let Some(cred) = self.media_credential_get(domain).await? {
            return Ok(Some(cred));
        }
        let parts: Vec<&str> = domain.split('.').collect();
        for i in 1..parts.len() {
            if parts.len() - i < 2 {
                break;
            }
            let parent = parts[i..].join(".");
            if let Some(cred) = self.media_credential_get(&parent).await? {
                return Ok(Some(cred));
            }
        }
        let platform = crate::media_platforms::detect_platform(&format!("https://{domain}"));
        for &alt_domain in platform.candidate_domains() {
            if alt_domain != domain {
                if let Some(cred) = self.media_credential_get(alt_domain).await? {
                    return Ok(Some(cred));
                }
            }
        }
        Ok(None)
    }

    /// Task 46: 按 domain 删除单条凭证。不存在不算错误（幂等）。
    pub async fn media_credential_delete(&self, domain: &str) -> Result<(), String> {
        let connection = self.connection.lock().await;
        connection
            .execute("DELETE FROM media_credentials WHERE domain=?1", [domain])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Task 46: 列出全部凭证，按 domain 升序返回。
    ///
    /// 任一行解密失败时该行被跳过（不阻塞其它行返回），调用方可以通过比对
    /// 列表长度与数据库行数感知到失败；前端在"凭证管理"面板中应单独调用
    /// `media_credential_get` 触发详细错误提示，便于用户重新录入。
    pub async fn media_credential_list(&self) -> Result<Vec<MediaCredential>, String> {
        let connection = self.connection.lock().await;
        let mut stmt = connection
            .prepare(
                "SELECT domain,cookie_encrypted,referer,user_agent,updated_at \
                 FROM media_credentials ORDER BY domain ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for row in rows {
            let (domain, cookie_encrypted, referer, user_agent, updated_at) =
                row.map_err(|e| e.to_string())?;
            let cookie = if cookie_encrypted.is_empty() {
                String::new()
            } else {
                match decrypt_password(&cookie_encrypted) {
                    Ok(plain) => plain,
                    // 列表场景：跳过解密失败的行，避免一条坏数据阻塞整个列表。
                    Err(_) => continue,
                }
            };
            list.push(MediaCredential {
                domain,
                cookie,
                referer,
                user_agent,
                updated_at,
            });
        }
        Ok(list)
    }

    // ===== Task 44: 平台兼容性矩阵 =====

    /// Task 44：列出全部平台兼容性记录，按 platform 升序返回。
    ///
    /// 内置 6 条默认数据（YouTube/哔哩哔哩=Verified，抖音/TikTok/Twitter/微博
    /// =Experimental）在打开数据库时通过 `INSERT OR IGNORE` 写入，用户对内置
    /// 记录的修改（同 platform）不会被覆盖。`known_issues_json` 反序列化失败时
    /// 视为空数组，保证损坏数据不阻塞列表返回。
    pub async fn platform_compatibility_list(
        &self,
    ) -> Result<Vec<PlatformCompatibility>, String> {
        let connection = self.connection.lock().await;
        let mut stmt = connection
            .prepare(
                "SELECT platform,level,notes,known_issues_json,last_tested_at \
                 FROM platform_compatibility ORDER BY platform ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for row in rows {
            let (platform, level_str, notes, known_issues_json, last_tested_at) =
                row.map_err(|e| e.to_string())?;
            let level = match level_str.as_str() {
                "verified" => SupportLevel::Verified,
                "unsupported" => SupportLevel::Unsupported,
                // 旧数据库或未知值默认 Experimental（SupportLevel::default）
                _ => SupportLevel::Experimental,
            };
            // known_issues_json 损坏时降级为空数组，不阻塞列表
            let known_issues: Vec<String> =
                serde_json::from_str(&known_issues_json).unwrap_or_default();
            list.push(PlatformCompatibility {
                platform,
                level,
                notes,
                known_issues,
                last_tested_at,
            });
        }
        Ok(list)
    }

    /// Task 44：按 platform 查询单条兼容性记录。不存在时返回 `None`。
    pub async fn platform_compatibility_get(
        &self,
        platform: &str,
    ) -> Result<Option<PlatformCompatibility>, String> {
        let connection = self.connection.lock().await;
        let row: Option<(String, String, String, String)> = connection
            .query_row(
                "SELECT level,notes,known_issues_json,last_tested_at \
                 FROM platform_compatibility WHERE platform=?1",
                [platform],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match row {
            None => Ok(None),
            Some((level_str, notes, known_issues_json, last_tested_at)) => {
                let level = match level_str.as_str() {
                    "verified" => SupportLevel::Verified,
                    "unsupported" => SupportLevel::Unsupported,
                    _ => SupportLevel::Experimental,
                };
                let known_issues: Vec<String> =
                    serde_json::from_str(&known_issues_json).unwrap_or_default();
                Ok(Some(PlatformCompatibility {
                    platform: platform.to_string(),
                    level,
                    notes,
                    known_issues,
                    last_tested_at,
                }))
            }
        }
    }

    // ===== Task 43: 平台命名模板 CRUD =====

    /// Task 43: 新增一条平台命名模板。
    ///
    /// `is_builtin` 必须由调用方决定（自定义模板应为 `false`）。
    /// 同 `id` 已存在时返回中文错误（由调用方决定是否切换为 update）。
    pub async fn platform_naming_template_add(
        &self,
        template: PlatformNamingTemplate,
    ) -> Result<PlatformNamingTemplate, String> {
        let connection = self.connection.lock().await;
        connection
            .execute(
                "INSERT INTO platform_naming_templates(id,platform,template,enabled,is_builtin) \
                 VALUES(?1,?2,?3,?4,?5)",
                params![
                    template.id,
                    template.platform,
                    template.template,
                    i64::from(template.enabled as i32),
                    i64::from(template.is_builtin as i32),
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(template)
    }

    /// Task 43: 更新一条平台命名模板。所有字段都会被覆盖。
    /// `is_builtin` 以数据库中既有值为准（不允许通过 update 把内置标记改为自定义或反之）；
    /// 由调用方在传入前保证逻辑正确（前端禁止修改 is_builtin 字段）。
    /// 不存在的模板返回中文错误。
    pub async fn platform_naming_template_update(
        &self,
        template: PlatformNamingTemplate,
    ) -> Result<(), String> {
        let connection = self.connection.lock().await;
        let affected = connection
            .execute(
                "UPDATE platform_naming_templates SET platform=?1, template=?2, \
                 enabled=?3 WHERE id=?4",
                params![
                    template.platform,
                    template.template,
                    i64::from(template.enabled as i32),
                    template.id,
                ],
            )
            .map_err(|e| e.to_string())?;
        if affected == 0 {
            return Err("平台命名模板不存在".into());
        }
        Ok(())
    }

    /// Task 43: 按 id 删除一条平台命名模板。
    /// 不存在的模板返回中文错误（与 `filename_cleanup_rule_delete` 一致的语义）。
    /// 内置模板的删除保护由调用方（前端）实现，此处不做强制校验，
    /// 以便未来需要时可以由命令层显式清理内置模板。
    pub async fn platform_naming_template_delete(&self, id: &str) -> Result<(), String> {
        let connection = self.connection.lock().await;
        let affected = connection
            .execute("DELETE FROM platform_naming_templates WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        if affected == 0 {
            return Err("平台命名模板不存在".into());
        }
        Ok(())
    }

    /// Task 43: 列出全部平台命名模板。
    ///
    /// 排序：platform 升序、enabled 降序（启用的在前）、id 升序。
    /// 此顺序保证前端展示时同平台的启用模板优先出现，
    /// 且 `find_template_for_platform` 可以取列表中第一条匹配 enabled 模板。
    pub async fn platform_naming_template_list(
        &self,
    ) -> Result<Vec<PlatformNamingTemplate>, String> {
        let connection = self.connection.lock().await;
        let mut stmt = connection
            .prepare(
                "SELECT id,platform,template,enabled,is_builtin \
                 FROM platform_naming_templates \
                 ORDER BY platform ASC, enabled DESC, id ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], platform_naming_template_from_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// Task 27.3 / 27.6: 在 SQLite 事务中恢复备份 Bundle。
    ///
    /// 依次恢复：设置 (settings)、分类规则 (category_rules)、文件名清理规则 (filename_cleanup_rules)、
    /// 下载预设 (download_presets)、URL 历史 (url_history)、任务列表 (tasks)。
    /// 任务按 ID 去重（已存在的任务跳过，不覆盖进度）。
    /// 返回 RestoreStats 统计信息与实际插入数据库的 restored_tasks 列表。
    pub async fn restore_backup_bundle(
        &self,
        bundle: &BackupBundle,
        sanitized_tasks: Vec<DownloadTask>,
    ) -> Result<(RestoreStats, Vec<DownloadTask>), String> {
        let mut connection = self.connection.lock().await;
        let tx = connection.transaction().map_err(|e| e.to_string())?;

        let mut stats = RestoreStats::default();

        // 1. 设置 (settings)
        if let Some(ref settings) = bundle.settings {
            let mut clone = settings.clone();
            if !clone.proxy_password.is_empty() {
                if let Ok(cipher) = encrypt_password(&clone.proxy_password) {
                    clone.proxy_password = cipher;
                }
            }
            let json = serde_json::to_string(&clone).map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT INTO app_state(key,value) VALUES('settings',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [json],
            )
            .map_err(|e| e.to_string())?;
            stats.settings_replaced = true;
        }

        // 2. 分类规则 (category_rules)
        for rule in &bundle.category_rules {
            tx.execute(
                "INSERT INTO category_rules(id,name,rule_type,pattern,target_directory,enabled,priority) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7) \
                 ON CONFLICT(id) DO UPDATE SET name=excluded.name, rule_type=excluded.rule_type, \
                 pattern=excluded.pattern, target_directory=excluded.target_directory, \
                 enabled=excluded.enabled, priority=excluded.priority",
                params![
                    rule.id,
                    rule.name,
                    rule.rule_type.as_str(),
                    rule.pattern,
                    rule.target_directory,
                    i64::from(rule.enabled as i32),
                    rule.priority,
                ],
            )
            .map_err(|e| e.to_string())?;
            stats.rules_applied += 1;
        }

        // 3. 文件名清理规则 (filename_cleanup_rules)
        for rule in &bundle.filename_cleanup_rules {
            tx.execute(
                "INSERT INTO filename_cleanup_rules(id,name,pattern,replacement,enabled,priority) \
                 VALUES(?1,?2,?3,?4,?5,?6) \
                 ON CONFLICT(id) DO UPDATE SET name=excluded.name, pattern=excluded.pattern, \
                 replacement=excluded.replacement, enabled=excluded.enabled, priority=excluded.priority",
                params![
                    rule.id,
                    rule.name,
                    rule.pattern,
                    rule.replacement,
                    i64::from(rule.enabled as i32),
                    rule.priority,
                ],
            )
            .map_err(|e| e.to_string())?;
            stats.rules_applied += 1;
        }

        // 4. 下载预设 (download_presets)
        for preset in &bundle.download_presets {
            let json = serde_json::to_string(preset).map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT INTO download_presets(id,value) VALUES(?1,?2) \
                 ON CONFLICT(id) DO UPDATE SET value=excluded.value",
                params![preset.id, json],
            )
            .map_err(|e| e.to_string())?;
            stats.rules_applied += 1;
        }

        // 5. URL 历史 (url_history)
        for entry in &bundle.url_history {
            tx.execute(
                "INSERT INTO url_history(url,last_used) VALUES(?1,?2) \
                 ON CONFLICT(url) DO UPDATE SET last_used=MAX(url_history.last_used, excluded.last_used)",
                params![entry.url, entry.last_used],
            )
            .map_err(|e| e.to_string())?;
            stats.url_history_added += 1;
        }

        // 6. 任务列表 (tasks)
        let mut restored_tasks = Vec::new();
        for task in sanitized_tasks {
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1)",
                    [&task.id],
                    |r| r.get(0),
                )
                .unwrap_or(false);

            if exists {
                stats.skipped_tasks += 1;
            } else {
                let headers_json = serde_json::to_string(&task.headers).map_err(|e| e.to_string())?;
                let media_json = task.media.as_ref().map(serde_json::to_string).transpose().map_err(|e| e.to_string())?;
                let segments_json = serde_json::to_string(&task.segments).map_err(|e| e.to_string())?;
                let completion_action_json = serde_json::to_string(&task.completion_action).map_err(|e| e.to_string())?;
                let retry_policy_json = task.retry_policy_override.as_ref().map(serde_json::to_string).transpose().map_err(|e| e.to_string())?;
                let proxy_auth_json = task.proxy_auth.as_ref().map(serde_json::to_string).transpose().map_err(|e| e.to_string())?;

                tx.execute(
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
                        task.created_at,
                        task.completed_at,
                        task.scheduled_at,
                        task.category,
                        task.queue_position,
                        task.priority,
                        task.retry_count as i64,
                        task.max_retries as i64,
                        task.checksum_sha256,
                        task.expected_checksum,
                        task.source,
                        task.etag,
                        task.last_modified,
                        task.final_url,
                        task.response_status.map(|v| v as i64),
                        task.content_type,
                        task.accepts_ranges.map(|v| v as i64),
                        headers_json,
                        media_json,
                        task.per_task_speed_limit as i64,
                        serde_json::to_string(&task.collision_policy).unwrap_or_else(|_| "\"rename\"".into()),
                        task.connection_count as i64,
                        segments_json,
                        completion_action_json,
                        retry_policy_json,
                        task.proxy_override,
                        proxy_auth_json,
                    ],
                )
                .map_err(|e| e.to_string())?;

                stats.added_tasks += 1;
                restored_tasks.push(task);
            }
        }

        tx.commit().map_err(|e| e.to_string())?;

        Ok((stats, restored_tasks))
    }
}


fn url_history_from_row(row: &Row<'_>) -> rusqlite::Result<UrlHistoryEntry> {
    Ok(UrlHistoryEntry {
        url: row.get("url")?,
        last_used: row.get("last_used")?,
    })
}

/// 返回 Unix 毫秒时间戳，与 `manager::now()` 保持一致语义。
fn now_unix_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn category_rule_from_row(row: &Row<'_>) -> rusqlite::Result<CategoryRule> {
    let rule_type_str: String = row.get("rule_type")?;
    let enabled_int: i64 = row.get("enabled")?;
    Ok(CategoryRule {
        id: row.get("id")?,
        name: row.get("name")?,
        rule_type: CategoryRuleType::from_db(&rule_type_str),
        pattern: row.get("pattern")?,
        target_directory: row.get("target_directory")?,
        enabled: enabled_int != 0,
        priority: row.get("priority")?,
    })
}

/// Task 20: 从 SQLite 行读取一条文件名清理规则。
fn filename_cleanup_rule_from_row(row: &Row<'_>) -> rusqlite::Result<FilenameCleanupRule> {
    let enabled_int: i64 = row.get("enabled")?;
    Ok(FilenameCleanupRule {
        id: row.get("id")?,
        name: row.get("name")?,
        pattern: row.get("pattern")?,
        replacement: row.get("replacement")?,
        enabled: enabled_int != 0,
        priority: row.get("priority")?,
    })
}

/// Task 25: 从 SQLite 行读取一条 Tag。
fn tag_from_row(row: &Row<'_>) -> rusqlite::Result<Tag> {
    Ok(Tag {
        id: row.get("id")?,
        name: row.get("name")?,
        color: row.get("color")?,
    })
}

/// Task 36: 从 SQLite 行读取一条任务模板。
///
/// `headers_json` / `completion_action` / `connections` / `speed_limit` /
/// `destination` 列在旧数据库迁移后为 NULL，反序列化为 `None`，保证向后兼容
/// （AGENTS.md §2）。
fn task_template_from_row(row: &Row<'_>) -> rusqlite::Result<TaskTemplate> {
    let connections: Option<i64> = row.get("connections")?;
    let speed_limit: Option<i64> = row.get("speed_limit")?;
    let headers_json: Option<String> = row.get("headers_json")?;
    let destination: Option<String> = row.get("destination")?;
    let completion_action_json: Option<String> = row.get("completion_action")?;
    let enabled_int: i64 = row.get("enabled")?;
    Ok(TaskTemplate {
        id: row.get("id")?,
        name: row.get("name")?,
        domain_pattern: row.get("domain_pattern")?,
        connections: connections.map(|v| v.clamp(0, 255) as u8),
        speed_limit: speed_limit.map(|v| v as u64),
        headers: headers_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<HashMap<String, String>>(s).ok()),
        destination,
        completion_action: completion_action_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<CompletionAction>(s).ok()),
        enabled: enabled_int != 0,
        priority: row.get("priority")?,
    })
}

/// Task 43: 从 SQLite 行读取一条平台命名模板。
///
/// `is_builtin` 列在旧数据库迁移后为 NULL（默认 0），反序列化为 `false`，
/// 与 `#[serde(default)]` 保证的向后兼容语义一致（AGENTS.md §2）。
fn platform_naming_template_from_row(row: &Row<'_>) -> rusqlite::Result<PlatformNamingTemplate> {
    let enabled_int: i64 = row.get("enabled")?;
    let builtin_int: i64 = row.get("is_builtin")?;
    Ok(PlatformNamingTemplate {
        id: row.get("id")?,
        platform: row.get("platform")?,
        template: row.get("template")?,
        enabled: enabled_int != 0,
        is_builtin: builtin_int != 0,
    })
}

fn download_preset_from_row(row: &Row<'_>) -> rusqlite::Result<DownloadPreset> {
    let speed_limit: Option<i64> = row.get("speed_limit")?;
    let completion_action_json: Option<String> = row.get("completion_action")?;
    let verify_int: i64 = row.get("verify_checksum")?;
    let builtin_int: i64 = row.get("is_builtin")?;
    let connections_int: i64 = row.get("connections")?;
    Ok(DownloadPreset {
        id: row.get("id")?,
        name: row.get("name")?,
        connections: connections_int.clamp(0, 255) as u8,
        speed_limit: speed_limit.map(|v| v as u64),
        completion_action: completion_action_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<CompletionAction>(s).ok()),
        verify_checksum: verify_int != 0,
        scheduled_at: row.get("scheduled_at")?,
        is_builtin: builtin_int != 0,
    })
}

/// 在打开数据库时按 INSERT OR IGNORE 写入 5 个内置预设。
///
/// - `INSERT OR IGNORE` 保证用户对内置预设的修改（同 id）不会被覆盖；
/// - 旧版本数据库首次升级时会一次性插入全部内置预设；
/// - 内置预设 `is_builtin = 1`，自定义预设为 0。
fn seed_builtin_download_presets(connection: &Connection) -> Result<(), String> {
    // 内置预设的 completion_action 用 JSON 字符串存储以与 tasks 表保持一致。
    // 夜间预设的 completion_action = "shutdown"（CompletionAction::Shutdown 的 kebab-case 序列化值）。
    let shutdown_json = serde_json::to_string(&CompletionAction::Shutdown)
        .unwrap_or_else(|_| "\"shutdown\"".into());
    connection
        .execute_batch(&format!(
            r#"INSERT OR IGNORE INTO download_presets(id,name,connections,speed_limit,completion_action,verify_checksum,scheduled_at,is_builtin) VALUES
              ('default','普通下载',8,NULL,NULL,0,NULL,1),
              ('lightweight','轻量模式',2,NULL,NULL,0,NULL,1),
              ('large-file','大文件',16,NULL,NULL,1,NULL,1),
              ('background','后台下载',4,1000000,NULL,0,NULL,1),
              ('night','夜间下载',8,NULL,'{shutdown_json}',0,'22:00',1);
            "#,
        ))
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Task 20: 在打开数据库时按 INSERT OR IGNORE 写入内置文件名清理规则。
///
/// 内置规则：
/// - `remove-bracket-site`：去除 `[www.xxx.com]` 标记
/// - `remove-paren-quality`：去除 `(1080p)` 等画质标记
/// - `remove-underscore-site`：去除 `_www.xxx.com_` 下划线包围
/// - `collapse-spaces`：合并多余空格和下划线为单空格
///
/// `INSERT OR IGNORE` 保证用户对内置规则的修改（同 id）不会被覆盖；
/// 旧版本数据库首次升级时会一次性插入全部内置规则。
fn seed_builtin_filename_cleanup_rules(connection: &Connection) -> Result<(), String> {
    // 正则中的反斜杠在 SQL 字符串里需要双写（`\\` 表示一个反斜杠）。
    // 这里使用 raw string + format! 便于阅读；ID 与 priority 见 spec Task 20.2。
    connection
        .execute_batch(
            r#"INSERT OR IGNORE INTO filename_cleanup_rules(id,name,pattern,replacement,enabled,priority) VALUES
              ('remove-bracket-site','去除 [站点] 标记','\[(www\.)?[\w.-]+\]','',1,10),
              ('remove-chinese-bracket-site','去除 【站点】 标记','【(www\.)?[\w.-]+】','',1,11),
              ('remove-chinese-bracket-promo','去除 【宣传语】 标记','【[^】]*?(最新|发布|免费|首发|高清|下载|分享|关注|精品|推荐|无水印)[^】]*?】','',1,12),
              ('remove-paren-quality','去除 (1080p) 画质标记','\(\d{3,4}[pP]\)','',1,20),
              ('remove-square-bracket-quality','去除 [1080p] 画质标记','\[\d{3,4}[pP]\]','',1,21),
              ('remove-media-codec-tags','去除影音格式编码噪音','(?i)[._-]?\b(h\.?264|x264|h\.?265|x265|hevc|bluray|web-?rip|hdr|ddp\d\.\d|aac|dts)\b','',1,25),
              ('remove-underscore-site','去除 _www.站点_ 包围','_www\.[\w.-]+_','',1,30),
              ('remove-hash-tags','去除 #话题 标记','#[^\s#.]+','',1,35),
              ('remove-copy-suffix','去除副本重名标记','\s*-\s*副本|\s*-\s*Copy','',1,38),
              ('collapse-spaces','合并空格与下划线','[\s_]+',' ',1,40),
              ('strip-trailing-spaces','去除点前面的空格','\s+(\.[a-zA-Z0-9]+)$','$1',1,45);
            "#,
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Task 43: 在打开数据库时按 INSERT OR IGNORE 写入内置平台命名模板。
///
/// 内置模板覆盖 6 个常用平台：
/// - 抖音：`{title}_{date}`
/// - TikTok：`{title}_{date}`
/// - Twitter/X：`{title}_{id}`（标题 + 推文 ID 便于回溯）
/// - YouTube：`{title}_{id}`
/// - B 站：`{title}_{bvid}`（BV 号是 B 站唯一标识）
/// - 微博：`{title}_{date}`
///
/// `INSERT OR IGNORE` 保证用户对内置模板的修改（同 id）不会被覆盖；
/// 旧版本数据库首次升级时会一次性插入全部内置模板。
/// 内置模板 `is_builtin = 1`，可编辑可禁用但前端应禁止删除（AGENTS.md §3）。
fn seed_builtin_platform_naming_templates(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            r#"INSERT OR IGNORE INTO platform_naming_templates(id,platform,template,enabled,is_builtin) VALUES
              ('douyin-default','douyin','{title}_{date}',1,1),
              ('tiktok-default','tiktok','{title}_{date}',1,1),
              ('twitter-default','twitter','{title}_{id}',1,1),
              ('youtube-default','youtube','{title}_{id}',1,1),
              ('bilibili-default','bilibili','{title}_{bvid}',1,1),
              ('weibo-default','weibo','{title}_{date}',1,1);
            "#,
        )
        .map_err(|e| e.to_string())?;

    // 升级已存在但未被用户定制过的内置模板
    let migrations = [
        ("douyin-default", "{author}_{title}_{date}", "{title}_{date}"),
        ("tiktok-default", "{author}_{title}_{date}", "{title}_{date}"),
        ("twitter-default", "{author}_{id}_{date}", "{id}_{date}"),
        ("twitter-default", "{id}_{date}", "{title}_{id}"),
        ("youtube-default", "{channel}_{title}_{id}", "{title}_{id}"),
        ("bilibili-default", "{author}_{title}_{bvid}", "{title}_{bvid}"),
        ("weibo-default", "{author}_{title}_{date}", "{title}_{date}"),
    ];
    for (id, old_val, new_val) in migrations {
        let _ = connection.execute(
            "UPDATE platform_naming_templates SET template = ?1 WHERE id = ?2 AND template = ?3",
            rusqlite::params![new_val, id, old_val],
        );
    }
    Ok(())
}

/// Task 44：在打开数据库时按 `INSERT OR IGNORE` 写入 6 条内置平台兼容性记录。
///
/// 内置记录：
/// - YouTube / 哔哩哔哩：`Verified`（经过完整回归测试，预期可用）
/// - 抖音 / TikTok / Twitter / 微博：`Experimental`（基本可用但成功率受
///   平台变更影响，可能需要用户手动提供 Cookie）
///
/// `INSERT OR IGNORE` 保证用户对内置记录的修改（同 platform）不会被覆盖；
/// 旧版本数据库首次升级时会一次性插入全部内置记录。
/// `known_issues_json` 使用 JSON 数组文本存储以保持与现有 JSON 列一致。
fn seed_builtin_platform_compatibility(connection: &Connection) -> Result<(), String> {
    // 双引号在 SQL 字符串里需要双写（`""` 表示一个双引号）。
    // 这里使用 raw string + format! 便于阅读，并预转义中文 notes 中的双引号。
    // 各平台 known_issues 为空数组（暂无已知问题，后续可由前端编辑补充）。
    let rows: [(&str, &str, &str); 6] = [
        ("bilibili", "verified", "哔哩哔哩普通视频、多P合集、番剧剧集与实时直播流均可正常下载"),
        ("douyin", "verified", "抖音短链、图集、单视频及实时直播流全功能正常支持"),
        ("tiktok", "experimental", "TikTok 普通视频、图集可下载，部分内容受地区限制"),
        ("twitter", "experimental", "Twitter/X 普通推文视频可下载，需提供 Cookie"),
        ("weibo", "experimental", "微博普通视频、图集可下载，需提供 Cookie"),
        ("youtube", "experimental", "YouTube 普通视频、直播回放、短视频受反爬机制影响"),
    ];
    for (platform, level, notes) in rows {
        connection
            .execute(
                "INSERT INTO platform_compatibility(platform,level,notes,known_issues_json,last_tested_at) \
                 VALUES(?1,?2,?3,'[]','') \
                 ON CONFLICT(platform) DO UPDATE SET level=?2, notes=?3 WHERE notes = '哔哩哔哩普通视频、番剧、直播回放可正常下载' OR notes = '抖音短链、图集、单视频全功能正常支持' OR notes = 'YouTube 普通视频、直播回放、短视频可正常下载'",
                params![platform, level, notes],
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn task_from_row(row: &Row<'_>) -> rusqlite::Result<DownloadTask> {
    let headers_json: String = row.get("headers_json")?;
    let media_json: String = row.get("media_json")?;
    let collision_json: String = row.get("collision_policy")?;
    let segments_json: String = row.get("segments_json")?;
    let completion_action_json: String = row.get("completion_action")?;
    // retry_policy_override 在旧数据库迁移后为 NULL，serde_json 反序列化
    // NULL/空字符串时返回 None，保证向后兼容。
    let retry_policy_override_json: Option<String> = row.get("retry_policy_override")?;
    // Task 31: proxy_override / proxy_auth_json 同样以 NULL 起步。
    let proxy_override: Option<String> = row.get("proxy_override")?;
    let proxy_auth_json: Option<String> = row.get("proxy_auth_json")?;
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
        retry_policy_override: retry_policy_override_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<crate::models::RetryPolicy>(s).ok()),
        proxy_override,
        proxy_auth: proxy_auth_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<crate::models::ProxyAuth>(s).ok()),
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
CREATE TABLE IF NOT EXISTS category_rules (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, rule_type TEXT NOT NULL, pattern TEXT NOT NULL,
  target_directory TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, priority INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_category_rules_priority ON category_rules(priority ASC);
CREATE TABLE IF NOT EXISTS download_presets (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, connections INTEGER NOT NULL,
  speed_limit INTEGER, completion_action TEXT, verify_checksum INTEGER NOT NULL DEFAULT 0,
  scheduled_at TEXT, is_builtin INTEGER NOT NULL DEFAULT 0
);
-- Task 19: URL 历史记录表（最多 20 条，LRU）。
-- url 唯一约束保证重复添加时仅更新 last_used；
-- last_used 为 Unix 毫秒时间戳，按降序排列即最近使用顺序。
CREATE TABLE IF NOT EXISTS url_history (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  url TEXT UNIQUE NOT NULL,
  last_used INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_url_history_last_used ON url_history(last_used DESC);
-- Task 20: 文件名清理规则表。pattern 为正则表达式，replacement 为替换字符串。
-- enabled=1 启用，priority 升序遍历。
CREATE TABLE IF NOT EXISTS filename_cleanup_rules (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, pattern TEXT NOT NULL, replacement TEXT NOT NULL DEFAULT '',
  enabled INTEGER NOT NULL DEFAULT 1, priority INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_filename_cleanup_rules_priority ON filename_cleanup_rules(priority ASC);
-- Task 25: 用户标签表与任务-标签关联表。
-- tags.name 在表内唯一；color 为 #RRGGBB 十六进制颜色字符串。
-- task_tags 为多对多关联表，主键为 (task_id, tag_id)；
-- task_tags.task_id 的外键约束为 ON DELETE CASCADE，保证任务被删除时自动清理关联。
-- task_tags.tag_id 的外键约束为 ON DELETE CASCADE，保证标签被删除时自动从所有任务中移除。
CREATE TABLE IF NOT EXISTS tags (
  id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, color TEXT NOT NULL DEFAULT '#3B82F6'
);
CREATE TABLE IF NOT EXISTS task_tags (
  task_id TEXT NOT NULL,
  tag_id TEXT NOT NULL,
  PRIMARY KEY(task_id, tag_id),
  FOREIGN KEY(task_id) REFERENCES tasks(id) ON DELETE CASCADE,
  FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_task_tags_tag_id ON task_tags(tag_id);
-- Task 36：任务模板表。`domain_pattern` 支持精确域名与 `*.example.com` 通配符。
-- 多字段以 JSON 文本存储以保持与现有协议头/预设一致的存储方式：
-- - headers_json：HashMap<String, String> 序列化，空对象 = "{}"，NULL = 不覆盖
-- - completion_action：CompletionAction 序列化（kebab-case），NULL = 不覆盖
-- - connections / speed_limit / destination：NULL = 不覆盖
-- enabled=1 启用；priority 升序遍历，数字越小越优先。
CREATE TABLE IF NOT EXISTS task_templates (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  domain_pattern TEXT NOT NULL,
  connections INTEGER,
  speed_limit INTEGER,
  headers_json TEXT,
  destination TEXT,
  completion_action TEXT,
  enabled INTEGER NOT NULL DEFAULT 1,
  priority INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_task_templates_priority ON task_templates(priority ASC);
-- Task 46: 媒体凭证表（按域名存储 Cookie/Referer/User-Agent）。
-- domain 为主键；cookie_encrypted 为 DPAPI 加密后的 base64 密文；
-- referer / user_agent 为明文（非机密辅助头）；
-- updated_at 为 ISO 8601 UTC 字符串，仅用于前端展示。
CREATE TABLE IF NOT EXISTS media_credentials (
  domain TEXT PRIMARY KEY,
  cookie_encrypted TEXT NOT NULL DEFAULT '',
  referer TEXT,
  user_agent TEXT,
  updated_at TEXT NOT NULL DEFAULT ''
);
-- Task 43: 平台命名模板表。每条模板绑定一个平台 key（小写英文，
-- 与 MediaPlatform::as_str() 对应）。同一平台可有多条模板，但匹配时
-- 只取第一条 enabled=1 的模板（按 id 升序）。
-- enabled=1 启用；is_builtin=1 为内置模板（可编辑/禁用，前端禁止删除）。
CREATE TABLE IF NOT EXISTS platform_naming_templates (
  id TEXT PRIMARY KEY,
  platform TEXT NOT NULL,
  template TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  is_builtin INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_platform_naming_templates_platform
  ON platform_naming_templates(platform ASC, enabled ASC, id ASC);
-- Task 44：平台兼容性矩阵。
-- platform 为主键，值为 MediaPlatform 序列化值（douyin/tiktok/twitter/
-- youtube/bilibili/weibo/unknown）。
-- level 为 SupportLevel 序列化值（verified/experimental/unsupported）。
-- notes 为前端展示用的中文说明（可空）。
-- known_issues_json 为 Vec<String> 的 JSON 序列化文本（默认 "[]"）。
-- last_tested_at 为 ISO 8601 UTC 字符串（可空）。
CREATE TABLE IF NOT EXISTS platform_compatibility (
  platform TEXT PRIMARY KEY,
  level TEXT NOT NULL DEFAULT 'experimental',
  notes TEXT NOT NULL DEFAULT '',
  known_issues_json TEXT NOT NULL DEFAULT '[]',
  last_tested_at TEXT NOT NULL DEFAULT ''
);
"#;

const UPSERT_TASK: &str = r#"
INSERT INTO tasks(id,url,file_name,destination,total_bytes,downloaded_bytes,speed,eta_seconds,status,error,created_at,completed_at,scheduled_at,category,queue_position,priority,retry_count,max_retries,checksum_sha256,expected_checksum,source,etag,last_modified,final_url,response_status,content_type,accepts_ranges,headers_json,media_json,per_task_speed_limit,collision_policy,connection_count,segments_json,completion_action,retry_policy_override,proxy_override,proxy_auth_json)
VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33,?34,?35,?36,?37)
ON CONFLICT(id) DO UPDATE SET url=excluded.url,file_name=excluded.file_name,destination=excluded.destination,total_bytes=excluded.total_bytes,downloaded_bytes=excluded.downloaded_bytes,speed=excluded.speed,eta_seconds=excluded.eta_seconds,status=excluded.status,error=excluded.error,completed_at=excluded.completed_at,scheduled_at=excluded.scheduled_at,category=excluded.category,queue_position=excluded.queue_position,priority=excluded.priority,retry_count=excluded.retry_count,max_retries=excluded.max_retries,checksum_sha256=excluded.checksum_sha256,expected_checksum=excluded.expected_checksum,source=excluded.source,etag=excluded.etag,last_modified=excluded.last_modified,final_url=excluded.final_url,response_status=excluded.response_status,content_type=excluded.content_type,accepts_ranges=excluded.accepts_ranges,headers_json=excluded.headers_json,media_json=excluded.media_json,per_task_speed_limit=excluded.per_task_speed_limit,collision_policy=excluded.collision_policy,connection_count=excluded.connection_count,segments_json=excluded.segments_json,completion_action=excluded.completion_action,retry_policy_override=excluded.retry_policy_override,proxy_override=excluded.proxy_override,proxy_auth_json=excluded.proxy_auth_json
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
            retry_policy_override: None,
            proxy_override: None,
            proxy_auth: None,
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

    #[test]
    fn download_preset_seeds_five_builtins_on_open() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let presets = store.download_preset_list().await.unwrap();
            let ids: Vec<&str> = presets.iter().map(|p| p.id.as_str()).collect();
            for expected in ["default", "lightweight", "large-file", "background", "night"] {
                assert!(ids.contains(&expected), "missing built-in preset {expected}");
            }
            let night = store.download_preset_get("night").await.unwrap().unwrap();
            assert!(night.is_builtin);
            assert_eq!(night.connections, 8);
            assert_eq!(night.scheduled_at.as_deref(), Some("22:00"));
            assert_eq!(night.completion_action, Some(CompletionAction::Shutdown));
            assert!(!night.verify_checksum);
            assert!(night.speed_limit.is_none());

            let bg = store.download_preset_get("background").await.unwrap().unwrap();
            assert_eq!(bg.speed_limit, Some(1_000_000));
            assert_eq!(bg.connections, 4);

            let large = store.download_preset_get("large-file").await.unwrap().unwrap();
            assert!(large.verify_checksum);
            assert_eq!(large.connections, 16);
        });
    }

    #[test]
    fn download_preset_seed_does_not_overwrite_user_edits() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 用户把 default 改成 16 连接
            let mut default = store.download_preset_get("default").await.unwrap().unwrap();
            default.connections = 16;
            store.download_preset_update(default.clone()).await.unwrap();
        });
        // 重新打开数据库，确认 INSERT OR IGNORE 没有覆盖用户的修改。
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let default = store.download_preset_get("default").await.unwrap().unwrap();
            assert_eq!(default.connections, 16);
        });
    }

    #[test]
    fn download_preset_add_update_delete_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let preset = DownloadPreset {
                id: "custom-1".into(),
                name: "我的预设".into(),
                connections: 4,
                speed_limit: Some(500_000),
                completion_action: Some(CompletionAction::OpenFolder),
                verify_checksum: true,
                scheduled_at: Some("23:30".into()),
                is_builtin: false,
            };
            store.download_preset_add(preset.clone()).await.unwrap();
            let restored = store.download_preset_get("custom-1").await.unwrap().unwrap();
            assert_eq!(restored, preset);

            let mut updated = restored.clone();
            updated.connections = 8;
            updated.name = "我的预设（已修改）".into();
            store.download_preset_update(updated.clone()).await.unwrap();
            let restored = store.download_preset_get("custom-1").await.unwrap().unwrap();
            assert_eq!(restored.connections, 8);
            assert_eq!(restored.name, "我的预设（已修改）");

            store.download_preset_delete("custom-1").await.unwrap();
            assert!(store.download_preset_get("custom-1").await.unwrap().is_none());
        });
    }

    // ===== Task 11：分类规则 CRUD + 迁移测试 =====

    fn sample_rule(id: &str, priority: i32, enabled: bool) -> CategoryRule {
        CategoryRule {
            id: id.into(),
            name: format!("规则-{id}"),
            rule_type: CategoryRuleType::Domain,
            pattern: "github.com".into(),
            target_directory: "D:\\Downloads\\GitHub".into(),
            enabled,
            priority,
        }
    }

    #[test]
    fn category_rule_crud_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 新增
            let rule = sample_rule("rule-1", 0, true);
            store.category_rule_add(rule.clone()).await.unwrap();

            // 列表
            let list = store.category_rule_list().await.unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list[0], rule);

            // 更新
            let mut updated = rule.clone();
            updated.name = "GitHub 仓库".into();
            updated.pattern = "api.github.com".into();
            updated.target_directory = "D:\\Downloads\\API".into();
            updated.priority = 5;
            updated.enabled = false;
            store.category_rule_update(updated.clone()).await.unwrap();

            // 读取校验
            let list = store.category_rule_list().await.unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].name, "GitHub 仓库");
            assert_eq!(list[0].pattern, "api.github.com");
            assert_eq!(list[0].target_directory, "D:\\Downloads\\API");
            assert_eq!(list[0].priority, 5);
            assert!(!list[0].enabled);

            // 删除
            store.category_rule_delete("rule-1").await.unwrap();
            assert!(store.category_rule_list().await.unwrap().is_empty());
        });
    }

    #[test]
    fn category_rule_list_returns_sorted_by_priority() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 故意按 3/1/2 顺序插入
            store.category_rule_add(sample_rule("r3", 30, true)).await.unwrap();
            store.category_rule_add(sample_rule("r1", 10, true)).await.unwrap();
            store.category_rule_add(sample_rule("r2", 20, true)).await.unwrap();

            let list = store.category_rule_list().await.unwrap();
            assert_eq!(list.len(), 3);
            assert_eq!(list[0].id, "r1");
            assert_eq!(list[1].id, "r2");
            assert_eq!(list[2].id, "r3");
        });
    }

    #[test]
    fn category_rule_update_nonexistent_returns_error() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let result = store.category_rule_update(sample_rule("ghost", 0, true)).await;
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("不存在"));
        });
    }

    #[test]
    fn category_rule_delete_nonexistent_returns_error() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let result = store.category_rule_delete("ghost").await;
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("不存在"));
        });
    }

    /// 增量迁移测试：模拟旧版本数据库（无 category_rules 表），
    /// 升级后新表应自动创建且可读写。
    #[test]
    fn legacy_database_upgraded_creates_category_rules_table() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("lumaget.db");
        // 模拟旧版本：手工创建一个只含 tasks 表的数据库
        {
            let connection = Connection::open(&database).unwrap();
            connection
                .execute_batch(
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
                )
                .unwrap();
            // 旧库中不应有 category_rules 表
            let has_table: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='category_rules')",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(!has_table, "测试前提：旧数据库不应有 category_rules 表");
        }

        // 用新版 Store::open 升级数据库
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 升级后表应存在且可读写
            let rule = sample_rule("legacy-1", 1, true);
            store.category_rule_add(rule.clone()).await.unwrap();
            let list = store.category_rule_list().await.unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].id, "legacy-1");
        });

        // 再次打开，确认数据持久化
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let list = store.category_rule_list().await.unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].id, "legacy-1");
        });
    }

    /// 验证 rule_type 在 DB 中以稳定英文存储，旧字符串值能正确反序列化。
    #[test]
    fn category_rule_rule_type_round_trips_through_db() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            for rule_type in [
                CategoryRuleType::Domain,
                CategoryRuleType::Mime,
                CategoryRuleType::Regex,
            ] {
                let id = format!("type-{:?}", rule_type);
                let rule = CategoryRule {
                    id: id.clone(),
                    name: id.clone(),
                    rule_type,
                    pattern: "x".into(),
                    target_directory: "D:\\DL".into(),
                    enabled: true,
                    priority: 0,
                };
                store.category_rule_add(rule).await.unwrap();
                let restored = store
                    .category_rule_list()
                    .await
                    .unwrap()
                    .into_iter()
                    .find(|r| r.id == id)
                    .unwrap();
                assert_eq!(restored.rule_type, rule_type);
            }
        });
    }

    /// 验证 enabled=0 的规则从 DB 读回时为 false。
    #[test]
    fn category_rule_disabled_flag_round_trips_as_false() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut rule = sample_rule("disabled-1", 0, false);
            store.category_rule_add(rule.clone()).await.unwrap();
            let restored = store
                .category_rule_list()
                .await
                .unwrap()
                .into_iter()
                .find(|r| r.id == rule.id)
                .unwrap();
            assert!(!restored.enabled);
            rule.enabled = true;
            store.category_rule_update(rule).await.unwrap();
            let restored = store
                .category_rule_list()
                .await
                .unwrap()
                .into_iter()
                .find(|r| r.id == "disabled-1")
                .unwrap();
            assert!(restored.enabled);
        });
    }

    // ===== Task 19：URL 历史 CRUD + LRU 测试 =====

    #[test]
    fn url_history_add_list_clear_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            store
                .url_history_add("https://example.com/a")
                .await
                .unwrap();
            store
                .url_history_add("https://example.com/b")
                .await
                .unwrap();

            let list = store.url_history_list().await.unwrap();
            assert_eq!(list.len(), 2);
            // 最近添加的应排在前面
            assert_eq!(list[0].url, "https://example.com/b");
            assert_eq!(list[1].url, "https://example.com/a");
            assert!(list[0].last_used >= list[1].last_used);

            store.url_history_clear().await.unwrap();
            let list = store.url_history_list().await.unwrap();
            assert!(list.is_empty());
        });
    }

    /// 同 URL 重复添加只更新 last_used，不会插入新记录。
    #[test]
    fn url_history_duplicate_add_updates_last_used_only() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            store
                .url_history_add("https://example.com/dup")
                .await
                .unwrap();
            // 让两次添加的 last_used 至少差 1ms
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            store
                .url_history_add("https://example.com/dup")
                .await
                .unwrap();

            let list = store.url_history_list().await.unwrap();
            assert_eq!(list.len(), 1, "重复 URL 不应新增记录");
            assert_eq!(list[0].url, "https://example.com/dup");
        });
    }

    /// 超过容量（20）时按 last_used 升序删除最旧记录，保持 LRU 语义。
    #[test]
    fn url_history_evicts_oldest_when_exceeding_capacity() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 依次插入 25 条；第一条应被淘汰
            for i in 0..25 {
                store
                    .url_history_add(&format!("https://example.com/{i}"))
                    .await
                    .unwrap();
                // 保证 last_used 单调递增
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
            let list = store.url_history_list().await.unwrap();
            assert_eq!(list.len(), 20, "容量应保持在 20 条");
            // 最旧的 5 条（0..5）应已被淘汰
            for i in 0..5 {
                let url = format!("https://example.com/{i}");
                assert!(
                    !list.iter().any(|entry| entry.url == url),
                    "最旧记录 {url} 应被淘汰"
                );
            }
            // 最新添加的应排在最前
            assert_eq!(list[0].url, "https://example.com/24");
            // 第 5 条（最早保留）应排在末尾
            assert_eq!(list.last().unwrap().url, "https://example.com/5");
        });
    }

    /// 重复添加旧 URL 会把它"提升"到最新位置，从而避免被淘汰。
    #[test]
    fn url_history_readding_old_url_promotes_it_to_most_recent() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 先添加 20 条填满容量
            for i in 0..20 {
                store
                    .url_history_add(&format!("https://example.com/{i}"))
                    .await
                    .unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
            // 提升第 0 条到最新
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            store
                .url_history_add("https://example.com/0")
                .await
                .unwrap();
            // 再添加一条新 URL，应淘汰当前最旧（即原第 1 条）
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            store
                .url_history_add("https://example.com/new")
                .await
                .unwrap();

            let list = store.url_history_list().await.unwrap();
            assert_eq!(list.len(), 20);
            assert_eq!(list[0].url, "https://example.com/new");
            assert_eq!(list[1].url, "https://example.com/0");
            // 第 1 条应被淘汰，第 0 条应保留
            assert!(
                !list.iter().any(|e| e.url == "https://example.com/1"),
                "最旧未被提升的第 1 条应被淘汰"
            );
            assert!(
                list.iter().any(|e| e.url == "https://example.com/0"),
                "被重新访问的第 0 条应保留"
            );
        });
    }

    /// 空字符串 URL 应返回中文错误，不应写入数据库。
    #[test]
    fn url_history_add_rejects_empty_url() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let result = store.url_history_add("   ").await;
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("URL"));
            let list = store.url_history_list().await.unwrap();
            assert!(list.is_empty());
        });
    }

    /// URL 历史表在旧版本数据库（无 url_history 表）上应自动创建，且数据可持久化。
    #[test]
    fn legacy_database_upgraded_creates_url_history_table() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("lumaget.db");
        // 模拟旧版本：手工创建只含 tasks 表的数据库
        {
            let connection = Connection::open(&database).unwrap();
            connection
                .execute_batch(
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
                )
                .unwrap();
        }
        // 用新版 Store::open 升级数据库
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            store
                .url_history_add("https://example.com/legacy")
                .await
                .unwrap();
            let list = store.url_history_list().await.unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].url, "https://example.com/legacy");
        });
        // 再次打开，确认数据持久化
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let list = store.url_history_list().await.unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].url, "https://example.com/legacy");
        });
    }

    /// url_history_list 在空表上应安全返回空 Vec，不报错。
    #[test]
    fn url_history_list_on_empty_table_returns_empty_vec() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let list = store.url_history_list().await.unwrap();
            assert!(list.is_empty());
        });
    }

    /// url_history_clear 在空表上应是幂等操作。
    #[test]
    fn url_history_clear_on_empty_table_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            store.url_history_clear().await.unwrap();
            store.url_history_clear().await.unwrap();
            let list = store.url_history_list().await.unwrap();
            assert!(list.is_empty());
        });
    }

    // ===== Task 20：文件名清理规则 CRUD + 迁移测试 =====

    fn sample_cleanup_rule(id: &str, priority: i32, enabled: bool) -> FilenameCleanupRule {
        FilenameCleanupRule {
            id: id.into(),
            name: format!("清理规则-{id}"),
            pattern: r"\(\d{3,4}[pP]\)".into(),
            replacement: String::new(),
            enabled,
            priority,
        }
    }

    #[test]
    fn filename_cleanup_rule_seeds_four_builtins_on_open() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let list = store.filename_cleanup_rule_list().await.unwrap();
            let ids: Vec<&str> = list.iter().map(|r| r.id.as_str()).collect();
            for expected in [
                "remove-bracket-site",
                "remove-paren-quality",
                "remove-underscore-site",
                "collapse-spaces",
            ] {
                assert!(ids.contains(&expected), "missing built-in rule {expected}");
            }
            // 验证 priority 与名称符合 spec Task 20.2
            let bracket = list.iter().find(|r| r.id == "remove-bracket-site").unwrap();
            assert_eq!(bracket.priority, 10);
            assert!(bracket.enabled);
            let paren = list.iter().find(|r| r.id == "remove-paren-quality").unwrap();
            assert_eq!(paren.priority, 20);
            let underscore = list.iter().find(|r| r.id == "remove-underscore-site").unwrap();
            assert_eq!(underscore.priority, 30);
            let collapse = list.iter().find(|r| r.id == "collapse-spaces").unwrap();
            assert_eq!(collapse.priority, 40);
            assert_eq!(collapse.replacement, " ");
        });
    }

    #[test]
    fn filename_cleanup_rule_seed_does_not_overwrite_user_edits() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 用户把 collapse-spaces 改成空替换
            let mut rule = store
                .filename_cleanup_rule_list()
                .await
                .unwrap()
                .into_iter()
                .find(|r| r.id == "collapse-spaces")
                .unwrap();
            rule.replacement = "_".into();
            rule.enabled = false;
            store.filename_cleanup_rule_update(rule).await.unwrap();
        });
        // 重新打开数据库，确认 INSERT OR IGNORE 没有覆盖用户的修改。
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let rule = store
                .filename_cleanup_rule_list()
                .await
                .unwrap()
                .into_iter()
                .find(|r| r.id == "collapse-spaces")
                .unwrap();
            assert_eq!(rule.replacement, "_");
            assert!(!rule.enabled);
        });
    }

    #[test]
    fn filename_cleanup_rule_crud_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 新增
            let rule = sample_cleanup_rule("custom-1", 100, true);
            store
                .filename_cleanup_rule_add(rule.clone())
                .await
                .unwrap();

            // 读取校验（含 4 个内置规则，共 5 条）
            let list = store.filename_cleanup_rule_list().await.unwrap();
            assert!(list.iter().any(|r| r.id == "custom-1"));
            let restored = list.iter().find(|r| r.id == "custom-1").unwrap();
            assert_eq!(restored.name, rule.name);
            assert_eq!(restored.pattern, rule.pattern);
            assert_eq!(restored.replacement, rule.replacement);
            assert_eq!(restored.priority, 100);
            assert!(restored.enabled);

            // 更新
            let mut updated = restored.clone();
            updated.name = "已修改".into();
            updated.pattern = r"\s+".into();
            updated.replacement = "_".into();
            updated.priority = 200;
            updated.enabled = false;
            store
                .filename_cleanup_rule_update(updated.clone())
                .await
                .unwrap();

            let restored = store
                .filename_cleanup_rule_list()
                .await
                .unwrap()
                .into_iter()
                .find(|r| r.id == "custom-1")
                .unwrap();
            assert_eq!(restored.name, "已修改");
            assert_eq!(restored.pattern, r"\s+");
            assert_eq!(restored.replacement, "_");
            assert_eq!(restored.priority, 200);
            assert!(!restored.enabled);

            // 删除
            store
                .filename_cleanup_rule_delete("custom-1")
                .await
                .unwrap();
            let list = store.filename_cleanup_rule_list().await.unwrap();
            assert!(!list.iter().any(|r| r.id == "custom-1"));
            // 内置规则不应被影响
            assert!(list.iter().any(|r| r.id == "collapse-spaces"));
        });
    }

    #[test]
    fn filename_cleanup_rule_list_returns_sorted_by_priority() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 内置规则优先，再插入 priority 25/15/35
            store
                .filename_cleanup_rule_add(sample_cleanup_rule("r25", 25, true))
                .await
                .unwrap();
            store
                .filename_cleanup_rule_add(sample_cleanup_rule("r15", 15, true))
                .await
                .unwrap();
            store
                .filename_cleanup_rule_add(sample_cleanup_rule("r35", 35, true))
                .await
                .unwrap();

            let list = store.filename_cleanup_rule_list().await.unwrap();
            let priorities: Vec<i32> = list.iter().map(|r| r.priority).collect();
            let mut sorted = priorities.clone();
            sorted.sort();
            assert_eq!(priorities, sorted);
            assert!(priorities.contains(&15));
            assert!(priorities.contains(&25));
            assert!(priorities.contains(&35));
        });
    }

    #[test]
    fn filename_cleanup_rule_update_nonexistent_returns_error() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let result = store
                .filename_cleanup_rule_update(sample_cleanup_rule("ghost", 0, true))
                .await;
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("不存在"));
        });
    }

    #[test]
    fn filename_cleanup_rule_delete_nonexistent_returns_error() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let result = store.filename_cleanup_rule_delete("ghost").await;
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("不存在"));
        });
    }

    /// 增量迁移测试：模拟旧版本数据库（无 filename_cleanup_rules 表），
    /// 升级后新表应自动创建并 seed 内置规则。
    #[test]
    fn legacy_database_upgraded_creates_filename_cleanup_rules_table() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("lumaget.db");
        // 模拟旧版本：手工创建只含 tasks 表的数据库
        {
            let connection = Connection::open(&database).unwrap();
            connection
                .execute_batch(
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
                )
                .unwrap();
            // 旧库中不应有 filename_cleanup_rules 表
            let has_table: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='filename_cleanup_rules')",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(!has_table, "测试前提：旧数据库不应有 filename_cleanup_rules 表");
        }

        // 用新版 Store::open 升级数据库
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 升级后表应存在且内置规则已 seed
            let list = store.filename_cleanup_rule_list().await.unwrap();
            assert_eq!(list.len(), 11);
            // 用户也可新增自定义规则
            store
                .filename_cleanup_rule_add(sample_cleanup_rule("legacy-1", 100, true))
                .await
                .unwrap();
            let list = store.filename_cleanup_rule_list().await.unwrap();
            assert_eq!(list.len(), 12);
            assert!(list.iter().any(|r| r.id == "legacy-1"));
        });

        // 再次打开，确认数据持久化
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let list = store.filename_cleanup_rule_list().await.unwrap();
            assert_eq!(list.len(), 12);
            assert!(list.iter().any(|r| r.id == "legacy-1"));
            // 内置规则不应被重复插入
            let builtin_count = list
                .iter()
                .filter(|r| r.id.starts_with("remove-") || r.id == "collapse-spaces" || r.id == "strip-trailing-spaces")
                .count();
            assert_eq!(builtin_count, 11);
        });
    }

    /// 验证 enabled=0 的规则从 DB 读回时为 false，并可重新启用。
    #[test]
    fn filename_cleanup_rule_disabled_flag_round_trips_as_false() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut rule = sample_cleanup_rule("disabled-1", 100, false);
            store
                .filename_cleanup_rule_add(rule.clone())
                .await
                .unwrap();
            let restored = store
                .filename_cleanup_rule_list()
                .await
                .unwrap()
                .into_iter()
                .find(|r| r.id == rule.id)
                .unwrap();
            assert!(!restored.enabled);
            rule.enabled = true;
            store
                .filename_cleanup_rule_update(rule)
                .await
                .unwrap();
            let restored = store
                .filename_cleanup_rule_list()
                .await
                .unwrap()
                .into_iter()
                .find(|r| r.id == "disabled-1")
                .unwrap();
            assert!(restored.enabled);
        });
    }

    // ===== Task 25：标签 CRUD + task_tags 关联 + 级联清理 + 增量迁移 =====

    fn sample_tag(id: &str, name: &str, color: &str) -> Tag {
        Tag {
            id: id.into(),
            name: name.into(),
            color: color.into(),
        }
    }

    #[test]
    fn tag_add_list_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            store
                .tag_add(sample_tag("t1", "工作", "#3B82F6"))
                .await
                .unwrap();
            store
                .tag_add(sample_tag("t2", "个人", "#10B981"))
                .await
                .unwrap();
            let list = store.tag_list().await.unwrap();
            assert_eq!(list.len(), 2);
            // 按 name 升序：个人 < 工作
            assert_eq!(list[0].id, "t2");
            assert_eq!(list[0].name, "个人");
            assert_eq!(list[0].color, "#10B981");
            assert_eq!(list[1].id, "t1");
            assert_eq!(list[1].name, "工作");
        });
    }

    #[test]
    fn tag_add_duplicate_name_returns_chinese_error() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            store
                .tag_add(sample_tag("t1", "工作", "#3B82F6"))
                .await
                .unwrap();
            let result = store.tag_add(sample_tag("t2", "工作", "#EF4444")).await;
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.contains("已存在"), "错误信息应包含中文提示：{err}");
            // 第二个标签不应被插入
            assert_eq!(store.tag_list().await.unwrap().len(), 1);
        });
    }

    #[test]
    fn tag_update_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            store
                .tag_add(sample_tag("t1", "工作", "#3B82F6"))
                .await
                .unwrap();
            store
                .tag_update(sample_tag("t1", "重要工作", "#EF4444"))
                .await
                .unwrap();
            let list = store.tag_list().await.unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].name, "重要工作");
            assert_eq!(list[0].color, "#EF4444");
        });
    }

    #[test]
    fn tag_update_nonexistent_returns_error() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let result = store
                .tag_update(sample_tag("missing", "测试", "#000000"))
                .await;
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("不存在"));
        });
    }

    #[test]
    fn tag_update_to_duplicate_name_returns_error() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            store
                .tag_add(sample_tag("t1", "工作", "#3B82F6"))
                .await
                .unwrap();
            store
                .tag_add(sample_tag("t2", "个人", "#10B981"))
                .await
                .unwrap();
            // 尝试把 t2 改名为已存在的 "工作"
            let result = store
                .tag_update(sample_tag("t2", "工作", "#10B981"))
                .await;
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("已存在"));
        });
    }

    #[test]
    fn tag_delete_nonexistent_returns_error() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let result = store.tag_delete("missing").await;
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("不存在"));
        });
    }

    #[test]
    fn task_tags_set_and_get_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut task = test_task(&directory.path().to_path_buf());
            task.id = "task-1".into();
            store.upsert_task(&task).await.unwrap();
            store
                .tag_add(sample_tag("t1", "工作", "#3B82F6"))
                .await
                .unwrap();
            store
                .tag_add(sample_tag("t2", "重要", "#EF4444"))
                .await
                .unwrap();

            // 初始为空
            let empty = store.task_tags_get("task-1").await.unwrap();
            assert!(empty.is_empty());

            // 设置两个标签
            store
                .task_tags_set("task-1", vec!["t1".into(), "t2".into()])
                .await
                .unwrap();
            let tags = store.task_tags_get("task-1").await.unwrap();
            assert_eq!(tags.len(), 2);
            // 按 name 升序：工作 < 重要
            assert_eq!(tags[0].id, "t1");
            assert_eq!(tags[1].id, "t2");

            // 替换为只剩一个
            store
                .task_tags_set("task-1", vec!["t2".into()])
                .await
                .unwrap();
            let tags = store.task_tags_get("task-1").await.unwrap();
            assert_eq!(tags.len(), 1);
            assert_eq!(tags[0].id, "t2");

            // 清空
            store.task_tags_set("task-1", vec![]).await.unwrap();
            let tags = store.task_tags_get("task-1").await.unwrap();
            assert!(tags.is_empty());
        });
    }

    #[test]
    fn task_tags_set_with_unknown_tag_returns_error() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut task = test_task(&directory.path().to_path_buf());
            task.id = "task-1".into();
            store.upsert_task(&task).await.unwrap();
            // 引用不存在的 tag_id 应失败
            let result = store
                .task_tags_set("task-1", vec!["unknown-tag".into()])
                .await;
            assert!(result.is_err());
            // 失败时不应留下任何关联
            let tags = store.task_tags_get("task-1").await.unwrap();
            assert!(tags.is_empty());
        });
    }

    #[test]
    fn task_tags_list_all_groups_by_task() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut task_a = test_task(&directory.path().to_path_buf());
            task_a.id = "task-a".into();
            let mut task_b = test_task(&directory.path().to_path_buf());
            task_b.id = "task-b".into();
            store.upsert_task(&task_a).await.unwrap();
            store.upsert_task(&task_b).await.unwrap();
            store
                .tag_add(sample_tag("t1", "工作", "#3B82F6"))
                .await
                .unwrap();
            store
                .tag_add(sample_tag("t2", "重要", "#EF4444"))
                .await
                .unwrap();
            store
                .task_tags_set("task-a", vec!["t1".into()])
                .await
                .unwrap();
            store
                .task_tags_set("task-b", vec!["t1".into(), "t2".into()])
                .await
                .unwrap();

            let map = store.task_tags_list_all().await.unwrap();
            assert_eq!(map.len(), 2);
            assert_eq!(map.get("task-a").unwrap().len(), 1);
            assert_eq!(map.get("task-b").unwrap().len(), 2);
        });
    }

    #[test]
    fn deleting_tag_cascades_to_task_tags() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut task = test_task(&directory.path().to_path_buf());
            task.id = "task-1".into();
            store.upsert_task(&task).await.unwrap();
            store
                .tag_add(sample_tag("t1", "工作", "#3B82F6"))
                .await
                .unwrap();
            store
                .tag_add(sample_tag("t2", "重要", "#EF4444"))
                .await
                .unwrap();
            store
                .task_tags_set("task-1", vec!["t1".into(), "t2".into()])
                .await
                .unwrap();
            assert_eq!(store.task_tags_get("task-1").await.unwrap().len(), 2);

            // 删除 t1，task-1 应自动只剩 t2 关联
            store.tag_delete("t1").await.unwrap();
            let tags = store.task_tags_get("task-1").await.unwrap();
            assert_eq!(tags.len(), 1);
            assert_eq!(tags[0].id, "t2");

            // task_tags_list_all 也不应包含已删除的 tag
            let map = store.task_tags_list_all().await.unwrap();
            assert_eq!(map.get("task-1").unwrap().len(), 1);
        });
    }

    #[test]
    fn deleting_task_cascades_to_task_tags() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut task = test_task(&directory.path().to_path_buf());
            task.id = "task-1".into();
            store.upsert_task(&task).await.unwrap();
            store
                .tag_add(sample_tag("t1", "工作", "#3B82F6"))
                .await
                .unwrap();
            store
                .task_tags_set("task-1", vec!["t1".into()])
                .await
                .unwrap();
            assert_eq!(store.task_tags_get("task-1").await.unwrap().len(), 1);

            // 删除任务，task_tags 中相关行应被级联删除
            store.remove_task("task-1").await.unwrap();
            let map = store.task_tags_list_all().await.unwrap();
            assert!(!map.contains_key("task-1"));
            // 标签本身不应被删除
            assert_eq!(store.tag_list().await.unwrap().len(), 1);
        });
    }

    #[test]
    fn legacy_database_upgraded_creates_tags_and_task_tags_tables() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("lumaget.db");
        // 模拟旧版本：手工创建只含 tasks 表的数据库
        {
            let connection = Connection::open(&database).unwrap();
            connection
                .execute_batch(
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
                )
                .unwrap();
        }
        // 用新版 Store::open 升级数据库
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // tags 表已创建，可正常 CRUD
            store
                .tag_add(sample_tag("t1", "工作", "#3B82F6"))
                .await
                .unwrap();
            let list = store.tag_list().await.unwrap();
            assert_eq!(list.len(), 1);

            // task_tags 表已创建，可正常关联
            let mut task = test_task(&directory.path().to_path_buf());
            task.id = "legacy-task".into();
            store.upsert_task(&task).await.unwrap();
            store
                .task_tags_set("legacy-task", vec!["t1".into()])
                .await
                .unwrap();
            let tags = store.task_tags_get("legacy-task").await.unwrap();
            assert_eq!(tags.len(), 1);
        });
        // 再次打开，确认数据持久化
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let list = store.tag_list().await.unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].name, "工作");
        });
    }

    #[test]
    fn tag_list_on_empty_table_returns_empty_vec() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let list = store.tag_list().await.unwrap();
            assert!(list.is_empty());
        });
    }

    #[test]
    fn task_tags_get_on_task_with_no_tags_returns_empty_vec() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut task = test_task(&directory.path().to_path_buf());
            task.id = "task-no-tags".into();
            store.upsert_task(&task).await.unwrap();
            let tags = store.task_tags_get("task-no-tags").await.unwrap();
            assert!(tags.is_empty());
        });
    }

    // ===== Task 46: 媒体凭证 CRUD 与加密存储测试 =====

    fn sample_credential(domain: &str, cookie: &str) -> MediaCredential {
        MediaCredential {
            domain: domain.into(),
            cookie: cookie.into(),
            referer: Some("https://example.com/".into()),
            user_agent: Some("Mozilla/5.0 Test".into()),
            updated_at: "2026-07-20T10:00:00Z".into(),
        }
    }

    #[test]
    fn media_credential_upsert_and_get_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let credential = sample_credential("example.com", "session=abc123; theme=dark");
            store.media_credential_upsert(credential).await.unwrap();
            let fetched = store
                .media_credential_get("example.com")
                .await
                .unwrap()
                .expect("credential should exist");
            assert_eq!(fetched.domain, "example.com");
            assert_eq!(fetched.cookie, "session=abc123; theme=dark");
            assert_eq!(fetched.referer.as_deref(), Some("https://example.com/"));
            assert_eq!(fetched.user_agent.as_deref(), Some("Mozilla/5.0 Test"));
            assert_eq!(fetched.updated_at, "2026-07-20T10:00:00Z");
        });
    }

    #[test]
    fn media_credential_upsert_overwrites_existing() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            store
                .media_credential_upsert(sample_credential("example.com", "old-cookie"))
                .await
                .unwrap();
            let mut updated = sample_credential("example.com", "new-cookie");
            updated.updated_at = "2026-07-21T10:00:00Z".into();
            store.media_credential_upsert(updated).await.unwrap();
            let fetched = store
                .media_credential_get("example.com")
                .await
                .unwrap()
                .expect("credential should exist");
            assert_eq!(fetched.cookie, "new-cookie");
            assert_eq!(fetched.updated_at, "2026-07-21T10:00:00Z");
        });
    }

    #[test]
    fn media_credential_get_nonexistent_returns_none() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let result = store.media_credential_get("missing.example.com").await;
            assert!(result.is_ok());
            assert!(result.unwrap().is_none());
        });
    }

    #[test]
    fn media_credential_delete_removes_record() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            store
                .media_credential_upsert(sample_credential("example.com", "session=abc"))
                .await
                .unwrap();
            store.media_credential_delete("example.com").await.unwrap();
            let result = store.media_credential_get("example.com").await.unwrap();
            assert!(result.is_none());
        });
    }

    #[test]
    fn media_credential_delete_nonexistent_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 删除不存在的凭证不应报错（幂等）
            let result = store.media_credential_delete("missing.example.com").await;
            assert!(result.is_ok());
        });
    }

    #[test]
    fn media_credential_list_returns_all_sorted_by_domain() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            store
                .media_credential_upsert(sample_credential("zeta.example.com", "z=1"))
                .await
                .unwrap();
            store
                .media_credential_upsert(sample_credential("alpha.example.com", "a=1"))
                .await
                .unwrap();
            store
                .media_credential_upsert(sample_credential("mid.example.com", "m=1"))
                .await
                .unwrap();
            let list = store.media_credential_list().await.unwrap();
            assert_eq!(list.len(), 3);
            // 按 domain 升序排列
            assert_eq!(list[0].domain, "alpha.example.com");
            assert_eq!(list[1].domain, "mid.example.com");
            assert_eq!(list[2].domain, "zeta.example.com");
            // cookie 应被正确解密
            assert_eq!(list[0].cookie, "a=1");
        });
    }

    #[test]
    fn media_credential_list_empty_when_no_records() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let list = store.media_credential_list().await.unwrap();
            assert!(list.is_empty());
        });
    }

    // ===== Task 44: 平台兼容性矩阵 CRUD 测试 =====

    /// Task 44：内置 6 条默认记录在打开数据库时已写入。
    ///
    /// 验证：
    /// - YouTube / 哔哩哔哩 = Verified
    /// - 抖音 / TikTok / Twitter / 微博 = Experimental
    /// - 列表长度恰好为 6（按 platform 升序）
    #[test]
    fn platform_compatibility_list_seeds_builtin_records() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let list = store.platform_compatibility_list().await.unwrap();
            assert_eq!(list.len(), 6, "expected 6 builtin records, got {}", list.len());
            // 按 platform 升序：bilibili / douyin / tiktok / twitter / weibo / youtube
            assert_eq!(list[0].platform, "bilibili");
            assert_eq!(list[0].level, SupportLevel::Verified);
            assert_eq!(list[1].platform, "douyin");
            assert_eq!(list[1].level, SupportLevel::Verified);
            assert_eq!(list[2].platform, "tiktok");
            assert_eq!(list[2].level, SupportLevel::Experimental);
            assert_eq!(list[3].platform, "twitter");
            assert_eq!(list[3].level, SupportLevel::Experimental);
            assert_eq!(list[4].platform, "weibo");
            assert_eq!(list[4].level, SupportLevel::Experimental);
            assert_eq!(list[5].platform, "youtube");
            assert_eq!(list[5].level, SupportLevel::Experimental);
            // notes 不为空（含中文说明）
            assert!(!list[0].notes.is_empty());
            assert!(!list[5].notes.is_empty());
            // known_issues 默认为空数组
            assert!(list[0].known_issues.is_empty());
        });
    }

    /// Task 44：按 platform 查询单条记录。
    #[test]
    fn platform_compatibility_get_returns_record() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let youtube = store
                .platform_compatibility_get("youtube")
                .await
                .unwrap()
                .expect("youtube record should exist");
            assert_eq!(youtube.platform, "youtube");
            assert_eq!(youtube.level, SupportLevel::Experimental);
            assert!(youtube.notes.contains("YouTube"));
            assert!(youtube.known_issues.is_empty());

            let douyin = store
                .platform_compatibility_get("douyin")
                .await
                .unwrap()
                .expect("douyin record should exist");
            assert_eq!(douyin.platform, "douyin");
            assert_eq!(douyin.level, SupportLevel::Verified);
        });
    }

    /// Task 44：不存在的 platform 返回 None（不视为错误）。
    #[test]
    fn platform_compatibility_get_nonexistent_returns_none() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let result = store.platform_compatibility_get("nonexistent").await.unwrap();
            assert!(result.is_none());
        });
    }

    /// Task 44：重复打开数据库不会覆盖用户修改（INSERT OR IGNORE 语义）。
    ///
    /// 模拟用户修改 youtube 的 level 后重新打开数据库，
    /// 验证用户改动被保留、未被内置默认覆盖。
    #[test]
    fn platform_compatibility_seed_does_not_overwrite_user_changes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_path_buf();
        {
            let store = Store::open(path.clone()).unwrap();
            let connection = store.connection.blocking_lock();
            // 模拟用户修改：将 youtube 的 level 改为 unsupported 并设置自定义说明
            connection
                .execute(
                    "UPDATE platform_compatibility SET level='unsupported', notes='用户自定义说明' WHERE platform='youtube'",
                    [],
                )
                .unwrap();
        }
        // 重新打开数据库（再次执行 seed）
        let store = Store::open(path).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let youtube = store
                .platform_compatibility_get("youtube")
                .await
                .unwrap()
                .expect("youtube record should exist");
            // 用户修改被保留，未被覆盖为 verified
            assert_eq!(youtube.level, SupportLevel::Unsupported);
            // 其它记录仍为默认值
            let bilibili = store
                .platform_compatibility_get("bilibili")
                .await
                .unwrap()
                .expect("bilibili record should exist");
            assert_eq!(bilibili.level, SupportLevel::Verified);
        });
    }

    #[test]
    fn platform_compatibility_seed_updates_old_douyin_notes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_path_buf();
        {
            let store = Store::open(path.clone()).unwrap();
            let connection = store.connection.blocking_lock();
            // 模拟旧版本数据库：设置为旧抖音文案
            connection
                .execute(
                    "UPDATE platform_compatibility SET notes='抖音短链、图集、单视频全功能正常支持' WHERE platform='douyin'",
                    [],
                )
                .unwrap();
        }
        // 重新打开数据库（执行增量升级 seed）
        let store = Store::open(path).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let douyin = store
                .platform_compatibility_get("douyin")
                .await
                .unwrap()
                .expect("douyin record should exist");
            assert_eq!(
                douyin.notes,
                "抖音短链、图集、单视频及实时直播流全功能正常支持"
            );
        });
    }

    /// Task 44：损坏的 known_issues_json 不阻塞列表返回，降级为空数组。
    #[test]
    fn platform_compatibility_list_handles_corrupt_json() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        // 直接写入损坏的 JSON 文本
        {
            let connection = store.connection.blocking_lock();
            connection
                .execute(
                    "UPDATE platform_compatibility SET known_issues_json='not-a-json-array' \
                     WHERE platform='youtube'",
                    [],
                )
                .unwrap();
        }
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let list = store.platform_compatibility_list().await.unwrap();
            assert_eq!(list.len(), 6);
            // 损坏 JSON 降级为空数组，不报错
            let youtube = list.iter().find(|r| r.platform == "youtube").unwrap();
            assert!(youtube.known_issues.is_empty());
        });
    }

    /// Task 44：未知的 level 值默认为 Experimental（向后兼容）。
    #[test]
    fn platform_compatibility_unknown_level_defaults_to_experimental() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        {
            let connection = store.connection.blocking_lock();
            connection
                .execute(
                    "UPDATE platform_compatibility SET level='unknown-value' \
                     WHERE platform='youtube'",
                    [],
                )
                .unwrap();
        }
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let youtube = store
                .platform_compatibility_get("youtube")
                .await
                .unwrap()
                .expect("youtube record should exist");
            assert_eq!(youtube.level, SupportLevel::Experimental);
        });
    }

    #[test]
    fn media_credential_stores_encrypted_cookie_in_database() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let plain_cookie = "session=secret-value-12345";
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            store
                .media_credential_upsert(sample_credential("example.com", plain_cookie))
                .await
                .unwrap();
        });
        // 直接查询底层数据库列，绕过 store 的解密
        let connection = store.connection.blocking_lock();
        let stored: String = connection
            .query_row(
                "SELECT cookie_encrypted FROM media_credentials WHERE domain=?1",
                ["example.com"],
                |r| r.get(0),
            )
            .unwrap();
        // 密文不得等于明文
        assert_ne!(stored, plain_cookie);
        // 密文非空
        assert!(!stored.is_empty());
        // Windows 平台 DPAPI 密文是 base64；非 Windows 平台回退为 base64(plain)
        // 两种情况下密文都不应等于明文
    }

    #[test]
    fn media_credential_get_matching_supports_platform_aliases() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            store
                .media_credential_upsert(sample_credential("youtube.com", "LOGIN_INFO=yt_test_cookie"))
                .await
                .unwrap();
            let matched = store
                .media_credential_get_matching("youtu.be")
                .await
                .unwrap()
                .expect("youtu.be should match youtube.com credential");
            assert_eq!(matched.cookie, "LOGIN_INFO=yt_test_cookie");
        });
    }

    /// Task 46：解密失败（密文损坏）时返回中文错误信息。
    ///
    /// 直接向数据库写入无效密文，验证 `media_credential_get` 返回中文错误。
    #[test]
    fn media_credential_get_returns_chinese_error_on_decrypt_failure() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 直接写入无效密文（不是合法的 base64 或不是 DPAPI 密文）
            {
                let connection = store.connection.lock().await;
                connection
                    .execute(
                        "INSERT INTO media_credentials(domain,cookie_encrypted,referer,user_agent,updated_at) \
                         VALUES(?1,?2,?3,?4,?5)",
                        params!["broken.example.com", "not-a-valid-cipher", "https://example.com/", "UA", "2026-07-20"],
                    )
                    .unwrap();
            }
            let result = store.media_credential_get("broken.example.com").await;
            assert!(result.is_err());
            let error = result.unwrap_err();
            assert!(
                error.contains("凭证解密失败"),
                "expected Chinese decrypt-failure message, got: {error}"
            );
        });
    }

    /// Task 46：解密失败的行在 list 中被跳过，不阻塞其它行返回。
    #[test]
    fn media_credential_list_skips_rows_with_corrupt_ciphertext() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // 写入一条正常凭证
            store
                .media_credential_upsert(sample_credential("good.example.com", "valid=1"))
                .await
                .unwrap();
            // 直接写入一条损坏凭证
            {
                let connection = store.connection.lock().await;
                connection
                    .execute(
                        "INSERT INTO media_credentials(domain,cookie_encrypted,referer,user_agent,updated_at) \
                         VALUES(?1,?2,?3,?4,?5)",
                        params!["bad.example.com", "not-a-valid-cipher", "", "", "2026-07-20"],
                    )
                    .unwrap();
            }
            let list = store.media_credential_list().await.unwrap();
            // 只返回能解密的那条
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].domain, "good.example.com");
            assert_eq!(list[0].cookie, "valid=1");
        });
    }

    /// Task 46：空 Cookie 字符串是合法的（用于"清除 Cookie 但保留域名条目"场景）。
    #[test]
    fn media_credential_upsert_empty_cookie_is_allowed() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut credential = sample_credential("example.com", "");
            credential.cookie = String::new();
            store.media_credential_upsert(credential).await.unwrap();
            let fetched = store
                .media_credential_get("example.com")
                .await
                .unwrap()
                .expect("credential should exist");
            assert_eq!(fetched.cookie, "");
        });
    }

    /// Task 46：可选字段 referer / user_agent 可为 None。
    #[test]
    fn media_credential_upsert_without_referer_or_user_agent() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().to_path_buf()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let credential = MediaCredential {
                domain: "example.com".into(),
                cookie: "session=abc".into(),
                referer: None,
                user_agent: None,
                updated_at: "2026-07-20T10:00:00Z".into(),
            };
            store.media_credential_upsert(credential).await.unwrap();
            let fetched = store
                .media_credential_get("example.com")
                .await
                .unwrap()
                .expect("credential should exist");
            assert_eq!(fetched.cookie, "session=abc");
            assert!(fetched.referer.is_none());
            assert!(fetched.user_agent.is_none());
        });
    }
}
