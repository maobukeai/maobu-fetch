/// External extractors use global custom headers across multiple HTTPS origins. A transparent
/// CONNECT proxy cannot enforce per-request credential scope without decrypting TLS.
pub(super) fn validate_external_headers<'a>(
    headers: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<(), String> {
    for (name, _) in headers {
        let name = name.to_ascii_lowercase();
        let sensitive = name == "authorization"
            || name == "proxy-authorization"
            || name == "set-cookie"
            || name.contains("api-key")
            || name.contains("apikey")
            || name.contains("token")
            || name == "x-pin"
            || name.contains("secret")
            || name.contains("credential");
        let sensitive = sensitive
            || name.contains("cookie")
            || name.contains("session")
            || name.contains("csrf");
        if sensitive {
            return Err("MEDIA_AUTH_SCOPE: 外部媒体工具无法保证此认证头只发送给原站点，已停止处理。请改用有域名作用域的站点 Cookie，或使用原生 HTTP 直链下载".into());
        }
    }
    Ok(())
}

pub(super) fn validate_referer(value: Option<&str>) -> Result<(), String> {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return Ok(());
    };
    let parsed = url::Url::parse(value).map_err(|_| "媒体 Referer 必须是有效网页地址")?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("媒体 Referer 必须是无账号密码的 HTTP 网页地址".into());
    }
    if parsed.query_pairs().any(|(key, _)| {
        let key = key.to_ascii_lowercase();
        [
            "token",
            "sign",
            "sig",
            "auth",
            "key",
            "pin",
            "password",
            "credential",
        ]
        .iter()
        .any(|sensitive| key.contains(sensitive))
    }) {
        return Err("MEDIA_AUTH_SCOPE: Referer 含令牌或签名查询参数，外部工具可能把它发给其他媒体域名。请仅保留普通页面地址，或使用原生 HTTP 直链下载".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_sensitive_headers_are_explicitly_refused_without_echoing_values() {
        for name in [
            "Authorization",
            "X-API-Key",
            "x-access-token",
            "X-Pin",
            "Proxy-Authorization",
        ] {
            let error = validate_external_headers([(name, "synthetic-private")]).unwrap_err();
            assert!(error.contains("原生 HTTP"));
            assert!(!error.contains("synthetic-private"));
        }
        assert!(
            validate_external_headers([("User-Agent", "Maobu Fetch"), ("Accept", "*/*")]).is_ok()
        );
    }
    #[test]
    fn ordinary_referers_work_and_signed_query_credentials_never_cross_origins() {
        assert!(validate_referer(Some("https://www.bilibili.com/video/BV1TEST?p=2")).is_ok());
        for value in [
            "https://example.test/?token=synthetic",
            "https://example.test/?X-Amz-Signature=synthetic",
            "https://user:synthetic@example.test/",
        ] {
            let error = validate_referer(Some(value)).unwrap_err();
            assert!(!error.contains("synthetic"));
        }
    }
}
