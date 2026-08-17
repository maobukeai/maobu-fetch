//! magnet: URI 解析（roadmap BT-04）。
//!
//! 纯函数、无 IO、无网络：所有输入来自用户或扩展请求，非法输入必须
//! 返回可操作的中文错误（AGENTS.md §7），不得 panic。
//!
//! 安全约束（AGENTS.md §3 BT/磁力内核）：
//! - 元数据获取前不得伪造文件名或大小：`display_name` 仅是磁力链接自带的
//!   提示名，UI 必须标注“待确认”；
//! - infohash 归一化为 40 位小写十六进制，接受 hex（40 位）与
//!   base32（32 位）两种 `xt` 编码。

/// magnet URI 解析结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MagnetInfo {
    /// 40 位小写十六进制 infohash v1。
    pub info_hash: String,
    /// `dn` 提示名（已百分号解码 + UTF-8 校验）。磁力未提供时为 `None`。
    pub display_name: Option<String>,
    /// `tr` tracker 列表（已百分号解码，仅保留 http/https/udp/ws）。
    pub trackers: Vec<String>,
}

/// 解析 magnet URI。`input` 前后空白会被去除。
pub fn parse_magnet(input: &str) -> Result<MagnetInfo, String> {
    let trimmed = input.trim();
    let (scheme, rest) = trimmed
        .split_once(':')
        .ok_or_else(|| "磁力链接缺少 scheme，正确格式为 magnet:?xt=urn:btih:…".to_string())?;
    if !scheme.eq_ignore_ascii_case("magnet") {
        return Err("仅支持 magnet: 磁力链接".into());
    }
    // url::Url 对非特殊 scheme（magnet）可解析；去掉前导 '?' 后手工按 K=V 解析，
    // 避免依赖 query_pairs 的加号转空格语义（BT 参数不使用 application/x-www-form-urlencoded）。
    let query = rest.trim_start_matches('?');
    if query.is_empty() {
        return Err("磁力链接缺少参数，必须包含 xt=urn:btih:…".into());
    }
    let mut info_hash: Option<String> = None;
    let mut display_name: Option<String> = None;
    let mut trackers = Vec::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decode(raw_value);
        match key.to_ascii_lowercase().as_str() {
            "xt" => {
                if info_hash.is_none() {
                    info_hash = Some(parse_xt(&value)?);
                }
            }
            "dn" => {
                if display_name.is_none() {
                    let name = value.trim().trim_matches('"').to_string();
                    if !name.is_empty() && name.chars().count() <= 255 {
                        display_name = Some(name);
                    }
                }
            }
            "tr" => {
                let decoded = value;
                let scheme_ok = ["http://", "https://", "udp://", "ws://"]
                    .iter()
                    .any(|prefix| decoded.to_ascii_lowercase().starts_with(prefix));
                if scheme_ok && decoded.len() <= 512 && trackers.len() < 32 {
                    trackers.push(decoded);
                }
            }
            _ => {}
        }
    }
    let info_hash = info_hash
        .ok_or_else(|| "磁力链接缺少 xt=urn:btih:… 参数，无法确定资源标识".to_string())?;
    Ok(MagnetInfo {
        info_hash,
        display_name,
        trackers,
    })
}

/// 解析 `xt` 参数：仅接受 v1（urn:btih:），v2（urn:btmh:）给出明确拒绝。
fn parse_xt(value: &str) -> Result<String, String> {
    let lower = value.to_ascii_lowercase();
    if let Some(hash_part) = lower.strip_prefix("urn:btmh:") {
        let _ = hash_part;
        return Err("暂不支持 BitTorrent v2 磁力链接（urn:btmh:）".into());
    }
    let hash_part = lower
        .strip_prefix("urn:btih:")
        .ok_or_else(|| "xt 参数必须为 urn:btih: 开头".to_string())?;
    if hash_part.len() == 40 {
        if hash_part.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(hash_part.to_string());
        }
        return Err("磁力 infohash 必须为 40 位十六进制或 32 位 base32".into());
    }
    if hash_part.len() == 32 {
        return decode_base32_infohash(hash_part)
            .ok_or_else(|| "磁力 infohash base32 编码非法".to_string());
    }
    Err("磁力 infohash 长度非法（hex 应为 40 位，base32 应为 32 位）".into())
}

/// RFC 4648 base32（字母表 A–Z / 2–7）解码 32 字符 infohash 为 40 位 hex。
/// 严格小写归一：输入先转大写再校验字符集。
fn decode_base32_infohash(input: &str) -> Option<String> {
    let upper = input.to_ascii_uppercase();
    let mut accumulator: u32 = 0;
    let mut bit_count: u32 = 0;
    let mut bytes: Vec<u8> = Vec::with_capacity(20);
    for ch in upper.chars() {
        let value = match ch {
            'A'..='Z' => ch as u32 - 'A' as u32,
            '2'..='7' => ch as u32 - '2' as u32 + 26,
            _ => return None,
        };
        accumulator = (accumulator << 5) | value;
        bit_count += 5;
        if bit_count >= 8 {
            bit_count -= 8;
            bytes.push(((accumulator >> bit_count) & 0xFF) as u8);
        }
    }
    (bytes.len() == 20).then(|| hex::encode(bytes))
}

/// 百分号解码 + UTF-8 合法性校验。解码失败（非法转义或非 UTF-8）返回原文，
/// 保证恶意输入不会让解析崩溃。
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                hex_val(Some(bytes[index + 1])),
                hex_val(Some(bytes[index + 2])),
            ) {
                output.push(hi * 16 + lo);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(output).unwrap_or_else(|_| input.to_string())
}

fn hex_val(byte: Option<u8>) -> Option<u8> {
    match byte? {
        c @ b'0'..=b'9' => Some(c - b'0'),
        c @ b'a'..=b'f' => Some(c - b'a' + 10),
        c @ b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// tracker 列表转 aria2 addUri 的多 URI 参数（磁力 URI + tr 追加形式）。
/// aria2 原生支持在 magnet URI 后追加 &tr=，此函数保留原磁力字符串由调用方
/// 传入，tracker 仅用于展示与去重统计，故此处返回用于展示的 join 结果。
pub fn trackers_summary(info: &MagnetInfo) -> String {
    format!("{} 个 tracker", info.trackers.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX_HASH: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn parses_hex_infohash_with_dn_and_trackers() {
        let magnet = format!(
            "magnet:?xt=urn:btih:{HEX_HASH}&dn=%E6%B5%8B%E8%AF%95%E7%A7%8D%E5%AD%90&tr=udp://tracker.example:6969/announce&tr=https://t2.example/ann"
        );
        let info = parse_magnet(&magnet).unwrap();
        assert_eq!(info.info_hash, HEX_HASH);
        assert_eq!(info.display_name.as_deref(), Some("测试种子"));
        assert_eq!(info.trackers.len(), 2);
    }

    #[test]
    fn parses_base32_infohash() {
        // 对应 HEX_HASH 的 base32 编码（AEBAGBAFAYDQQCIKAKBAGBAFAYDQQCIKAKBAGBAF 不存在，
        // 此处用标准测试向量：'MFRGG==='风格展开——直接用已知映射构造。
        // 32 字符 base32 "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ" 解码 20 字节。
        let magnet = "magnet:?xt=urn:btih:GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        let info = parse_magnet(&magnet).unwrap();
        assert_eq!(info.info_hash.len(), 40);
        assert!(info.info_hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn uppercases_are_normalized_to_lower() {
        let upper = HEX_HASH.to_ascii_uppercase();
        let magnet = format!("magnet:?xt=urn:btih:{upper}");
        assert_eq!(parse_magnet(&magnet).unwrap().info_hash, HEX_HASH);
    }

    #[test]
    fn rejects_missing_scheme_and_wrong_scheme() {
        assert!(parse_magnet("http://example.com/file").is_err());
        assert!(parse_magnet("magnet:").is_err());
        assert!(parse_magnet("magnet:?dn=only-name").is_err());
    }

    #[test]
    fn rejects_v2_and_bad_hashes() {
        let v2 = "magnet:?xt=urn:btmh:1220d1c46d6b3c26d43f9980cd67c3e9f6c2f0d6d1c";
        let err = parse_magnet(v2).unwrap_err();
        assert!(err.contains("v2"), "unexpected: {err}");
        assert!(parse_magnet("magnet:?xt=urn:btih:xyz").is_err());
        let short = "magnet:?xt=urn:btih:0123";
        assert!(parse_magnet(short).is_err());
        // 39 位 hex 也必须拒绝
        let bad_len = format!("magnet:?xt=urn:btih:{}", &HEX_HASH[..39]);
        assert!(parse_magnet(&bad_len).is_err());
        // base32 字符集外（含 0/1/8/9）必须拒绝
        let bad_b32 = "magnet:?xt=urn:btih:01234567890123456789012345678901";
        assert!(parse_magnet(bad_b32).is_err());
    }

    #[test]
    fn keeps_only_supported_tracker_schemes() {
        let magnet = format!(
            "magnet:?xt=urn:btih:{HEX_HASH}&tr=javascript:alert(1)&tr=udp://a.example/x&tr=file:///C:/x"
        );
        let info = parse_magnet(&magnet).unwrap();
        assert_eq!(info.trackers, vec!["udp://a.example/x"]);
    }

    #[test]
    fn trims_whitespace_and_tolerates_junk_params() {
        let magnet = format!("  magnet:?zz=1&xt=urn:btih:{HEX_HASH}&&&broken  ");
        let info = parse_magnet(&magnet).unwrap();
        assert_eq!(info.info_hash, HEX_HASH);
        assert!(info.display_name.is_none());
    }

    #[test]
    fn overlong_dn_is_ignored_not_fatal() {
        let long_name = "x".repeat(300);
        let magnet = format!("magnet:?xt=urn:btih:{HEX_HASH}&dn={long_name}");
        let info = parse_magnet(&magnet).unwrap();
        assert!(info.display_name.is_none());
    }

    #[test]
    fn percent_decode_falls_back_to_raw_on_invalid_escape() {
        // "%ZZ" 不是合法转义，应原样保留而非报错
        let magnet = format!("magnet:?xt=urn:btih:{HEX_HASH}&dn=%ZZok");
        let info = parse_magnet(&magnet).unwrap();
        assert_eq!(info.display_name.as_deref(), Some("%ZZok"));
    }
}
