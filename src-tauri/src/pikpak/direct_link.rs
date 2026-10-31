//! A blank media entry must not hide the original file's download link.
use serde_json::Value;

pub(super) fn select(json: &Value) -> Result<String, String> {
    let file = json
        .get("file_info")
        .filter(|file| file.is_object())
        .or_else(|| json.get("file").filter(|file| file.is_object()));
    let media = file
        .and_then(|file| file.get("medias"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|media| media.get("link"))
        .filter_map(|link| link.get("url"));
    let original = file.and_then(|file| file.get("web_content_link"));
    let selected = media
        .chain(original)
        .chain(json.get("web_content_link"))
        .filter_map(Value::as_str)
        .map(str::trim)
        .find(|candidate| !candidate.is_empty())
        .ok_or("PikPak 未返回该文件的有效下载直链，请重新解析分享或检查访问权限")?;
    let parsed =
        url::Url::parse(selected).map_err(|_| "PikPak 返回的文件下载地址无效，请重新解析分享")?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || selected.chars().any(char::is_control)
    {
        return Err("PikPak 返回的文件下载地址不安全，请重新解析分享或检查访问权限".into());
    }
    Ok(selected.to_owned())
}

#[cfg(test)]
#[path = "direct_link_tests.rs"]
mod tests;
