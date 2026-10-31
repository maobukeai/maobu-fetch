//! 能力说明与验证证据分开存储。未验证不能因内置默认值变为 Verified。
use super::Store;
use crate::models::{PlatformCompatibility, PlatformVerificationEvidence, SupportLevel};
use rusqlite::{Connection, OptionalExtension, Row};

mod builtins;
mod evidence;
#[cfg(test)]
mod tests;

pub(super) fn migrate_and_seed(connection: &Connection) -> Result<(), String> {
    let exists = connection
        .prepare("PRAGMA table_info(platform_compatibility)")
        .map_err(|e| e.to_string())?
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?
        .iter()
        .any(|name| name == "verification_json");
    if !exists {
        connection
            .execute(
                "ALTER TABLE platform_compatibility ADD COLUMN verification_json TEXT",
                [],
            )
            .map_err(|e| format!("升级兼容性验证记录失败：{e}"))?;
    }
    builtins::seed(connection)
}

impl Store {
    pub async fn platform_compatibility_list(&self) -> Result<Vec<PlatformCompatibility>, String> {
        let connection = self.connection.lock().await;
        let mut statement = connection.prepare(
            "SELECT platform,level,notes,known_issues_json,last_tested_at,verification_json FROM platform_compatibility ORDER BY platform ASC"
        ).map_err(|e| format!("读取兼容性说明失败：{e}"))?;
        let rows = statement
            .query_map([], from_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("读取兼容性记录失败：{e}"))
    }

    pub async fn platform_compatibility_get(
        &self,
        platform: &str,
    ) -> Result<Option<PlatformCompatibility>, String> {
        let connection = self.connection.lock().await;
        connection.query_row(
            "SELECT platform,level,notes,known_issues_json,last_tested_at,verification_json FROM platform_compatibility WHERE platform=?1",
            [platform], from_row
        ).optional().map_err(|e| format!("读取兼容性记录失败：{e}"))
    }
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<PlatformCompatibility> {
    let declared: String = row.get("level")?;
    let raw_issues: String = row.get("known_issues_json")?;
    let raw_evidence: Option<String> = row.get("verification_json")?;
    let verification: Option<PlatformVerificationEvidence> = raw_evidence
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok());
    let mut result = PlatformCompatibility {
        platform: row.get("platform")?,
        level: match declared.as_str() {
            "verified" => SupportLevel::Verified,
            "unsupported" => SupportLevel::Unsupported,
            _ => SupportLevel::Experimental,
        },
        notes: row.get("notes")?,
        known_issues: serde_json::from_str(&raw_issues).unwrap_or_default(),
        last_tested_at: row.get("last_tested_at")?,
        verification,
    };
    if result.level == SupportLevel::Verified && !evidence::verified_download(&result) {
        // 不重写用户声明；API/徽章只给证据实际支持的等级。
        result.level = SupportLevel::Experimental;
    }
    Ok(result)
}
