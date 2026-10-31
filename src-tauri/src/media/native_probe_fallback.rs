//! Keep native candidates unless the optional tool supplies usable, more certain evidence.
use super::{drm, parse_formats};
use serde_json::Value;

#[derive(Default)]
struct Evidence {
    playable: bool,
    audio: bool,
    images: bool,
}

fn evidence(value: &Value) -> Evidence {
    let mut result = Evidence::default();
    for format in parse_formats(value) {
        let valid_url = format.url.as_deref().is_some_and(|url| {
            url::Url::parse(url).is_ok_and(|url| {
                matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
            })
        });
        if !valid_url {
            continue;
        }
        if format.image_url.is_some() {
            result.images = true;
        } else if format.has_video || format.has_audio || format.audio_unknown {
            result.playable = true;
            result.audio |= format.has_audio && !format.audio_unknown;
        }
    }
    result
}

pub(super) fn needs_tool(native: &Value) -> Result<bool, String> {
    drm::ensure_unprotected(native)?;
    let native = evidence(native);
    Ok(!native.audio && (native.playable || !native.images))
}

pub(super) fn select(native: Value, tool: Option<Result<Value, String>>) -> Result<Value, String> {
    drm::ensure_unprotected(&native)?;
    let original = evidence(&native);
    let usable_native = original.playable || original.images;
    let candidate = match tool {
        Some(Ok(value)) => {
            drm::ensure_unprotected(&value)?;
            Some(value)
        }
        Some(Err(error)) => {
            // An optional metadata fallback must not turn a DRM refusal or cancellation
            // into an apparently successful unprotected native result.
            if error.starts_with("MEDIA_DRM_PROTECTED:")
                || error == "任务已暂停"
                || error == "任务已取消"
                || !usable_native
            {
                return Err(error);
            }
            None
        }
        None => None,
    };
    if let Some(candidate) = candidate {
        let candidate_evidence = evidence(&candidate);
        if candidate_evidence.playable
            && (!usable_native || (!original.audio && candidate_evidence.audio))
        {
            return Ok(candidate);
        }
    }
    if usable_native {
        Ok(native)
    } else {
        Err("MEDIA_FORMATS_MISSING: 未解析到可下载的媒体格式，请更新媒体组件，或确认该链接包含可访问的视频后重试".into())
    }
}

#[cfg(test)]
mod tests;
