use super::*;
use serde_json::json;

fn metadata(title: &str, audio: Value) -> Value {
    json!({"title":title,"formats":[{
        "format_id":title,"url":"https://cdn.example.test/video.mp4",
        "width":1280,"height":720,"vcodec":null,"acodec":audio,"ext":"mp4"
    }]})
}

#[test]
fn empty_native_accepts_playable_unknown_tool_metadata_without_claiming_audio() {
    let native = json!({"title":"native-title","formats":[]});
    assert!(needs_tool(&native).unwrap());
    let tool = metadata("tool-unknown", Value::Null);
    let selected = select(native, Some(Ok(tool.clone()))).unwrap();
    assert_eq!(selected, tool);
    let formats = parse_formats(&selected);
    assert!(formats[0].audio_unknown && !formats[0].has_audio);
    assert!(
        super::super::format_selection::select_default_video_format(&formats, true)
            .unwrap_err()
            .starts_with("MEDIA_AUDIO_UNKNOWN:")
    );
}

#[test]
fn unknown_native_prefers_explicit_tool_audio() {
    let native = metadata("native-unknown", Value::Null);
    assert!(needs_tool(&native).unwrap());
    let tool = metadata("tool-confirmed", json!("mp4a.40.2"));
    assert_eq!(select(native, Some(Ok(tool.clone()))).unwrap(), tool);
}

#[test]
fn empty_invalid_or_failed_tool_preserves_existing_native_candidates() {
    for tool in [
        Ok(json!({"formats":[]})),
        Ok(json!({"formats":[{"format_id":"no-url","vcodec":"h264","acodec":"aac"}]})),
        Ok(json!({"formats":[{"format_id":"bad-url","url":"file:///private.mp4","acodec":"aac"}]})),
        Ok(Value::Null),
        Err("媒体工具返回无效元数据".into()),
    ] {
        let native = metadata("native-unknown", Value::Null);
        assert_eq!(select(native.clone(), Some(tool)).unwrap(), native);
    }
}

#[test]
fn unknown_tool_does_not_replace_existing_unknown_native() {
    let native = metadata("native", Value::Null);
    let tool = metadata("tool", Value::Null);
    assert_eq!(select(native.clone(), Some(Ok(tool))).unwrap(), native);
}

#[test]
fn both_empty_or_unavailable_returns_an_actionable_error() {
    for tool in [None, Some(Ok(json!({"formats":[]})))] {
        assert!(select(json!({"formats":[]}), tool)
            .unwrap_err()
            .starts_with("MEDIA_FORMATS_MISSING:"));
    }
    let error = "媒体分析网络不可用".to_owned();
    assert_eq!(
        select(json!({"formats":[]}), Some(Err(error.clone()))).unwrap_err(),
        error
    );
}

#[test]
fn drm_on_either_result_or_tool_error_cannot_be_discarded() {
    for protected in [
        json!({"has_drm":true,"formats":[]}),
        json!({"formats":[{"has_drm":true}]}),
        json!({"requested_formats":[{"_has_drm":true}]}),
    ] {
        assert!(needs_tool(&protected)
            .unwrap_err()
            .starts_with("MEDIA_DRM_PROTECTED:"));
        assert!(
            select(protected.clone(), Some(Ok(metadata("clear", json!("aac")))))
                .unwrap_err()
                .starts_with("MEDIA_DRM_PROTECTED:")
        );
        assert!(select(metadata("clear", Value::Null), Some(Ok(protected)))
            .unwrap_err()
            .starts_with("MEDIA_DRM_PROTECTED:"));
    }
    assert!(select(
        metadata("native", Value::Null),
        Some(Err("MEDIA_DRM_PROTECTED: 已拒绝".into()))
    )
    .unwrap_err()
    .starts_with("MEDIA_DRM_PROTECTED:"));
}

#[test]
fn native_images_and_confirmed_audio_do_not_trigger_optional_probe() {
    let confirmed = metadata("confirmed", json!("aac"));
    assert!(!needs_tool(&confirmed).unwrap());
    assert_eq!(select(confirmed.clone(), None).unwrap(), confirmed);
    let gallery = json!({"formats":[{
        "format_id":"image-1","vcodec":"none","acodec":"none",
        "ext":"jpeg","url":"https://cdn.example.test/image.jpeg"
    }]});
    assert!(!needs_tool(&gallery).unwrap());
    assert_eq!(select(gallery.clone(), None).unwrap(), gallery);
}

#[test]
fn cancellation_is_not_swallowed_by_optional_tool_fallback() {
    for error in ["任务已暂停", "任务已取消"] {
        assert_eq!(
            select(metadata("native", Value::Null), Some(Err(error.into()))).unwrap_err(),
            error
        );
    }
}
