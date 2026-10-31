use crate::media_platforms::MediaPlatform;
use crate::models::{MediaEpisode, MediaProbeResult, MediaType};
use serde_json::Value;
use std::collections::HashSet;
use tokio::process::Command;

pub(super) const MAX_EPISODES: usize = 1000;

pub(super) fn configure_probe_playlist(
    command: &mut Command,
    url: &str,
    platform: MediaPlatform,
    include_collection: bool,
) {
    let explicit_part = url::Url::parse(url)
        .ok()
        .is_some_and(|url| url.query_pairs().any(|(key, _)| key == "p"));
    if include_collection && platform == MediaPlatform::Bilibili && !explicit_part {
        command
            .args(["--yes-playlist", "--flat-playlist", "--playlist-end"])
            .arg((MAX_EPISODES + 1).to_string());
    } else {
        command.arg("--no-playlist");
    }
}

/// B站提取器的平面 playlist 保留每一集真实 URL，不递归预取全部媒体流。
pub(super) fn parse_collection(
    value: &Value,
    ffmpeg_available: bool,
) -> Result<Option<MediaProbeResult>, String> {
    let Some(entries) = value.get("entries").and_then(Value::as_array) else {
        return Ok(None);
    };
    if entries.is_empty() {
        return Err("合集未返回可下载的分集，请检查登录状态、访问权限或分享链接".into());
    }
    let reported_count = value
        .get("playlist_count")
        .or_else(|| value.get("entry_count"))
        .and_then(Value::as_u64);
    if entries.len() > MAX_EPISODES
        || reported_count.is_some_and(|count| count > MAX_EPISODES as u64)
    {
        return Err(format!(
            "合集超过 {MAX_EPISODES} 集，请分批选择明确的分集链接，避免截断或误下载"
        ));
    }
    let mut indices = HashSet::new();
    let mut urls = HashSet::new();
    let mut episodes = Vec::with_capacity(entries.len());
    for (position, entry) in entries.iter().enumerate() {
        super::drm::ensure_unprotected(entry)?;
        let url = entry
            .get("webpage_url")
            .or_else(|| entry.get("url"))
            .and_then(Value::as_str)
            .and_then(|text| url::Url::parse(text).ok())
            .filter(|url| {
                matches!(url.scheme(), "http" | "https")
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.host_str().is_some_and(|host| {
                        host == "bilibili.com" || host.ends_with(".bilibili.com")
                    })
            })
            .ok_or_else(|| {
                format!(
                    "合集第 {} 项缺少有效分集链接，请检查权限后重试",
                    position + 1
                )
            })?;
        let index = entry
            .get("playlist_index")
            .and_then(Value::as_u64)
            .filter(|index| *index > 0 && *index <= u32::MAX as u64)
            .map(|index| index as u32)
            .unwrap_or(position as u32 + 1);
        if !indices.insert(index) || !urls.insert(url.to_string()) {
            return Err("合集返回重复编号或链接，无法可靠区分分集，请改用具体分集链接".into());
        }
        episodes.push(MediaEpisode {
            index,
            title: entry
                .get("title")
                .and_then(Value::as_str)
                .filter(|title| !title.trim().is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("第 {index} 集")),
            url: url.into(),
            duration: entry
                .get("duration")
                .and_then(Value::as_f64)
                .filter(|duration| duration.is_finite() && *duration >= 0.0),
        });
    }
    Ok(Some(MediaProbeResult {
        is_live: false,
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("视频合集")
            .into(),
        thumbnail: value
            .get("thumbnail")
            .and_then(Value::as_str)
            .map(str::to_owned),
        extractor: value
            .get("extractor_key")
            .or_else(|| value.get("extractor"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        duration: None,
        formats: Vec::new(),
        subtitles: Vec::new(),
        drm: false,
        media_type: MediaType::Collection,
        episodes,
        ffmpeg_available,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn playlist_expansion_requires_desktop_intent_and_preserves_explicit_part() {
        for (url, platform, include, expected) in [
            (
                "https://www.bilibili.com/video/BV1TEST",
                MediaPlatform::Bilibili,
                false,
                vec!["--no-playlist"],
            ),
            (
                "https://www.bilibili.com/video/BV1TEST?p=2",
                MediaPlatform::Bilibili,
                true,
                vec!["--no-playlist"],
            ),
            (
                "https://www.youtube.com/watch?v=test",
                MediaPlatform::YouTube,
                true,
                vec!["--no-playlist"],
            ),
            (
                "https://www.bilibili.com/video/BV1TEST",
                MediaPlatform::Bilibili,
                true,
                vec![
                    "--yes-playlist",
                    "--flat-playlist",
                    "--playlist-end",
                    "1001",
                ],
            ),
            (
                "https://www.bilibili.com/bangumi/play/ss123",
                MediaPlatform::Bilibili,
                true,
                vec![
                    "--yes-playlist",
                    "--flat-playlist",
                    "--playlist-end",
                    "1001",
                ],
            ),
        ] {
            let mut command = Command::new("yt-dlp");
            configure_probe_playlist(&mut command, url, platform, include);
            assert_eq!(
                command
                    .as_std()
                    .get_args()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn bilibili_flat_playlist_becomes_real_episode_choices() {
        let value = json!({ "_type": "playlist", "title": "多P示例", "extractor_key": "BiliBili", "entries": [
            {"_type":"url", "url":"https://www.bilibili.com/video/BV1TEST?p=1", "title":"开场", "duration":42},
            {"_type":"url", "url":"https://www.bilibili.com/video/BV1TEST?p=2", "title":"结束"}
        ] });
        let result = parse_collection(&value, true).unwrap().unwrap();
        assert_eq!(result.media_type, MediaType::Collection);
        assert_eq!(result.episodes.len(), 2);
        assert_eq!(result.episodes[1].index, 2);
        assert!(result.episodes[1].url.ends_with("?p=2"));
        assert_eq!(result.episodes[0].duration, Some(42.0));
        assert!(result.ffmpeg_available);
    }

    #[test]
    fn collection_rejects_missing_drm_invalid_duplicate_and_truncated_entries() {
        for value in [
            json!({"entries":[]}),
            json!({"entries":[null]}),
            json!({"entries":[{"url":"javascript:alert(1)"}]}),
            json!({"entries":[{"url":"https://www.bilibili.com/a","_has_drm":false,"has_drm":true}]}),
            json!({"entries":[{"url":"https://www.bilibili.com/a"},{"url":"https://www.bilibili.com/a"}]}),
            json!({"entries":[{"url":"https://example.com/a"}]}),
            json!({"entries":[{"url":"https://www.bilibili.com/a"}],"playlist_count":1001}),
        ] {
            assert!(parse_collection(&value, false).is_err());
        }
        assert!(parse_collection(&json!({"formats":[]}), false)
            .unwrap()
            .is_none());
    }
}
