pub(super) fn from_metadata(value: &serde_json::Value) -> bool {
    value.get("is_live").and_then(serde_json::Value::as_bool) == Some(true)
        || value.get("live_status").and_then(serde_json::Value::as_str) == Some("is_live")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_actual_live_metadata_enables_live_controls() {
        assert!(from_metadata(&serde_json::json!({"is_live":true})));
        assert!(from_metadata(&serde_json::json!({"live_status":"is_live"})));
        for value in [
            serde_json::json!({}),
            serde_json::json!({"url":"https://example.test/live.m3u8"}),
            serde_json::json!({"is_live":false,"live_status":"was_live"}),
            serde_json::json!({"live_status":"is_upcoming"}),
        ] {
            assert!(!from_metadata(&value));
        }
    }
}
