//! Task 29：`maobu://` 协议与 `.maobu-task` 文件关联处理。
//!
//! 本模块仅负责把 URI 字符串解析为强类型 [`DeepLinkAction`]，
//! 不访问网络、不读写数据库、不触发副作用，便于单元测试覆盖。
//!
//! 后端在 `lib.rs` 的 `setup` 中通过 `tauri-plugin-deep-link` 接收 URI，
//! 调用 [`parse_deep_link`] 得到动作后，再分发给 [`crate::manager::DownloadManager`]。

use url::Url;

/// `maobu://` 深链可触发的动作。
///
/// 序列化字段使用稳定英文，与后端命令保持一致；用户可见文案在前端按动作类型渲染。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLinkAction {
    /// `maobu://add?url=<encoded>`：新建下载任务。
    Add { url: String },
    /// `maobu://pause?id=<task_id>`：暂停指定任务。
    Pause { id: String },
    /// `maobu://resume?id=<task_id>`：恢复/重试指定任务。
    Resume { id: String },
}

/// 协议 scheme，固定为 `maobu`。
const SCHEME: &str = "maobu";

/// 解析 `maobu://` 深链为 [`DeepLinkAction`]。
///
/// 接受的格式：
/// - `maobu://add?url=<percent-encoded URL>`
/// - `maobu://pause?id=<task_id>`
/// - `maobu://resume?id=<task_id>`
///
/// 失败时返回可操作的简体中文错误，不暴露内部解析细节。
/// 不会因为 `unwrap`/`expect` 引发 panic（AGENTS.md §7）。
pub fn parse_deep_link(uri: &str) -> Result<DeepLinkAction, String> {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        return Err("深链为空".into());
    }
    let parsed = Url::parse(trimmed).map_err(|_| "无效的深链格式")?;
    if parsed.scheme() != SCHEME {
        return Err(format!(
            "不支持的协议：{}（仅支持 maobu://）",
            parsed.scheme()
        ));
    }
    // `maobu://add?url=...` 中 host 即动作名；host_str 在缺省时返回 None。
    let host = parsed.host_str().unwrap_or("");
    if host.is_empty() {
        return Err("缺少动作（add / pause / resume）".into());
    }
    let query: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    match host {
        "add" => {
            let url = take_query(&query, "url").ok_or("缺少 url 参数")?;
            if url.trim().is_empty() {
                return Err("url 参数不能为空".into());
            }
            // 校验是合法的 HTTP/HTTPS 链接，避免后续 manager.add 二次失败时给前端模糊错误。
            let inner = Url::parse(&url).map_err(|_| "url 参数不是合法的链接")?;
            if !matches!(inner.scheme(), "http" | "https") {
                return Err("url 参数仅支持 HTTP/HTTPS 链接".into());
            }
            Ok(DeepLinkAction::Add {
                url: inner.to_string(),
            })
        }
        "pause" => {
            let id = take_query(&query, "id").ok_or("缺少 id 参数")?;
            if id.trim().is_empty() {
                return Err("id 参数不能为空".into());
            }
            Ok(DeepLinkAction::Pause { id })
        }
        "resume" => {
            let id = take_query(&query, "id").ok_or("缺少 id 参数")?;
            if id.trim().is_empty() {
                return Err("id 参数不能为空".into());
            }
            Ok(DeepLinkAction::Resume { id })
        }
        other => Err(format!("不支持的动作：{other}")),
    }
}

/// 从已收集的 query pairs 中取出首个匹配项的值（大小写敏感）。
fn take_query(pairs: &[(String, String)], key: &str) -> Option<String> {
    pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_add_with_percent_encoded_url() {
        let action = parse_deep_link("maobu://add?url=https%3A%2F%2Fexample.com%2Ffile.zip")
            .expect("encoded add URL should parse");
        assert_eq!(
            action,
            DeepLinkAction::Add {
                url: "https://example.com/file.zip".to_string()
            }
        );
    }

    #[test]
    fn parses_add_with_plain_url() {
        let action = parse_deep_link("maobu://add?url=https://example.com/file.zip")
            .expect("plain add URL should parse");
        assert_eq!(
            action,
            DeepLinkAction::Add {
                url: "https://example.com/file.zip".to_string()
            }
        );
    }

    #[test]
    fn parses_pause_with_id() {
        let action = parse_deep_link("maobu://pause?id=abc123").expect("pause should parse");
        assert_eq!(
            action,
            DeepLinkAction::Pause {
                id: "abc123".into()
            }
        );
    }

    #[test]
    fn parses_resume_with_id() {
        let action = parse_deep_link("maobu://resume?id=abc123").expect("resume should parse");
        assert_eq!(
            action,
            DeepLinkAction::Resume {
                id: "abc123".into()
            }
        );
    }

    #[test]
    fn rejects_non_maobu_scheme() {
        let err =
            parse_deep_link("https://example.com/file.zip").expect_err("https should be rejected");
        assert!(err.contains("不支持的协议"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_empty_input() {
        parse_deep_link("").expect_err("empty input should be rejected");
        parse_deep_link("   ").expect_err("whitespace input should be rejected");
    }

    #[test]
    fn rejects_invalid_uri() {
        // 包含空格的 URL 会被 url crate 拒绝，或在内层 URL 解析时失败。
        parse_deep_link("maobu://add?url=not a url").expect_err("invalid url should be rejected");
    }

    #[test]
    fn rejects_missing_url_parameter() {
        let err = parse_deep_link("maobu://add").expect_err("missing url should be rejected");
        assert!(err.contains("url"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_empty_url_parameter() {
        let err = parse_deep_link("maobu://add?url=").expect_err("empty url should be rejected");
        assert!(err.contains("url"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_non_http_url() {
        let err = parse_deep_link("maobu://add?url=ftp://example.com/file.zip")
            .expect_err("ftp should be rejected");
        assert!(err.contains("HTTP/HTTPS"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_missing_id_parameter_for_pause() {
        let err = parse_deep_link("maobu://pause").expect_err("missing id should be rejected");
        assert!(err.contains("id"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_missing_id_parameter_for_resume() {
        let err = parse_deep_link("maobu://resume").expect_err("missing id should be rejected");
        assert!(err.contains("id"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_unknown_action() {
        let err = parse_deep_link("maobu://delete?id=abc")
            .expect_err("unknown action should be rejected");
        assert!(err.contains("不支持的动作"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_missing_host() {
        // `maobu://?url=...` 没有 host。url crate 可能直接拒绝该 URL，
        // 也可能解析成功但 host 为空。两种情况都应返回错误。
        let err = parse_deep_link("maobu://?url=https://example.com")
            .expect_err("missing host should be rejected");
        // 不检查具体错误消息，因为 url crate 的行为在不同版本可能不同。
        assert!(!err.is_empty(), "error message should not be empty");
    }

    #[test]
    fn trims_leading_whitespace() {
        let action =
            parse_deep_link("  maobu://pause?id=abc123  ").expect("trimmed input should parse");
        assert_eq!(
            action,
            DeepLinkAction::Pause {
                id: "abc123".into()
            }
        );
    }

    #[test]
    fn ignores_extra_query_parameters() {
        let action =
            parse_deep_link("maobu://add?url=https://example.com/file.zip&source=deep-link")
                .expect("extra params should be ignored");
        assert_eq!(
            action,
            DeepLinkAction::Add {
                url: "https://example.com/file.zip".to_string()
            }
        );
    }
}
