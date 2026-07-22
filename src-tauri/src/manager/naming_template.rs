//! 平台命名模板模块（Task 43）。
//!
//! 提供纯逻辑函数，对模板字符串与媒体元数据变量进行替换：
//! - `apply_naming_template`：按模板字符串替换变量，清理非法字符，截断到 100 字符
//! - `find_template_for_platform`：从模板列表中按平台 key 找到启用的模板
//!
//! 模板变量（AGENTS.md §3：所有变量来自真实状态，不使用模拟数据）：
//! - `{author}`：作者/上传者昵称（yt-dlp `uploader`/`channel`/`uploader_id` 优先级回退）
//! - `{title}`：媒体标题（yt-dlp `title`）
//! - `{date}`：上传日期（yt-dlp `upload_date`，已格式化为 `YYYYMMDD`）
//! - `{platform}`：平台 key（`MediaPlatform::as_str()`，如 `douyin`）
//! - `{id}`：站点视频 ID（yt-dlp `id`，如推文 ID / YouTube 视频 ID）
//! - `{channel}`：频道名（yt-dlp `channel`，与 `author` 区分；YouTube 等平台有意义）
//! - `{bvid}`：B 站 BV 号（yt-dlp `display_id` 在 B 站场景下为 BV 号）
//!
//! 安全约束（AGENTS.md §3 / §7）：
//! - 不使用 `unwrap()` / `expect()` 处理可恢复错误
//! - 模板中的未知变量（如 `{foo}`）原样保留，方便用户辨识拼写错误
//! - 替换后的非法字符（`\ / : * ? " < > |` 与控制字符）替换为 `_`，
//!   压缩连续下划线，去除首尾下划线
//! - 总长度超过 100 字符时按 Unicode 标量截断（不含扩展名）
//! - 全空结果回退为 `<platform>_<id>` 或 `media_<timestamp>`，避免空文件名

use crate::models::PlatformNamingTemplate;

/// 模板变量集合（Task 43）。
///
/// 所有字段均为 `Option<String>`，缺失时对应变量替换为空字符串。
/// 调用方（`media::download`）负责从 yt-dlp metadata 提取真实值后构造此结构。
///
/// `bvid` 单独存在（与 `id` 区分）：B 站的 BV 号在 yt-dlp 中通过 `display_id` 返回，
/// 与 `id` 字段（通常是数字 aid）不同，前端期望文件名包含 BV 号。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamingVars {
    pub author: Option<String>,
    pub title: Option<String>,
    pub date: Option<String>,
    pub platform: Option<String>,
    pub id: Option<String>,
    pub channel: Option<String>,
    pub bvid: Option<String>,
}

/// 文件名中 Windows 不允许的字符，需替换为 `_`。
///
/// 与 `media_platforms::ILLEGAL_FILENAME_CHARS` 保持一致，这里单独定义以避免
/// 跨模块耦合（`media_platforms` 中常量为私有，未来如需统一可重新导出）。
const ILLEGAL_CHARS: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// 命名模板最大长度（按 Unicode 标量计数，不含扩展名）。
///
/// 与 `media_platforms::DOUYIN_FILENAME_MAX_LEN` 保持一致，100 字符是保守上限，
/// 避免 NTFS 255 字符上限在加上扩展名、目录路径后溢出。
const MAX_NAME_LEN: usize = 100;

/// 按模板字符串替换变量，清理非法字符，截断到 100 字符。
///
/// - `template`：模板字符串（如 `{author}_{title}_{date}`）
/// - `vars`：变量集合，缺失字段替换为空字符串
///
/// 处理流程：
/// 1. 逐字符扫描模板，遇到 `{var}` 形式时查表替换；未知变量原样保留
/// 2. 替换后的字符串中非法字符与控制字符替换为 `_`
/// 3. 压缩连续下划线为单个，去除首尾下划线
/// 4. 按 Unicode 标量截断到 100 字符
/// 5. 全空时回退为 `media`（调用方负责追加扩展名与避免重名）
///
/// 返回处理后的文件名（不含扩展名）。空模板或全空变量返回 `"media"`。
pub fn apply_naming_template(template: &str, vars: &NamingVars) -> String {
    if template.trim().is_empty() {
        return "media".to_string();
    }
    let replaced = replace_variables(template, vars);
    let sanitized = sanitize_filename(&replaced);
    if sanitized.is_empty() {
        return "media".to_string();
    }
    truncate_unicode(&sanitized, MAX_NAME_LEN)
}

/// 从模板列表中按平台 key 找到第一条启用的模板。
///
/// - `templates`：任意顺序的模板列表
/// - `platform`：平台 key（小写英文，与 `MediaPlatform::as_str()` 对应）
///
/// 返回 `Some(&PlatformNamingTemplate)` 表示命中；`None` 表示无模板命中
/// （包括"该平台有模板但全部禁用"的情况）。
///
/// 大小写不敏感：`Douyin` 与 `douyin` 视为同一平台。
pub fn find_template_for_platform<'a>(
    templates: &'a [PlatformNamingTemplate],
    platform: &str,
) -> Option<&'a PlatformNamingTemplate> {
    let platform_lower = platform.to_ascii_lowercase();
    templates
        .iter()
        .find(|t| t.enabled && t.platform.to_ascii_lowercase() == platform_lower)
}

/// 扫描模板字符串并替换 `{var}` 形式的变量。
///
/// 未知变量（如 `{foo}`）原样保留，方便用户辨识拼写错误。
/// 嵌套花括号（如 `{a{b}}`）不特殊处理，按字符顺序消费。
fn replace_variables(template: &str, vars: &NamingVars) -> String {
    let mut output = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '{' {
            output.push(c);
            continue;
        }
        // 收集 `{` 之后到 `}` 之前的字符作为变量名
        let mut name = String::new();
        let mut found_close = false;
        for inner in chars.by_ref() {
            if inner == '}' {
                found_close = true;
                break;
            }
            name.push(inner);
        }
        if !found_close {
            // 未闭合的 `{` 原样输出，避免吞掉用户输入
            output.push('{');
            output.push_str(&name);
            break;
        }
        let value = lookup_variable(&name, vars);
        match value {
            Some(v) => output.push_str(&v),
            None => {
                // 未知变量：原样保留 `{name}`，方便用户辨识
                output.push('{');
                output.push_str(&name);
                output.push('}');
            }
        }
    }
    output
}

/// 查表得到变量值。
///
/// 返回值语义：
/// - `Some(non_empty)`：已知变量且有值，返回 trim 后的字符串
/// - `Some("")`：已知变量但值为 None 或纯空白，返回空字符串（替换后由 sanitize 清理多余下划线）
/// - `None`：未知变量名（如 `{foo}`），由调用方原样保留 `{name}` 以便用户辨识拼写错误
///
/// 这种区分让用户能区分"拼写错误的变量"（保留 `{foo}`）与"平台未提供该字段"（替换为空）。
fn lookup_variable(name: &str, vars: &NamingVars) -> Option<String> {
    let value: Option<&str> = match name {
        "author" => vars.author.as_deref(),
        "title" => vars.title.as_deref(),
        "date" => vars.date.as_deref(),
        "platform" => vars.platform.as_deref(),
        "id" => vars.id.as_deref(),
        "channel" => vars.channel.as_deref(),
        "bvid" => vars.bvid.as_deref(),
        _ => return None,
    };
    Some(value.unwrap_or("").trim().to_string())
}

/// 把字符串中的非法字符与控制字符替换为 `_`，压缩连续下划线，去除首尾下划线。
pub(crate) fn sanitize_filename(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut prev_underscore = false;
    for c in input.chars() {
        let replaced = if ILLEGAL_CHARS.contains(&c) || c.is_control() {
            '_'
        } else {
            c
        };
        if replaced == '_' {
            if !prev_underscore && !result.is_empty() {
                result.push('_');
                prev_underscore = true;
            }
            // 跳过连续下划线与首部下划线
        } else {
            result.push(replaced);
            prev_underscore = false;
        }
    }
    // 去除尾部下划线（循环去除可能存在的多个）
    while result.ends_with('_') {
        result.pop();
    }
    result
}

/// 按 Unicode 标量截断字符串到 `max` 个字符。
///
/// 不会在 UTF-8 多字节字符中间截断（按 `char_indices` 而非字节切分）。
fn truncate_unicode(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        return input.to_string();
    }
    let end = input
        .char_indices()
        .nth(max)
        .map(|(idx, _)| idx)
        .unwrap_or(input.len());
    input[..end].to_string()
}

// ===== 单元测试 =====
#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> NamingVars {
        NamingVars {
            author: Some("张三".into()),
            title: Some("测试视频".into()),
            date: Some("20260720".into()),
            platform: Some("douyin".into()),
            id: Some("7012345678901234567".into()),
            channel: Some("ZhangSanChannel".into()),
            bvid: Some("BV1xx411c7mD".into()),
        }
    }

    // ---- apply_naming_template: 变量替换 ----

    #[test]
    fn apply_template_replaces_author_title_date() {
        let template = "{author}_{title}_{date}";
        let result = apply_naming_template(template, &vars());
        assert_eq!(result, "张三_测试视频_20260720");
    }

    #[test]
    fn apply_template_replaces_all_seven_variables() {
        let template = "{platform}_{author}_{channel}_{title}_{id}_{bvid}_{date}";
        let result = apply_naming_template(template, &vars());
        assert_eq!(
            result,
            "douyin_张三_ZhangSanChannel_测试视频_7012345678901234567_BV1xx411c7mD_20260720"
        );
    }

    #[test]
    fn apply_template_handles_missing_vars() {
        // 缺失 author / channel / bvid：对应变量替换为空，
        // 但 sanitize 会清理多余的下划线
        let template = "{author}_{title}_{date}_{channel}_{bvid}";
        let mut partial = vars();
        partial.author = None;
        partial.channel = None;
        partial.bvid = None;
        let result = apply_naming_template(template, &partial);
        // 模板: "_测试视频_20260720__"
        // sanitize: 压缩连续下划线 -> "_测试视频_20260720_"
        // 去除首尾下划线 -> "测试视频_20260720"
        assert_eq!(result, "测试视频_20260720");
    }

    #[test]
    fn apply_template_unknown_variable_preserved_as_literal() {
        let template = "{author}_{foo}_{title}";
        let result = apply_naming_template(template, &vars());
        // 未知变量 {foo} 原样保留
        assert_eq!(result, "张三_{foo}_测试视频");
    }

    #[test]
    fn apply_template_empty_value_treated_as_missing() {
        let template = "{author}_{title}";
        let mut partial = vars();
        partial.author = Some("   ".into()); // 空白字符串
        let result = apply_naming_template(template, &partial);
        // author 视为缺失，sanitize 清理首部下划线
        assert_eq!(result, "测试视频");
    }

    // ---- apply_naming_template: 非法字符清理 ----

    #[test]
    fn apply_template_strips_illegal_chars() {
        let template = "{author}_{title}";
        let mut dirty = vars();
        // author 含路径分隔符与非法字符
        dirty.author = Some("a/b:c*d?e\"f<g>h|i".into());
        dirty.title = Some("正常标题".into());
        let result = apply_naming_template(template, &dirty);
        // 所有非法字符替换为 _，然后压缩连续下划线
        assert_eq!(result, "a_b_c_d_e_f_g_h_i_正常标题");
    }

    #[test]
    fn apply_template_strips_control_chars() {
        let template = "{author}_{title}";
        let mut dirty = vars();
        dirty.author = Some("a\nb\tc".into()); // 换行与制表符
        dirty.title = Some("标题".into());
        let result = apply_naming_template(template, &dirty);
        assert_eq!(result, "a_b_c_标题");
    }

    #[test]
    fn apply_template_collapses_consecutive_underscores() {
        let template = "{author}___{title}___{date}";
        let result = apply_naming_template(template, &vars());
        assert_eq!(result, "张三_测试视频_20260720");
    }

    #[test]
    fn apply_template_trims_leading_and_trailing_underscores() {
        let template = "___{author}___";
        let result = apply_naming_template(template, &vars());
        assert_eq!(result, "张三");
    }

    // ---- apply_naming_template: 长度截断 ----

    #[test]
    fn apply_template_truncates_long_names() {
        let template = "{title}_{title}_{title}_{title}_{title}";
        let mut long_title = vars();
        // 每个标题 30 字符 × 5 = 150 字符，需要截断到 100
        long_title.title =
            Some("一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十".into());
        let result = apply_naming_template(template, &long_title);
        let char_count = result.chars().count();
        assert_eq!(char_count, 100);
        // 截断后的前 30 字符应为第一个完整标题
        let first_segment: String = result.chars().take(30).collect();
        assert_eq!(
            first_segment,
            "一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十"
        );
    }

    #[test]
    fn apply_template_short_names_not_truncated() {
        let template = "{author}_{title}";
        let result = apply_naming_template(template, &vars());
        assert_eq!(result.chars().count(), "张三_测试视频".chars().count());
    }

    // ---- apply_naming_template: 边界情况 ----

    #[test]
    fn apply_template_empty_template_returns_media() {
        let result = apply_naming_template("", &vars());
        assert_eq!(result, "media");
    }

    #[test]
    fn apply_template_whitespace_only_template_returns_media() {
        let result = apply_naming_template("   ", &vars());
        assert_eq!(result, "media");
    }

    #[test]
    fn apply_template_all_empty_vars_returns_media() {
        let result = apply_naming_template("{author}_{title}_{date}", &NamingVars::default());
        assert_eq!(result, "media");
    }

    #[test]
    fn apply_template_unclosed_brace_preserved() {
        let template = "{author}_{title";
        let result = apply_naming_template(template, &vars());
        // 未闭合的 { 后续按字面输出
        assert_eq!(result, "张三_{title");
    }

    // ---- find_template_for_platform ----

    fn make_template(
        id: &str,
        platform: &str,
        enabled: bool,
        is_builtin: bool,
    ) -> PlatformNamingTemplate {
        PlatformNamingTemplate {
            id: id.into(),
            platform: platform.into(),
            template: format!("{{author}}_{{title}}_{id}"),
            enabled,
            is_builtin,
        }
    }

    #[test]
    fn find_template_for_platform_returns_matching_template() {
        let templates = vec![
            make_template("t1", "douyin", true, true),
            make_template("t2", "tiktok", true, true),
            make_template("t3", "youtube", false, true),
        ];
        let found = find_template_for_platform(&templates, "douyin");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "t1");
    }

    #[test]
    fn find_template_for_platform_returns_none_for_unknown_platform() {
        let templates = vec![
            make_template("t1", "douyin", true, true),
            make_template("t2", "tiktok", true, true),
        ];
        let found = find_template_for_platform(&templates, "unknown");
        assert!(found.is_none());
    }

    #[test]
    fn find_template_for_platform_skips_disabled() {
        let templates = vec![
            make_template("disabled", "douyin", false, true),
            make_template("enabled", "douyin", true, false),
        ];
        let found = find_template_for_platform(&templates, "douyin");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "enabled");
    }

    #[test]
    fn find_template_for_platform_returns_none_when_all_disabled() {
        let templates = vec![
            make_template("d1", "douyin", false, true),
            make_template("d2", "douyin", false, false),
        ];
        let found = find_template_for_platform(&templates, "douyin");
        assert!(found.is_none());
    }

    #[test]
    fn find_template_for_platform_is_case_insensitive() {
        let templates = vec![make_template("t1", "douyin", true, true)];
        let found = find_template_for_platform(&templates, "DOUYIN");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "t1");
    }

    #[test]
    fn find_template_for_platform_returns_first_match_in_slice_order() {
        // 多条匹配时返回切片中第一条（不按 id 排序，保持简单语义）
        let templates = vec![
            make_template("later", "douyin", true, false),
            make_template("first", "douyin", true, true),
        ];
        let found = find_template_for_platform(&templates, "douyin");
        assert_eq!(found.unwrap().id, "later");
    }

    #[test]
    fn find_template_for_platform_empty_list_returns_none() {
        let found = find_template_for_platform(&[], "douyin");
        assert!(found.is_none());
    }

    // ---- 内置模板的 apply 验证（覆盖 6 个平台默认模板）----

    #[test]
    fn builtin_douyin_template_applies_correctly() {
        let template = "{author}_{title}_{date}";
        let result = apply_naming_template(template, &vars());
        assert_eq!(result, "张三_测试视频_20260720");
    }

    #[test]
    fn builtin_twitter_template_applies_correctly() {
        let template = "{title}_{id}";
        let result = apply_naming_template(template, &vars());
        assert_eq!(result, "测试视频_7012345678901234567");
    }

    #[test]
    fn builtin_youtube_template_applies_correctly() {
        let template = "{channel}_{title}_{id}";
        let result = apply_naming_template(template, &vars());
        assert_eq!(result, "ZhangSanChannel_测试视频_7012345678901234567");
    }

    #[test]
    fn builtin_bilibili_template_applies_correctly() {
        let template = "{author}_{title}_{bvid}";
        let result = apply_naming_template(template, &vars());
        assert_eq!(result, "张三_测试视频_BV1xx411c7mD");
    }
}
