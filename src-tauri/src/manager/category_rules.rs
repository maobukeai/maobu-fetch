//! 自动分类规则模块（Task 11）。
//!
//! 提供纯逻辑函数，对一组分类规则与下载请求字段进行匹配：
//! - `apply_category_rules`：按 priority 升序遍历启用的规则，返回首个命中规则的目标目录
//! - `test_category_rule`：单条规则测试，返回是否命中
//!
//! 匹配算法：
//! - `Domain`：解析 URL 主机名，pattern 作为后缀匹配，支持子域名（`github.com` 匹配 `api.github.com`）
//! - `Mime`：从 `Content-Type` 提取主类型（`/` 之前部分，去除空白与参数），与 pattern 大小写不敏感比较
//! - `Regex`：使用 `regex::Regex` 全文匹配文件名
//!
//! 安全约束（AGENTS.md §3 / §7）：
//! - 不使用 `unwrap()` / `expect()` 处理可恢复错误
//! - 规则测试不写入日志，不泄露 URL 中可能存在的认证参数
//! - 正则编译失败按“不命中”处理，避免单条坏规则阻塞整个流程

use crate::models::{CategoryRule, CategoryRuleType};
use regex::Regex;
use url::Url;

/// 按优先级遍历启用的规则，返回首个命中规则的目标目录。
///
/// - `rules`：任意顺序的规则列表，函数内部按 `priority` 升序排序
/// - `url`：任务 URL（用于 Domain 匹配）
/// - `file_name`：任务文件名（用于 Regex 匹配）
/// - `content_type`：响应 Content-Type（用于 Mime 匹配），可为 None
///
/// 返回 `Some(target_directory)` 表示命中；`None` 表示无规则命中。
/// 返回的目标目录字符串与规则中保存的一致（已规范化）。
pub fn apply_category_rules(
    rules: &[CategoryRule],
    url: &str,
    file_name: &str,
    content_type: Option<&str>,
) -> Option<String> {
    // 按 priority 升序排序，priority 相同时保持稳定（使用稳定排序）。
    // 不修改原切片，先复制再排序。
    let mut sorted: Vec<&CategoryRule> = rules.iter().filter(|r| r.enabled).collect();
    sorted.sort_by_key(|r| r.priority);
    for rule in sorted {
        if rule_matches(rule, url, file_name, content_type) {
            return Some(rule.target_directory.clone());
        }
    }
    None
}

/// 测试单条规则是否命中。忽略 `enabled` 标志，专用于“规则测试”功能。
pub fn test_category_rule(
    rule: &CategoryRule,
    url: &str,
    file_name: &str,
    content_type: Option<&str>,
) -> bool {
    rule_matches(rule, url, file_name, content_type)
}

/// 规范化目标目录：去除尾部 `/` 或 `\`，去除首尾空白。
pub fn normalize_directory(directory: &str) -> String {
    directory.trim().trim_end_matches(['/', '\\']).to_string()
}

fn rule_matches(
    rule: &CategoryRule,
    url: &str,
    file_name: &str,
    content_type: Option<&str>,
) -> bool {
    match rule.rule_type {
        CategoryRuleType::Domain => matches_domain(url, &rule.pattern),
        CategoryRuleType::Mime => matches_mime(content_type, &rule.pattern),
        CategoryRuleType::Regex => matches_regex(file_name, &rule.pattern),
    }
}

/// Domain 匹配：pattern 作为主机名后缀，支持子域名。
///
/// - `github.com` 匹配 `api.github.com`、`github.com`
/// - 大小写不敏感
/// - 必须在域名边界处匹配（不能 `evilgithub.com` 匹配 `github.com`）
/// - URL 无法解析时返回 false
fn matches_domain(url: &str, pattern: &str) -> bool {
    let host = match Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
    {
        Some(h) => h,
        None => return false,
    };
    let host = host.to_ascii_lowercase();
    let pattern = pattern.trim().to_ascii_lowercase();
    if pattern.is_empty() {
        return false;
    }
    if host == pattern {
        return true;
    }
    // 子域名匹配：host 必须以 `.` + pattern 结尾
    let suffix = format!(".{pattern}");
    host.ends_with(&suffix)
}

/// Mime 匹配：从 Content-Type 提取主类型，与 pattern 大小写不敏感比较。
///
/// - `video/mp4` → 主类型 `video`
/// - `application/json; charset=utf-8` → 主类型 `application`
/// - Content-Type 为 None 或空字符串时返回 false
fn matches_mime(content_type: Option<&str>, pattern: &str) -> bool {
    let Some(ct) = content_type.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    let main_type = ct.split(';').next().unwrap_or("").trim();
    let main_type = main_type
        .split('/')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let pattern = pattern.trim().to_ascii_lowercase();
    if pattern.is_empty() || main_type.is_empty() {
        return false;
    }
    main_type == pattern
}

/// Regex 匹配：使用 `regex::Regex` 全文匹配文件名。
///
/// 正则编译失败时返回 false（不阻塞流程）。
/// 使用 `is_match` 即可（部分匹配即可命中，与用户期望一致）。
fn matches_regex(file_name: &str, pattern: &str) -> bool {
    let Ok(regex) = Regex::new(pattern) else {
        return false;
    };
    regex.is_match(file_name)
}

// ===== 单元测试 =====
#[cfg(test)]
mod tests {
    use super::*;

    fn make_rule(
        id: &str,
        rule_type: CategoryRuleType,
        pattern: &str,
        target: &str,
        enabled: bool,
        priority: i32,
    ) -> CategoryRule {
        CategoryRule {
            id: id.into(),
            name: format!("rule-{id}"),
            rule_type,
            pattern: pattern.into(),
            target_directory: target.into(),
            enabled,
            priority,
        }
    }

    // ---- Domain 匹配 ----

    #[test]
    fn domain_matches_exact_host() {
        let rule = make_rule(
            "d1",
            CategoryRuleType::Domain,
            "github.com",
            "D:\\DL\\github",
            true,
            0,
        );
        assert!(test_category_rule(
            &rule,
            "https://github.com/file.zip",
            "file.zip",
            None
        ));
    }

    #[test]
    fn domain_matches_subdomain() {
        let rule = make_rule(
            "d2",
            CategoryRuleType::Domain,
            "github.com",
            "D:\\DL\\github",
            true,
            0,
        );
        assert!(test_category_rule(
            &rule,
            "https://api.github.com/users/octocat",
            "octocat.json",
            None
        ));
    }

    #[test]
    fn domain_does_not_match_partial_suffix() {
        // evilgithub.com 不应匹配 github.com（不在域名边界）
        let rule = make_rule(
            "d3",
            CategoryRuleType::Domain,
            "github.com",
            "D:\\DL\\github",
            true,
            0,
        );
        assert!(!test_category_rule(
            &rule,
            "https://evilgithub.com/file",
            "file",
            None
        ));
    }

    #[test]
    fn domain_case_insensitive() {
        let rule = make_rule(
            "d4",
            CategoryRuleType::Domain,
            "GitHub.COM",
            "D:\\DL\\github",
            true,
            0,
        );
        assert!(test_category_rule(
            &rule,
            "https://api.github.com/x",
            "x",
            None
        ));
    }

    #[test]
    fn domain_invalid_url_returns_false() {
        let rule = make_rule(
            "d5",
            CategoryRuleType::Domain,
            "github.com",
            "D:\\DL\\github",
            true,
            0,
        );
        assert!(!test_category_rule(&rule, "not-a-url", "file", None));
    }

    #[test]
    fn domain_empty_pattern_returns_false() {
        let rule = make_rule(
            "d6",
            CategoryRuleType::Domain,
            "",
            "D:\\DL\\github",
            true,
            0,
        );
        assert!(!test_category_rule(
            &rule,
            "https://github.com/x",
            "x",
            None
        ));
    }

    // ---- Mime 匹配 ----

    #[test]
    fn mime_matches_main_type() {
        let rule = make_rule(
            "m1",
            CategoryRuleType::Mime,
            "video",
            "D:\\DL\\video",
            true,
            0,
        );
        assert!(test_category_rule(
            &rule,
            "https://example.com/file",
            "file.mp4",
            Some("video/mp4")
        ));
    }

    #[test]
    fn mime_matches_with_parameters() {
        let rule = make_rule(
            "m2",
            CategoryRuleType::Mime,
            "application",
            "D:\\DL\\app",
            true,
            0,
        );
        assert!(test_category_rule(
            &rule,
            "https://example.com/file",
            "file.json",
            Some("application/json; charset=utf-8")
        ));
    }

    #[test]
    fn mime_does_not_match_subtype() {
        // pattern 不应匹配 subtype
        let rule = make_rule("m3", CategoryRuleType::Mime, "mp4", "D:\\DL\\mp4", true, 0);
        assert!(!test_category_rule(
            &rule,
            "https://example.com/file",
            "file.mp4",
            Some("video/mp4")
        ));
    }

    #[test]
    fn mime_none_content_type_returns_false() {
        let rule = make_rule(
            "m4",
            CategoryRuleType::Mime,
            "video",
            "D:\\DL\\video",
            true,
            0,
        );
        assert!(!test_category_rule(
            &rule,
            "https://example.com/file",
            "file.mp4",
            None
        ));
    }

    #[test]
    fn mime_case_insensitive() {
        let rule = make_rule(
            "m5",
            CategoryRuleType::Mime,
            "VIDEO",
            "D:\\DL\\video",
            true,
            0,
        );
        assert!(test_category_rule(
            &rule,
            "https://example.com/file",
            "file.mp4",
            Some("video/mp4")
        ));
    }

    // ---- Regex 匹配 ----

    #[test]
    fn regex_matches_filename() {
        let rule = make_rule(
            "r1",
            CategoryRuleType::Regex,
            r"\.mp4$",
            "D:\\DL\\video",
            true,
            0,
        );
        assert!(test_category_rule(
            &rule,
            "https://example.com/file",
            "movie.mp4",
            None
        ));
    }

    #[test]
    fn regex_does_not_match_non_matching() {
        let rule = make_rule(
            "r2",
            CategoryRuleType::Regex,
            r"\.mp4$",
            "D:\\DL\\video",
            true,
            0,
        );
        assert!(!test_category_rule(
            &rule,
            "https://example.com/file",
            "movie.avi",
            None
        ));
    }

    #[test]
    fn regex_invalid_pattern_returns_false() {
        // 不合法正则不应 panic，按不命中处理
        let rule = make_rule(
            "r3",
            CategoryRuleType::Regex,
            "[invalid",
            "D:\\DL\\video",
            true,
            0,
        );
        assert!(!test_category_rule(
            &rule,
            "https://example.com/file",
            "movie.mp4",
            None
        ));
    }

    #[test]
    fn regex_partial_match_succeeds() {
        // 不锚定 ^ $ 时，部分匹配即可
        let rule = make_rule(
            "r4",
            CategoryRuleType::Regex,
            "report",
            "D:\\DL\\reports",
            true,
            0,
        );
        assert!(test_category_rule(
            &rule,
            "https://example.com/file",
            "2024-quarterly-report.pdf",
            None
        ));
    }

    // ---- apply_category_rules 优先级排序 ----

    #[test]
    fn apply_returns_none_when_no_rules() {
        assert_eq!(
            apply_category_rules(&[], "https://example.com", "file.zip", None),
            None
        );
    }

    #[test]
    fn apply_returns_none_when_all_disabled() {
        let rules = vec![
            make_rule(
                "a",
                CategoryRuleType::Domain,
                "github.com",
                "D:\\DL\\github",
                false,
                0,
            ),
            make_rule(
                "b",
                CategoryRuleType::Mime,
                "video",
                "D:\\DL\\video",
                false,
                1,
            ),
        ];
        assert_eq!(
            apply_category_rules(
                &rules,
                "https://github.com/x",
                "file.mp4",
                Some("video/mp4")
            ),
            None
        );
    }

    #[test]
    fn apply_priority_lower_number_wins() {
        // 两条命中规则，priority=10 应输给 priority=1
        let rules = vec![
            make_rule(
                "low",
                CategoryRuleType::Domain,
                "github.com",
                "D:\\DL\\github-low",
                true,
                10,
            ),
            make_rule(
                "high",
                CategoryRuleType::Domain,
                "github.com",
                "D:\\DL\\github-high",
                true,
                1,
            ),
        ];
        let result = apply_category_rules(&rules, "https://github.com/x", "file", None);
        assert_eq!(result.as_deref(), Some("D:\\DL\\github-high"));
    }

    #[test]
    fn apply_returns_first_match_when_multiple_match() {
        // priority 相同时，按切片中原始顺序返回第一个
        let rules = vec![
            make_rule(
                "first",
                CategoryRuleType::Domain,
                "github.com",
                "D:\\DL\\first",
                true,
                0,
            ),
            make_rule(
                "second",
                CategoryRuleType::Mime,
                "video",
                "D:\\DL\\second",
                true,
                0,
            ),
        ];
        let result = apply_category_rules(
            &rules,
            "https://github.com/x",
            "file.mp4",
            Some("video/mp4"),
        );
        assert_eq!(result.as_deref(), Some("D:\\DL\\first"));
    }

    #[test]
    fn apply_skips_disabled_even_if_lower_priority() {
        let rules = vec![
            make_rule(
                "disabled",
                CategoryRuleType::Domain,
                "github.com",
                "D:\\DL\\disabled",
                false,
                0,
            ),
            make_rule(
                "enabled",
                CategoryRuleType::Mime,
                "video",
                "D:\\DL\\enabled",
                true,
                10,
            ),
        ];
        let result = apply_category_rules(
            &rules,
            "https://github.com/x",
            "file.mp4",
            Some("video/mp4"),
        );
        assert_eq!(result.as_deref(), Some("D:\\DL\\enabled"));
    }

    #[test]
    fn apply_does_not_modify_input_slice_order() {
        let rules = vec![
            make_rule(
                "b",
                CategoryRuleType::Domain,
                "github.com",
                "D:\\DL\\b",
                true,
                10,
            ),
            make_rule(
                "a",
                CategoryRuleType::Domain,
                "github.com",
                "D:\\DL\\a",
                true,
                1,
            ),
        ];
        let _ = apply_category_rules(&rules, "https://github.com/x", "file", None);
        // 原切片顺序应保持不变
        assert_eq!(rules[0].id, "b");
        assert_eq!(rules[1].id, "a");
    }

    // ---- normalize_directory ----

    #[test]
    fn normalize_strips_trailing_slashes() {
        assert_eq!(normalize_directory("D:\\DL\\video\\"), "D:\\DL\\video");
        assert_eq!(normalize_directory("D:/DL/video/"), "D:/DL/video");
        assert_eq!(normalize_directory("D:\\DL\\video"), "D:\\DL\\video");
    }

    #[test]
    fn normalize_trims_whitespace() {
        assert_eq!(normalize_directory("  D:\\DL\\video  "), "D:\\DL\\video");
    }

    #[test]
    fn normalize_empty_returns_empty() {
        assert_eq!(normalize_directory(""), "");
        assert_eq!(normalize_directory("   "), "");
    }
}
