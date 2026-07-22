//! 文件名清理规则模块（Task 20）。
//!
//! 提供纯逻辑函数，对一组清理规则与文件名进行正则替换：
//! - `apply_filename_cleanup`：按 `priority` 升序遍历启用的规则，依次执行正则替换
//! - `apply_filename_cleanup_owned`：返回 `String`，便于在 manager 中链式调用
//!
//! 安全约束（AGENTS.md §7）：
//! - 不使用 `unwrap()` / `expect()` 处理可恢复错误
//! - 单条规则正则编译失败按"跳过该规则"处理，避免阻塞整个流程
//! - 不修改原切片，复制后按 priority 排序
//!
//! 应用语义（AGENTS.md §3）：
//! - 仅在用户未手动编辑文件名时应用
//! - 不影响下载本身（仅在写入目标文件前对文件名做替换）
//! - 不在 UI 中伪造分片或状态

use crate::models::FilenameCleanupRule;
use regex::Regex;

/// 按优先级升序遍历启用的规则，依次执行正则替换。
///
/// - `filename`：原始文件名
/// - `rules`：任意顺序的规则列表，函数内部按 `priority` 升序排序
///
/// 编译失败的规则会被跳过（不 panic）。空文件名原样返回。
pub fn apply_filename_cleanup(filename: &str, rules: &[FilenameCleanupRule]) -> String {
    if filename.is_empty() {
        return String::new();
    }
    let mut sorted: Vec<&FilenameCleanupRule> = rules.iter().filter(|r| r.enabled).collect();
    // 稳定排序：priority 相同时保持原切片顺序
    sorted.sort_by_key(|r| r.priority);
    let mut current = filename.to_string();
    for rule in sorted {
        let Some(regex) = compile_rule(&rule.pattern) else {
            continue;
        };
        current = regex.replace_all(&current, &rule.replacement).into_owned();
    }
    current
}

/// 编译单条规则的正则表达式。编译失败返回 `None`。
fn compile_rule(pattern: &str) -> Option<Regex> {
    if pattern.is_empty() {
        return None;
    }
    Regex::new(pattern).ok()
}

// ===== 单元测试 =====
#[cfg(test)]
mod tests {
    use super::*;

    fn make_rule(
        id: &str,
        pattern: &str,
        replacement: &str,
        enabled: bool,
        priority: i32,
    ) -> FilenameCleanupRule {
        FilenameCleanupRule {
            id: id.into(),
            name: format!("rule-{id}"),
            pattern: pattern.into(),
            replacement: replacement.into(),
            enabled,
            priority,
        }
    }

    // ---- 默认规则的应用 ----

    #[test]
    fn default_rule_remove_bracket_site_strips_www_mark() {
        let rule = make_rule("remove-bracket-site", r"\[(www\.)?[\w.-]+\]", "", true, 10);
        let cleaned = apply_filename_cleanup("movie [www.example.com].mp4", &[rule]);
        assert_eq!(cleaned, "movie .mp4");
    }

    #[test]
    fn default_rule_remove_hash_tags_strips_topics() {
        let rule = make_rule("remove-hash-tags", r"#[^\s#.]+", "", true, 35);
        let cleaned = apply_filename_cleanup("大疆 P4P vs 影石 Luna 两周使用，我更推荐谁？ #pocket4pro #lunaultra.mp4", &[rule]);
        assert_eq!(cleaned, "大疆 P4P vs 影石 Luna 两周使用，我更推荐谁？  .mp4");
    }

    #[test]
    fn default_rule_remove_bracket_site_strips_bare_domain() {
        let rule = make_rule("remove-bracket-site", r"\[(www\.)?[\w.-]+\]", "", true, 10);
        let cleaned = apply_filename_cleanup("video [example.com].mkv", &[rule]);
        assert_eq!(cleaned, "video .mkv");
    }

    #[test]
    fn default_rule_remove_paren_quality_strips_1080p() {
        let rule = make_rule("remove-paren-quality", r"\(\d{3,4}[pP]\)", "", true, 20);
        let cleaned = apply_filename_cleanup("clip (1080p).mp4", &[rule]);
        assert_eq!(cleaned, "clip .mp4");
    }

    #[test]
    fn default_rule_remove_paren_quality_strips_uppercase_720p() {
        let rule = make_rule("remove-paren-quality", r"\(\d{3,4}[pP]\)", "", true, 20);
        let cleaned = apply_filename_cleanup("clip (720P).mp4", &[rule]);
        assert_eq!(cleaned, "clip .mp4");
    }

    #[test]
    fn default_rule_remove_paren_quality_does_not_strip_two_digits() {
        // \d{3,4} 要求 3 或 4 位数字，"10p" 不应被匹配
        let rule = make_rule("remove-paren-quality", r"\(\d{3,4}[pP]\)", "", true, 20);
        let cleaned = apply_filename_cleanup("clip (10p).mp4", &[rule]);
        assert_eq!(cleaned, "clip (10p).mp4");
    }

    #[test]
    fn default_rule_remove_underscore_site_strips包围() {
        let rule = make_rule("remove-underscore-site", r"_www\.[\w.-]+_", "", true, 30);
        let cleaned = apply_filename_cleanup("file_www.example.com_.zip", &[rule]);
        assert_eq!(cleaned, "file.zip");
    }

    #[test]
    fn default_rule_collapse_spaces_merges_runs() {
        let rule = make_rule("collapse-spaces", r"[\s_]+", " ", true, 40);
        let cleaned = apply_filename_cleanup("a   b__c	d", &[rule]);
        assert_eq!(cleaned, "a b c d");
    }

    #[test]
    fn default_rule_remove_chinese_bracket_site_strips_mark() {
        let rule = make_rule("remove-chinese-bracket-site", r"【(www\.)?[\w.-]+】", "", true, 11);
        let cleaned = apply_filename_cleanup("video【www.example.com】.mp4", &[rule]);
        assert_eq!(cleaned, "video.mp4");
    }

    #[test]
    fn default_rule_remove_chinese_bracket_promo_strips_mark() {
        let rule = make_rule(
            "remove-chinese-bracket-promo",
            r"【[^】]*?(最新|发布|免费|首发|高清|下载|分享|关注|精品|推荐|无水印)[^】]*?】",
            "",
            true,
            12,
        );
        let cleaned = apply_filename_cleanup("video【最新发布免费下载】.mp4", &[rule]);
        assert_eq!(cleaned, "video.mp4");
    }

    #[test]
    fn default_rule_remove_square_bracket_quality_strips_mark() {
        let rule = make_rule("remove-square-bracket-quality", r"\[\d{3,4}[pP]\]", "", true, 21);
        let cleaned = apply_filename_cleanup("video [1080p].mp4", &[rule]);
        assert_eq!(cleaned, "video .mp4");
    }

    #[test]
    fn default_rule_remove_media_codec_tags_strips_mark() {
        let rule = make_rule(
            "remove-media-codec-tags",
            r"(?i)[._-]?\b(h\.?264|x264|h\.?265|x265|hevc|bluray|web-?rip|hdr|ddp\d\.\d|aac|dts)\b",
            "",
            true,
            25,
        );
        let cleaned = apply_filename_cleanup("video.x264.AAC.mp4", &[rule]);
        assert_eq!(cleaned, "video.mp4");
    }

    #[test]
    fn default_rule_remove_copy_suffix_strips_mark() {
        let rule = make_rule("remove-copy-suffix", r"\s*-\s*副本|\s*-\s*Copy", "", true, 38);
        let cleaned = apply_filename_cleanup("video - 副本 - Copy.mp4", &[rule]);
        assert_eq!(cleaned, "video.mp4");
    }

    #[test]
    fn default_rule_strip_trailing_spaces_strips_space() {
        let rule = make_rule("strip-trailing-spaces", r"\s+(\.[a-zA-Z0-9]+)$", "$1", true, 45);
        let cleaned = apply_filename_cleanup("video .mp4", &[rule]);
        assert_eq!(cleaned, "video.mp4");
    }

    // ---- 多条规则按优先级 ----

    #[test]
    fn multiple_rules_apply_in_priority_order() {
        // 故意按乱序传入，验证按 priority 升序执行
        let rules = vec![
            // 后执行：合并空格
            make_rule("collapse", r"[\s_]+", " ", true, 40),
            // 先执行：去除 [www.x.com]
            make_rule("bracket", r"\[(www\.)?[\w.-]+\]", "", true, 10),
            // 中间执行：去除 (1080p)
            make_rule("paren", r"\(\d{3,4}[pP]\)", "", true, 20),
        ];
        let cleaned = apply_filename_cleanup("Movie [www.site.com] (1080p) clip.mp4", &rules);
        // 执行顺序：bracket -> paren -> collapse
        // "Movie [www.site.com] (1080p) clip.mp4"
        // -> "Movie  (1080p) clip.mp4" (bracket 删除)
        // -> "Movie  clip.mp4" (paren 删除)
        // -> "Movie clip.mp4" (collapse 合并双空格)
        assert_eq!(cleaned, "Movie clip.mp4");
    }

    #[test]
    fn disabled_rules_are_skipped() {
        let rules = vec![
            make_rule("bracket", r"\[(www\.)?[\w.-]+\]", "", false, 10),
            make_rule("paren", r"\(\d{3,4}[pP]\)", "", true, 20),
        ];
        let cleaned = apply_filename_cleanup("Movie [www.site.com] (1080p).mp4", &rules);
        // bracket 被禁用 -> 仅 paren 生效
        assert_eq!(cleaned, "Movie [www.site.com] .mp4");
    }

    #[test]
    fn no_rules_returns_input_unchanged() {
        let cleaned = apply_filename_cleanup("file.mp4", &[]);
        assert_eq!(cleaned, "file.mp4");
    }

    #[test]
    fn all_disabled_rules_return_input_unchanged() {
        let rules = vec![
            make_rule("a", r"x", "y", false, 0),
            make_rule("b", r"z", "w", false, 1),
        ];
        let cleaned = apply_filename_cleanup("xyz", &rules);
        assert_eq!(cleaned, "xyz");
    }

    // ---- 编译失败的规则跳过 ----

    #[test]
    fn invalid_regex_rule_is_skipped() {
        // 不合法正则不应 panic，按跳过处理
        let rules = vec![
            make_rule("bad", "[invalid", "", true, 0),
            make_rule("good", r"\(\d{3,4}[pP]\)", "", true, 10),
        ];
        let cleaned = apply_filename_cleanup("clip (1080p).mp4", &rules);
        // bad 被跳过，good 生效
        assert_eq!(cleaned, "clip .mp4");
    }

    #[test]
    fn empty_pattern_rule_is_skipped() {
        // 空模式不应触发正则编译，按跳过处理
        let rules = vec![
            make_rule("empty", "", "X", true, 0),
            make_rule("good", r"\s+", "_", true, 10),
        ];
        let cleaned = apply_filename_cleanup("a b c", &rules);
        // empty 跳过，good 生效
        assert_eq!(cleaned, "a_b_c");
    }

    #[test]
    fn invalid_regex_does_not_panic_with_only_bad_rules() {
        let rules = vec![make_rule("bad", "[invalid", "", true, 0)];
        let cleaned = apply_filename_cleanup("anything.mp4", &rules);
        assert_eq!(cleaned, "anything.mp4");
    }

    // ---- 边界情况 ----

    #[test]
    fn empty_filename_returns_empty() {
        let rules = vec![make_rule("a", r"x", "y", true, 0)];
        let cleaned = apply_filename_cleanup("", &rules);
        assert!(cleaned.is_empty());
    }

    #[test]
    fn apply_does_not_modify_input_slice_order() {
        let rules = vec![
            make_rule("b", r"x", "y", true, 10),
            make_rule("a", r"z", "w", true, 1),
        ];
        let _ = apply_filename_cleanup("xz", &rules);
        // 原切片顺序应保持不变
        assert_eq!(rules[0].id, "b");
        assert_eq!(rules[1].id, "a");
    }

    #[test]
    fn priority_ties_preserve_input_order() {
        // 同 priority 时按切片顺序执行：先 a 后 b
        let rules = vec![
            make_rule("a", r"X", "1", true, 0),
            make_rule("b", r"1", "2", true, 0),
        ];
        let cleaned = apply_filename_cleanup("X", &rules);
        // a 把 X 替换为 1，b 把 1 替换为 2
        assert_eq!(cleaned, "2");
    }

    #[test]
    fn replacement_with_capture_groups_works() {
        // 验证 replacement 中可以使用正则捕获组。
        // regex crate 要求使用 ${N} 语法在捕获组后紧跟下划线等标识符字符时，
        // 否则 "$2_" 会被解析为名为 "2_" 的捕获组（不存在 -> 空字符串）。
        let rules = vec![make_rule("swap", r"(\w+)_(\w+)", "${2}_${1}", true, 0)];
        let cleaned = apply_filename_cleanup("hello_world", &rules);
        assert_eq!(cleaned, "world_hello");
    }

    #[test]
    fn apply_filename_cleanup_owned_round_trips() {
        let rules = vec![make_rule("a", r"\s+", "_", true, 0)];
        let cleaned = apply_filename_cleanup("a b c", &rules);
        assert_eq!(cleaned, "a_b_c");
    }
}
