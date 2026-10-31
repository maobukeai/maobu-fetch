use crate::{
    models::{MediaFormat, MediaProbeResult, MediaType},
    pikpak::PikPakFileItem,
};

fn extension(file: &PikPakFileItem) -> Option<String> {
    file.file_extension
        .as_deref()
        .or_else(|| {
            std::path::Path::new(&file.name)
                .extension()
                .and_then(|value| value.to_str())
        })
        .map(|value| value.trim_start_matches('.').to_ascii_lowercase())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 12
                && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

pub(super) fn result(
    file: &PikPakFileItem,
    direct: &str,
    ffmpeg_available: bool,
) -> Result<MediaProbeResult, String> {
    if file.kind != "drive#file" {
        return Err("该 PikPak 条目是目录，请使用网盘文件选择器".into());
    }
    let extension = extension(file);
    let mime = file.mime_type.as_deref().unwrap_or("");
    let media_type = if mime.starts_with("audio/")
        || matches!(
            extension.as_deref(),
            Some("mp3" | "m4a" | "aac" | "flac" | "wav" | "ogg" | "opus")
        ) {
        MediaType::Audio
    } else if mime.starts_with("image/")
        || matches!(
            extension.as_deref(),
            Some("jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp")
        )
    {
        MediaType::Gallery
    } else if mime.starts_with("video/")
        || matches!(
            extension.as_deref(),
            Some("mp4" | "mkv" | "webm" | "avi" | "mov" | "flv" | "ts" | "m4v")
        )
    {
        MediaType::Video
    } else {
        return Err("该 PikPak 分享是普通文件，不能当作视频解析；请使用已有网盘文件选择器或原生 HTTP 原文件下载".into());
    };
    let url = url::Url::parse(direct)
        .ok()
        .filter(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none()
        })
        .ok_or("未取得有效原文件地址，请在网盘文件选择器中重试")?;
    Ok(MediaProbeResult {
        is_live: false,
        title: if file.name.trim().is_empty() {
            "PikPak 原文件".into()
        } else {
            file.name.clone()
        },
        thumbnail: file.thumbnail_url.clone(),
        extractor: Some("pikpak".into()),
        duration: None,
        formats: vec![MediaFormat {
            original_file: true,
            bitrate_kbps: None,
            id: "original".into(),
            label: "原文件（音视频属性待检测）".into(),
            extension,
            width: None,
            height: None,
            file_size: (file.size > 0).then_some(file.size),
            file_size_approximate: false,
            has_video: false,
            has_audio: false,
            audio_unknown: true,
            requires_ffmpeg: false,
            image_url: (media_type == MediaType::Gallery).then(|| url.to_string()),
            url: Some(url.into()),
        }],
        subtitles: Vec::new(),
        drm: false,
        media_type,
        episodes: Vec::new(),
        ffmpeg_available,
    })
}

/// Separate from the audible-video default: only a declared original-file response has this path.
pub(crate) fn select_original_file_format(
    result: &MediaProbeResult,
) -> Result<&MediaFormat, String> {
    if result
        .extractor
        .as_deref()
        .is_none_or(|extractor| !extractor.eq_ignore_ascii_case("pikpak"))
    {
        return Err("当前结果不是明确的网盘原文件入口".into());
    }
    let mut formats = result.formats.iter().filter(|format| format.original_file);
    let selected = formats.next().ok_or("没有可选择的原文件")?;
    if formats.next().is_some() {
        return Err("包含多个原文件，请在网盘文件选择器中明确选择".into());
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(name: &str) -> PikPakFileItem {
        PikPakFileItem {
            id: "fixture".into(),
            name: name.into(),
            kind: "drive#file".into(),
            size: 123,
            path: name.into(),
            mime_type: None,
            file_extension: None,
            thumbnail_url: None,
            web_content_link: None,
        }
    }
    #[test]
    fn real_original_metadata_never_invents_quality_or_tracks() {
        let result = result(
            &file("actual-360p.mp4"),
            "https://cdn.example.test/file",
            false,
        )
        .unwrap();
        let format = select_original_file_format(&result).unwrap();
        assert_eq!(result.title, "actual-360p.mp4");
        assert_eq!(format.extension.as_deref(), Some("mp4"));
        assert_eq!(format.file_size, Some(123));
        assert!(
            format.original_file && !format.has_video && !format.has_audio && format.audio_unknown
        );
        assert!(format.width.is_none() && format.height.is_none() && format.bitrate_kbps.is_none());
        assert!(super::super::select_default_video_format(&result.formats, true).is_err());
    }
    #[test]
    fn ordinary_files_and_folders_are_not_returned_as_video() {
        assert!(result(&file("archive.zip"), "https://cdn.example.test/file", true).is_err());
        let mut folder = file("video.mp4");
        folder.kind = "drive#folder".into();
        assert!(result(&folder, "https://cdn.example.test/file", true).is_err());
        let audio = result(&file("song.flac"), "https://cdn.example.test/file", false).unwrap();
        assert_eq!(audio.media_type, MediaType::Audio);
        assert!(
            !audio.formats[0].has_audio,
            "audio track presence is still unprobed"
        );
    }
}
