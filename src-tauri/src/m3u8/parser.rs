// HLS M3U8 播放列表解析器（纯 Rust 实现，遵循 RFC 8216 标准）

use std::collections::{HashMap, HashSet};
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
    /// 此切片实际使用的初始化段，支持直播/不连续点切换 EXT-X-MAP。
    pub initialization: Option<Box<MediaSegment>>,
    pub discontinuity: u64,
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
    if trimmed.lines().next() != Some("#EXTM3U") {
        return Err("无效的 M3U8 文件：缺少 #EXTM3U 头部".to_string());
    }

    for line in content.lines().map(str::trim) {
        if let Some(attributes) = line.strip_prefix("#EXT-X-SESSION-KEY:") {
            if super::media_parser::parse_key(&parse_attributes(attributes)?, base_url)?.is_none() {
                return Err("M3U8 会话密钥不能使用 METHOD=NONE".into());
            }
        }
    }

    if trimmed.contains("#EXT-X-STREAM-INF") {
        if content
            .lines()
            .map(str::trim)
            .any(|line| line.starts_with("#EXTINF:") || line.starts_with("#EXT-X-MEDIA-SEQUENCE:"))
        {
            return Err("M3U8 主播放列表混入媒体列表标签，已拒绝解析".into());
        }
        let variants = parse_master_playlist(content, base_url)?;
        Ok(ParsedPlaylist::Master(variants))
    } else {
        let media = parse_media_playlist(content, base_url)?;
        Ok(ParsedPlaylist::Media(media))
    }
}

/// 解析 Master Playlist
pub fn parse_master_playlist(content: &str, base_url: &str) -> Result<Vec<MasterVariant>, String> {
    let audio_groups = master_audio_groups(content)?;
    let mut variants = Vec::new();
    let mut current_stream_inf: Option<HashMap<String, String>> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with("#EXT-X-STREAM-INF:") {
            if current_stream_inf.is_some() {
                return Err("M3U8 变体流缺少资源地址".into());
            }
            let attrs = parse_attributes(&line["#EXT-X-STREAM-INF:".len()..])?;
            if attrs
                .get("AUDIO")
                .is_some_and(|group| !audio_groups.contains(group))
            {
                return Err("M3U8 变体引用了不存在的音轨组，未下载不完整媒体".into());
            }
            current_stream_inf = Some(attrs);
        } else if !line.starts_with('#') {
            if let Some(attrs) = current_stream_inf.take() {
                let variant_url = resolve_url(base_url, line)?;
                let bandwidth = Some(
                    attrs
                        .get("BANDWIDTH")
                        .and_then(|v| v.parse::<u64>().ok())
                        .filter(|value| *value > 0)
                        .ok_or("M3U8 变体流缺少有效 BANDWIDTH")?,
                );
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

    if current_stream_inf.is_some() {
        return Err("M3U8 变体流缺少资源地址".into());
    }
    if variants.is_empty() {
        return Err("Master Playlist 中未找到有效的变体流 (#EXT-X-STREAM-INF)".to_string());
    }

    Ok(variants)
}

fn master_audio_groups(content: &str) -> Result<HashSet<String>, String> {
    let mut groups = HashSet::new();
    for line in content.lines().map(str::trim) {
        let Some(raw) = line.strip_prefix("#EXT-X-MEDIA:") else {
            continue;
        };
        let attributes = parse_attributes(raw)?;
        if attributes
            .get("TYPE")
            .is_none_or(|kind| !kind.eq_ignore_ascii_case("AUDIO"))
        {
            continue;
        }
        if attributes.contains_key("URI") {
            return Err("原生 M3U8 暂不支持主列表的外部独立音轨，未只下载无声视频；请使用媒体解析选择完整音视频格式或安装媒体工具".into());
        }
        let group = attributes
            .get("GROUP-ID")
            .filter(|group| !group.is_empty())
            .ok_or("M3U8 音轨组缺少有效 GROUP-ID")?;
        groups.insert(group.clone());
    }
    Ok(groups)
}

/// 选择最佳清晰度/码率的变体流
pub fn select_best_variant(variants: &[MasterVariant]) -> Option<String> {
    variants
        .iter()
        .max_by_key(|v| {
            let res_score = v
                .resolution
                .map(|(w, h)| (w as u64) * (h as u64))
                .unwrap_or(0);
            let bw_score = v.bandwidth.unwrap_or(0);
            (res_score, bw_score)
        })
        .map(|v| v.url.clone())
}

/// 解析 Media Playlist
pub fn parse_media_playlist(content: &str, base_url: &str) -> Result<MediaPlaylist, String> {
    super::media_parser::parse_media_playlist(content, base_url)
}

/// 解析 M3U8 标签属性列表，如 `KEY=VALUE,URI="https://..."`
pub fn parse_attributes(input: &str) -> Result<HashMap<String, String>, String> {
    let bytes = input.as_bytes();
    let mut cursor = 0;
    let mut attributes = HashMap::new();
    while cursor < bytes.len() {
        skip_spaces(bytes, &mut cursor);
        let start = cursor;
        while cursor < bytes.len()
            && (bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == b'-')
        {
            cursor += 1;
        }
        if start == cursor {
            return Err("M3U8 属性名称无效".into());
        }
        let key = input[start..cursor].to_ascii_uppercase();
        skip_spaces(bytes, &mut cursor);
        if bytes.get(cursor) != Some(&b'=') {
            return Err("M3U8 属性缺少等号".into());
        }
        cursor += 1;
        skip_spaces(bytes, &mut cursor);
        let value = parse_attribute_value(input, &mut cursor)?;
        if attributes.insert(key, value).is_some() {
            return Err("M3U8 属性重复，已拒绝有歧义的声明".into());
        }
        skip_spaces(bytes, &mut cursor);
        if cursor == bytes.len() {
            break;
        }
        if bytes[cursor] != b',' {
            return Err("M3U8 属性之间缺少逗号".into());
        }
        cursor += 1;
        if cursor == bytes.len() {
            return Err("M3U8 属性列表不能以逗号结尾".into());
        }
    }
    Ok(attributes)
}

fn skip_spaces(bytes: &[u8], cursor: &mut usize) {
    while *cursor < bytes.len() && bytes[*cursor].is_ascii_whitespace() {
        *cursor += 1;
    }
}

fn parse_attribute_value(input: &str, cursor: &mut usize) -> Result<String, String> {
    let bytes = input.as_bytes();
    if bytes.get(*cursor) == Some(&b'"') {
        *cursor += 1;
        let start = *cursor;
        while *cursor < bytes.len() && bytes[*cursor] != b'"' {
            *cursor += 1;
        }
        if *cursor == bytes.len() {
            return Err("M3U8 属性引号没有闭合".into());
        }
        let value = input[start..*cursor].to_owned();
        *cursor += 1;
        Ok(value)
    } else {
        let start = *cursor;
        while *cursor < bytes.len()
            && bytes[*cursor] != b','
            && !bytes[*cursor].is_ascii_whitespace()
        {
            *cursor += 1;
        }
        if *cursor == start {
            return Err("M3U8 属性值为空".into());
        }
        Ok(input[start..*cursor].to_owned())
    }
}

/// 将相对 URL 转换为基于 base_url 的绝对 URL
pub fn resolve_url(base: &str, relative_or_abs: &str) -> Result<String, String> {
    let trimmed = relative_or_abs.trim();
    let base_parsed = Url::parse(base).map_err(|_| "M3U8 基础地址无效")?;
    let joined = base_parsed.join(trimmed).map_err(|_| "M3U8 资源地址无效")?;
    if !matches!(joined.scheme(), "http" | "https") || joined.host_str().is_none() {
        return Err("M3U8 仅允许 HTTP/HTTPS 媒体、播放列表和密钥地址".into());
    }
    Ok(joined.to_string())
}

/// 解析 16 字节 Hex IV 字符串（如 `0x1234...`）
pub fn parse_hex_iv(iv_str: &str) -> Option<[u8; 16]> {
    let raw = iv_str.trim();
    let hex_part = if let Some(stripped) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X"))
    {
        stripped
    } else {
        raw
    };

    if hex_part.is_empty()
        || hex_part.len() > 32
        || !hex_part.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    let mut padded = hex_part.to_string();
    if padded.len() < 32 {
        padded = format!("{:0>32}", padded);
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
    if parts.is_empty() || parts.len() > 2 {
        return None;
    }
    let length = parts[0].parse::<u64>().ok()?;
    if length == 0 {
        return None;
    }
    let offset = if parts.len() > 1 {
        Some(parts[1].parse::<u64>().ok()?)
    } else {
        None
    };
    if offset.is_some_and(|start| start.checked_add(length).is_none()) {
        return None;
    }
    Some((length, offset))
}
