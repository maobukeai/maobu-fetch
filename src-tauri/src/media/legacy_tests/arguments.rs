use super::*;
use std::collections::HashMap;
fn parse_yt_dlp_progress(line: &str) -> Option<(f64, u64, u64, u64)> {
    let trimmed = line.trim_matches(|c: char| c == '\r' || c == '\n' || c == ' ' || c == '\t');
    let clean = if let Some(idx) = trimmed.find("[download]") {
        &trimmed[idx..]
    } else {
        return None;
    };
    let parts: Vec<&str> = clean.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    if let Some(pct_idx) = parts.iter().position(|p| p.ends_with('%')) {
        let pct_str = parts[pct_idx].strip_suffix('%')?;
        let percent: f64 = pct_str.parse().ok()?;

        let of_idx = parts.iter().position(|p| *p == "of")?;
        // yt-dlp 对 HLS fragment 进度输出形如：
        //   [download] 10.0% of ~   8.82KiB at 412.33B/s ETA Unknown (frag 0/10)
        // `~` 是独立 token（表示估算值），需要跳过取下一个 token 作为 size。
        let size_idx = of_idx + 1;
        let size_str = if parts.get(size_idx) == Some(&"~") {
            parts.get(size_idx + 1)?
        } else {
            parts.get(size_idx)?
        };
        let total_bytes = parse_size_to_bytes(size_str)?;

        let speed = parts
            .iter()
            .position(|p| *p == "at")
            .and_then(|at_idx| parts.get(at_idx + 1))
            .and_then(|s| parse_speed_to_bytes(s))
            .unwrap_or(0);

        let downloaded_bytes = ((percent / 100.0) * (total_bytes as f64)) as u64;
        return Some((percent, downloaded_bytes, total_bytes, speed));
    }
    if let Some(at_idx) = parts.iter().position(|p| *p == "at") {
        if at_idx >= 2 {
            let downloaded = parse_size_to_bytes(parts[at_idx - 1])?;
            let speed = parts
                .get(at_idx + 1)
                .and_then(|s| parse_speed_to_bytes(s))
                .unwrap_or(0);
            return Some((0.0, downloaded, 0, speed));
        }
    }
    None
}

fn parse_size_to_bytes(s: &str) -> Option<u64> {
    let clean = s.trim_start_matches('~');
    let (num_part, unit) = if clean.ends_with("KiB")
        || clean.ends_with("KIB")
        || clean.ends_with("kb")
        || clean.ends_with("KB")
    {
        (
            clean
                .get(..clean.len() - 3)
                .or_else(|| clean.get(..clean.len() - 2))?,
            1024_f64,
        )
    } else if clean.ends_with("MiB")
        || clean.ends_with("MIB")
        || clean.ends_with("mb")
        || clean.ends_with("MB")
    {
        (
            clean
                .get(..clean.len() - 3)
                .or_else(|| clean.get(..clean.len() - 2))?,
            1024.0 * 1024.0,
        )
    } else if clean.ends_with("GiB")
        || clean.ends_with("GIB")
        || clean.ends_with("gb")
        || clean.ends_with("GB")
    {
        (
            clean
                .get(..clean.len() - 3)
                .or_else(|| clean.get(..clean.len() - 2))?,
            1024.0 * 1024.0 * 1024.0,
        )
    } else if clean.ends_with("B") {
        (clean.get(..clean.len() - 1)?, 1.0)
    } else {
        (clean, 1.0)
    };
    let val: f64 = num_part.trim().parse().ok()?;
    Some((val * unit) as u64)
}

fn parse_speed_to_bytes(s: &str) -> Option<u64> {
    let clean = s.strip_suffix("/s")?;
    parse_size_to_bytes(clean)
}

#[test]
fn lightweight_media_download_does_not_require_ffmpeg_arguments() {
    let arguments = media_arguments("18", "video.mp4", false);
    assert!(!arguments
        .iter()
        .any(|value| value == "--merge-output-format"));
    assert!(arguments.windows(2).any(|pair| pair == ["-f", "18"]));
}

/// 验证 build_youtube_extractor_arg 在所有 (has_cookie, has_po) 组合下
/// 都把 android_vr 作为最后回退客户端纳入列表。
#[test]
fn youtube_extractor_args_always_includes_android_vr_as_fallback() {
    // 关键回归测试：android_vr 是唯一不需要 PO Token 的客户端，
    // 必须在所有 4 种 (has_cookie, has_po) 组合中都出现作为最后回退。
    // 否则用户配置 Cookie 后反而无法下载（因为 web/mweb/ios 都需要 PO Token）。
    let cases = [
        ("token-abc", true),
        ("", true),
        ("token-abc", false),
        ("", false),
    ];
    for (po_token, has_cookie) in cases {
        let arg = build_youtube_extractor_arg(po_token, has_cookie);
        assert!(
            arg.contains("android_vr"),
            "android_vr must be in client list for (has_cookie={}, po_token={:?}). Got: {}",
            has_cookie,
            po_token,
            arg
        );
    }
}

#[test]
fn youtube_extractor_args_includes_po_token_when_provided() {
    let arg = build_youtube_extractor_arg("my-po-token", false);
    assert!(
        arg.contains("po_token=my-po-token"),
        "po_token must be in args. Got: {}",
        arg
    );
}

#[test]
fn youtube_extractor_args_omits_po_token_when_empty() {
    let arg = build_youtube_extractor_arg("", true);
    assert!(
        !arg.contains("po_token="),
        "po_token must NOT be in args when empty. Got: {}",
        arg
    );
}

#[test]
fn youtube_extractor_args_with_cookie_uses_web_clients_first() {
    // 有 Cookie 时优先用 web 系客户端（带登录态），android_vr 作为回退
    let arg = build_youtube_extractor_arg("", true);
    let clients_part = arg.split(';').next().unwrap_or("");
    assert!(
        clients_part.contains("web"),
        "web client must be first when has_cookie. Got: {}",
        arg
    );
    // android_vr 应该在 web 系之后
    let web_pos = clients_part.find("web").unwrap();
    let vr_pos = clients_part.find("android_vr").unwrap();
    assert!(
        vr_pos > web_pos,
        "android_vr must come after web clients. Got: {}",
        arg
    );
}

#[test]
fn test_parse_yt_dlp_progress() {
    // Test case 1: Standard progress with percent, total size, speed and ETA
    let line = "[download]   5.2% of  201.95MiB at    8.43MiB/s ETA 00:22";
    let res = parse_yt_dlp_progress(line);
    assert!(res.is_some());
    let (pct, downloaded, total, speed) = res.unwrap();
    assert!((pct - 5.2).abs() < 1e-6);
    assert_eq!(total, (201.95 * 1024.0 * 1024.0) as u64);
    assert_eq!(speed, (8.43 * 1024.0 * 1024.0) as u64);
    assert_eq!(downloaded, ((5.2 / 100.0) * (total as f64)) as u64);

    // Test case 2: Estimated size progress (with ~)
    let line_est = "[download]  50.2% of ~100.00MB at  1.50MB/s ETA 00:30";
    let res_est = parse_yt_dlp_progress(line_est);
    assert!(res_est.is_some());
    let (pct, downloaded, total, speed) = res_est.unwrap();
    assert!((pct - 50.2).abs() < 1e-6);
    assert_eq!(total, 100 * 1024 * 1024);
    assert_eq!(downloaded, ((50.2 / 100.0) * (total as f64)) as u64);
    assert_eq!(speed, (1.5 * 1024.0 * 1024.0) as u64);

    // Test case 3: Unknown total size progress (DASH/HLS stream)
    let line_unk = "[download]   50.2MiB at    8.43MiB/s ETA --:--";
    let res_unk = parse_yt_dlp_progress(line_unk);
    assert!(res_unk.is_some());
    let (pct, downloaded, total, speed) = res_unk.unwrap();
    assert!((pct - 0.0).abs() < 1e-6);
    assert_eq!(downloaded, (50.2 * 1024.0 * 1024.0) as u64);
    assert_eq!(total, 0);
    assert_eq!(speed, (8.43 * 1024.0 * 1024.0) as u64);

    // Test case 4: Non-download line
    let line_other = "[youtube] abc123xyz: Downloading webpage";
    assert!(parse_yt_dlp_progress(line_other).is_none());

    // Test case 5: Twitter HLS fragment 进度（`~` 是独立 token）
    // 实际 yt-dlp 输出：`[download]  10.0% of ~   8.82KiB at 412.33B/s ETA Unknown (frag 0/10)`
    // `~` 和 size 之间有多个空格，split_whitespace 会把它们分开。
    let line_frag = "[download]  10.0% of ~   8.82KiB at    412.33B/s ETA Unknown (frag 0/10)";
    let res_frag = parse_yt_dlp_progress(line_frag);
    assert!(res_frag.is_some(), "HLS fragment 进度应能解析");
    let (pct, _downloaded, total, speed) = res_frag.unwrap();
    assert!((pct - 10.0).abs() < 1e-6);
    assert_eq!(total, (8.82 * 1024.0) as u64);
    assert_eq!(speed, 412);

    // Test case 6: Twitter HLS fragment 后续进度（~ 后跟 MiB）
    let line_frag2 = "[download] 100.0% of ~   5.14MiB at  275.66KiB/s ETA 00:29 (frag 10/10)";
    let res_frag2 = parse_yt_dlp_progress(line_frag2);
    assert!(res_frag2.is_some());
    let (pct, _downloaded, total, speed) = res_frag2.unwrap();
    assert!((pct - 100.0).abs() < 1e-6);
    assert_eq!(total, (5.14 * 1024.0 * 1024.0) as u64);
    assert_eq!(speed, (275.66 * 1024.0) as u64);
}

#[test]
fn merged_media_download_enables_ffmpeg_output_format() {
    let arguments = media_arguments("bestvideo+bestaudio", "video.mp4", true);
    assert!(arguments
        .windows(2)
        .any(|pair| pair == ["--merge-output-format", "mp4"]));
}

#[test]
fn split_auth_headers_separates_cookie_referer_user_agent() {
    let mut headers = HashMap::new();
    headers.insert("Cookie".to_string(), "session=abc".to_string());
    headers.insert(
        "Referer".to_string(),
        "https://example.com/page".to_string(),
    );
    headers.insert("User-Agent".to_string(), "TestUA/1.0".to_string());
    headers.insert("X-Custom".to_string(), "keep-me".to_string());

    let (cookie, referer, user_agent, rest) = split_auth_headers(&headers);
    assert_eq!(cookie.as_deref(), Some("session=abc"));
    assert_eq!(referer.as_deref(), Some("https://example.com/page"));
    assert_eq!(user_agent.as_deref(), Some("TestUA/1.0"));
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].0, "X-Custom");
    assert_eq!(rest[0].1, "keep-me");
}

#[test]
fn split_auth_headers_is_case_insensitive() {
    let mut headers = HashMap::new();
    headers.insert("cookie".to_string(), "v1".to_string());
    headers.insert("REFERER".to_string(), "v2".to_string());
    headers.insert("user-AGENT".to_string(), "v3".to_string());

    let (cookie, referer, user_agent, rest) = split_auth_headers(&headers);
    assert_eq!(cookie.as_deref(), Some("v1"));
    assert_eq!(referer.as_deref(), Some("v2"));
    assert_eq!(user_agent.as_deref(), Some("v3"));
    assert!(rest.is_empty());
}

#[test]
fn split_auth_headers_handles_empty_map() {
    let headers = HashMap::new();
    let (cookie, referer, user_agent, rest) = split_auth_headers(&headers);
    assert!(cookie.is_none());
    assert!(referer.is_none());
    assert!(user_agent.is_none());
    assert!(rest.is_empty());
}
