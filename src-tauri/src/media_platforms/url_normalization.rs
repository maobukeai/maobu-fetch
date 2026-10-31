/// Bilibili's verified localized video route is the same BV resource, but older
/// extractors treat it as a generic HTML player. Preserve all business query/fragment data.
pub fn normalize_bilibili_video_url(input: &str) -> String {
    let Ok(mut url) = url::Url::parse(input) else {
        return input.into();
    };
    if url.scheme() != "https"
        || !matches!(url.host_str(), Some("www.bilibili.com" | "bilibili.com"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return input.into();
    }
    let Some(rest) = url.path().strip_prefix("/zh-Hans/video/") else {
        return input.into();
    };
    let id = rest.strip_suffix('/').unwrap_or(rest);
    if id.len() != 12
        || !id.starts_with("BV")
        || !id.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return input.into();
    }
    let path = url.path().trim_start_matches("/zh-Hans").to_owned();
    url.set_path(&path);
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verified_locale_route_keeps_bv_page_query_and_fragment() {
        assert_eq!(
            normalize_bilibili_video_url(
                "https://www.bilibili.com/zh-Hans/video/BV1VeHQ6tEaS/?p=2&session=synthetic#t=60"
            ),
            "https://www.bilibili.com/video/BV1VeHQ6tEaS/?p=2&session=synthetic#t=60"
        );
        for input in [
            "https://evil.bilibili.com/zh-Hans/video/BV1VeHQ6tEaS/",
            "https://www.bilibili.com/en/video/BV1VeHQ6tEaS/",
            "https://www.bilibili.com/zh-Hans/video/BVbad/",
            "https://www.bilibili.com/zh-Hans/video/BV1VeHQ6tEaS/extra",
            "https://www.bilibili.com/video/BV1VeHQ6tEaS/",
        ] {
            assert_eq!(normalize_bilibili_video_url(input), input);
        }
    }
}
