use super::*;

/// 提取器把页面解析为 CDN 直链时，页面认证信息不能跟着跨站点发送。
pub(super) fn for_direct_url(
    headers: &HashMap<String, String>,
    source: &str,
    target: &str,
) -> HashMap<String, String> {
    retarget(headers, source, target)
        .into_iter()
        .filter_map(|(name, value)| {
            if name.eq_ignore_ascii_case("cookie") {
                let value = crate::media_cookies::header_for_url(&value, source, target);
                (!value.is_empty()).then_some((name, value))
            } else {
                Some((name, value))
            }
        })
        .collect()
}

/// 保留jar的域/路径信息；只有真正发送请求时才构造Cookie头。
pub(super) fn retarget(
    headers: &HashMap<String, String>,
    source: &str,
    target: &str,
) -> HashMap<String, String> {
    let same_origin = url::Url::parse(source)
        .ok()
        .zip(url::Url::parse(target).ok())
        .is_some_and(|(source, target)| source.origin() == target.origin());
    headers
        .iter()
        .filter(|(name, value)| {
            same_origin
                || !sensitive_name(name)
                || (name.eq_ignore_ascii_case("cookie")
                    && crate::media_cookies::is_netscape_format(value))
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn sensitive_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "cookie"
            | "authorization"
            | "proxy-authorization"
            | "x-api-key"
            | "x-auth-token"
            | "x-access-token"
            | "x-csrf-token"
            | "x-luma-token"
            | "x-luma-signature"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_cdn_requests_keep_referer_but_not_page_credentials() {
        let headers = HashMap::from([
            ("Cookie".into(), "session=synthetic-session".into()),
            ("aUtHoRiZaTiOn".into(), "synthetic-bearer".into()),
            ("X-API-Key".into(), "synthetic-key".into()),
            ("Referer".into(), "https://page.example/video/1".into()),
            ("User-Agent".into(), "Synthetic".into()),
        ]);
        assert_eq!(
            for_direct_url(
                &headers,
                "https://page.example/video/1",
                "https://page.example/data"
            ),
            headers
        );
        let malformed = HashMap::from([("Cookie".into(), "synthetic-session".into())]);
        assert!(for_direct_url(
            &malformed,
            "https://page.example/video/1",
            "https://page.example/data"
        )
        .is_empty());
        for target in [
            "https://cdn.example/video",
            "http://page.example/video",
            "https://page.example:8443/video",
        ] {
            let scoped = for_direct_url(&headers, "https://page.example/video/1", target);
            assert_eq!(scoped.len(), 2);
            assert!(scoped.contains_key("Referer"));
            assert!(scoped.contains_key("User-Agent"));
        }
    }

    #[test]
    fn cookie_jar_remains_scoped_in_storage_and_is_formatted_only_for_its_target() {
        let jar="# Netscape HTTP Cookie File\ncdn.example\tFALSE\t/private\tTRUE\t0\tsid\tsynthetic-cdn\nother.example\tFALSE\t/\tTRUE\t0\tsid\tsynthetic-other";
        let headers = HashMap::from([("Cookie".into(), jar.into())]);
        let stored = retarget(
            &headers,
            "https://page.example/video",
            "https://cdn.example/private/file",
        );
        assert_eq!(stored.get("Cookie").unwrap(), jar);
        let sent = for_direct_url(
            &stored,
            "https://cdn.example/private/file",
            "https://cdn.example/private/file",
        );
        assert_eq!(sent.get("Cookie").unwrap(), "sid=synthetic-cdn");
        assert!(for_direct_url(
            &stored,
            "https://cdn.example/private/file",
            "https://cdn.example/public"
        )
        .is_empty());
        assert!(for_direct_url(
            &stored,
            "https://cdn.example/private/file",
            "http://cdn.example/private/file"
        )
        .is_empty());
    }
}
