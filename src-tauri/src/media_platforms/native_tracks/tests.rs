use super::*;
use serde_json::json;

#[test]
fn container_flags_and_music_never_prove_codecs() {
    let codecs = read(
        &json!({
            "content_type":"video/mp4", "is_h265":false, "is_bytevc1":true,
            "width":1920,"height":1080,"has_audio":true,"volume":1,
            "music":{"audio_codec":"aac"},"url":"https://example.test/avc1/aac.mp4"
        }),
        None,
    );
    assert!(codecs.video.is_none());
    assert!(codecs.audio.is_none());
}

#[test]
fn explicit_codec_strings_and_absent_track_are_preserved() {
    for (video, audio) in [
        ("vcodec", "acodec"),
        ("video_codec", "audio_codec"),
        ("videoCodec", "audioCodec"),
    ] {
        let codecs = read(&json!({video:"hvc1.1.6.L93.B0",audio:"mp4a.40.2"}), None);
        assert_eq!(codecs.video.as_deref(), Some("hvc1.1.6.L93.B0"));
        assert_eq!(codecs.audio.as_deref(), Some("mp4a.40.2"));
    }
    let primary = json!({"acodec":" NONE "});
    let fallback = json!({"acodec":"aac","vcodec":"avc1.640028"});
    let codecs = read(&primary, Some(&fallback));
    assert_eq!(codecs.audio.as_deref(), Some("none"));
    assert_eq!(codecs.video.as_deref(), Some("avc1.640028"));
}

#[test]
fn invalid_or_unknown_codec_values_remain_unknown() {
    for value in [
        Value::Null,
        json!(0),
        json!(false),
        json!(""),
        json!("unknown"),
        json!("N/A"),
        json!("null"),
        json!("undefined"),
        json!("0"),
        json!("aac\nunsafe"),
    ] {
        let codecs = read(&json!({"vcodec":value,"acodec":value}), None);
        assert!(codecs.video.is_none());
        assert!(codecs.audio.is_none());
    }
}

#[test]
fn ssr_variant_codecs_follow_the_selected_url_only() {
    let source = json!({
        "videoCodec":"h264", "audioCodec":"aac",
        "playAddr":[
            {"src":"https://example.test/api","videoCodec":"avc1.640028","audioCodec":"aac"},
            {"src":"//cdn.example.test/selected","videoCodec":"hev1.1.6.L93.B0","audioCodec":"none"}
        ]
    });
    let mut target = json!({"play_addr":{"url_list":["https://cdn.example.test/selected"]}});
    copy_douyin_ssr(&source, &mut target);
    let codecs = read(&target["play_addr"], Some(&target));
    assert_eq!(codecs.video.as_deref(), Some("hev1.1.6.L93.B0"));
    assert_eq!(codecs.audio.as_deref(), Some("none"));

    let mixed = json!({
        "play_addr":{"url_list":["https://example.test/not-selected"],"acodec":"aac"},
        "playAddr":[{"src":"//cdn.example.test/selected"}]
    });
    let mut target = json!({"play_addr":{"url_list":["https://cdn.example.test/selected"]}});
    copy_douyin_ssr(&mixed, &mut target);
    assert!(read(&target["play_addr"], None).audio.is_none());
}
