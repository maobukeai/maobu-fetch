//! Codec declarations bound to the selected native media response, never container guesses.
use serde_json::Value;

const VIDEO_FIELDS: [&str; 3] = ["vcodec", "video_codec", "videoCodec"];
const AUDIO_FIELDS: [&str; 3] = ["acodec", "audio_codec", "audioCodec"];

pub(super) struct Codecs {
    pub video: Option<String>,
    pub audio: Option<String>,
}

pub(super) fn read(primary: &Value, fallback: Option<&Value>) -> Codecs {
    let resolve = |fields: &[&str]| {
        declared(primary, fields).or_else(|| fallback.and_then(|value| declared(value, fields)))
    };
    Codecs {
        video: resolve(&VIDEO_FIELDS),
        audio: resolve(&AUDIO_FIELDS),
    }
}

fn declared(source: &Value, fields: &[&str]) -> Option<String> {
    fields.iter().find_map(|field| {
        let codec = source.get(*field)?.as_str()?.trim();
        if codec.len() > 128
            || matches!(
                codec.to_ascii_lowercase().as_str(),
                "" | "unknown" | "n/a" | "null" | "undefined" | "unspecified" | "0" | "false" | "-"
            )
            || !codec
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return None;
        }
        Some(if codec.eq_ignore_ascii_case("none") {
            "none".to_owned()
        } else {
            codec.to_owned()
        })
    })
}

/// SSR normalization must keep explicit codec evidence, including an explicit absent track.
/// Music/BGM, h265 flags, MIME, dimensions and URL names are deliberately not evidence here.
pub(super) fn copy_declared_fields(source: &Value, target: &mut Value) {
    let Some(target) = target.as_object_mut() else {
        return;
    };
    for field in VIDEO_FIELDS.into_iter().chain(AUDIO_FIELDS) {
        if let Some(value) = source.get(field) {
            target.insert(field.to_owned(), value.clone());
        }
    }
}

pub(super) fn copy_douyin_ssr(source: &Value, target: &mut Value) {
    copy_declared_fields(source, target);
    // Different playAddr entries may be different encodings. Copy only the selected URL.
    let Some(selected) = target["play_addr"]["url_list"]
        .as_array()
        .and_then(|urls| urls.first())
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return;
    };
    if let Some(play_addr) = source.get("play_addr") {
        let matches = play_addr
            .get("url_list")
            .and_then(Value::as_array)
            .is_some_and(|urls| {
                urls.iter()
                    .filter_map(Value::as_str)
                    .any(|url| super::normalize_douyin_media_url(url) == selected)
            });
        if matches {
            copy_declared_fields(play_addr, &mut target["play_addr"]);
        }
    }
    let entry = source
        .get("playAddr")
        .and_then(Value::as_array)
        .and_then(|entries| {
            entries.iter().find(|entry| {
                entry
                    .get("src")
                    .and_then(Value::as_str)
                    .is_some_and(|url| super::normalize_douyin_media_url(url) == selected)
            })
        });
    if let Some(entry) = entry {
        copy_declared_fields(entry, &mut target["play_addr"]);
    }
}

#[cfg(test)]
mod tests;
