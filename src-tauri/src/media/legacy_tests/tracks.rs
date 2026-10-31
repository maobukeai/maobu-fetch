use super::*;

#[test]
fn explicit_no_video_is_not_overridden_by_cover_dimensions() {
    let format=parse_single_format(&serde_json::json!({"format_id":"audio-with-cover","vcodec":"none","acodec":"mp4a.40.2","width":1920,"height":1080,"ext":"m4a"})).unwrap();
    assert!(!format.has_video);
    assert!(format.has_audio);
    assert!(!format.audio_unknown);
    assert!(select_default_video_format(&[format], true).is_err());
}

#[test]
fn unknown_codec_sentinels_are_not_confirmed_audio() {
    for codec in ["", "unknown", "N/A"] {
        let format=parse_single_format(&serde_json::json!({"format_id":"unknown","vcodec":"h264","acodec":codec,"width":1920,"height":1080})).unwrap();
        assert!(!format.has_audio);
        assert!(format.audio_unknown);
    }
}
/// Twitter progressive 流：yt-dlp 对 http-* 格式返回 vcodec/acodec 为 null，
/// 保留实际尺寸但音轨未知，不能按格式ID或尺寸推断有声。
#[test]
fn parse_single_format_twitter_progressive_null_codecs() {
    let json = serde_json::json!({
        "formats": [
            {
                "format_id": "http-2176",
                "vcodec": null,
                "acodec": null,
                "ext": "mp4",
                "width": 1280,
                "height": 720,
                "filesize": 8115664
            },
            {
                "format_id": "http-832",
                "vcodec": null,
                "acodec": null,
                "ext": "mp4",
                "width": 640,
                "height": 360
            }
        ]
    });
    let formats = parse_formats(&json);
    assert_eq!(formats.len(), 2);
    // 两个 progressive 格式仅确认视频尺寸，音轨未知。
    for f in &formats {
        assert!(f.has_video, "Twitter progressive 流应有视频：{}", f.id);
        assert!(!f.has_audio, "未知编解码不得声称有音轨：{}", f.id);
        assert!(f.audio_unknown);
        assert!(!f.requires_ffmpeg);
    }
    // 按 height 排序时 720p 应在前
    assert_eq!(formats[0].height, Some(720));
    assert_eq!(formats[1].height, Some(360));
}

/// Twitter HLS 分离流：hls-* 视频轨 acodec="none" 应识别为无声视频；
/// hls-audio-* 的 acodec=null 仍不能证明存在音轨。
#[test]
fn parse_single_format_twitter_hls_split_streams() {
    let json = serde_json::json!({
        "formats": [
            {
                "format_id": "hls-1570",
                "vcodec": "avc1.64001F",
                "acodec": "none",
                "ext": "mp4",
                "width": 1280,
                "height": 720
            },
            {
                "format_id": "hls-audio-128000-Audio",
                "vcodec": "none",
                "acodec": null,
                "ext": "mp4"
            }
        ]
    });
    let formats = parse_formats(&json);
    assert_eq!(formats.len(), 2);
    // hls-1570: 无声视频
    assert!(formats[0].has_video, "hls 视频轨应有视频");
    assert!(!formats[0].has_audio, "hls 视频轨应无音频");
    // hls-audio-128000: 音频名称无法替代编解码确认。
    assert!(!formats[1].has_video, "hls 音频轨应无视频");
    assert!(!formats[1].has_audio);
    assert!(
        formats[1].audio_unknown,
        "音频格式ID不能证明未知编解码存在音轨"
    );
}
/// 合成格式（bestvideo*+bestaudio/best）应携带最高视频流的 height，
/// 便于前端按高度排序正确选中默认格式。Twitter 场景：hls-1570 (720p) + hls-audio。
#[test]
fn merged_format_carries_best_video_height() {
    let json = serde_json::json!({
        "formats": [
            {
                "format_id": "hls-audio-128000-Audio",
                "vcodec": "none",
                "acodec": "mp4a.40.2",
                "ext": "mp4"
            },
            {
                "format_id": "hls-522",
                "vcodec": "avc1.4D401E",
                "acodec": "none",
                "ext": "mp4",
                "width": 640,
                "height": 360
            },
            {
                "format_id": "hls-1570",
                "vcodec": "avc1.64001F",
                "acodec": "none",
                "ext": "mp4",
                "width": 1280,
                "height": 720
            }
        ]
    });
    let mut formats = parse_formats(&json);
    format_selection::append_merged_format(&mut formats);
    // 合成格式应在第一位，且 height=720（最高视频流）
    assert_eq!(formats[0].id, "hls-1570+hls-audio-128000-Audio");
    assert_eq!(
        formats[0].height,
        Some(720),
        "合成格式应携带最高视频流的 height"
    );
    assert_eq!(formats[0].width, Some(1280));
    assert!(formats[0].has_video);
    assert!(formats[0].has_audio);
    assert!(formats[0].requires_ffmpeg);
}

/// Task 42：图集类型 URL 在 formats 不含图片项时，应从 thumbnails 兜底提取。
/// 模拟抖音 note URL 的 yt-dlp 输出：formats 为空数组，thumbnails 含 3 个图片。
/// `extract_thumbnail_images` 应返回 3 个 `MediaFormat`，每个 `image_url` 非空。
#[test]
fn extract_thumbnail_images_fallback_when_formats_has_no_image() {
    let json = serde_json::json!({
        "formats": [],
        "thumbnails": [
            {"id": "0", "url": "https://example.com/thumb0.jpg", "width": 720, "height": 1280},
            {"id": "1", "url": "https://example.com/thumb1.jpg"},
            {"id": "2", "url": ""}
        ]
    });
    let images = extract_thumbnail_images(&json);
    // 第 3 项 url 为空，应被跳过；最终解析出 2 个图片
    assert_eq!(images.len(), 2, "应跳过 url 为空的 thumbnail");
    assert_eq!(images[0].id, "image-0");
    assert_eq!(images[0].width, Some(720));
    assert_eq!(images[0].height, Some(1280));
    assert_eq!(
        images[0].image_url.as_deref(),
        Some("https://example.com/thumb0.jpg")
    );
    assert_eq!(images[1].id, "image-1");
    assert_eq!(
        images[1].image_url.as_deref(),
        Some("https://example.com/thumb1.jpg")
    );
    // 第 2 项 label 应显示 "图片 2"（idx+1）
    assert_eq!(images[1].label, "图片 2");
}

/// Task 42：`extract_thumbnail_images` 在 thumbnails 缺失时应返回空列表。
#[test]
fn extract_thumbnail_images_returns_empty_when_no_thumbnails() {
    let json = serde_json::json!({"formats": []});
    let images = extract_thumbnail_images(&json);
    assert!(images.is_empty(), "无 thumbnails 时应返回空列表");

    // thumbnails 为非数组类型时也应安全返回空
    let json2 = serde_json::json!({"thumbnails": "not-an-array"});
    let images2 = extract_thumbnail_images(&json2);
    assert!(images2.is_empty(), "thumbnails 非数组时应返回空列表");
}
