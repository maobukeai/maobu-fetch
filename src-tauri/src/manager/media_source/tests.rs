use super::*;
const SOURCE: &str = "https://www.douyin.com/video/7694589931460594953";
const CDN: &str = "https://v3-dy-o.zjcdn.com/media/file.mp4?signature=synthetic";
fn headers(referer: &str) -> HashMap<String, String> {
    HashMap::from([("Referer".to_owned(), referer.to_owned())])
}

#[test]
fn concrete_task_source_survives_automatically_injected_root_referer() {
    assert_eq!(
        probe_source(SOURCE, None, &headers("https://www.douyin.com/")),
        SOURCE
    );
}

#[test]
fn different_video_referer_never_replaces_selected_source() {
    for referer in [
        "https://www.douyin.com/video/7408129200156101907",
        "https://www.youtube.com/watch?v=YE7VzlLtp-4",
        "https://evil.example/?x=douyin.com",
    ] {
        assert_eq!(probe_source(SOURCE, None, &headers(referer)), SOURCE);
    }
    assert_eq!(
        probe_source("https://www.douyin.com/", None, &headers(SOURCE)),
        "https://www.douyin.com/"
    );
}

#[test]
fn known_cdn_can_restore_a_specific_page_from_the_matching_platform() {
    for (cdn, page) in [
        (CDN, SOURCE),
        ("https://v.example.douyinvod.com/file", SOURCE),
        (
            "https://upos.bilivideo.com/video",
            "https://www.bilibili.com/video/BV13x41117TL/?p=1",
        ),
        (
            "https://r1.googlevideo.com/videoplayback",
            "https://www.youtube.com/watch?v=YE7VzlLtp-4",
        ),
        (
            "https://v.tiktokv.com/file",
            "https://www.tiktok.com/@sample/video/7408129200156101907",
        ),
    ] {
        assert_eq!(probe_source(cdn, None, &headers(page)), page);
    }
}

#[test]
fn recorded_source_identity_wins_over_a_different_valid_referer() {
    let other = "https://www.douyin.com/video/7408129200156101907";
    assert_eq!(probe_source(CDN, Some(SOURCE), &headers(other)), SOURCE);
}

#[test]
fn root_unrelated_path_userinfo_downgrade_and_foreign_origin_cannot_restore() {
    for referer in [
        "https://www.douyin.com/",
        "https://www.douyin.com/search/test",
        "https://www.douyin.com/redirect?url=https://www.douyin.com/video/7694589931460594953",
        "https://www.douyin.com.evil.example/video/7694589931460594953",
        "https://evil.example/?referer=https://www.douyin.com/video/7694589931460594953",
        "https://douyin.com@evil.example/video/7694589931460594953",
        "https://user:password@www.douyin.com/video/7694589931460594953",
        "https://www.douyin.com:8443/video/7694589931460594953",
        "http://www.douyin.com/video/7694589931460594953",
        "https://www.youtube.com/watch?v=YE7VzlLtp-4",
    ] {
        assert_eq!(probe_source(CDN, None, &headers(referer)), CDN);
    }
}

#[test]
fn cdn_domain_must_match_exactly_and_not_from_query_or_substring() {
    for source in [
        "https://evilzjcdn.com/file",
        "https://zjcdn.com.evil.example/file",
        "https://evil.example/file?source=zjcdn.com",
        "https://cdn.example/douyinvod.com/file",
    ] {
        assert_eq!(probe_source(source, None, &headers(SOURCE)), source);
    }
}

#[test]
fn aliases_accept_concrete_ids_and_never_a_nested_url_substring() {
    let page = "https://www.douyin.com/discover?modal_id=7694589931460594953";
    assert_eq!(probe_source(CDN, None, &headers(page)), page);
    let mut request = headers(SOURCE);
    request.insert(
        "referer".into(),
        "https://www.douyin.com/video/7408129200156101907".into(),
    );
    assert_eq!(probe_source(CDN, None, &request), CDN);
}

#[test]
fn malformed_sources_and_invalid_recorded_page_remain_safe() {
    assert_eq!(probe_source("invalid", None, &headers(SOURCE)), "invalid");
    assert_eq!(
        probe_source(CDN, Some("https://www.douyin.com/"), &HashMap::new()),
        CDN
    );
    assert_eq!(probe_source(CDN, None, &HashMap::new()), CDN);
}
