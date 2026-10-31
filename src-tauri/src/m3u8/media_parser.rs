//! Stateful media-playlist parsing, kept separate from master and attribute parsing.
use super::parser::{
    parse_attributes, parse_byte_range, parse_hex_iv, resolve_url, EncryptionKey, EncryptionMethod,
    MediaPlaylist, MediaSegment,
};
use std::collections::HashMap;

pub fn parse_media_playlist(content: &str, base: &str) -> Result<MediaPlaylist, String> {
    let mut state = MediaParser::default();
    for line in content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        state.consume(line, base)?;
    }
    if state.duration.is_some() || state.range.is_some() {
        return Err("M3U8 最后一个切片缺少资源地址，已拒绝不完整列表".into());
    }
    Ok(MediaPlaylist {
        target_duration: state.target_duration,
        media_sequence: state.media_sequence,
        is_endlist: state.endlist,
        init_segment: state.init,
        segments: state.segments,
    })
}

#[derive(Default)]
struct MediaParser {
    target_duration: Option<f64>,
    media_sequence: u64,
    endlist: bool,
    init: Option<MediaSegment>,
    segments: Vec<MediaSegment>,
    key: Option<EncryptionKey>,
    duration: Option<f64>,
    range: Option<(u64, Option<u64>)>,
    next_offset: u64,
    previous_range_url: Option<String>,
    discontinuity: u64,
}

impl MediaParser {
    fn consume(&mut self, line: &str, base: &str) -> Result<(), String> {
        if let Some(value) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            let value = parse_duration(value)?;
            if value <= 0.0 {
                return Err("M3U8 目标切片时长必须大于零".into());
            }
            self.target_duration = Some(value);
        } else if let Some(value) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            if !self.segments.is_empty() {
                return Err("M3U8 媒体序列号必须先于媒体切片".into());
            }
            self.media_sequence = value.parse().map_err(|_| "M3U8 媒体序列号无效")?;
        } else if let Some(value) = line.strip_prefix("#EXT-X-DISCONTINUITY-SEQUENCE:") {
            if !self.segments.is_empty() {
                return Err("M3U8 不连续序列号必须先于媒体切片".into());
            }
            self.discontinuity = value.parse().map_err(|_| "M3U8 不连续序列号无效")?;
        } else if line == "#EXT-X-DISCONTINUITY" {
            self.discontinuity = self
                .discontinuity
                .checked_add(1)
                .ok_or("M3U8 不连续序列号溢出")?;
        } else if let Some(value) = line.strip_prefix("#EXT-X-KEY:") {
            self.key = parse_key(&parse_attributes(value)?, base)?;
        } else if let Some(value) = line.strip_prefix("#EXT-X-MAP:") {
            self.set_map(value, base)?;
        } else if let Some(value) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            self.range = Some(parse_byte_range(value).ok_or("M3U8 切片字节范围无效")?);
        } else if let Some(value) = line.strip_prefix("#EXTINF:") {
            self.duration = Some(parse_duration(value.split(',').next().unwrap_or(""))?);
        } else if line == "#EXT-X-ENDLIST" {
            self.endlist = true;
        } else if line == "#EXT-X-GAP" {
            return Err("M3U8 源包含缺失切片，无法保证完整录制，请稍后重试".into());
        } else if line.starts_with("#EXT-X-SKIP:") {
            return Err("M3U8 增量列表缺少完整媒体序列，请使用完整播放列表地址".into());
        } else if !line.starts_with('#') {
            self.add_segment(line, base)?;
        }
        Ok(())
    }

    fn set_map(&mut self, value: &str, base: &str) -> Result<(), String> {
        let attrs = parse_attributes(value)?;
        let uri = attrs
            .get("URI")
            .filter(|uri| !uri.is_empty())
            .ok_or("M3U8 初始化段缺少 URI")?;
        let url = resolve_url(base, uri)?;
        let range = attrs
            .get("BYTERANGE")
            .map(|value| parse_byte_range(value).ok_or("M3U8 初始化段字节范围无效"))
            .transpose()?;
        let range = self.resolve_range(range, &url)?;
        if self.key.as_ref().is_some_and(|key| key.iv.is_none()) {
            return Err("加密的 M3U8 初始化段必须显式指定 IV".into());
        }
        self.init = Some(MediaSegment {
            index: 0,
            sequence: self.media_sequence,
            duration: 0.0,
            url,
            key: self.key.clone(),
            byte_range: range,
            is_init_segment: true,
            initialization: None,
            discontinuity: self.discontinuity,
        });
        Ok(())
    }

    fn add_segment(&mut self, line: &str, base: &str) -> Result<(), String> {
        let url = resolve_url(base, line)?;
        let duration = self
            .duration
            .take()
            .ok_or("M3U8 媒体切片缺少 EXTINF 时长声明")?;
        let range = self.range.take();
        let range = self.resolve_range(range, &url)?;
        if let Some((length, Some(offset))) = range {
            self.next_offset = offset.checked_add(length).ok_or("M3U8 字节范围溢出")?;
            self.previous_range_url = Some(url.clone());
        } else {
            self.previous_range_url = None;
        }
        let index = self.segments.len();
        let sequence = self
            .media_sequence
            .checked_add(index as u64)
            .ok_or("M3U8 媒体序列号溢出")?;
        self.segments.push(MediaSegment {
            index,
            sequence,
            duration,
            url,
            key: self.key.clone(),
            byte_range: range,
            is_init_segment: false,
            initialization: self.init.clone().map(Box::new),
            discontinuity: self.discontinuity,
        });
        Ok(())
    }

    fn resolve_range(
        &self,
        range: Option<(u64, Option<u64>)>,
        url: &str,
    ) -> Result<Option<(u64, Option<u64>)>, String> {
        let Some((length, offset)) = range else {
            return Ok(None);
        };
        if offset.is_none() && self.previous_range_url.as_deref() != Some(url) {
            return Err("M3U8 隐式字节范围必须续接同一资源的上一范围".into());
        }
        let offset = offset.unwrap_or(self.next_offset);
        offset.checked_add(length).ok_or("M3U8 字节范围溢出")?;
        Ok(Some((length, Some(offset))))
    }
}

fn parse_duration(value: &str) -> Result<f64, String> {
    let duration: f64 = value.trim().parse().map_err(|_| "M3U8 切片时长无效")?;
    if !duration.is_finite() || duration < 0.0 {
        return Err("M3U8 切片时长必须是非负有限数".into());
    }
    Ok(duration)
}

pub(super) fn parse_key(
    attrs: &HashMap<String, String>,
    base: &str,
) -> Result<Option<EncryptionKey>, String> {
    let method = attrs
        .get("METHOD")
        .map(String::as_str)
        .ok_or("M3U8 加密声明缺少 METHOD，已拒绝下载")?;
    if method == "NONE" {
        if attrs.len() != 1 {
            return Err("M3U8 METHOD=NONE 不应包含密钥或 DRM 属性".into());
        }
        return Ok(None);
    }
    if method != "AES-128"
        || attrs
            .get("KEYFORMAT")
            .is_some_and(|format| format != "identity")
    {
        return Err("此 M3U8 使用 DRM 或不支持的加密方式，已拒绝下载，无法绕过保护".into());
    }
    if attrs
        .get("KEYFORMATVERSIONS")
        .is_some_and(|version| version != "1")
    {
        return Err("此 M3U8 使用不支持的密钥格式版本，已拒绝下载".into());
    }
    let uri = attrs
        .get("URI")
        .filter(|uri| !uri.is_empty())
        .ok_or("M3U8 AES-128 加密声明缺少密钥 URI")?;
    let iv = attrs
        .get("IV")
        .map(|value| {
            parse_hex_iv(value).ok_or("M3U8 加密 IV 无效，必须为不超过 128 位的十六进制数")
        })
        .transpose()?;
    Ok(Some(EncryptionKey {
        method: EncryptionMethod::Aes128,
        uri: resolve_url(base, uri)?,
        iv,
    }))
}
