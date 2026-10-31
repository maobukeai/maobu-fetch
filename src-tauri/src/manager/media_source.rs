//! A request Referer must not replace the user's selected media source page.
use std::collections::HashMap;
use url::Url;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Site {
    Douyin,
    Bilibili,
    YouTube,
    TikTok,
}

/// Original source URLs always win. Only a known media CDN can restore a specific
/// source page, preferring the stored original page over a possibly unrelated Referer.
pub(super) fn probe_source<'a>(
    source: &'a str,
    original_source: Option<&'a str>,
    headers: &'a HashMap<String, String>,
) -> &'a str {
    let Some(site) = Url::parse(source)
        .ok()
        .and_then(|url| cdn_site(url.host_str()?))
    else {
        return source;
    };
    if let Some(original) = original_source.filter(|url| specific_page(url, site)) {
        return original;
    }
    let mut referer = None;
    for (name, value) in headers {
        if !name.eq_ignore_ascii_case("referer") && !name.eq_ignore_ascii_case("referrer") {
            continue;
        }
        if referer.is_some_and(|previous| previous != value.as_str()) {
            return source;
        }
        referer = Some(value.as_str());
    }
    referer
        .filter(|url| specific_page(url, site))
        .unwrap_or(source)
}

fn domain(host: &str, suffix: &str) -> bool {
    host.eq_ignore_ascii_case(suffix) || host.to_ascii_lowercase().ends_with(&format!(".{suffix}"))
}
fn cdn_site(host: &str) -> Option<Site> {
    if [
        "douyinvod.com",
        "byteplusvod.com",
        "ibytedtos.com",
        "zjcdn.com",
    ]
    .iter()
    .any(|suffix| domain(host, suffix))
    {
        Some(Site::Douyin)
    } else if ["bilivideo.com", "bilivideo.cn"]
        .iter()
        .any(|suffix| domain(host, suffix))
    {
        Some(Site::Bilibili)
    } else if domain(host, "googlevideo.com") {
        Some(Site::YouTube)
    } else if ["tiktokcdn.com", "tiktokv.com"]
        .iter()
        .any(|suffix| domain(host, suffix))
    {
        Some(Site::TikTok)
    } else {
        None
    }
}
fn digits(value: &str, minimum: usize) -> bool {
    value.len() >= minimum && value.bytes().all(|byte| byte.is_ascii_digit())
}
fn youtube_id(value: &str) -> bool {
    value.len() == 11
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}
fn specific_page(input: &str, site: Site) -> bool {
    let Ok(url) = Url::parse(input) else {
        return false;
    };
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return false;
    }
    let host = url.host_str().unwrap_or("");
    let path: Vec<_> = url.path().trim_matches('/').split('/').collect();
    match site {
        Site::Douyin
            if [
                "douyin.com",
                "www.douyin.com",
                "iesdouyin.com",
                "www.iesdouyin.com",
            ]
            .contains(&host) =>
        {
            (matches!(path.as_slice(), ["video" | "note", id] if digits(id,10)))
                || (matches!(path.as_slice(), ["share", "video", id] if digits(id,10)))
                || (matches!(url.path(), "/" | "/discover" | "/discover/")
                    && url.query_pairs().any(|(key, value)| {
                        matches!(key.as_ref(), "modal_id" | "aweme_id" | "item_id")
                            && digits(&value, 10)
                    }))
        }
        Site::Douyin if host == "live.douyin.com" => {
            matches!(path.as_slice(), [id] if !id.is_empty() && id.len() <= 64 && id.bytes().all(|byte|byte.is_ascii_alphanumeric()))
        }
        Site::Bilibili
            if ["bilibili.com", "www.bilibili.com", "m.bilibili.com"].contains(&host) =>
        {
            matches!(path.as_slice(), ["video", id] if (id.starts_with("BV") && id.len()==12 && id.bytes().all(|byte|byte.is_ascii_alphanumeric())) || id.strip_prefix("av").is_some_and(|id|digits(id,1)))
                || matches!(path.as_slice(), ["bangumi","play",id] if id.strip_prefix("ep").or_else(||id.strip_prefix("ss")).is_some_and(|id|digits(id,1)))
        }
        Site::YouTube
            if [
                "youtube.com",
                "www.youtube.com",
                "m.youtube.com",
                "music.youtube.com",
            ]
            .contains(&host) =>
        {
            (url.path() == "/watch"
                && url
                    .query_pairs()
                    .any(|(key, value)| key == "v" && youtube_id(&value)))
                || matches!(path.as_slice(), ["shorts"|"embed"|"live",id] if youtube_id(id))
        }
        Site::YouTube if host == "youtu.be" => matches!(path.as_slice(), [id] if youtube_id(id)),
        Site::TikTok if ["tiktok.com", "www.tiktok.com", "m.tiktok.com"].contains(&host) => {
            matches!(path.as_slice(), [user,"video"|"photo",id] if user.starts_with('@') && user.len()>1 && digits(id,10))
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "media_source/tests.rs"]
mod tests;
