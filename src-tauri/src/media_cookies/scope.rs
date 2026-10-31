/// 解析 `name=value; name2=value2` 格式的 Cookie 字符串。
///
/// 返回 (name, value) 元组列表。空值或格式错误的条目会被跳过。
/// 不对 name/value 内容做 URL 解码，因为 yt-dlp 接受原始值。
pub(crate) fn parse_cookie_header(cookie: &str) -> Vec<(String, String)> {
    cookie
        .split(';')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .filter_map(|segment| {
            let idx = segment.find('=')?;
            let name = segment[..idx].trim();
            let value = segment[idx + 1..].trim();
            if name.is_empty() {
                None
            } else {
                Some((name.to_string(), value.to_string()))
            }
        })
        .collect()
}

/// 判断 Cookie 字符串是否为 Netscape cookies.txt 格式。
///
/// 识别条件：包含 `# Netscape` 头或任一行含 TAB 分隔的 7 列。
/// 用于检测「设置 → 媒体凭证」里存的 Cookie 是 HTTP 头格式还是 cookies.txt 文件格式，
/// 以便在用作 HTTP `Cookie:` 头前先转换格式（HTTP 头不允许 TAB/换行字符）。
pub(crate) fn is_netscape_format(cookie: &str) -> bool {
    let trimmed = cookie.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.contains("# Netscape") {
        return true;
    }
    // 检查是否有 TAB 分隔的至少 7 列的行（Netscape 格式特征）
    trimmed
        .lines()
        .any(|line| !line.starts_with('#') && line.split('\t').count() >= 7)
}

/// Raw headers belong to their source origin; a Netscape jar retains each cookie's scope.
pub(crate) fn header_for_url(cookie: &str, source_url: &str, target_url: &str) -> String {
    let Ok(target) = url::Url::parse(target_url) else {
        return String::new();
    };
    if !matches!(target.scheme(), "http" | "https") {
        return String::new();
    }
    if !is_netscape_format(cookie) {
        let Ok(source) = url::Url::parse(source_url) else {
            return String::new();
        };
        return if source.origin() == target.origin() && !cookie.chars().any(char::is_control) {
            parse_cookie_header(cookie)
                .into_iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ")
        } else {
            String::new()
        };
    }
    let Some(host) = target.host_str() else {
        return String::new();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs())
        .unwrap_or(u64::MAX);
    let mut matches = Vec::new();
    for (index, line) in cookie.lines().enumerate() {
        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if line.starts_with('#') {
            continue;
        }
        let columns: Vec<_> = line.split('\t').collect();
        if columns.len() != 7 {
            continue;
        }
        let domain = columns[0].trim_start_matches('.');
        let domain_matches = host.eq_ignore_ascii_case(domain)
            || columns[1].eq_ignore_ascii_case("TRUE")
                && host
                    .to_ascii_lowercase()
                    .ends_with(&format!(".{}", domain.to_ascii_lowercase()));
        let path = columns[2];
        let path_matches = path.starts_with('/')
            && target.path().starts_with(path)
            && (path.ends_with('/')
                || target.path().len() == path.len()
                || target.path().as_bytes().get(path.len()) == Some(&b'/'));
        let expiration = columns[4].parse::<u64>();
        if !domain_matches
            || !path_matches
            || columns[3].eq_ignore_ascii_case("TRUE") && target.scheme() != "https"
            || !expiration.is_ok_and(|expires| expires == 0 || expires > now)
        {
            continue;
        }
        let (name, value) = (columns[5], columns[6]);
        if name.is_empty()
            || name.contains(['=', ';'])
            || name.chars().any(char::is_control)
            || value.contains(';')
            || value.chars().any(char::is_control)
        {
            continue;
        }
        matches.push((path.len(), index, format!("{name}={value}")));
    }
    matches.sort_by_key(|(length, index, _)| (std::cmp::Reverse(*length), *index));
    matches
        .into_iter()
        .map(|(_, _, pair)| pair)
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_header_is_bound_to_exact_origin() {
        assert_eq!(
            header_for_url("sid=synthetic", "https://x.com/a", "https://x.com/b"),
            "sid=synthetic"
        );
        for other in [
            "https://cdn.syndication.twimg.com/",
            "https://sub.x.com/",
            "http://x.com/",
            "https://x.com:444/",
        ] {
            assert!(header_for_url("sid=synthetic", "https://x.com/a", other).is_empty());
        }
    }
    #[test]
    fn jar_respects_host_path_secure_expiry_and_duplicate_order() {
        let jar = "# Netscape HTTP Cookie File\n.douyin.com\tTRUE\t/\tTRUE\t0\tsid\troot\n#HttpOnly_www.douyin.com\tFALSE\t/video\tTRUE\t0\tsid\tpath\nwww.douyin.com\tFALSE\t/\tTRUE\t1\texpired\tno\nx.com\tFALSE\t/\tTRUE\t0\tforeign\tno\n";
        assert_eq!(
            header_for_url(jar, "", "https://www.douyin.com/video/1"),
            "sid=path; sid=root"
        );
        assert_eq!(
            header_for_url(jar, "", "https://live.douyin.com/video/1"),
            "sid=root"
        );
        assert_eq!(
            header_for_url(jar, "", "https://www.douyin.com/videos"),
            "sid=root"
        );
        assert!(header_for_url(jar, "", "http://www.douyin.com/video").is_empty());
        assert!(header_for_url(jar, "", "https://evildouyin.com/video").is_empty());
    }
}
