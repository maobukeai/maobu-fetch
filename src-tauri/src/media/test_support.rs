pub(super) fn task() -> crate::models::DownloadTask {
    serde_json::from_value(serde_json::json!({
        "id":"fixture","url":"https://example.test/video","file_name":"video.mp4","destination":"C:/AuditOnly","total_bytes":0,"downloaded_bytes":0,"speed":0,"status":"downloading","created_at":0,"category":"video","headers":{},
        "queue_position":0,"priority":0,"retry_count":0,"max_retries":0,"source":"desktop","per_task_speed_limit":0,"collision_policy":"rename","connection_count":1
    })).unwrap()
}
