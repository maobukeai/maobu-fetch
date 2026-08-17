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
use crate::models::{
    ExtensionCompatibilityResult, UpdateAssetInfo, UpdateCheckResult, UpdateInfo,
};
use reqwest::Client;
use std::cmp::Ordering;
use std::time::Duration;

// 与 package.json、tauri.conf.json、extension/manifest.json 及 git remote 保持一致；
// 发布流程中需验证 GitHub Releases API 可访问（见 AGENTS.md §10）。
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
    let assets = parse_assets(json.get("assets"));
    Some(UpdateInfo {
        version,
        release_date,
        download_url,
        sha256,
        release_notes,
        assets,
    })
}

/// 从 release 的 `assets[]` 解析下载资产（一键更新用）。
///
/// `digest` 字段形如 `sha256:<hex>`（GitHub 对新上传资产自动生成），
/// 解析失败或缺失时 `sha256 = None`——一键更新要求必须有校验值。
fn parse_assets(value: Option<&serde_json::Value>) -> Vec<UpdateAssetInfo> {
    let Some(entries) = value.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?.to_owned();
            let url = entry
                .get("browser_download_url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            if url.is_empty() {
                return None;
            }
            let size = entry.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
            let sha256 = entry
                .get("digest")
                .and_then(|v| v.as_str())
                .and_then(parse_digest_sha256);
            Some(UpdateAssetInfo {
                name,
                url,
                size,
                sha256,
            })
        })
        .collect()
}

/// 解析 GitHub 资产 `digest` 字段（`sha256:<64 位十六进制>`）。
fn parse_digest_sha256(digest: &str) -> Option<String> {
    let hex = digest.strip_prefix("sha256:")?;
    if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(hex.to_ascii_lowercase())
    } else {
        None
    }
}

/// 从资产列表中选择 Windows NSIS 安装包（`*-setup.exe` / `*_setup.exe`，
/// 含 `setup` 的 `.exe` 亦可），同等条件下优先 `x64`。
pub fn select_installer_asset(assets: &[UpdateAssetInfo]) -> Option<&UpdateAssetInfo> {
    let mut best: Option<&UpdateAssetInfo> = None;
    for asset in assets {
        let name = asset.name.to_ascii_lowercase();
        let is_installer = name.ends_with("-setup.exe")
            || name.ends_with("_setup.exe")
            || (name.contains("setup") && name.ends_with(".exe"));
        if !is_installer {
            continue;
        }
        let is_x64 = name.contains("x64");
        best = Some(match best {
            None => asset,
            Some(current) => {
                let current_x64 = current.name.to_ascii_lowercase().contains("x64");
                if is_x64 && !current_x64 {
                    asset
                } else {
                    current
                }
            }
        });
    }
    best
}

/// 从资产列表中选择浏览器扩展 ZIP（`extension.zip` 或 `*-extension.zip`）。
pub fn select_extension_asset(assets: &[UpdateAssetInfo]) -> Option<&UpdateAssetInfo> {
    assets.iter().find(|asset| {
        let name = asset.name.to_ascii_lowercase();
        name == "extension.zip"
            || name.ends_with("-extension.zip")
            || name.ends_with("_extension.zip")
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

// ---- 一键更新：下载、校验与解压（仅在用户显式点击时调用，AGENTS.md §6） ----

/// 下载进度事件 payload（emit 到前端 `update-download-progress`）。
#[derive(Clone, serde::Serialize)]
pub struct UpdateDownloadProgress {
    pub kind: &'static str,
    pub downloaded: u64,
    pub total: u64,
}

/// 用户主动触发的更新资产下载：流式写入系统临时目录，边下边算 SHA-256，
/// 完成后与 GitHub 官方 digest 比对；不一致立即删除文件并报错，
/// 绝不让未校验/校验失败的文件进入安装环节（AGENTS.md §6）。
///
/// 资产未提供官方校验值时直接拒绝下载（安全默认，不做降级放行）。
pub async fn download_release_asset(
    app: &tauri::AppHandle,
    kind: &'static str,
    asset: &UpdateAssetInfo,
    file_name: &str,
) -> Result<std::path::PathBuf, String> {
    use futures_util::StreamExt;
    use sha2::{Digest, Sha256};
    use tauri::Emitter;
    use tokio::io::AsyncWriteExt;

    let Some(expected) = asset.sha256.as_deref() else {
        return Err(format!(
            "资产 {} 未提供官方 SHA-256 校验值，为安全起见已取消下载，请前往发布页手动下载",
            asset.name
        ));
    };
    let client = Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败：{e}"))?;
    let response = client
        .get(&asset.url)
        .send()
        .await
        .map_err(|e| format!("无法连接下载服务器：{e}"))?;
    if !response.status().is_success() {
        return Err(format!("下载服务器返回 HTTP {}", response.status()));
    }
    let total = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(asset.size);
    let target = std::env::temp_dir().join(file_name);
    let file = tokio::fs::File::create(&target)
        .await
        .map_err(|e| format!("无法创建临时文件：{e}"))?;
    let mut writer = tokio::io::BufWriter::new(file);
    let mut hasher = Sha256::new();
    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_emit = std::time::Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("下载中断：{e}"))?;
        writer
            .write_all(&chunk)
            .await
            .map_err(|e| format!("写入临时文件失败：{e}"))?;
        hasher.update(&chunk);
        downloaded += chunk.len() as u64;
        // 进度事件节流至 ~150ms 一次，避免高频 emit（AGENTS.md §8）。
        if last_emit.elapsed() >= Duration::from_millis(150) {
            last_emit = std::time::Instant::now();
            let _ = app.emit(
                "update-download-progress",
                UpdateDownloadProgress {
                    kind,
                    downloaded,
                    total,
                },
            );
        }
    }
    writer
        .flush()
        .await
        .map_err(|e| format!("写入临时文件失败：{e}"))?;
    let _ = app.emit(
        "update-download-progress",
        UpdateDownloadProgress {
            kind,
            downloaded,
            total,
        },
    );
    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        let _ = tokio::fs::remove_file(&target).await;
        return Err(format!(
            "SHA-256 校验失败（期望 {expected}，实际 {actual}），已删除下载文件，请重试或手动下载"
        ));
    }
    Ok(target)
}

/// 将扩展 ZIP 安全解压到目标目录。
///
/// - `enclosed_name` 阻止绝对路径与 `..` 穿越路径（AGENTS.md §6）；
/// - 只接受常规文件条目（目录条目在创建父目录时自然生成）；
/// - 解压完成后校验根级 `manifest.json` 存在，防止误用非扩展压缩包。
pub fn extract_extension_zip(archive: &std::path::Path, target_dir: &std::path::Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("扩展压缩包无效：{e}"))?;
    std::fs::create_dir_all(target_dir).map_err(|e| e.to_string())?;
    let mut has_manifest = false;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| format!("扩展压缩包无效：{e}"))?;
        if entry.is_dir() {
            continue;
        }
        let Some(enclosed) = entry.enclosed_name() else {
            return Err("扩展压缩包含非法路径条目，已中止".into());
        };
        if enclosed.file_name().is_some_and(|n| n == "manifest.json")
            && enclosed.components().count() == 1
        {
            has_manifest = true;
        }
        let output_path = target_dir.join(&enclosed);
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut output = std::fs::File::create(&output_path).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut output).map_err(|e| e.to_string())?;
    }
    if !has_manifest {
        return Err("扩展压缩包缺少 manifest.json，不是有效的扩展包".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;
    use std::io::Write;

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

    // ---- 一键更新：资产解析与选择 ----

    #[test]
    fn parse_assets_extracts_digest_sha256() {
        let json = serde_json::json!({
            "assets": [
                {
                    "name": "Maobu.Fetch_0.6.9_x64-setup.exe",
                    "browser_download_url": "https://example.com/setup.exe",
                    "size": 3952000,
                    "digest": "sha256:AE67A0E890E95B518FB5139BBC725FEC1C428EB208E5F5880472C153B2E108CA"
                },
                { "name": "extension.zip", "browser_download_url": "https://example.com/ext.zip", "size": 48000 },
                { "name": "no-url.txt", "size": 1 }
            ]
        });
        let assets = parse_assets(json.get("assets"));
        assert_eq!(assets.len(), 2, "缺少下载地址的资产应被过滤");
        assert_eq!(assets[0].name, "Maobu.Fetch_0.6.9_x64-setup.exe");
        assert_eq!(assets[0].size, 3_952_000);
        assert_eq!(
            assets[0].sha256.as_deref(),
            Some("ae67a0e890e95b518fb5139bbc725fec1c428eb208e5f5880472c153b2e108ca"),
            "digest 应转小写"
        );
        assert_eq!(assets[1].sha256, None, "无 digest 字段时 sha256 为 None");
    }

    #[test]
    fn parse_assets_handles_missing_or_invalid_digest() {
        assert_eq!(parse_digest_sha256("sha256:abc123"), None, "长度不足");
        assert_eq!(
            parse_digest_sha256("sha256:xyz48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a27"),
            None,
            "非十六进制"
        );
        assert_eq!(parse_digest_sha256("md5:abc"), None, "非 sha256 前缀");
        let valid = "sha256:3a48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a27";
        assert_eq!(
            parse_digest_sha256(valid).as_deref(),
            Some("3a48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a27")
        );
    }

    #[test]
    fn select_installer_asset_prefers_x64_setup() {
        let assets = vec![
            UpdateAssetInfo {
                name: "readme.txt".into(),
                url: "https://example.com/readme.txt".into(),
                size: 10,
                sha256: None,
            },
            UpdateAssetInfo {
                name: "Maobu.Fetch_0.6.9_x86-setup.exe".into(),
                url: "https://example.com/x86.exe".into(),
                size: 10,
                sha256: None,
            },
            UpdateAssetInfo {
                name: "Maobu.Fetch_0.6.9_x64-setup.exe".into(),
                url: "https://example.com/x64.exe".into(),
                size: 10,
                sha256: None,
            },
        ];
        let selected = select_installer_asset(&assets).expect("应选中安装包");
        assert_eq!(selected.name, "Maobu.Fetch_0.6.9_x64-setup.exe");
        assert!(select_installer_asset(&assets[..1]).is_none(), "无 exe 资产时应为 None");
    }

    #[test]
    fn select_extension_asset_matches_extension_zip() {
        let assets = vec![
            UpdateAssetInfo {
                name: "Maobu.Fetch_0.6.9_x64-setup.exe".into(),
                url: "https://example.com/setup.exe".into(),
                size: 10,
                sha256: None,
            },
            UpdateAssetInfo {
                name: "extension.zip".into(),
                url: "https://example.com/extension.zip".into(),
                size: 10,
                sha256: None,
            },
        ];
        let selected = select_extension_asset(&assets).expect("应选中扩展包");
        assert_eq!(selected.name, "extension.zip");
        assert!(select_extension_asset(&assets[..1]).is_none());
    }

    #[test]
    fn parse_release_includes_assets() {
        let json = serde_json::json!({
            "tag_name": "v0.6.9",
            "published_at": "2026-08-01T11:57:02Z",
            "html_url": "https://github.com/maobukeai/maobu-fetch/releases/tag/v0.6.9",
            "body": "SHA-256: 3a48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a27",
            "assets": [
                {
                    "name": "extension.zip",
                    "browser_download_url": "https://example.com/extension.zip",
                    "size": 48000,
                    "digest": "sha256:2ba9b3ab667119d929fefbae14e9fd001edfc9fe6cc5d2b75cdd6c0ad647cba9"
                }
            ]
        });
        let info = parse_release(&json).expect("应解析成功");
        assert_eq!(info.assets.len(), 1);
        assert_eq!(info.assets[0].name, "extension.zip");
        assert!(info.assets[0].sha256.is_some());
    }

    // ---- 一键更新：安全解压 ----

    #[test]
    fn extract_extension_zip_extracts_and_requires_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("ext.zip");
        let file = std::fs::File::create(&archive).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("manifest.json", options).unwrap();
        writer.write_all(b"{}").unwrap();
        writer.start_file("src/background.js", options).unwrap();
        writer.write_all(b"// ok").unwrap();
        writer.finish().unwrap();

        let target = dir.path().join("out");
        extract_extension_zip(&archive, &target).expect("应解压成功");
        assert!(target.join("manifest.json").exists());
        assert!(target.join("src").join("background.js").exists());

        // 缺少 manifest.json 的压缩包必须被拒绝。
        let bad = dir.path().join("bad.zip");
        let file = std::fs::File::create(&bad).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer.start_file("a.txt", options).unwrap();
        writer.write_all(b"x").unwrap();
        writer.finish().unwrap();
        assert!(extract_extension_zip(&bad, &target).is_err());
    }

    #[test]
    fn extract_extension_zip_rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("evil.zip");
        let file = std::fs::File::create(&archive).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        // 模拟携带 `..` 穿越路径的恶意压缩包：enclosed_name 必须拒绝。
        writer.start_file("../evil.txt", options).unwrap();
        writer.write_all(b"pwn").unwrap();
        writer.finish().unwrap();

        let target = dir.path().join("out2");
        let result = extract_extension_zip(&archive, &target);
        assert!(result.is_err(), "穿越路径条目必须导致解压失败");
        assert!(
            !dir.path().join("evil.txt").exists(),
            "不得写出目标目录之外"
        );
    }
}
