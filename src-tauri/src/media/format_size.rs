pub(super) fn parse(item: &serde_json::Value) -> (Option<u64>, bool) {
    let positive = |key| {
        item.get(key)
            .and_then(serde_json::Value::as_u64)
            .filter(|bytes| *bytes > 0)
    };
    if let Some(exact) = positive("filesize") {
        (Some(exact), false)
    } else {
        let approximate = positive("filesize_approx");
        (approximate, approximate.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn null_exact_size_uses_real_approximation_without_claiming_exact_bytes() {
        assert_eq!(
            parse(&serde_json::json!({"filesize":null,"filesize_approx":113054403})),
            (Some(113054403), true)
        );
        assert_eq!(
            parse(&serde_json::json!({"filesize":123,"filesize_approx":999})),
            (Some(123), false)
        );
        for value in [
            serde_json::json!({}),
            serde_json::json!({"filesize":null,"filesize_approx":null}),
            serde_json::json!({"filesize":-1,"filesize_approx":0}),
        ] {
            assert_eq!(parse(&value), (None, false));
        }
    }
    #[test]
    fn old_format_json_defaults_size_estimate_flag_safely() {
        let format: crate::models::MediaFormat = serde_json::from_value(
            serde_json::json!({"id":"legacy","label":"legacy","has_video":true,"has_audio":true}),
        )
        .unwrap();
        assert!(!format.file_size_approximate);
    }
}
