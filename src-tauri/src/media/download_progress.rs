use crate::models::DownloadTask;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct Progress {
    tracks: HashMap<String, (u64, Option<u64>)>,
}

impl Progress {
    pub fn record(&mut self, line: &str, task: &mut DownloadTask) -> Result<bool, String> {
        let Some(text) = line.strip_prefix("MAOBU_PROGRESS:") else {
            return Ok(false);
        };
        let value: Value = serde_json::from_str(text).map_err(|_| "媒体工具进度数据无效")?;
        let Some(id) = value.get("id").and_then(Value::as_str) else {
            return Ok(true);
        };
        let downloaded = value.get("downloaded").and_then(Value::as_u64).unwrap_or(0);
        let total = value.get("total").and_then(Value::as_u64);
        let entry = self.tracks.entry(id.into()).or_default();
        entry.0 = entry.0.max(downloaded);
        if total.is_some() {
            entry.1 = total;
        }
        task.downloaded_bytes = self
            .tracks
            .values()
            .map(|(bytes, _)| *bytes)
            .fold(0u64, u64::saturating_add);
        task.total_bytes = self
            .tracks
            .values()
            .map(|(_, total)| *total)
            .collect::<Option<Vec<_>>>()
            .map(|totals| totals.into_iter().fold(0u64, u64::saturating_add))
            .unwrap_or(0);
        Ok(true)
    }
}

pub(super) const PROGRESS_TEMPLATE: &str = r#"download:MAOBU_PROGRESS:{"id":%(info.format_id|null)j,"downloaded":%(progress.downloaded_bytes|null)j,"total":%(progress.total_bytes|null)j}"#;
pub(super) const METADATA_TEMPLATE: &str = r#"before_dl:MAOBU_META:{"title":%(title|null)j,"uploader":%(uploader|null)j,"channel":%(channel|null)j,"uploader_id":%(uploader_id|null)j,"upload_date":%(upload_date|null)j,"id":%(id|null)j,"display_id":%(display_id|null)j,"extractor_key":%(extractor_key|null)j,"is_live":%(is_live|null)j,"live_status":%(live_status|null)j}"#;
pub(super) const OUTPUT_TEMPLATE: &str = r#"after_move:MAOBU_OUTPUT:{"file":%(filepath|null)j,"tracks":%(requested_formats|null)j,"subtitles":%(requested_subtitles|null)j}"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_track_progress_is_real_monotonic_and_unknown_total_stays_unknown() {
        let mut task: DownloadTask = serde_json::from_value(serde_json::json!({
            "id":"fixture","url":"https://example.test","file_name":"x.mp4","destination":"C:/AuditOnly","total_bytes":0,"downloaded_bytes":0,"speed":0,"status":"downloading","created_at":0,"category":"video","headers":{},
            "queue_position":0,"priority":0,"retry_count":0,"max_retries":0,"source":"desktop","per_task_speed_limit":0,"collision_policy":"rename","connection_count":1
        })).unwrap();
        let mut progress = Progress::default();
        progress
            .record(
                "MAOBU_PROGRESS:{\"id\":\"v\",\"downloaded\":80,\"total\":100}",
                &mut task,
            )
            .unwrap();
        progress
            .record(
                "MAOBU_PROGRESS:{\"id\":\"a\",\"downloaded\":20,\"total\":\"NA\"}",
                &mut task,
            )
            .unwrap();
        assert_eq!(task.downloaded_bytes, 100);
        assert_eq!(task.total_bytes, 0);
        progress
            .record(
                "MAOBU_PROGRESS:{\"id\":\"v\",\"downloaded\":0,\"total\":100}",
                &mut task,
            )
            .unwrap();
        assert_eq!(task.downloaded_bytes, 100);
    }
}
