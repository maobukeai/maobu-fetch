// HLS M3U8 播放列表解析器（纯 Rust 实现，遵循 RFC 8216 标准）

use std::collections::HashMap;
use url::Url;

/// 加密方法类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncryptionMethod {
    None,
    Aes128,
    SampleAes,
    Other(String),
}

/// 切片加密密钥信息（#EXT-X-KEY）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptionKey {
    pub method: EncryptionMethod,
    pub uri: String,
    pub iv: Option<[u8; 16]>,
}

/// 媒体切片信息（#EXTINF / #EXT-X-MAP）
#[derive(Debug, Clone, PartialEq)]
pub struct MediaSegment {
    /// 切片在播放列表中的顺序索引（0 开始）
    pub index: usize,
    /// HLS 媒体序列号（Media Sequence Number）
    pub sequence: u64,
    /// 切片时长（秒）
    pub duration: f64,
    /// 完整绝对下载 URL
    pub url: String,
    /// 加密密钥信息（若未加密则为 None）
    pub key: Option<EncryptionKey>,
    /// 字节范围 (length, optional_offset)
    pub byte_range: Option<(u64, Option<u64>)>,
    /// 是否为 fMP4 初始化分片 (#EXT-X-MAP)
    pub is_init_segment: bool,
}

/// 主播放列表（Master Playlist）变体流信息
#[derive(Debug, Clone, PartialEq)]
pub struct MasterVariant {
    pub url: String,
    pub bandwidth: Option<u64>,
    pub resolution: Option<(u32, u32)>,
    pub codecs: Option<String>,
}

/// 媒体播放列表（Media Playlist）完整元数据
#[derive(Debug, Clone, PartialEq)]
pub struct MediaPlaylist {
    pub target_duration: Option<f64>,
    pub media_sequence: u64,
    /// 是否有点播结束标识（#EXT-X-ENDLIST）
    pub is_endlist: bool,
    /// fMP4 初始化分片 (#EXT-X-MAP)
    pub init_segment: Option<MediaSegment>,
    /// 切片列表
    pub segments: Vec<MediaSegment>,
}

/// 判断 M3U8 内容类型（Master Playlist 还是 Media Playlist）
pub enum ParsedPlaylist {
    Master(Vec<MasterVariant>),
    Media(MediaPlaylist),
}

/// 解析 M3U8 文本内容
pub fn parse_m3u8(content: &str, base_url: &str) -> Result<ParsedPlaylist, String> {
    let trimmed = content.trim();
    if !trimmed.starts_with("#EXTM3U") {
        return Err("无效的 M3U8 文件：缺少 #EXTM3U 头部".to_string());
    }

    if trimmed.contains("#EXT-X-STREAM-INF") {
        let variants = parse_master_playlist(content, base_url)?;
        Ok(ParsedPlaylist::Master(variants))
    } else {
        let media = parse_media_playlist(content, base_url)?;
        Ok(ParsedPlaylist::Media(media))
    }
}

/// 解析 Master Playlist
pub fn parse_master_playlist(content: &str, base_url: &str) -> Result<Vec<MasterVariant>, String> {
    let mut variants = Vec::new();
    let mut current_stream_inf: Option<HashMap<String, String>> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with("#EXT-X-STREAM-INF:") {
            let attrs = parse_attributes(&line["#EXT-X-STREAM-INF:".len()..]);
            current_stream_inf = Some(attrs);
        } else if !line.starts_with('#') {
            if let Some(attrs) = current_stream_inf.take() {
                let variant_url = resolve_url(base_url, line)?;
                let bandwidth = attrs.get("BANDWIDTH").and_then(|v| v.parse::<u64>().ok());
                let resolution = attrs.get("RESOLUTION").and_then(|v| {
                    let parts: Vec<&str> = v.split('x').collect();
                    if parts.len() == 2 {
                        let w = parts[0].parse::<u32>().ok()?;
                        let h = parts[1].parse::<u32>().ok()?;
                        Some((w, h))
                    } else {
                        None
                    }
                });
                let codecs = attrs.get("CODECS").cloned();

                variants.push(MasterVariant {
                    url: variant_url,
                    bandwidth,
                    resolution,
                    codecs,
                });
            }
        }
    }

    if variants.is_empty() {
        return Err("Master Playlist 中未找到有效的变体流 (#EXT-X-STREAM-INF)".to_string());
    }

    Ok(variants)
}

/// 选择最佳清晰度/码率的变体流
pub fn select_best_variant(variants: &[MasterVariant]) -> Option<String> {
    variants
        .iter()
        .max_by_key(|v| {
            let res_score = v.resolution.map(|(w, h)| (w as u64) * (h as u64)).unwrap_or(0);
            let bw_score = v.bandwidth.unwrap_or(0);
            (res_score, bw_score)
        })
        .map(|v| v.url.clone())
}

/// 解析 Media Playlist
pub fn parse_media_playlist(content: &str, base_url: &str) -> Result<MediaPlaylist, String> {
    let mut target_duration = None;
    let mut media_sequence = 0u64;
    let mut is_endlist = false;
    let mut init_segment = None;
    let mut segments = Vec::new();

    let mut current_key: Option<EncryptionKey> = None;
    let mut next_duration = 0.0f64;
    let mut next_byte_range = None;
    let mut current_byte_offset = 0u64;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with("#EXT-X-TARGETDURATION:") {
            target_duration = line["#EXT-X-TARGETDURATION:".len()..]
                .trim()
                .parse::<f64>()
                .ok();
        } else if line.starts_with("#EXT-X-MEDIA-SEQUENCE:") {
            if let Ok(seq) = line["#EXT-X-MEDIA-SEQUENCE:".len()..].trim().parse::<u64>() {
                media_sequence = seq;
            }
        } else if line.starts_with("#EXT-X-KEY:") {
            let attrs = parse_attributes(&line["#EXT-X-KEY:".len()..]);
            let method_str = attrs.get("METHOD").map(|s| s.as_str()).unwrap_or("NONE");
            let method = match method_str.to_uppercase().as_str() {
                "NONE" => EncryptionMethod::None,
                "AES-128" => EncryptionMethod::Aes128,
                "SAMPLE-AES" => EncryptionMethod::SampleAes,
                other => EncryptionMethod::Other(other.to_string()),
            };

            if method == EncryptionMethod::None {
                current_key = None;
            } else if let Some(key_uri_raw) = attrs.get("URI") {
                let key_url = resolve_url(base_url, key_uri_raw)?;
                let iv = attrs.get("IV").and_then(|iv_str| parse_hex_iv(iv_str));
                current_key = Some(EncryptionKey {
                    method,
                    uri: key_url,
                    iv,
                });
            }
        } else if line.starts_with("#EXT-X-MAP:") {
            let attrs = parse_attributes(&line["#EXT-X-MAP:".len()..]);
            if let Some(uri_raw) = attrs.get("URI") {
                let init_url = resolve_url(base_url, uri_raw)?;
                let byte_range = attrs.get("BYTERANGE").and_then(|br_str| parse_byte_range(br_str));
                init_segment = Some(MediaSegment {
                    index: 0,
                    sequence: media_sequence,
                    duration: 0.0,
                    url: init_url,
                    key: current_key.clone(),
                    byte_range,
                    is_init_segment: true,
                });
            }
        } else if line.starts_with("#EXT-X-BYTERANGE:") {
            next_byte_range = parse_byte_range(&line["#EXT-X-BYTERANGE:".len()..]);
        } else if line.starts_with("#EXTINF:") {
            let inf_part = &line["#EXTINF:".len()..];
            let dur_str = inf_part.split(',').next().unwrap_or("0").trim();
            next_duration = dur_str.parse::<f64>().unwrap_or(0.0);
        } else if line == "#EXT-X-ENDLIST" {
            is_endlist = true;
        } else if !line.starts_with('#') {
            let seg_url = resolve_url(base_url, line)?;
            let seg_index = segments.len();
            let seg_seq = media_sequence.saturating_add(seg_index as u64);

            let byte_range = if let Some((len, opt_offset)) = next_byte_range.take() {
                let actual_offset = opt_offset.unwrap_or(current_byte_offset);
                current_byte_offset = actual_offset.saturating_add(len);
                Some((len, Some(actual_offset)))
            } else {
                None
            };

            segments.push(MediaSegment {
                index: seg_index,
                sequence: seg_seq,
                duration: next_duration,
                url: seg_url,
                key: current_key.clone(),
                byte_range,
                is_init_segment: false,
            });

            next_duration = 0.0;
        }
    }

    Ok(MediaPlaylist {
        target_duration,
        media_sequence,
        is_endlist,
        init_segment,
        segments,
    })
}

/// 解析 M3U8 标签属性列表，如 `KEY=VALUE,URI="https://..."`
pub fn parse_attributes(input: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut chars = input.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c.is_whitespace() || c == ',' {
            chars.next();
            continue;
        }

        // 读取键名
        let mut key = String::new();
        while let Some(&k) = chars.peek() {
            if k == '=' || k.is_whitespace() {
                break;
            }
            key.push(k);
            chars.next();
        }

        while let Some(&k) = chars.peek() {
            if k == '=' {
                chars.next();
                break;
            }
            chars.next();
        }

        let key = key.trim().to_uppercase();
        if key.is_empty() {
            continue;
        }

        // 读取值
        let mut value = String::new();
        if let Some(&'"') = chars.peek() {
            chars.next(); // 跳过开头的引号
            for v in chars.by_ref() {
                if v == '"' {
                    break;
                }
                value.push(v);
            }
        } else {
            while let Some(&v) = chars.peek() {
                if v == ',' || v.is_whitespace() {
                    break;
                }
                value.push(v);
                chars.next();
            }
        }

        map.insert(key, value);
    }

    map
}

/// 将相对 URL 转换为基于 base_url 的绝对 URL
pub fn resolve_url(base: &str, relative_or_abs: &str) -> Result<String, String> {
    let trimmed = relative_or_abs.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Ok(trimmed.to_string());
    }

    let base_parsed = Url::parse(base).map_err(|e| format!("无效的 Base URL '{base}': {e}"))?;
    let joined = base_parsed
        .join(trimmed)
        .map_err(|e| format!("无法将 '{trimmed}' 拼接至 '{base}': {e}"))?;
    Ok(joined.to_string())
}

/// 解析 16 字节 Hex IV 字符串（如 `0x1234...`）
pub fn parse_hex_iv(iv_str: &str) -> Option<[u8; 16]> {
    let raw = iv_str.trim();
    let hex_part = if let Some(stripped) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        stripped
    } else {
        raw
    };

    let mut padded = hex_part.to_string();
    if padded.len() < 32 {
        padded = format!("{:0>32}", padded);
    } else if padded.len() > 32 {
        padded = padded[..32].to_string();
    }

    let bytes = hex::decode(&padded).ok()?;
    if bytes.len() == 16 {
        let mut arr = [0u8; 16];
        arr.copy_from_slice(&bytes);
        Some(arr)
    } else {
        None
    }
}

/// 解析 ByteRange 字符串（如 `1024@512` 或 `1024`）
pub fn parse_byte_range(range_str: &str) -> Option<(u64, Option<u64>)> {
    let trimmed = range_str.trim();
    let parts: Vec<&str> = trimmed.split('@').collect();
    if parts.is_empty() {
        return None;
    }
    let length = parts[0].parse::<u64>().ok()?;
    let offset = if parts.len() > 1 {
        parts[1].parse::<u64>().ok()
    } else {
        None
    };
    Some((length, offset))
}
