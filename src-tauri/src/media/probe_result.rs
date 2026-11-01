use super::*;

pub(super) fn parse(
    value: Value,
    effective_url: &str,
    platform: MediaPlatform,
    include_collection: bool,
    ffmpeg_available: bool,
) -> Result<MediaProbeResult, String> {
    drm::ensure_unprotected(&value)?;
    let drm = false;
    // Task 37.4 / Task 38.1：识别抖音与 TikTok 图集类型，记录日志供后续处理参考。
    // yt-dlp 对图集的支持有限，识别后由前端据 extractor 字段决定后续处理。
    // Task 38：图集类型映射到 `MediaType::Gallery`，前端据此选择多图下载策略。
    // Task 39.1：识别 Twitter Spaces 音频 URL，映射到 `MediaType::Audio`，
    // 前端据此展示音频流选择，并提醒用户 Spaces 通常需要登录态。
    // Task 42：识别逻辑提取为 `determine_media_type` 纯函数，便于单元测试，
    // 补充微博图集识别（`is_weibo_gallery`）。
    let media_type = determine_media_type(&effective_url);
    // 抖音直连解析后，API 返回的 aweme_detail.images[] 是图集的真实判定依据。
    // 不能仅靠 URL 路径判断，因为抖音短链可能展开为 /discover?modal_id=xxx
    // 等不含 /note/ 的路径，但 API 返回的 aweme_detail.images[] 仍为图集。
    // convert_douyin_aweme_to_yt_dlp_json 会设置 _is_douyin_gallery 标记。
    let media_type = if (value
        .get("_is_douyin_gallery")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || value
            .get("_is_twitter_gallery")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || value
            .get("_is_xiaohongshu_gallery")
            .and_then(Value::as_bool)
            .unwrap_or(false))
        && media_type == MediaType::Video
    {
        MediaType::Gallery
    } else {
        media_type
    };
    match media_type {
        MediaType::Gallery => {
            tracing::info!(platform = ?platform, "检测到图集类型 URL");
        }
        MediaType::Audio => {
            tracing::info!(platform = ?platform, "检测到音频类型 URL");
        }
        _ => {}
    }
    if include_collection && platform == MediaPlatform::Bilibili {
        if let Some(collection) = collection::parse_collection(&value, ffmpeg_available)? {
            return Ok(collection);
        }
    }
    // Task 42：formats 解析提取为 `parse_formats` 纯函数，便于单元测试。
    // 同时识别图片格式项（vcodec=none, acodec=none, ext 为图片扩展名），
    // 填充 `image_url` 字段供图集场景前端展示。
    let mut formats: Vec<MediaFormat> = parse_formats(&value);
    format_selection::append_merged_format(&mut formats);
    // Task 42：图集类型且 `formats` 中未含图片项时，从 `thumbnails` 数组补充。
    // 部分图集 URL（如抖音 note）yt-dlp 把图片放在 thumbnails 而非 formats，
    // 这里做兜底提取，保证前端始终能拿到图片直链列表。
    if media_type == MediaType::Gallery {
        let has_images = formats.iter().any(|f| f.image_url.is_some());
        if !has_images {
            formats.extend(extract_thumbnail_images(&value));
        }
    }
    let subtitles = value
        .get("subtitles")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    Ok(MediaProbeResult {
        is_live: live_marker::from_metadata(&value),
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("媒体下载")
            .to_string(),
        thumbnail: value
            .get("thumbnail")
            .and_then(Value::as_str)
            .map(str::to_owned),
        extractor: value
            .get("extractor_key")
            .or_else(|| value.get("extractor"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        duration: value.get("duration").and_then(Value::as_f64),
        formats,
        subtitles,
        drm,
        media_type,
        episodes: Vec::new(),
        ffmpeg_available,
    })
}
