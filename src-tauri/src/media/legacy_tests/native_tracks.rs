use super::*;
use crate::media_platforms::{
    convert_douyin_aweme_to_yt_dlp_json, convert_douyin_live_to_yt_dlp_json,
    convert_twitter_tweet_to_yt_dlp_json, normalize_douyin_ssr_detail,
};
use serde_json::json;

fn douyin(extra: Value) -> Value {
    let mut detail = json!({"aweme_detail":{"video":{
        "duration":2000,"width":1920,"height":1080,
        "play_addr":{"url_list":["https://cdn.example.test/video.mp4?codec=aac"]}
    },"music":{"audio_codec":"aac"}}});
    detail["aweme_detail"]["video"]
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    convert_douyin_aweme_to_yt_dlp_json(&detail)
}

#[test]
fn native_douyin_without_codec_evidence_cannot_auto_select_audible_video() {
    let converted = douyin(json!({"is_h265":false,"has_audio":true}));
    assert!(converted["formats"][0]["vcodec"].is_null());
    assert!(converted["formats"][0]["acodec"].is_null());
    let formats = parse_formats(&converted);
    assert!(formats[0].has_video);
    assert!(!formats[0].has_audio);
    assert!(formats[0].audio_unknown);
    assert!(
        formats[0].url.is_some(),
        "Unknown remains available for explicit selection"
    );
    for ffmpeg in [false, true] {
        assert!(select_default_video_format(&formats, ffmpeg)
            .unwrap_err()
            .starts_with("MEDIA_AUDIO_UNKNOWN:"));
    }
}

#[test]
fn native_douyin_explicit_codecs_are_retained_and_audible_is_selectable() {
    let converted = douyin(json!({"video_codec":"hvc1.1.6.L93.B0","audio_codec":"mp4a.40.2"}));
    assert_eq!(converted["formats"][0]["vcodec"], "hvc1.1.6.L93.B0");
    assert_eq!(converted["formats"][0]["acodec"], "mp4a.40.2");
    let formats = parse_formats(&converted);
    assert!(formats[0].has_video && formats[0].has_audio && !formats[0].audio_unknown);
    assert_eq!(
        select_default_video_format(&formats, false).unwrap().id,
        "play-0"
    );
}

#[test]
fn native_douyin_explicit_silence_is_not_unknown_or_audible() {
    let converted = douyin(json!({"vcodec":"avc1.640028","acodec":"none"}));
    let formats = parse_formats(&converted);
    assert!(formats[0].has_video && !formats[0].has_audio && !formats[0].audio_unknown);
    assert!(select_default_video_format(&formats, true)
        .unwrap_err()
        .starts_with("MEDIA_AUDIO_MISSING:"));
}

#[test]
fn native_douyin_ssr_keeps_only_selected_variant_codec_evidence() {
    let detail = json!({"awemeId":"12345","video":{
        "duration":2000,"width":1280,"height":720,"audioCodec":"aac",
        "playAddr":[
            {"src":"https://www.douyin.com/aweme/v1/play/?video_id=1","vcodec":"h264","acodec":"aac"},
            {"src":"//cdn.example.test/selected.mp4","videoCodec":"hevc","audioCodec":"none"}
        ]
    }});
    let converted = convert_douyin_aweme_to_yt_dlp_json(&normalize_douyin_ssr_detail(&detail));
    assert_eq!(
        converted["formats"][0]["url"],
        "https://cdn.example.test/selected.mp4"
    );
    assert_eq!(converted["formats"][0]["vcodec"], "hevc");
    assert_eq!(converted["formats"][0]["acodec"], "none");
    let formats = parse_formats(&converted);
    assert!(!formats[0].has_audio && !formats[0].audio_unknown);
}

#[test]
fn native_twitter_variant_mime_and_url_do_not_prove_audio_or_codec() {
    let mut detail = json!({"mediaDetails":[{"video_info":{
        "acodec":"aac",
        "variants":[
            {"content_type":"video/mp4","bitrate":5000000,"url":"https://cdn.example.test/avc1/1920x1080/high.mp4"},
            {"content_type":"video/mp4","bitrate":1000000,"url":"https://cdn.example.test/640x360/low.mp4"}
        ]
    }}]});
    let converted = convert_twitter_tweet_to_yt_dlp_json(&detail);
    let formats = parse_formats(&converted);
    assert!(formats
        .iter()
        .all(|format| !format.has_audio && format.audio_unknown));
    assert!(converted["formats"][0]["vcodec"].is_null());
    assert!(select_default_video_format(&formats, true)
        .unwrap_err()
        .starts_with("MEDIA_AUDIO_UNKNOWN:"));
    let known = &mut detail["mediaDetails"][0]["video_info"]["variants"][1];
    known["vcodec"] = json!("avc1.640028");
    known["acodec"] = json!("mp4a.40.2");
    let formats = parse_formats(&convert_twitter_tweet_to_yt_dlp_json(&detail));
    assert_eq!(
        select_default_video_format(&formats, true).unwrap().id,
        "http-360"
    );
}

#[test]
fn native_live_urls_alone_keep_unknown_codecs_and_bitrate() {
    let converted = convert_douyin_live_to_yt_dlp_json(&json!({
        "status":2,"flv_url":"https://cdn.example.test/live.flv",
        "hls_url":"https://cdn.example.test/live.m3u8"
    }));
    for format in converted["formats"].as_array().unwrap() {
        assert!(format["vcodec"].is_null() && format["acodec"].is_null());
        assert!(format.get("tbr").is_none());
    }
    let formats = parse_formats(&converted);
    assert!(formats
        .iter()
        .all(|format| format.audio_unknown && !format.has_audio));
    assert!(select_default_video_format(&formats, true)
        .unwrap_err()
        .starts_with("MEDIA_AUDIO_UNKNOWN:"));
}
