//! 快手（Kuaishou）与西瓜视频（Xigua）平台适配支持。
//!
//! 提供 URL 模式提取、防盗链与 UA 规范、错误模式分类。
//! 遵循 AGENTS.md §3（真实状态）、§7（可恢复错误）、§8（轻量拆解、单一职责）。

use crate::manager::diagnose::MediaPlatformError;
use regex::Regex;
use std::sync::OnceLock;

/// 快手标准 Chrome User-Agent，避免非浏览器 UA 触发快手“405/安全威胁”拦截页。
pub const KUAISHOU_DESKTOP_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36";

/// 西瓜视频标准 Referer，避免 CDN 直链 403 防盗链拦截。
pub const XIGUA_DEFAULT_REFERER: &str = "https://www.ixigua.com/";

/// 快手标准 Referer。
pub const KUAISHOU_DEFAULT_REFERER: &str = "https://www.kuaishou.com/";

/// 从快手 URL 中提取 photoId。
///
/// 支持模式：
/// - `https://www.kuaishou.com/short-video/3xabcdef123`
/// - `https://www.kuaishou.com/f/kuaishou_code`
/// - `https://v.kuaishou.com/3xabcdef`
pub fn extract_kuaishou_photo_id(url: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"/(?:short-video|f)/([a-zA-Z0-9_-]+)").expect("Regex is statically validated")
    });
    re.captures(url)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

/// 从西瓜视频 URL 中提取视频 ID。
///
/// 支持模式：
/// - `https://www.ixigua.com/7123456789012345678`
/// - `https://www.ixigua.com/i7123456789012345678/`
/// - `https://m.ixigua.com/video/7123456789012345678`
pub fn extract_xigua_video_id(url: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?:ixigua\.com/(?:i|video/)?|v\.ixigua\.com/)([0-9]{15,22})")
            .expect("Regex is statically validated")
    });
    re.captures(url)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

/// 快手平台特定的错误分类。
pub fn classify_kuaishou_error(stderr: &str) -> MediaPlatformError {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("captcha")
        || lower.contains("安全验证")
        || lower.contains("验证码")
        || lower.contains("login")
        || lower.contains("登录")
        || lower.contains("403")
        || lower.contains("405")
    {
        return MediaPlatformError::LoginExpired;
    }
    if lower.contains("not found")
        || lower.contains("404")
        || lower.contains("已删除")
        || lower.contains("不存在")
        || lower.contains("photo not found")
    {
        return MediaPlatformError::LinkExpired;
    }
    MediaPlatformError::Unknown
}

/// 西瓜视频平台特定的错误分类。
pub fn classify_xigua_error(stderr: &str) -> MediaPlatformError {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("cookies (not necessarily logged in) are needed")
        || lower.contains("fresh cookies")
        || lower.contains("login required")
        || lower.contains("vip")
        || lower.contains("付费")
        || lower.contains("403")
    {
        return MediaPlatformError::LoginExpired;
    }
    if lower.contains("not found")
        || lower.contains("404")
        || lower.contains("video unavailable")
        || lower.contains("deleted")
        || lower.contains("已删除")
    {
        return MediaPlatformError::LinkExpired;
    }
    MediaPlatformError::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_kuaishou_photo_id() {
        assert_eq!(
            extract_kuaishou_photo_id("https://www.kuaishou.com/short-video/3xabcdef123"),
            Some("3xabcdef123".into())
        );
        assert_eq!(
            extract_kuaishou_photo_id("https://kuaishou.com/f/share_code_99"),
            Some("share_code_99".into())
        );
        assert_eq!(extract_kuaishou_photo_id("https://example.com/video/123"), None);
    }

    #[test]
    fn test_extract_xigua_video_id() {
        assert_eq!(
            extract_xigua_video_id("https://www.ixigua.com/7123456789012345678"),
            Some("7123456789012345678".into())
        );
        assert_eq!(
            extract_xigua_video_id("https://www.ixigua.com/i7123456789012345678/"),
            Some("7123456789012345678".into())
        );
        assert_eq!(
            extract_xigua_video_id("https://m.ixigua.com/video/7123456789012345678?wid_try=1"),
            Some("7123456789012345678".into())
        );
    }

    #[test]
    fn test_classify_kuaishou_error() {
        assert_eq!(
            classify_kuaishou_error("error 405: 触发安全验证，请在浏览器中打开"),
            MediaPlatformError::LoginExpired
        );
        assert_eq!(
            classify_kuaishou_error("photo not found 404"),
            MediaPlatformError::LinkExpired
        );
    }

    #[test]
    fn test_classify_xigua_error() {
        assert_eq!(
            classify_xigua_error("ERROR: [Ixigua] Cookies (not necessarily logged in) are needed"),
            MediaPlatformError::LoginExpired
        );
        assert_eq!(
            classify_xigua_error("video unavailable 404"),
            MediaPlatformError::LinkExpired
        );
    }
}
