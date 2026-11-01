use super::normalize_douyin_ssr_detail;

// A page can include recommendation metadata before the requested work. Native
// probes must keep searching until the response identity matches the source.
pub(super) fn extract(html: &str, expected_aweme_id: Option<&str>) -> Option<serde_json::Value> {
    let marker = "__pace_f.push([1,";
    let mut search_from = 0;
    while let Some(rel_pos) = html[search_from..].find(marker) {
        let chunk_start = search_from + rel_pos + marker.len();
        search_from = chunk_start;
        let rest = html[chunk_start..].trim_start();
        if !rest.starts_with('"') {
            continue;
        }
        let script_end = rest.find("</script>").unwrap_or(rest.len());
        let script_slice = &rest[..script_end];
        if !script_slice.contains("aweme") && !script_slice.contains("detail") {
            continue;
        }
        let mut str_de = serde_json::Deserializer::from_str(script_slice).into_iter::<String>();
        let Some(Ok(inner)) = str_de.next() else {
            continue;
        };
        for aweme_marker in ["\"aweme\":{", "\"detail\":{"] {
            let mut inner_from = 0;
            while let Some(m_pos) = inner[inner_from..].find(aweme_marker) {
                let brace_offset = aweme_marker
                    .rfind('{')
                    .expect("aweme_marker 必含 '{'（构建期不变量）");
                let obj_start = inner_from + m_pos + brace_offset;
                inner_from = obj_start + 1;
                let mut val_de = serde_json::Deserializer::from_str(&inner[obj_start..])
                    .into_iter::<serde_json::Value>();
                if let Some(Ok(obj)) = val_de.next() {
                    let target_detail = obj.get("detail").unwrap_or(&obj);
                    let has_id = target_detail
                        .get("awemeId")
                        .or_else(|| target_detail.get("aweme_id"))
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|s| !s.is_empty());
                    let has_media = target_detail.get("video").is_some()
                        || target_detail
                            .get("images")
                            .and_then(serde_json::Value::as_array)
                            .is_some_and(|a| !a.is_empty());
                    let matches_requested = expected_aweme_id.is_none_or(|expected| {
                        target_detail
                            .get("awemeId")
                            .or_else(|| target_detail.get("aweme_id"))
                            .and_then(serde_json::Value::as_str)
                            == Some(expected)
                    });
                    if has_id && has_media && matches_requested {
                        return Some(normalize_douyin_ssr_detail(target_detail));
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(ids: &[&str]) -> String {
        ids.iter()
            .map(|id| {
                let payload = serde_json::json!({
                    "aweme":{"detail":{"awemeId":id,"desc":"作品","video":{}}}
                });
                format!(
                    "<script>self.__pace_f.push([1,{}])</script>",
                    serde_json::to_string(&payload.to_string()).unwrap()
                )
            })
            .collect()
    }

    #[test]
    fn recommendation_before_requested_work_is_skipped() {
        let html = page(&["7702744149895712000", "7702744149895712063"]);
        let found = extract(&html, Some("7702744149895712063")).unwrap();
        assert_eq!(found["aweme_detail"]["aweme_id"], "7702744149895712063");
    }

    #[test]
    fn response_containing_only_other_work_is_rejected() {
        let html = page(&["7702744149895712000"]);
        assert!(extract(&html, Some("7702744149895712063")).is_none());
    }

    #[test]
    fn compatibility_parser_can_still_extract_first_work() {
        let html = page(&["7702744149895712000"]);
        assert!(extract(&html, None).is_some());
    }
}
