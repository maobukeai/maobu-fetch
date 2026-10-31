use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TTL_MS: u64 = 5 * 60 * 1000;
const MAX_ENTRIES: usize = 32;

fn now_ms() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_millis().min(u64::MAX as u128) as u64)
        .map_err(|_| "系统时间无效，无法使用媒体探测缓存".into())
}
fn cookie_identity(cookie: Option<&str>) -> String {
    // Scope is part of identity: jars with identical pairs on different domains are distinct.
    hex::encode(Sha256::digest(cookie.unwrap_or("").trim().as_bytes()))
}
fn cache_path(base: &Path, source: &str) -> PathBuf {
    let source = crate::media_platforms::extract_url_from_share_text(source)
        .unwrap_or_else(|| source.to_owned());
    base.join("media_probe_cache").join(format!(
        "{}.json",
        hex::encode(Sha256::digest(source.trim().as_bytes()))
    ))
}

pub(super) fn minimal(metadata: &Value, platform: crate::media_platforms::MediaPlatform) -> Value {
    let mut selected = serde_json::json!({"maobu_platform":platform.as_str()});
    for key in [
        "title",
        "uploader",
        "channel",
        "uploader_id",
        "upload_date",
        "id",
        "display_id",
        "extractor_key",
        "live_status",
    ] {
        if let Some(text) = metadata
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| text.len() <= 4096)
        {
            selected[key] = text.into();
        }
    }
    if let Some(live) = metadata.get("is_live").and_then(Value::as_bool) {
        selected["is_live"] = live.into();
    }
    selected
}

pub(super) async fn save(
    base: &Path,
    source: &str,
    cookie: Option<&str>,
    metadata: &Value,
) -> Result<(), String> {
    let path = cache_path(base, source);
    let directory = path.parent().ok_or("媒体缓存目录无效")?;
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(|_| "创建媒体探测缓存失败")?;
    let now = now_ms()?;
    let mut safe_metadata = minimal(metadata, crate::media_platforms::MediaPlatform::Unknown);
    if let Some(platform) = metadata
        .get("maobu_platform")
        .and_then(Value::as_str)
        .filter(|platform| platform.len() <= 32)
    {
        safe_metadata["maobu_platform"] = platform.into();
    }
    let value = serde_json::json!({"created_at":now,"expires_at":now.saturating_add(TTL_MS),"identity":cookie_identity(cookie),"metadata":safe_metadata});
    let staged = directory.join(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&staged)
        .await
        .map_err(|_| "创建媒体安全缓存失败")?;
    crate::media_cookies::restrict_to_current_user(&staged)?;
    file.write_all(value.to_string().as_bytes())
        .await
        .map_err(|_| "写入媒体安全缓存失败")?;
    file.sync_all().await.map_err(|_| "保存媒体安全缓存失败")?;
    drop(file);
    crate::manager::publish_file_atomically(&staged, &path, true).await?;
    prune(directory, now).await
}

pub(super) async fn load(
    base: &Path,
    source: &str,
    cookie: Option<&str>,
) -> Result<Option<Value>, String> {
    let path = cache_path(base, source);
    let Some(value) = read_entry(&path).await? else {
        return Ok(None);
    };
    let now = now_ms()?;
    if value
        .get("expires_at")
        .and_then(Value::as_u64)
        .is_none_or(|expiration| expiration < now || expiration > now.saturating_add(TTL_MS))
        || value.get("identity").and_then(Value::as_str) != Some(&cookie_identity(cookie))
    {
        return Ok(None);
    }
    Ok(value
        .get("metadata")
        .filter(|metadata| metadata.is_object())
        .cloned())
}

async fn read_entry(path: &Path) -> Result<Option<Value>, String> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("读取媒体缓存状态失败".into()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 64 * 1024 {
        return Err("媒体缓存路径或长度无效".into());
    }
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| "读取媒体缓存失败")?;
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "读取媒体缓存失败")?;
    if bytes.len() > 64 * 1024 {
        return Err("媒体缓存超过安全上限".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "媒体缓存损坏，请重新分析媒体".into())
}

async fn prune(directory: &Path, now: u64) -> Result<(), String> {
    let mut entries = tokio::fs::read_dir(directory)
        .await
        .map_err(|_| "读取媒体缓存目录失败")?;
    let mut active = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|_| "读取媒体缓存项目失败")?
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.len() != 69
            || !name.ends_with(".json")
            || !name.as_bytes()[..64]
                .iter()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            continue;
        }
        let Some(value) = read_entry(&entry.path()).await? else {
            continue;
        };
        if value
            .get("expires_at")
            .and_then(Value::as_u64)
            .is_none_or(|expiration| expiration < now)
        {
            tokio::fs::remove_file(entry.path())
                .await
                .map_err(|_| "清理过期媒体缓存失败")?;
        } else {
            active.push((
                value.get("created_at").and_then(Value::as_u64).unwrap_or(0),
                entry.path(),
            ));
            if active.len() > 256 {
                return Err("媒体探测缓存项目过多，请关闭程序后检查应用缓存目录".into());
            }
        }
    }
    active.sort_by_key(|(created, _)| *created);
    let excess = active.len().saturating_sub(MAX_ENTRIES);
    for (_, path) in active.into_iter().take(excess) {
        tokio::fs::remove_file(path)
            .await
            .map_err(|_| "限制媒体缓存数量失败")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn missing_ui_probe_cache_is_explicit_and_identity_and_ttl_are_enforced() {
        let root = tempfile::tempdir().unwrap();
        let url = "https://example.test/video";
        assert!(load(root.path(), url, Some("session=one"))
            .await
            .unwrap()
            .is_none());
        save(
            root.path(),
            url,
            Some("session=one"),
            &serde_json::json!({"title":"actual","uploader":"author","upload_date":"20261031","url":"https://private.test/?token=synthetic-private","http_headers":{"Authorization":"synthetic-private"}}),
        )
        .await
        .unwrap();
        assert!(!tokio::fs::read_to_string(cache_path(root.path(), url))
            .await
            .unwrap()
            .contains("synthetic-private"));
        assert_eq!(
            load(root.path(), url, Some("session=one"))
                .await
                .unwrap()
                .unwrap()["uploader"],
            "author"
        );
        assert!(load(root.path(), url, Some("session=two"))
            .await
            .unwrap()
            .is_none());
        let path = cache_path(root.path(), url);
        let mut value = read_entry(&path).await.unwrap().unwrap();
        value["expires_at"] = 0.into();
        tokio::fs::write(path, value.to_string()).await.unwrap();
        assert!(load(root.path(), url, Some("session=one"))
            .await
            .unwrap()
            .is_none());
    }
}
