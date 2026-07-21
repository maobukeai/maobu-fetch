//! Task 26: 应用更新检查与提醒。
//!
//! 只读检查 GitHub Releases 最新版本，**不自动下载**（AGENTS.md §6）。
//! 模块职责单一：
//! - 拉取最新 release 的 tag、发布时间、HTML 页面、release notes；
//! - 与当前编译期版本比较，判断是否有更新；
//! - 提供扩展与桌面端版本兼容性的简化检查。
//!
//! 不引入新依赖（AGENTS.md §8），复用现有 reqwest。
//! 所有网络/解析错误使用 `redact_sensitive` 脱敏后以中文返回，不泄露内部细节。

use crate::manager::redact_sensitive;
use crate::models::{ExtensionCompatibilityResult, UpdateCheckResult, UpdateInfo};
use reqwest::Client;
use std::cmp::Ordering;
use std::time::Duration;

// TODO(user): 仓库地址占位符。发布前请填入真实 owner/repo，并在 release 流程中验证 GitHub Releases API 可访问。
// 当前为开源项目主页占位（与 package.json、tauri.conf.json、extension/manifest.json 保持一致）。
const GITHUB_OWNER: &str = "maobukeai";
const GITHUB_REPO: &str = "maobu-fetch";

/// GitHub Releases API 端点（最新 release）。
const RELEASES_LATEST_URL: &str =
    "https://api.github.com/repos/maobukeai/maobu-fetch/releases/latest";

/// `html_url` 缺失时的回退页面。
const RELEASES_PAGE: &str = "https://github.com/maobukeai/maobu-fetch/releases";

/// 当前应用版本（编译期从 Cargo.toml 注入）。
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// GitHub API 要求显式 User-Agent，否则返回 403。
const USER_AGENT: &str = concat!(
    "MaobuFetch/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/maobukeai/maobu-fetch)"
);

/// 构造专用 HTTP 客户端：固定 UA、连接超时 15s、总超时 20s。
///
/// 不复用下载内核的 `build_client`：更新检查是低频独立调用，
/// 不应受用户代理/代理覆盖等下载偏好的影响，避免本地代理拦截 GitHub API。
fn build_update_client() -> Result<Client, String> {
    Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败：{e}"))
}

/// 异步检查应用更新（Task 26.2）。
///
/// 通过 GitHub Releases API 读取最新 release 的 `tag_name`、`published_at`、
/// `html_url`、`body`。**不下载任何资产**，仅返回信息供前端展示（AGENTS.md §6）。
///
/// 失败时返回 `UpdateCheckResult`，`error` 字段为脱敏后的中文错误，
/// `latest = None`、`has_update = false`。不会 panic，不会 unwrap 可恢复错误。
pub async fn check_app_update() -> UpdateCheckResult {
    let current = APP_VERSION;

    let client = match build_update_client() {
        Ok(c) => c,
        Err(e) => {
            return error_result(current, &e);
        }
    };

    let response = match client.get(RELEASES_LATEST_URL).send().await {
        Ok(r) => r,
        Err(e) => {
            return error_result(current, &format!("无法连接更新服务器：{e}"));
        }
    };

    let status = response.status();
    if !status.is_success() {
        let err_body = response.text().await.unwrap_or_default();
        let display_err = if status.as_u16() == 403 && (err_body.contains("rate limit") || err_body.contains("Rate limit")) {
            "当前网络 IP 请求 GitHub 接口太频繁，已触发限流 (403)，请稍后重试或更换代理节点".to_string()
        } else if status.as_u16() == 404 {
            "未找到可用版本 (404)。请确认 GitHub 仓库已设置为公开 (Public) 且已发布至少一个 Release 包".to_string()
        } else {
            format!("更新服务器返回 HTTP {}：{}", status.as_u16(), err_body)
        };
        return error_result(current, &display_err);
    }

    let body = match response.text().await {
        Ok(t) => t,
        Err(e) => {
            return error_result(current, &format!("读取更新响应失败：{e}"));
        }
    };
    let json: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return error_result(current, &format!("解析更新信息失败：{e}"));
        }
    };

    let Some(info) = parse_release(&json) else {
        return error_result(current, "更新服务器响应缺少必要字段");
    };

    let has_update = version_compare(&info.version, current) == Ordering::Greater;
    UpdateCheckResult {
        latest: Some(info),
        has_update,
        current_version: current.into(),
        error: None,
    }
}

/// 构造错误结果，对消息做脱敏后返回。
fn error_result(current: &str, message: &str) -> UpdateCheckResult {
    UpdateCheckResult {
        latest: None,
        has_update: false,
        current_version: current.into(),
        error: Some(redact_sensitive(message)),
    }
}

/// 从 GitHub Releases API JSON 解析最新 release 信息（Task 26.2）。
///
/// 解析字段：
/// - `tag_name`：剥离前导 `v`/`V` 后作为版本号；
/// - `published_at`：原值字符串（ISO 8601）；
/// - `html_url`：作为"前往下载页"目标，缺失时回退到 releases 列表页；
/// - `body`：release notes 原文（Markdown）；
/// - `sha256`：尝试从 `body` 中解析 `SHA-256: <hex>` 行，找不到为 `None`。
///
/// `tag_name` 缺失或非字符串时返回 `None`，调用方据此报告解析失败。
fn parse_release(json: &serde_json::Value) -> Option<UpdateInfo> {
    let tag = json.get("tag_name")?.as_str()?;
    let version = strip_leading_v(tag).to_owned();
    let release_date = json
        .get("published_at")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let download_url = json
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or(RELEASES_PAGE)
        .to_owned();
    let release_notes = json
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let sha256 = parse_sha256_from_body(&release_notes);
    Some(UpdateInfo {
        version,
        release_date,
        download_url,
        sha256,
        release_notes,
    })
}

/// 从 release notes 中尝试解析 `SHA-256: <hex>` 行（Task 26.1）。
///
/// 支持中英文冒号、大小写不敏感。找不到或长度/字符不合法时返回 `None`。
fn parse_sha256_from_body(body: &str) -> Option<String> {
    for line in body.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        let Some(rest) = lower
            .strip_prefix("sha-256:")
            .or_else(|| lower.strip_prefix("sha256:"))
            .or_else(|| lower.strip_prefix("sha-256："))
            .or_else(|| lower.strip_prefix("sha256："))
        else {
            continue;
        };
        let hex = rest.trim();
        if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(hex.to_ascii_lowercase());
        }
    }
    None
}

/// 剥离版本号前导 `v`/`V`（如 `v0.5.7` → `0.5.7`）。
fn strip_leading_v(tag: &str) -> &str {
    let trimmed = tag.trim();
    if let Some(rest) = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
    {
        rest
    } else {
        trimmed
    }
}

/// 简化版 semver 比较：major.minor.patch 三段数字比较（Task 26.6）。
///
/// 仅解析前 3 段数字（忽略 prerelease 后缀如 `-rc.1`）。
/// 非数字段视为 0。返回 `Ordering`，调用方可与 `Ordering::Greater` 比较判断是否有更新。
pub fn version_compare(a: &str, b: &str) -> Ordering {
    let a_parts = parse_version_parts(a);
    let b_parts = parse_version_parts(b);
    for i in 0..3 {
        let av = a_parts.get(i).copied().unwrap_or(0);
        let bv = b_parts.get(i).copied().unwrap_or(0);
        match av.cmp(&bv) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

/// 解析版本字符串前 3 段数字。
///
/// 仅读取数字字符，遇到非数字字符停止该段解析；
/// 不足 3 段时用 0 补齐比较（由 `version_compare` 调用方处理）。
fn parse_version_parts(version: &str) -> Vec<u32> {
    let cleaned = strip_leading_v(version);
    let mut parts = Vec::with_capacity(3);
    for segment in cleaned.split('.') {
        if parts.len() >= 3 {
            break;
        }
        let digits: String = segment.chars().take_while(|c| c.is_ascii_digit()).collect();
        // 解析失败视为 0（可恢复回退，不 unwrap/expect）
        let value = digits.parse::<u32>().unwrap_or(0);
        parts.push(value);
    }
    parts
}

/// 简化版扩展兼容性检查（Task 26.3）。
///
/// 当前策略：扩展版本必须等于桌面端版本（major.minor.patch 全等）。
/// 避免引入复杂的兼容性矩阵，保证扩展和桌面端协议同步发布。
/// 后续如需放宽，可改为只比较 major.minor。
pub fn check_extension_compatibility(app_version: &str, ext_version: &str) -> bool {
    version_compare(app_version, ext_version) == Ordering::Equal
}

/// 构造扩展兼容性结果（含中文提示）。
///
/// `compatible = true` 时 `message` 为空；否则返回面向用户的中文说明，
/// 指导用户更新桌面端或扩展。
pub fn build_extension_compatibility_result(
    app_version: &str,
    ext_version: &str,
) -> ExtensionCompatibilityResult {
    let order = version_compare(app_version, ext_version);
    let compatible = order == Ordering::Equal;
    let message = if compatible {
        String::new()
    } else if order == Ordering::Greater {
        format!(
            "扩展版本 {} 低于桌面端 {}，请更新浏览器扩展以避免协议不兼容。",
            ext_version, app_version
        )
    } else {
        format!(
            "扩展版本 {} 高于桌面端 {}，请更新猫步下载器以使用最新扩展功能。",
            ext_version, app_version
        )
    };
    ExtensionCompatibilityResult {
        compatible,
        app_version: app_version.into(),
        extension_version: ext_version.into(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    // ---- version_compare ----

    #[test]
    fn version_compare_equal_versions() {
        assert_eq!(version_compare("0.5.7", "0.5.7"), Ordering::Equal);
        assert_eq!(version_compare("v0.5.7", "0.5.7"), Ordering::Equal);
        assert_eq!(version_compare("0.5.7", "v0.5.7"), Ordering::Equal);
    }

    #[test]
    fn version_compare_greater_minor() {
        assert_eq!(version_compare("0.6.0", "0.5.7"), Ordering::Greater);
        assert_eq!(version_compare("1.0.0", "0.99.99"), Ordering::Greater);
    }

    #[test]
    fn version_compare_greater_patch() {
        assert_eq!(version_compare("0.5.8", "0.5.7"), Ordering::Greater);
    }

    #[test]
    fn version_compare_less_major() {
        assert_eq!(version_compare("0.5.7", "1.0.0"), Ordering::Less);
    }

    #[test]
    fn version_compare_less_minor() {
        assert_eq!(version_compare("0.5.0", "0.6.0"), Ordering::Less);
    }

    #[test]
    fn version_compare_handles_short_versions() {
        assert_eq!(version_compare("1", "1.0"), Ordering::Equal);
        assert_eq!(version_compare("1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(version_compare("1.2", "1.2.3"), Ordering::Less);
    }

    #[test]
    fn version_compare_ignores_prerelease_suffix() {
        // 忽略 prerelease 后缀，仅比较数字段。
        assert_eq!(version_compare("1.0.0-rc.1", "1.0.0"), Ordering::Equal);
        assert_eq!(version_compare("1.0.0-beta", "1.0.0"), Ordering::Equal);
    }

    #[test]
    fn version_compare_handles_invalid_as_zero() {
        assert_eq!(version_compare("invalid", "0.0.0"), Ordering::Equal);
        assert_eq!(version_compare("1.x.0", "1.0.0"), Ordering::Equal);
    }

    // ---- strip_leading_v ----

    #[test]
    fn strip_leading_v_handles_v_prefix() {
        assert_eq!(strip_leading_v("v0.5.7"), "0.5.7");
        assert_eq!(strip_leading_v("V0.5.7"), "0.5.7");
        assert_eq!(strip_leading_v("0.5.7"), "0.5.7");
        assert_eq!(strip_leading_v("  v1.0.0  "), "1.0.0");
    }

    // ---- parse_sha256_from_body ----

    #[test]
    fn parse_sha256_from_body_finds_valid_line() {
        let body = "## 更新内容\nSHA-256: 3a48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a27\n下载：https://example.com";
        let sha = parse_sha256_from_body(body);
        assert_eq!(
            sha.as_deref(),
            Some("3a48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a27")
        );
    }

    #[test]
    fn parse_sha256_from_body_finds_chinese_colon() {
        let body = "SHA-256：ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789";
        let sha = parse_sha256_from_body(body);
        assert_eq!(
            sha.as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789")
        );
    }

    #[test]
    fn parse_sha256_from_body_returns_none_when_missing() {
        assert!(parse_sha256_from_body("没有 SHA 行").is_none());
        assert!(parse_sha256_from_body("").is_none());
    }

    #[test]
    fn parse_sha256_from_body_rejects_invalid_hex() {
        // 长度不对
        assert!(parse_sha256_from_body("SHA-256: abc123").is_none());
        // 含非十六进制字符
        let body = "SHA-256: xyz48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a27";
        assert!(parse_sha256_from_body(body).is_none());
    }

    // ---- parse_release ----

    #[test]
    fn parse_release_extracts_all_fields() {
        let json = serde_json::json!({
            "tag_name": "v0.6.0",
            "published_at": "2026-07-20T10:00:00Z",
            "html_url": "https://github.com/maobukeai/maobu-fetch/releases/tag/v0.6.0",
            "body": "## 新功能\n- 更新检查"
        });
        let info = parse_release(&json).expect("应解析成功");
        assert_eq!(info.version, "0.6.0");
        assert_eq!(info.release_date, "2026-07-20T10:00:00Z");
        assert_eq!(
            info.download_url,
            "https://github.com/maobukeai/maobu-fetch/releases/tag/v0.6.0"
        );
        assert!(info.sha256.is_none());
        assert!(info.release_notes.contains("更新检查"));
    }

    #[test]
    fn parse_release_extracts_sha_from_body() {
        let json = serde_json::json!({
            "tag_name": "v0.6.0",
            "published_at": "2026-07-20T10:00:00Z",
            "html_url": "https://github.com/maobukeai/maobu-fetch/releases/tag/v0.6.0",
            "body": "SHA-256: 3a48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a27"
        });
        let info = parse_release(&json).expect("应解析成功");
        assert_eq!(
            info.sha256.as_deref(),
            Some("3a48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a27")
        );
    }

    #[test]
    fn parse_release_returns_none_without_tag_name() {
        let json = serde_json::json!({
            "published_at": "2026-07-20T10:00:00Z",
            "html_url": "https://github.com/maobukeai/maobu-fetch/releases/tag/v0.6.0",
            "body": "无 tag"
        });
        assert!(parse_release(&json).is_none());
    }

    #[test]
    fn parse_release_uses_fallback_url_when_html_url_missing() {
        let json = serde_json::json!({
            "tag_name": "v0.6.0",
            "published_at": "2026-07-20T10:00:00Z"
        });
        let info = parse_release(&json).expect("应解析成功");
        assert_eq!(info.download_url, RELEASES_PAGE);
    }

    #[test]
    fn parse_release_handles_non_string_tag_name() {
        let json = serde_json::json!({
            "tag_name": 123,
            "published_at": "2026-07-20T10:00:00Z"
        });
        assert!(parse_release(&json).is_none());
    }

    // ---- check_extension_compatibility ----

    #[test]
    fn extension_compatibility_equal_versions_are_compatible() {
        assert!(check_extension_compatibility("0.5.7", "0.5.7"));
    }

    #[test]
    fn extension_compatibility_extension_older_is_incompatible() {
        assert!(!check_extension_compatibility("0.6.0", "0.5.7"));
    }

    #[test]
    fn extension_compatibility_extension_newer_is_incompatible() {
        assert!(!check_extension_compatibility("0.5.7", "0.6.0"));
    }

    #[test]
    fn extension_compatibility_strips_v_prefix() {
        assert!(check_extension_compatibility("v0.5.7", "0.5.7"));
        assert!(check_extension_compatibility("0.5.7", "V0.5.7"));
    }

    // ---- build_extension_compatibility_result ----

    #[test]
    fn build_result_compatible_has_empty_message() {
        let result = build_extension_compatibility_result("0.5.7", "0.5.7");
        assert!(result.compatible);
        assert!(result.message.is_empty());
        assert_eq!(result.app_version, "0.5.7");
        assert_eq!(result.extension_version, "0.5.7");
    }

    #[test]
    fn build_result_extension_older_mentions_low_version() {
        let result = build_extension_compatibility_result("0.6.0", "0.5.7");
        assert!(!result.compatible);
        assert!(result.message.contains("低于"));
        assert!(result.message.contains("0.5.7"));
        assert!(result.message.contains("0.6.0"));
    }

    #[test]
    fn build_result_extension_newer_mentions_high_version() {
        let result = build_extension_compatibility_result("0.5.7", "0.6.0");
        assert!(!result.compatible);
        assert!(result.message.contains("高于"));
    }

    // ---- Mock GitHub API 整体响应解析 ----

    #[test]
    fn mock_github_response_parses_correctly() {
        // 构造一个最小的 GitHub Releases API 响应 JSON。
        let mock_response = serde_json::json!({
            "url": "https://api.github.com/repos/maobukeai/maobu-fetch/releases/12345",
            "html_url": "https://github.com/maobukeai/maobu-fetch/releases/tag/v0.6.0",
            "assets_url": "https://api.github.com/repos/maobukeai/maobu-fetch/releases/12345/assets",
            "upload_url": "https://uploads.github.com/repos/maobukeai/maobu-fetch/releases/12345/assets",
            "id": 12345,
            "tag_name": "v0.6.0",
            "target_commitish": "main",
            "name": "Maobu Fetch 0.6.0",
            "draft": false,
            "prerelease": false,
            "created_at": "2026-07-19T12:00:00Z",
            "published_at": "2026-07-20T10:00:00Z",
            "body": "## 新增\n- 更新检查与提醒功能\n\n## 修复\n- 修复连接级状态推送",
            "assets": []
        });
        let info = parse_release(&mock_response).expect("应解析成功");
        assert_eq!(info.version, "0.6.0");
        assert_eq!(info.release_date, "2026-07-20T10:00:00Z");
        assert_eq!(
            info.download_url,
            "https://github.com/maobukeai/maobu-fetch/releases/tag/v0.6.0"
        );
        assert!(info.release_notes.contains("更新检查"));
        assert!(info.release_notes.contains("修复"));
    }

    #[test]
    fn mock_github_response_with_sha_in_body() {
        let mock_response = serde_json::json!({
            "tag_name": "v0.6.0",
            "published_at": "2026-07-20T10:00:00Z",
            "html_url": "https://github.com/maobukeai/maobu-fetch/releases/tag/v0.6.0",
            "body": "校验值：\nSHA-256: 3a48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a27"
        });
        let info = parse_release(&mock_response).expect("应解析成功");
        assert_eq!(
            info.sha256.as_deref(),
            Some("3a48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a27")
        );
    }

    #[test]
    fn mock_github_response_minimal() {
        // 仅包含必需字段
        let mock_response = serde_json::json!({
            "tag_name": "v1.0.0",
            "published_at": "2026-01-01T00:00:00Z",
            "html_url": "https://github.com/maobukeai/maobu-fetch/releases/tag/v1.0.0",
            "body": ""
        });
        let info = parse_release(&mock_response).expect("应解析成功");
        assert_eq!(info.version, "1.0.0");
        assert!(info.release_notes.is_empty());
        assert!(info.sha256.is_none());
    }

    #[test]
    fn app_version_constant_is_non_empty() {
        // 编译期版本注入必须成功；测试在 Cargo.toml version 改动时会自动跟随。
        assert!(!APP_VERSION.is_empty());
    }

    #[test]
    fn github_constants_are_consistent() {
        // 防止有人改 owner/repo 但忘了同步 URL 常量。
        assert!(RELEASES_LATEST_URL.contains(GITHUB_OWNER));
        assert!(RELEASES_LATEST_URL.contains(GITHUB_REPO));
        assert!(RELEASES_PAGE.contains(GITHUB_OWNER));
        assert!(RELEASES_PAGE.contains(GITHUB_REPO));
    }
}
