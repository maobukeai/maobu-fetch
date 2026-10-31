use crate::models::MediaFormat;
use std::cmp::Ordering;

fn bitrate(format: &MediaFormat) -> Option<f64> {
    format
        .bitrate_kbps
        .filter(|rate| rate.is_finite() && *rate > 0.0)
}

pub(super) fn parse_bitrate_kbps(
    item: &serde_json::Value,
    video: bool,
    audio: bool,
) -> Option<f64> {
    if !video && !audio {
        return None;
    }
    let rate = |key| {
        item.get(key)
            .and_then(serde_json::Value::as_f64)
            .filter(|rate| rate.is_finite() && *rate > 0.0)
    };
    rate("tbr")
        .or_else(|| match (video, audio) {
            (true, true) => rate("vbr")
                .zip(rate("abr"))
                .map(|(video, audio)| video + audio),
            (true, false) => rate("vbr"),
            (false, true) => rate("abr"),
            _ => None,
        })
        .filter(|rate| rate.is_finite() && *rate > 0.0)
}

fn compare_quality(left: &MediaFormat, right: &MediaFormat) -> Ordering {
    left.height
        .cmp(&right.height)
        .then(left.width.cmp(&right.width))
        // 缺少码率保持 None；已知层按真实值排序，不把未知数据伪造为 0 kbps。
        .then_with(|| match (bitrate(left), bitrate(right)) {
            (Some(left), Some(right)) => left.total_cmp(&right),
            (Some(_), None) => Ordering::Greater,
            (None, Some(_)) => Ordering::Less,
            (None, None) => Ordering::Equal,
        })
        .then((!left.requires_ffmpeg).cmp(&!right.requires_ffmpeg))
        .then(left.file_size.cmp(&right.file_size))
}

/// 只有实际解析到分离音轨与视频流时才提供合并候选。
pub(super) fn append_merged_format(formats: &mut Vec<MediaFormat>) {
    let Some(video) = formats
        .iter()
        .filter(|format| format.has_video && !format.has_audio && !format.audio_unknown)
        .max_by(|left, right| compare_quality(left, right))
    else {
        return;
    };
    let Some(audio) = formats
        .iter()
        .filter(|format| format.has_audio && !format.has_video && !format.audio_unknown)
        .max_by(|left, right| compare_quality(left, right))
    else {
        return;
    };
    let combined_size = video
        .file_size
        .zip(audio.file_size)
        .and_then(|(video, audio)| video.checked_add(audio));
    let merged = MediaFormat {
        original_file: false,
        id: format!("{}+{}", video.id, audio.id),
        label: "最高画质（合并音视频，需要 FFmpeg）".into(),
        extension: Some("mp4".into()),
        width: video.width,
        height: video.height,
        bitrate_kbps: bitrate(video)
            .zip(bitrate(audio))
            .map(|(video, audio)| video + audio)
            .filter(|rate| rate.is_finite() && *rate > 0.0),
        file_size: combined_size,
        file_size_approximate: combined_size.is_some(),
        has_video: true,
        has_audio: true,
        audio_unknown: false,
        requires_ffmpeg: true,
        image_url: None,
        url: None,
    };
    formats.insert(0, merged);
}

/// 默认视频必须具备音轨；FFmpeg 可用时选最佳画质，同分辨率优先直接有声文件。
pub(crate) fn select_default_video_format(
    formats: &[MediaFormat],
    ffmpeg_available: bool,
) -> Result<&MediaFormat, String> {
    let audible = formats
        .iter()
        .filter(|format| format.has_video && format.has_audio && !format.audio_unknown);
    if let Some(format) = audible
        .clone()
        .filter(|format| ffmpeg_available || !format.requires_ffmpeg)
        .max_by(|left, right| compare_quality(left, right))
    {
        return Ok(format);
    }
    if audible.clone().any(|format| format.requires_ffmpeg) {
        return Err(
            "MEDIA_FFMPEG_MISSING: 当前有声画质需要 FFmpeg 合并组件，请先安装后重试".into(),
        );
    }
    if formats.iter().any(|format| format.audio_unknown) {
        return Err("MEDIA_AUDIO_UNKNOWN: 来源未确认音轨，不能自动当作有声视频；可手动选择音轨未确认的原文件，或更新媒体组件后重新分析".into());
    }
    Err(
        "MEDIA_AUDIO_MISSING: 未找到带音轨的视频格式；如需无声视频或纯音频，请明确选择对应格式"
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(id: &str, height: u64, video: bool, audio: bool, merge: bool) -> MediaFormat {
        MediaFormat {
            id: id.into(),
            label: id.into(),
            extension: Some("mp4".into()),
            width: None,
            height: Some(height),
            original_file: false,
            bitrate_kbps: None,
            file_size: None,
            file_size_approximate: false,
            has_video: video,
            has_audio: audio,
            audio_unknown: false,
            requires_ffmpeg: merge,
            image_url: None,
            url: None,
        }
    }

    #[test]
    fn silent_high_resolution_does_not_override_audible_video() {
        let formats = [
            format("silent-4k", 2160, true, false, false),
            format("av-360", 360, true, true, false),
        ];
        assert_eq!(
            select_default_video_format(&formats, true).unwrap().id,
            "av-360"
        );
    }

    #[test]
    fn unknown_audio_never_proves_audible_or_supplies_a_merged_track() {
        let mut unknown = format("unknown-4k", 2160, true, true, false);
        unknown.audio_unknown = true;
        let known = format("known-360", 360, true, true, false);
        assert_eq!(
            select_default_video_format(&[unknown.clone(), known], true)
                .unwrap()
                .id,
            "known-360"
        );
        assert!(select_default_video_format(&[unknown.clone()], true)
            .unwrap_err()
            .contains("未确认"));
        unknown.has_audio = false;
        let mut formats = vec![unknown, format("audio", 0, false, true, false)];
        append_merged_format(&mut formats);
        assert_eq!(formats.len(), 2);
    }

    #[test]
    fn ffmpeg_enables_higher_quality_merge_without_silent_downgrade() {
        let formats = [
            format("av-360", 360, true, true, false),
            format("merged-1080", 1080, true, true, true),
        ];
        assert_eq!(
            select_default_video_format(&formats, true).unwrap().id,
            "merged-1080"
        );
        assert_eq!(
            select_default_video_format(&formats, false).unwrap().id,
            "av-360"
        );
    }

    #[test]
    fn equal_quality_prefers_direct_audio_video_and_missing_audio_is_explicit() {
        let formats = [
            format("direct", 1080, true, true, false),
            format("merged", 1080, true, true, true),
        ];
        assert_eq!(
            select_default_video_format(&formats, true).unwrap().id,
            "direct"
        );
        assert!(
            select_default_video_format(&[format("silent", 1080, true, false, false)], true)
                .unwrap_err()
                .contains("MEDIA_AUDIO_MISSING")
        );
        assert!(
            select_default_video_format(&[format("audio", 0, false, true, false)], true).is_err()
        );
        assert!(
            select_default_video_format(&[format("merge", 1080, true, true, true)], false)
                .unwrap_err()
                .contains("MEDIA_FFMPEG_MISSING")
        );
    }

    #[test]
    fn actual_split_streams_form_an_audible_candidate_with_real_dimensions() {
        let mut video = format("video", 1080, true, false, false);
        video.width = Some(1920);
        video.file_size = Some(80);
        let mut audio = format("audio", 0, false, true, false);
        audio.file_size = Some(20);
        let mut formats = vec![video.clone(), audio];
        append_merged_format(&mut formats);
        assert_eq!(formats[0].height, Some(1080));
        assert_eq!(formats[0].width, Some(1920));
        assert_eq!(formats[0].file_size, Some(100));
        assert!(formats[0].has_audio && formats[0].requires_ffmpeg);
        assert_eq!(
            select_default_video_format(&formats, true).unwrap().id,
            formats[0].id
        );
        let mut silent = vec![video];
        append_merged_format(&mut silent);
        assert_eq!(silent.len(), 1);
        assert!(!silent[0].has_audio);
    }

    #[test]
    fn known_bitrate_precedes_direct_preference_without_inventing_unknown_rates() {
        let mut direct = format("direct", 1080, true, true, false);
        direct.bitrate_kbps = Some(1000.0);
        let mut merged = format("merge", 1080, true, true, true);
        merged.bitrate_kbps = Some(3000.0);
        let formats = [direct.clone(), merged];
        assert_eq!(
            select_default_video_format(&formats, true).unwrap().id,
            "merge"
        );
        assert_eq!(
            select_default_video_format(&formats, false).unwrap().id,
            "direct"
        );
        direct.bitrate_kbps = None;
        assert_eq!(bitrate(&direct), None);
        direct.bitrate_kbps = Some(f64::NAN);
        assert_eq!(bitrate(&direct), None);
    }

    #[test]
    fn bitrate_uses_only_actual_positive_metadata() {
        use serde_json::json;
        assert_eq!(
            parse_bitrate_kbps(&json!({"tbr":2200.5}), true, true),
            Some(2200.5)
        );
        assert_eq!(
            parse_bitrate_kbps(&json!({"vbr":2000,"abr":128}), true, true),
            Some(2128.0)
        );
        assert_eq!(parse_bitrate_kbps(&json!({"abr":128}), true, true), None);
        assert_eq!(
            parse_bitrate_kbps(&json!({"abr":128}), false, true),
            Some(128.0)
        );
        assert_eq!(
            parse_bitrate_kbps(&json!({"tbr":-1,"vbr":0}), true, false),
            None
        );
        assert_eq!(parse_bitrate_kbps(&json!({}), true, true), None);
    }
}
