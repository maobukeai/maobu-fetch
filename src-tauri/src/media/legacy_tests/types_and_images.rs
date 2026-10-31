use super::*;
// ===== Task 42：图集/音频类型识别与解析 =====

/// 普通视频 URL 应识别为 `MediaType::Video`。
///
/// 覆盖 YouTube / B 站 / 抖音视频 / TikTok 视频 / Twitter 推文 / 微博状态，
/// 这些 URL 都不含图集或音频专有路径段，应回退到默认的 Video 类型。
#[test]
fn media_type_video_for_normal_video() {
    // YouTube 普通视频
    assert_eq!(
        determine_media_type("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
        MediaType::Video
    );
    // B 站普通视频
    assert_eq!(
        determine_media_type("https://www.bilibili.com/video/BV1xx411c7mD"),
        MediaType::Video
    );
    // 抖音普通视频（/video/，非 /note/）
    assert_eq!(
        determine_media_type("https://www.douyin.com/video/7283698765432109876"),
        MediaType::Video
    );
    // TikTok 普通视频
    assert_eq!(
        determine_media_type("https://www.tiktok.com/@user/video/123"),
        MediaType::Video
    );
    // Twitter 普通推文（非 Spaces）
    assert_eq!(
        determine_media_type("https://twitter.com/user/status/1234567890"),
        MediaType::Video
    );
    // 微博普通状态（非 album）
    assert_eq!(
        determine_media_type("https://weibo.com/1234567890/N0abcdef"),
        MediaType::Video
    );
}

/// Twitter Spaces 音频 URL 应识别为 `MediaType::Audio`。
///
/// 覆盖 `twitter.com/i/spaces/` 与 `x.com/i/spaces/` 两种 host。
#[test]
fn media_type_audio_for_twitter_space() {
    assert_eq!(
        determine_media_type("https://twitter.com/i/spaces/1DXxyv"),
        MediaType::Audio
    );
    assert_eq!(
        determine_media_type("https://x.com/i/spaces/1DXxyvXYZ?s=20"),
        MediaType::Audio
    );
}

/// 抖音 note 图集 URL 应识别为 `MediaType::Gallery`。
///
/// 覆盖 `/note/` 路径与 iesdouyin 旧版 `/share/note/` 路径。
/// 同时验证 TikTok `/photo/` 与微博 `/album/` 也识别为 Gallery。
#[test]
fn media_type_gallery_for_douyin_note() {
    // 抖音 note 图集
    assert_eq!(
        determine_media_type("https://www.douyin.com/note/1234567890"),
        MediaType::Gallery
    );
    assert_eq!(
        determine_media_type("https://www.iesdouyin.com/share/note/1234567890"),
        MediaType::Gallery
    );
    // TikTok photo 图集
    assert_eq!(
        determine_media_type("https://www.tiktok.com/@user/photo/123"),
        MediaType::Gallery
    );
    // 微博 album 图集
    assert_eq!(
        determine_media_type("https://weibo.com/album/123456"),
        MediaType::Gallery
    );
}

#[test]
fn parse_formats_filters_out_youtube_storyboard_formats() {
    let json = serde_json::json!({
        "formats": [
            {
                "format_id": "sb3",
                "format_note": "storyboard",
                "vcodec": "none",
                "acodec": "none",
                "ext": "mhtml",
                "width": 168,
                "height": 94,
                "url": "https://i.ytimg.com/sb/pZHf0913SGI/storyboard3_L0/default.jpg"
            },
            {
                "format_id": "18",
                "format_note": "360p",
                "vcodec": "avc1.42001E",
                "acodec": "mp4a.40.2",
                "ext": "mp4",
                "width": 640,
                "height": 360,
                "url": "https://rr3---sn-oguelnze.googlevideo.com/videoplayback?id=123"
            }
        ]
    });
    let formats = parse_formats(&json);
    assert_eq!(formats.len(), 1);
    assert_eq!(formats[0].id, "18");
    assert!(formats[0].has_video);
    assert!(formats[0].has_audio);
}

/// Task 42：`parse_formats` 应正确识别图片格式项并填充 `image_url`。
///
/// 模拟 yt-dlp 返回的图集 JSON：formats 中含 2 个图片项（vcodec=none, acodec=none,
/// ext=jpg/jpeg），应被解析为 `MediaFormat`，`has_video=false`、`has_audio=false`、
/// `image_url` 非空。
#[test]
fn parse_formats_extracts_image_items_from_yt_dlp_json() {
    let json = serde_json::json!({
        "formats": [
            {
                "format_id": "image-0",
                "vcodec": "none",
                "acodec": "none",
                "ext": "jpeg",
                "width": 1080,
                "height": 1440,
                "url": "https://p3-sign.douyinpic.com/image0.jpg"
            },
            {
                "format_id": "image-1",
                "vcodec": "none",
                "acodec": "none",
                "ext": "jpg",
                "url": "https://p3-sign.douyinpic.com/image1.jpg"
            }
        ]
    });
    let formats = parse_formats(&json);
    assert_eq!(formats.len(), 2, "应解析出 2 个图片项");
    // 第一个图片项：含宽高，label 应显示尺寸
    let first = &formats[0];
    assert_eq!(first.id, "image-0");
    assert!(!first.has_video);
    assert!(!first.has_audio);
    assert_eq!(first.extension.as_deref(), Some("jpeg"));
    assert_eq!(first.width, Some(1080));
    assert_eq!(first.height, Some(1440));
    assert_eq!(
        first.image_url.as_deref(),
        Some("https://p3-sign.douyinpic.com/image0.jpg")
    );
    assert!(
        first.label.contains("1080"),
        "label 应包含宽度信息：{}",
        first.label
    );
    // 第二个图片项：无宽高，label 应为 "图片"
    let second = &formats[1];
    assert_eq!(second.id, "image-1");
    assert_eq!(second.extension.as_deref(), Some("jpg"));
    assert_eq!(second.width, None);
    assert_eq!(second.height, None);
    assert_eq!(
        second.image_url.as_deref(),
        Some("https://p3-sign.douyinpic.com/image1.jpg")
    );
}

/// Task 42：视频/音频格式项的 `image_url` 应始终为 `None`。
///
/// 即使 format 项含 `url` 字段，只要 vcodec/acodec 非 none 或 ext 非图片扩展名，
/// `image_url` 字段都不应被填充。保证图集前端不会把视频流误识别为图片。
#[test]
fn parse_formats_does_not_set_image_url_for_video_audio_items() {
    let json = serde_json::json!({
        "formats": [
            {
                "format_id": "137",
                "vcodec": "avc1.640028",
                "acodec": "none",
                "ext": "mp4",
                "height": 1080,
                "url": "https://example.com/video.mp4"
            },
            {
                "format_id": "140",
                "vcodec": "none",
                "acodec": "mp4a.40.2",
                "ext": "m4a",
                "url": "https://example.com/audio.m4a"
            }
        ]
    });
    let formats = parse_formats(&json);
    assert_eq!(formats.len(), 2);
    for format in &formats {
        assert!(
            format.image_url.is_none(),
            "视频/音频格式项不应填充 image_url：{:?}",
            format.id
        );
    }
    // 视频项 has_video=true，音频项 has_audio=true
    assert!(formats[0].has_video);
    assert!(!formats[0].has_audio);
    assert!(!formats[1].has_video);
    assert!(formats[1].has_audio);
}
