mod auth_scope;
mod cached_naming;
#[cfg(test)]
mod cached_naming_tests;
pub use cached_naming::{need_native_naming_probe, prepare_native_output_name};
mod collection;
mod command_config;
mod download;
mod download_process;
mod download_progress;
mod drm;
pub use download::download;
mod format_selection;
mod format_size;
mod live_output;
mod live_marker;
mod metadata_client;
pub(crate) use metadata_client::metadata_client;
mod native_file;
mod native_probe_fallback;
pub use native_file::prepare_native_media_file;
mod network_policy;
pub use live_output::{finalize_live_task_output, task_output_is_live};
mod original_file;
mod output;
#[cfg(test)]
mod output_tests;
mod path_safety;
pub(crate) use original_file::select_original_file_format;
#[cfg(test)]
pub(crate) mod bilibili_sample;
#[cfg(test)]
pub(crate) mod public_samples;
#[cfg(test)]
pub(crate) mod execution_e2e;
#[cfg(test)]
pub(crate) mod execution_fixtures;
#[cfg(test)]
mod execution_negative;
#[cfg(test)]
mod execution_proxy_test;
mod probe_cache;
mod process;
#[cfg(test)]
mod sample_validation;
mod subtitles;
mod workspace_cleanup;
pub use workspace_cleanup::cleanup_task_workspace;
#[cfg(test)]
mod test_support;
mod traffic_proxy;
pub(crate) use format_selection::select_default_video_format;

use crate::{
    manager::{
        diagnose::{classify_platform_error, platform_error_to_chinese_with_evidence},
        naming_template::{apply_naming_template, find_template_for_platform, NamingVars},
    },
    media_platforms::{
        convert_douyin_aweme_to_yt_dlp_json, convert_douyin_live_to_yt_dlp_json, detect_platform,
        extract_douyin_aweme_id, extract_douyin_live_room_id, extract_url_from_share_text,
        is_douyin_gallery, is_douyin_live, is_tiktok_gallery, is_twitter_space, is_weibo_gallery,
        strip_tracking_params, MediaPlatform,
    },
    media_tools::{create_hidden_tokio_command, resolve_ffmpeg, resolve_yt_dlp},
    models::AppSettings,
    models::{
        DownloadTask, MediaFormat, MediaProbeResult, MediaType, PlatformNamingTemplate, TaskStatus,
    },
    portable,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

mod auth;
use auth::split_auth_headers;
pub(crate) use auth::{attach_auth_args_in_dir, build_youtube_extractor_arg};

mod probe_platforms;
mod probe_result;
mod probe_runtime;
pub use probe_runtime::{probe, probe_collection, probe_for_task};

#[cfg(test)]
fn media_arguments(format: &str, template: &str, has_ffmpeg: bool) -> Vec<String> {
    download::arguments(format, template, has_ffmpeg)
}

/// 从 yt-dlp `--dump-single-json` 输出构建命名模板变量集合。
///
/// 字段映射（AGENTS.md §3：所有变量来自真实状态）：
/// - `author`：`uploader` > `channel` > `uploader_id`（优先级回退）
/// - `title`：`title`
/// - `date`：`upload_date`（yt-dlp 已格式化为 `YYYYMMDD`）
/// - `platform`：`MediaPlatform::as_str()`
/// - `id`：`id`（站点视频 ID）
/// - `channel`：`channel`
/// - `bvid`：`display_id`（B 站场景下为 BV 号；非 B 站通常与 `id` 相同或缺失，
///   模板不使用 `{bvid}` 时不影响结果）
fn build_naming_vars(value: &Value, platform: MediaPlatform) -> NamingVars {
    let str_field = |key: &str| -> Option<String> {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let author = str_field("uploader")
        .or_else(|| str_field("channel"))
        .or_else(|| str_field("uploader_id"));
    NamingVars {
        author,
        title: str_field("title"),
        date: str_field("upload_date"),
        platform: Some(platform.as_str().to_string()),
        id: str_field("id"),
        channel: str_field("channel"),
        bvid: str_field("display_id"),
    }
}

/// Task 42：根据 URL 模式判定媒体内容类型（纯函数）。
///
/// 识别规则：
/// - `Gallery`：抖音 note / TikTok photo / 微博 album 图集 URL
/// - `Audio`：Twitter Spaces 音频 URL
/// - `Video`：其它所有 URL（默认值，向后兼容）
///
/// 提取为纯函数以便单元测试，无需启动 yt-dlp 子进程即可验证识别逻辑。
/// AGENTS.md §3：识别基于真实 URL 模式，不使用模拟数据。
fn determine_media_type(url: &str) -> MediaType {
    if is_douyin_gallery(url) || is_tiktok_gallery(url) || is_weibo_gallery(url) {
        MediaType::Gallery
    } else if is_twitter_space(url) {
        MediaType::Audio
    } else {
        MediaType::Video
    }
}

/// Task 42：从 yt-dlp JSON 输出中解析格式列表（纯函数）。
///
/// 同时识别视频/音频格式与图片格式（图集场景）。图片项的判定：
/// - `vcodec == "none"` 且 `acodec == "none"`
/// - `ext` 为 jpg/jpeg/png/webp/gif/bmp 之一（大小写不敏感）
///
/// 图片项填充 `image_url` 字段；视频/音频项的 `image_url` 始终为 `None`。
/// 提取为纯函数便于单元测试，不依赖网络或子进程。
fn parse_formats(value: &Value) -> Vec<MediaFormat> {
    value
        .get("formats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(parse_single_format)
        .collect()
}

/// Task 42：解析单个 yt-dlp format 项为 `MediaFormat`。
///
/// 同时识别图片项并填充 `image_url`，便于图集场景前端展示。
/// 缺失的 `vcodec` / `acodec` 保持未知，明确 `"none"` 才表示没有对应轨道；
/// `format_id` 缺失时返回 `None`，跳过该格式项。
///
/// 来源未声明音频编解码时保持音轨未知；尺寸和格式ID不能证明有声。
/// 未知格式可由用户明确选择原文件，不能自动作为已确认有声视频。
fn parse_single_format(item: &Value) -> Option<MediaFormat> {
    let id = item.get("format_id")?.as_str()?.to_string();
    // vcodec/acodec 保留 Option 以区分三种状态：
    //   Some("none") = 明确无此轨；Some("avc1...") = 有此轨；None = JSON null/缺失（未知）
    // yt-dlp 对 Twitter http-* progressive 流返回 vcodec/acodec 为 null（而非 "none"），
    // 旧逻辑把 null 当 "none" 处理，导致最高画质有声直链被误判为"无视频无音频"。
    let vcodec: Option<&str> = item.get("vcodec").and_then(Value::as_str);
    let acodec: Option<&str> = item.get("acodec").and_then(Value::as_str);
    let width = item.get("width").and_then(Value::as_u64);
    let height = item.get("height").and_then(Value::as_u64);
    let ext = item.get("ext").and_then(Value::as_str).map(str::to_owned);
    let (size, file_size_approximate) = format_size::parse(item);
    let url = item.get("url").and_then(Value::as_str).map(str::to_owned);
    let ext_lower = ext.as_deref().map(|e| e.to_ascii_lowercase());
    let format_note = item.get("format_note").and_then(Value::as_str);
    // 过滤 YouTube 预览图/底片流（storyboard）：
    // yt-dlp 对 YouTube 序列帧图片格式返回 format_id 为 sb3/sb2/sb1/sb0 或 format_note 为 "storyboard"
    if format_note == Some("storyboard")
        || id.starts_with("sb0")
        || id.starts_with("sb1")
        || id.starts_with("sb2")
        || id.starts_with("sb3")
        || ext_lower.as_deref() == Some("mhtml")
    {
        return None;
    }
    // 明确无此轨（yt-dlp 返回字符串 "none"）
    let vcodec_is_explicit_none = vcodec.is_some_and(|codec| codec.eq_ignore_ascii_case("none"));
    let acodec_is_explicit_none = acodec.is_some_and(|codec| codec.eq_ignore_ascii_case("none"));
    // 未知：JSON null/缺失/空字符串（yt-dlp 对 progressive 流不返回编解码信息）
    let unknown = |codec: Option<&str>| {
        codec.is_none_or(|codec| {
            codec.trim().is_empty()
                || codec.eq_ignore_ascii_case("unknown")
                || codec.eq_ignore_ascii_case("n/a")
        })
    };
    let vcodec_is_unknown = unknown(vcodec);
    let acodec_is_unknown = unknown(acodec);
    let has_dimensions = width.is_some() || height.is_some();
    // 图片项：vcodec/acodec 均明确为 "none" 且扩展名为图片格式（优先判定，避免被 has_dimensions 误判为视频）
    let is_image = vcodec_is_explicit_none
        && acodec_is_explicit_none
        && matches!(
            ext_lower.as_deref(),
            Some("jpg") | Some("jpeg") | Some("png") | Some("webp") | Some("gif") | Some("bmp")
        );
    // has_video: 明确有编解码，或有画面尺寸（排除图片）
    let has_video = !is_image && !vcodec_is_explicit_none && (!vcodec_is_unknown || has_dimensions);
    // Unknown codecs never prove an audio track, including dimensioned progressive streams.
    let has_audio = !is_image && !acodec_is_explicit_none && !acodec_is_unknown;
    let audio_unknown = !is_image && acodec_is_unknown;
    let label = if is_image {
        match (width, height) {
            (Some(w), Some(h)) => format!("图片 {w}×{h}"),
            _ => "图片".to_string(),
        }
    } else {
        item.get("format_note")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| match (height, ext.as_deref()) {
                (Some(h), Some(ext)) => format!("{h}p · {ext}"),
                (None, Some(ext)) => ext.to_string(),
                _ => id.clone(),
            })
    };
    Some(MediaFormat {
        id,
        label,
        extension: ext,
        width,
        height,
        file_size: size,
        file_size_approximate,
        original_file: false,
        bitrate_kbps: format_selection::parse_bitrate_kbps(item, has_video, has_audio),
        has_video,
        has_audio,
        audio_unknown,
        requires_ffmpeg: false,
        image_url: if is_image { url.clone() } else { None },
        url,
    })
}

/// Task 42：从 yt-dlp `thumbnails` 数组中提取图片格式项（纯函数）。
///
/// 仅在 `formats` 中未含图片项时（图集类型 URL 但 yt-dlp 把图片放在 thumbnails）使用。
/// 每个含非空 `url` 字段 of thumbnail 转换为一个 `MediaFormat`，`id` 形如 `image-<idx>`，
/// `extension` 默认 `"jpg"`（与抖音/TikTok 图集常见格式一致）。
fn extract_thumbnail_images(value: &Value) -> Vec<MediaFormat> {
    let mut result = Vec::new();
    let thumbnails = match value.get("thumbnails").and_then(Value::as_array) {
        Some(arr) => arr,
        None => return result,
    };
    for (idx, thumb) in thumbnails.iter().enumerate() {
        let url = match thumb.get("url").and_then(Value::as_str) {
            Some(u) if !u.is_empty() => u.to_owned(),
            _ => continue,
        };
        let width = thumb.get("width").and_then(Value::as_u64);
        let height = thumb.get("height").and_then(Value::as_u64);
        let label = match (width, height) {
            (Some(w), Some(h)) => format!("图片 {w}×{h}"),
            _ => format!("图片 {}", idx + 1),
        };
        result.push(MediaFormat {
            id: format!("image-{idx}"),
            label,
            extension: Some("jpg".to_string()),
            width,
            height,
            file_size: None,
            file_size_approximate: false,
            original_file: false,
            bitrate_kbps: None,
            has_video: false,
            has_audio: false,
            audio_unknown: false,
            requires_ffmpeg: false,
            image_url: Some(url.clone()),
            url: Some(url),
        });
    }
    result
}

#[cfg(test)]
mod legacy_tests;
