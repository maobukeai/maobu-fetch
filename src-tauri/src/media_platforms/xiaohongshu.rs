//! 小红书（Xiaohongshu）媒体解析与图文笔记抽取。
//!
//! 支持小红书图文笔记（Gallery 模式）与视频笔记的直连元数据解析。
//! 遵循 AGENTS.md §3（真实状态、不使用模拟数据）、§7（可恢复错误）、§8（职责单一、中文文案）。

use crate::manager::diagnose::MediaPlatformError;
use crate::media_platforms::{detect_platform, MediaPlatform};
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

/// 小红书标准桌面 User-Agent，避免触发轻量风控或返回移动端引导页。
pub const XHS_DESKTOP_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36";

/// 从小红书 URL 中提取 note ID（24 位 16 进制字符串或常见 ID 模式）。
///
/// 支持模式：
/// - `https://www.xiaohongshu.com/explore/64df8a01000000000800fa12`
/// - `https://www.xiaohongshu.com/discovery/item/64df8a01000000000800fa12`
/// - `https://xiaohongshu.com/explore/64df8a01000000000800fa12?xsec_token=...`
pub fn extract_xiaohongshu_note_id(url: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"/(?:explore|discovery/item|user/profile/[^/]+)/([a-zA-Z0-9_-]+)")
            .expect("Regex is statically validated")
    });
    re.captures(url)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

/// 判断 URL 是否为小红书图文或探索笔记。
pub fn is_xiaohongshu_gallery(url: &str) -> bool {
    if detect_platform(url) != MediaPlatform::Xiaohongshu {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    lower.contains("/explore/") || lower.contains("/discovery/item/")
}

/// 从小红书 HTML 中提取并解析 `window.__INITIAL_STATE__`。
///
/// 关键细节：
/// 1. 小红书 SSR 页面生成的 JavaScript 对象中可能包含未加引号的 `undefined`，
///    标准 JSON 解析器会报错，因此在反序列化前必须将 `\bundefined\b` 规范化为 `null`。
/// 2. 优先通过 `note_id` 在 `noteDetailMap` 中查找笔记详情；
///    若未指定或直接匹配失败，回退到 `noteDetailMap` 的首个笔记条目。
pub fn parse_xiaohongshu_html(html: &str, note_id: Option<&str>) -> Result<Value, String> {
    // 1. 定位 window.__INITIAL_STATE__
    let start_idx = html
        .find("window.__INITIAL_STATE__")
        .ok_or("未在小红书页面中找到状态数据 (INITIAL_STATE 缺失)")?;

    let after_marker = &html[start_idx..];
    let eq_idx = after_marker
        .find('=')
        .ok_or("小红书状态数据语法异常 (未找到赋值符号)")?;

    let script_tail = &after_marker[eq_idx + 1..];
    let end_idx = script_tail
        .find("</script>")
        .ok_or("小红书状态数据未闭合 (缺少 </script>)")?;

    let raw_payload = script_tail[..end_idx].trim().trim_end_matches(';').trim();

    // 2. 将非标准 JS 裸 undefined 替换为标准 JSON null
    static UNDEFINED_RE: OnceLock<Regex> = OnceLock::new();
    let undefined_re = UNDEFINED_RE.get_or_init(|| {
        Regex::new(r"\bundefined\b").expect("Regex is statically validated")
    });
    let sanitized = undefined_re.replace_all(raw_payload, "null");

    // 3. 反序列化
    let state: Value = serde_json::from_str(&sanitized)
        .map_err(|e| format!("小红书状态数据 JSON 解析失败：{e}"))?;

    // 4. 定位 note 对象
    let note_detail_map = state
        .get("note")
        .and_then(|n| n.get("noteDetailMap"))
        .and_then(Value::as_object);

    if let Some(map) = note_detail_map {
        // 尝试按 note_id 读取
        if let Some(id) = note_id {
            if let Some(item) = map.get(id).and_then(|v| v.get("note")) {
                if !item.is_null() {
                    return Ok(item.clone());
                }
            }
        }
        // 回退取 map 中第一个有效的 note
        for (_k, v) in map {
            if let Some(item) = v.get("note") {
                if !item.is_null() {
                    return Ok(item.clone());
                }
            }
        }
    }

    // 备用结构探测：firstNote / noteData
    if let Some(item) = state
        .get("note")
        .and_then(|n| n.get("firstNote"))
        .and_then(|fn_| fn_.get("note"))
    {
        if !item.is_null() {
            return Ok(item.clone());
        }
    }

    if let Some(item) = state.get("noteData") {
        if !item.is_null() {
            return Ok(item.clone());
        }
    }

    Err("未在小红书页面中提取到笔记详情，可能需要登录凭证或验证码校验".into())
}

/// 将小红书 note 数据转换为 yt-dlp 兼容的元数据格式。
///
/// - 图文笔记：将 `imageList` 转换为多个图片格式项，标记 `_is_xiaohongshu_gallery: true`。
/// - 视频笔记：提取 `video.media.stream` 中的 H.264 / H.265 真实流地址。
pub fn convert_xiaohongshu_note_to_yt_dlp_json(note: &Value, webpage_url: &str) -> Value {
    let title = note
        .get("title")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| note.get("desc").and_then(Value::as_str))
        .unwrap_or("小红书笔记")
        .trim();

    let uploader = note
        .get("user")
        .and_then(|u| u.get("nickname"))
        .and_then(Value::as_str)
        .unwrap_or("小红书作者");

    let uploader_id = note
        .get("user")
        .and_then(|u| u.get("userId"))
        .and_then(Value::as_str)
        .unwrap_or("");

    let note_type = note
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("normal");

    let mut formats = Vec::new();
    let mut thumbnails = Vec::new();
    let mut is_gallery = false;
    let mut duration: Option<f64> = None;

    if note_type == "video" {
        // 视频笔记解析
        if let Some(video) = note.get("video") {
            if let Some(capa) = video.get("capa") {
                duration = capa.get("duration").and_then(Value::as_f64);
            }
            if let Some(media) = video.get("media") {
                if let Some(stream) = media.get("stream") {
                    // 提取 H.264 (EF4) 最佳流
                    if let Some(ef4_list) = stream.get("ef4").or_else(|| stream.get("EF4")).and_then(Value::as_array) {
                        for (i, item) in ef4_list.iter().enumerate() {
                            if let Some(url) = item.get("masterUrl").and_then(Value::as_str) {
                                formats.push(serde_json::json!({
                                    "format_id": format!("video-h264-{}", i),
                                    "format_note": item.get("qualityType").and_then(Value::as_str).unwrap_or("高清 H.264"),
                                    "ext": "mp4",
                                    "url": url,
                                    "vcodec": "avc1",
                                    "acodec": "aac",
                                    "width": item.get("width").and_then(Value::as_i64),
                                    "height": item.get("height").and_then(Value::as_i64),
                                    "filesize": item.get("size").and_then(Value::as_i64),
                                }));
                            }
                        }
                    }
                    // 提取 H.265 (EF5) 流
                    if let Some(ef5_list) = stream.get("ef5").or_else(|| stream.get("EF5")).and_then(Value::as_array) {
                        for (i, item) in ef5_list.iter().enumerate() {
                            if let Some(url) = item.get("masterUrl").and_then(Value::as_str) {
                                formats.push(serde_json::json!({
                                    "format_id": format!("video-h265-{}", i),
                                    "format_note": item.get("qualityType").and_then(Value::as_str).unwrap_or("超清 H.265"),
                                    "ext": "mp4",
                                    "url": url,
                                    "vcodec": "hevc",
                                    "acodec": "aac",
                                    "width": item.get("width").and_then(Value::as_i64),
                                    "height": item.get("height").and_then(Value::as_i64),
                                    "filesize": item.get("size").and_then(Value::as_i64),
                                }));
                            }
                        }
                    }
                }
            }
        }
    }

    // 图文笔记或带有 imageList 的笔记
    let image_list = note.get("imageList").and_then(Value::as_array);
    if let Some(images) = image_list {
        if !images.is_empty() {
            if note_type != "video" || formats.is_empty() {
                is_gallery = true;
                for (i, img) in images.iter().enumerate() {
                    let default_url = img.get("urlDefault").and_then(Value::as_str)
                        .or_else(|| img.get("url").and_then(Value::as_str))
                        .unwrap_or("");
                    if !default_url.is_empty() {
                        let width = img.get("width").and_then(Value::as_i64);
                        let height = img.get("height").and_then(Value::as_i64);
                        formats.push(serde_json::json!({
                            "format_id": format!("image-{}", i),
                            "format_note": format!("图片 {}", i + 1),
                            "url": default_url,
                            "image_url": default_url,
                            "ext": "webp",
                            "vcodec": "none",
                            "acodec": "none",
                            "width": width,
                            "height": height,
                        }));
                    }
                }
            }

            // 补充 thumbnails 供预览
            for (i, img) in images.iter().enumerate() {
                let thumb_url = img.get("urlPre").and_then(Value::as_str)
                    .or_else(|| img.get("urlDefault").and_then(Value::as_str))
                    .unwrap_or("");
                if !thumb_url.is_empty() {
                    thumbnails.push(serde_json::json!({
                        "id": i.to_string(),
                        "url": thumb_url,
                    }));
                }
            }
        }
    }

    let thumbnail = thumbnails
        .first()
        .and_then(|t| t.get("url"))
        .and_then(Value::as_str)
        .map(ToString::to_string);

    serde_json::json!({
        "title": title,
        "extractor": "xiaohongshu",
        "extractor_key": "Xiaohongshu",
        "uploader": uploader,
        "uploader_id": uploader_id,
        "webpage_url": webpage_url,
        "thumbnail": thumbnail,
        "thumbnails": thumbnails,
        "duration": duration,
        "formats": formats,
        "_is_xiaohongshu_gallery": is_gallery,
    })
}

/// 发起小红书 HTTP 请求并解析笔记详情。
pub async fn fetch_xiaohongshu_note_detail_with_client(
    url: &str,
    cookie: Option<&str>,
    referer: Option<&str>,
    user_agent: Option<&str>,
    client: &reqwest::Client,
) -> Result<Value, String> {
    let note_id = extract_xiaohongshu_note_id(url);
    let ua = user_agent.unwrap_or(XHS_DESKTOP_UA);
    let ref_header = referer.unwrap_or("https://www.xiaohongshu.com/");

    let mut request = client
        .get(url)
        .header(reqwest::header::USER_AGENT, ua)
        .header(reqwest::header::REFERER, ref_header)
        .header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
        )
        .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.8");

    if let Some(c) = cookie.filter(|s| !s.is_empty()) {
        request = request.header(reqwest::header::COOKIE, c);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("小红书页面请求失败：{e}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("小红书页面响应异常：HTTP {status}"));
    }

    let html = response
        .text()
        .await
        .map_err(|e| format!("读取小红书页面内容失败：{e}"))?;

    let note = parse_xiaohongshu_html(&html, note_id.as_deref())?;
    Ok(convert_xiaohongshu_note_to_yt_dlp_json(&note, url))
}

/// 小红书平台特定的错误分类。
pub fn classify_xiaohongshu_error(lower: &str) -> MediaPlatformError {
    if lower.contains("login")
        || lower.contains("登录")
        || lower.contains("401")
        || lower.contains("403")
        || lower.contains("sign in")
        || lower.contains("验证码")
        || lower.contains("captcha")
    {
        return MediaPlatformError::LoginExpired;
    }
    if lower.contains("not found")
        || lower.contains("404")
        || lower.contains("不存在")
        || lower.contains("已删除")
        || lower.contains("300031")
    {
        return MediaPlatformError::LinkExpired;
    }
    MediaPlatformError::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_xiaohongshu_note_id() {
        assert_eq!(
            extract_xiaohongshu_note_id("https://www.xiaohongshu.com/explore/64df8a01000000000800fa12"),
            Some("64df8a01000000000800fa12".into())
        );
        assert_eq!(
            extract_xiaohongshu_note_id("https://xiaohongshu.com/discovery/item/64df8a01000000000800fa12?xsec_token=ABC"),
            Some("64df8a01000000000800fa12".into())
        );
        assert_eq!(
            extract_xiaohongshu_note_id("https://example.com/not-a-note"),
            None
        );
    }

    #[test]
    fn test_parse_xiaohongshu_html_normal_gallery() {
        // 包含裸 undefined 的测试数据，模拟真实 SSR payload
        let html = r#"
        <!DOCTYPE html>
        <html>
        <head><title>小红书</title></head>
        <body>
        <script>
        window.__INITIAL_STATE__ = {
            "note": {
                "firstNoteId": "64df8a01",
                "noteDetailMap": {
                    "64df8a01": {
                        "note": {
                            "noteId": "64df8a01",
                            "type": "normal",
                            "title": "巴黎旅行随记",
                            "desc": "塞纳河畔的秋日漫步 #摄影 #巴黎",
                            "user": {
                                "userId": "user_12345",
                                "nickname": "旅行家小美"
                            },
                            "imageList": [
                                {
                                    "urlDefault": "http://sns-webpic-qc.xhscdn.com/test_image_1.webp!nd_dft_wlteh_webp_3",
                                    "urlPre": "http://sns-webpic-qc.xhscdn.com/test_image_1_pre.webp",
                                    "width": 3072,
                                    "height": 4096
                                },
                                {
                                    "urlDefault": "http://sns-webpic-qc.xhscdn.com/test_image_2.webp!nd_dft_wlteh_webp_3",
                                    "urlPre": "http://sns-webpic-qc.xhscdn.com/test_image_2_pre.webp",
                                    "width": 3072,
                                    "height": 4096
                                }
                            ],
                            "commentCount": undefined,
                            "shareCount": undefined
                        }
                    }
                }
            }
        };
        </script>
        </body>
        </html>
        "#;

        let note = parse_xiaohongshu_html(html, Some("64df8a01")).expect("必须成功解析包含 undefined 的 HTML");
        assert_eq!(note["title"], "巴黎旅行随记");
        assert_eq!(note["user"]["nickname"], "旅行家小美");

        let converted = convert_xiaohongshu_note_to_yt_dlp_json(&note, "https://www.xiaohongshu.com/explore/64df8a01");
        assert_eq!(converted["title"], "巴黎旅行随记");
        assert_eq!(converted["uploader"], "旅行家小美");
        assert_eq!(converted["_is_xiaohongshu_gallery"], true);

        let formats = converted["formats"].as_array().expect("formats 必须为数组");
        assert_eq!(formats.len(), 2, "应生成 2 张图片格式项");
        assert_eq!(formats[0]["format_id"], "image-0");
        assert_eq!(formats[0]["ext"], "webp");
        assert_eq!(formats[0]["vcodec"], "none");
        assert_eq!(formats[0]["acodec"], "none");
        assert_eq!(formats[0]["width"], 3072);
        assert_eq!(formats[0]["height"], 4096);
        assert_eq!(formats[0]["image_url"], "http://sns-webpic-qc.xhscdn.com/test_image_1.webp!nd_dft_wlteh_webp_3");

        let thumbnails = converted["thumbnails"].as_array().expect("thumbnails 必须为数组");
        assert_eq!(thumbnails.len(), 2);
    }

    #[test]
    fn test_parse_xiaohongshu_html_video_note() {
        let html = r#"
        <script>
        window.__INITIAL_STATE__ = {
            "note": {
                "noteDetailMap": {
                    "v_note_999": {
                        "note": {
                            "noteId": "v_note_999",
                            "type": "video",
                            "title": "咖啡拉花教学",
                            "desc": "零基础学会天鹅拉花",
                            "user": {
                                "userId": "barista_01",
                                "nickname": "拉花冠军"
                            },
                            "video": {
                                "capa": { "duration": 45.5 },
                                "media": {
                                    "stream": {
                                        "ef4": [
                                            {
                                                "masterUrl": "https://sns-video-bd.xhscdn.com/stream.mp4",
                                                "qualityType": "1080P",
                                                "width": 1080,
                                                "height": 1920,
                                                "size": 18456000
                                            }
                                        ]
                                    }
                                }
                            },
                            "imageList": []
                        }
                    }
                }
            }
        };
        </script>
        "#;

        let note = parse_xiaohongshu_html(html, Some("v_note_999")).expect("必须解析视频笔记");
        let converted = convert_xiaohongshu_note_to_yt_dlp_json(&note, "https://www.xiaohongshu.com/explore/v_note_999");
        assert_eq!(converted["title"], "咖啡拉花教学");
        assert_eq!(converted["_is_xiaohongshu_gallery"], false);
        assert_eq!(converted["duration"], 45.5);

        let formats = converted["formats"].as_array().expect("formats 必须为数组");
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0]["format_id"], "video-h264-0");
        assert_eq!(formats[0]["vcodec"], "avc1");
        assert_eq!(formats[0]["url"], "https://sns-video-bd.xhscdn.com/stream.mp4");
    }

    #[test]
    fn test_classify_xiaohongshu_error() {
        assert_eq!(
            classify_xiaohongshu_error("error: login required to view full note"),
            MediaPlatformError::LoginExpired
        );
        assert_eq!(
            classify_xiaohongshu_error("http 404: note not found or removed"),
            MediaPlatformError::LinkExpired
        );
        assert_eq!(
            classify_xiaohongshu_error("note 300031 error"),
            MediaPlatformError::LinkExpired
        );
    }
}
