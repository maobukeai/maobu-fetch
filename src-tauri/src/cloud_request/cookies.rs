use reqwest::header::{HeaderMap, HeaderValue, COOKIE};

/// Scope the original jar before endpoint rewriting; a flat header is bound to its declared origin.
pub(crate) fn cookie_headers(
    cookie: &str,
    source: &str,
    target: &str,
) -> Result<HeaderMap, String> {
    let value = crate::media_cookies::header_for_url(cookie, source, target);
    let mut headers = HeaderMap::new();
    if !value.is_empty() {
        headers.insert(
            COOKIE,
            HeaderValue::from_str(&value).map_err(|_| "网盘 Cookie 格式无效，请重新导入凭证")?,
        );
    }
    Ok(headers)
}

/// Add an origin-bound share token without flattening unrelated Netscape domains or paths.
pub(crate) fn with_cookie_pair(
    cookie: &str,
    name: &str,
    value: &str,
    source: &str,
) -> Result<String, String> {
    if name.is_empty()
        || name.contains(['=', ';'])
        || name.chars().any(char::is_control)
        || value.contains(';')
        || value.chars().any(char::is_control)
    {
        return Err("网盘访问 Cookie 字段无效，请重新解析分享".into());
    }
    if crate::media_cookies::is_netscape_format(cookie) {
        let source = url::Url::parse(source).map_err(|_| "网盘凭证来源地址无效")?;
        let host = source.host_str().ok_or("网盘凭证来源地址缺少域名")?;
        let mut lines: Vec<&str> = cookie
            .lines()
            .filter(|line| line.split('\t').nth(5) != Some(name))
            .collect();
        let pair = format!(
            "{host}\tFALSE\t/\t{}\t0\t{name}\t{value}",
            if source.scheme() == "https" {
                "TRUE"
            } else {
                "FALSE"
            }
        );
        lines.push(&pair);
        Ok(lines.join("\n"))
    } else {
        let mut pairs = crate::media_cookies::parse_cookie_header(cookie);
        pairs.retain(|(key, _)| key != name);
        pairs.push((name.into(), value.into()));
        Ok(pairs
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; "))
    }
}
