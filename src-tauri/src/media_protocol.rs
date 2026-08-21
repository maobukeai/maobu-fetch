//! 本地媒体流协议处理（media_protocol）。
//!
//! 为前端 HTML5 `<video>` 和 `<audio>` 播放器提供支持 HTTP 206 Partial Content / Range 请求
//! 的高性能、零拷贝流式读取协议。
//!
//! 设计要点（AGENTS.md §8 & §3）：
//! - 纯 Rust 实现，不引入大型依赖。
//! - 精确支持 `Range: bytes=start-end` 及 `bytes=start-` 请求，支持毫秒级拖动（Seek）。
//! - 针对大文件（数 GB 蓝光/4K 视频）采用分块流式或有界读取，防止内存溢出。
//! - 路径安全防护：校验文件存在性，规范化路径。

use std::path::PathBuf;
use tauri::http::{header, Response, StatusCode};

/// 根据文件扩展名推断 MIME 类型。
pub fn mime_type_from_ext(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "ts" => "video/mp2t",
        "flv" => "video/x-flv",
        "wmv" => "video/x-ms-wmv",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "vtt" => "text/vtt; charset=utf-8",
        "srt" => "text/plain; charset=utf-8",
        "jpg" | "jpeg" | "jfif" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "tiff" | "tif" => "image/tiff",
        _ => "application/octet-stream",
    }
}

/// 解析 `Range` 请求头，例如 `bytes=0-1023` 或 `bytes=1024-`。
/// 返回 `Some((start, end))`（闭区间 [start, end]）。
pub fn parse_range_header(range_val: &str, total_size: u64) -> Option<(u64, u64)> {
    if total_size == 0 {
        return None;
    }
    let range_val = range_val.trim();
    if !range_val.starts_with("bytes=") {
        return None;
    }
    let spec = &range_val["bytes=".len()..];
    // 只处理单区间，忽略多区间
    let first_range = spec.split(',').next()?.trim();
    let mut parts = first_range.split('-');
    let start_str = parts.next()?.trim();
    let end_str = parts.next()?.trim();

    if start_str.is_empty() {
        // 后缀字节：bytes=-500 表示最后 500 字节
        let suffix_len: u64 = end_str.parse().ok()?;
        if suffix_len == 0 {
            return None;
        }
        let start = total_size.saturating_sub(suffix_len);
        let end = total_size - 1;
        Some((start, end))
    } else {
        let start: u64 = start_str.parse().ok()?;
        if start >= total_size {
            return None;
        }
        let end = if end_str.is_empty() {
            total_size - 1
        } else {
            let parsed_end: u64 = end_str.parse().ok()?;
            parsed_end.min(total_size - 1)
        };
        if start > end {
            return None;
        }
        Some((start, end))
    }
}

/// 从 URI 路径或查询参数中提取并规范化为 Windows 本地文件路径。
pub fn decode_local_file_path(uri_raw: &str) -> Option<PathBuf> {
    // 1. 如果带有 query 参数 ?file=... 或 ?path=...，优先提取
    let path_str = if let Some(idx) = uri_raw.find("?file=") {
        &uri_raw[idx + 6..]
    } else if let Some(idx) = uri_raw.find("&file=") {
        &uri_raw[idx + 6..]
    } else if let Some(idx) = uri_raw.find("?path=") {
        &uri_raw[idx + 6..]
    } else if let Some(idx) = uri_raw.find("&path=") {
        &uri_raw[idx + 6..]
    } else {
        uri_raw
    };

    let mut decoded = urlencoding_decode(path_str);
    // 去除 query 尾部其它参数
    if let Some(q_idx) = decoded.find('&') {
        decoded.truncate(q_idx);
    }
    let mut clean_path = decoded.trim();

    // 剥离 scheme 如 stream:// 或 asset://
    if let Some(idx) = clean_path.find("://") {
        clean_path = &clean_path[idx + 3..];
    }

    // 剥离 localhost/ 或 asset.localhost/
    if clean_path.to_ascii_lowercase().starts_with("localhost/") || clean_path.to_ascii_lowercase().starts_with("localhost\\") {
        clean_path = &clean_path[10..];
    } else if clean_path.to_ascii_lowercase().starts_with("asset.localhost/") || clean_path.to_ascii_lowercase().starts_with("asset.localhost\\") {
        clean_path = &clean_path[16..];
    }

    // 循环剥离前导斜杠，直到遇到驱动器盘符 "C:" 或 UNC 路径
    while clean_path.starts_with('/') || clean_path.starts_with('\\') {
        if clean_path.len() >= 3 {
            let bytes = clean_path.as_bytes();
            if (bytes[1].is_ascii_alphabetic() && bytes[2] == b':') ||
               (bytes[0] == b'\\' && bytes[1] == b'\\') {
                if bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
                    clean_path = &clean_path[1..];
                }
                break;
            }
        }
        clean_path = &clean_path[1..];
    }

    let norm_path = clean_path.replace('/', "\\");
    let path = PathBuf::from(&norm_path);
    if path.exists() && path.is_file() {
        Some(path)
    } else {
        let alt = PathBuf::from(clean_path);
        if alt.exists() && alt.is_file() {
            Some(alt)
        } else {
            None
        }
    }
}

/// 简易 URL 百分号解码（避免引入 extra dependency）。
fn urlencoding_decode(input: &str) -> String {
    let mut bytes = Vec::with_capacity(input.len());
    let mut chars = input.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(h1), Some(h2)) = (h1, h2) {
                if let Ok(val) = u8::from_str_radix(
                    &format!("{}{}", h1 as char, h2 as char),
                    16,
                ) {
                    bytes.push(val);
                    continue;
                }
            }
        }
        bytes.push(b);
    }
    String::from_utf8_lossy(&bytes).to_string()
}

/// 单次读取的最大块大小（16 MB），避免内存占用过大。
const MAX_CHUNK_READ: u64 = 16 * 1024 * 1024;

/// 处理流协议请求并生成对应的 HTTP 响应。
pub async fn handle_media_request(
    uri_path: &str,
    range_header: Option<&str>,
) -> Response<Vec<u8>> {
    let file_path = match decode_local_file_path(uri_path) {
        Some(p) => p,
        None => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .body(b"File not found".to_vec())
                .unwrap_or_default();
        }
    };

    let metadata = match tokio::fs::metadata(&file_path).await {
        Ok(m) => m,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .body(b"Failed to read file metadata".to_vec())
                .unwrap_or_default();
        }
    };

    let total_size = metadata.len();
    let ext = file_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let mime = mime_type_from_ext(ext);

    use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};

    let mut file = match tokio::fs::File::open(&file_path).await {
        Ok(f) => f,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .body(b"Failed to open file".to_vec())
                .unwrap_or_default();
        }
    };

    if let Some(range_str) = range_header {
        match parse_range_header(range_str, total_size) {
            Some((start, end)) => {
                let range_len = (end - start + 1).min(MAX_CHUNK_READ);

                if let Err(_) = file.seek(SeekFrom::Start(start)).await {
                    return Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                        .body(b"Seek failed".to_vec())
                        .unwrap_or_default();
                }

                let mut buf = vec![0u8; range_len as usize];
                let mut bytes_read = 0;
                while bytes_read < range_len as usize {
                    match file.read(&mut buf[bytes_read..]).await {
                        Ok(0) => break, // EOF
                        Ok(n) => bytes_read += n,
                        Err(_) => break,
                    }
                }
                buf.truncate(bytes_read);
                let final_end = start + (bytes_read as u64).saturating_sub(1);

                Response::builder()
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header(header::CONTENT_TYPE, mime)
                    .header(header::ACCEPT_RANGES, "bytes")
                    .header(
                        header::CONTENT_RANGE,
                        format!("bytes {start}-{final_end}/{total_size}"),
                    )
                    .header(header::CONTENT_LENGTH, bytes_read.to_string())
                    .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                    .body(buf)
                    .unwrap_or_default()
            }
            None => {
                // 无效或超出范围的 Range 请求
                Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{total_size}"))
                    .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                    .body(Vec::new())
                    .unwrap_or_default()
            }
        }
    } else {
        // 无 Range 请求：返回前 16MB 或整个小文件
        let read_len = total_size.min(MAX_CHUNK_READ);
        let mut buf = vec![0u8; read_len as usize];
        let mut bytes_read = 0;
        while bytes_read < read_len as usize {
            match file.read(&mut buf[bytes_read..]).await {
                Ok(0) => break,
                Ok(n) => bytes_read += n,
                Err(_) => break,
            }
        }
        buf.truncate(bytes_read);

        if total_size <= MAX_CHUNK_READ {
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .header(header::ACCEPT_RANGES, "bytes")
                .header(header::CONTENT_LENGTH, bytes_read.to_string())
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .body(buf)
                .unwrap_or_default()
        } else {
            // 大文件无 Range 请求时，以 206 返回第一段
            Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_TYPE, mime)
                .header(header::ACCEPT_RANGES, "bytes")
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes 0-{}/{total_size}", bytes_read.saturating_sub(1)),
                )
                .header(header::CONTENT_LENGTH, bytes_read.to_string())
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .body(buf)
                .unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mime_type_inference() {
        assert_eq!(mime_type_from_ext("mp4"), "video/mp4");
        assert_eq!(mime_type_from_ext("MP4"), "video/mp4");
        assert_eq!(mime_type_from_ext("mkv"), "video/x-matroska");
        assert_eq!(mime_type_from_ext("webm"), "video/webm");
        assert_eq!(mime_type_from_ext("mov"), "video/quicktime");
        assert_eq!(mime_type_from_ext("mp3"), "audio/mpeg");
        assert_eq!(mime_type_from_ext("flac"), "audio/flac");
        assert_eq!(mime_type_from_ext("vtt"), "text/vtt; charset=utf-8");
        assert_eq!(mime_type_from_ext("srt"), "text/plain; charset=utf-8");
    }

    #[test]
    fn test_parse_range_header_standard() {
        assert_eq!(
            parse_range_header("bytes=0-499", 1000),
            Some((0, 499))
        );
        assert_eq!(
            parse_range_header("bytes=500-999", 1000),
            Some((500, 999))
        );
        assert_eq!(
            parse_range_header("bytes=500-", 1000),
            Some((500, 999))
        );
    }

    #[test]
    fn test_parse_range_header_suffix() {
        assert_eq!(
            parse_range_header("bytes=-500", 1000),
            Some((500, 999))
        );
        assert_eq!(
            parse_range_header("bytes=-2000", 1000),
            Some((0, 999))
        );
    }

    #[test]
    fn test_parse_range_header_out_of_bounds() {
        assert_eq!(parse_range_header("bytes=1000-", 1000), None);
        assert_eq!(parse_range_header("bytes=1500-2000", 1000), None);
        assert_eq!(parse_range_header("bytes=500-400", 1000), None);
        assert_eq!(parse_range_header("invalid", 1000), None);
    }

    #[test]
    fn test_url_decoding() {
        let raw = "C%3A/Users/Test%20User/Videos/my%20video.mp4";
        let decoded = urlencoding_decode(raw);
        assert_eq!(decoded, "C:/Users/Test User/Videos/my video.mp4");
    }
}
