//! 任务模板模块（Task 36）。
//!
//! 提供纯逻辑函数，根据域名匹配任务模板并套用到 [`NewTaskRequest`]：
//! - [`match_template`]：按域名匹配模板，支持精确域名与通配符子域（`*.example.com`）
//! - [`apply_template_to_request`]：把模板字段套用到 [`NewTaskRequest`]
//! - [`extract_domain`]：从 URL 提取小写主机名
//!
//! 匹配语义：
//! - `enabled = false` 的模板不参与匹配
//! - 多模板同时命中时按 `priority` 升序取优先级最高（数字越小越优先），
//!   priority 相同按 `name` 升序作为稳定排序兜底
//! - 通配符 `*.example.com` 同时匹配 `example.com` 与其任意子域
//! - 精确域名 `example.com` 只匹配 `example.com` 本身
//! - 大小写不敏感
//! - 必须在域名边界处匹配（`evilgithub.com` 不匹配 `github.com`）
//!
//! 套用语义（不覆盖用户显式设置的字段，AGENTS.md §3 与 §7）：
//! - `connections`：仅在 `request.connection_count` 为 `None` 时套用；
//!   值必须为 1/2/4/8/16/32 之一，否则跳过该字段。
//! - `speed_limit`：仅在 `request.per_task_speed_limit == 0`（=未设置）时套用。
//! - `headers`：合并到 `request.headers`，请求已存在的键不被覆盖。
//! - `destination`：仅在 `request.destination` 为 `None` 或空字符串时套用。
//! - `completion_action`：仅在 `request.completion_action` 为 `CompletionAction::None`
//!   （默认值）时套用。
//!
//! 安全约束：
//! - 不使用 `unwrap()` / `expect()` 处理可恢复错误（AGENTS.md §7）。
//! - 模板匹配不写入日志，不泄露 URL 中可能存在的认证参数。
//! - `connections` 值由本模块校验，非法值静默跳过，不阻断任务创建。

use crate::models::{CompletionAction, NewTaskRequest, TaskTemplate, TaskTemplateTestResult};
use url::Url;

/// 允许的连接数枚举（AGENTS.md §3：仅 1/2/4/8/16/32）。
const ALLOWED_CONNECTIONS: [u8; 6] = [1, 2, 4, 8, 16, 32];

/// 按域名匹配任务模板。
///
/// 返回首个命中的模板引用（按 `priority` 升序、`name` 升序排序后）。
/// `domain` 应为 URL 主机名（函数内部会再转小写以保险）；
/// `templates` 顺序不限，函数内部稳定排序。
pub fn match_template<'a>(domain: &str, templates: &'a [TaskTemplate]) -> Option<&'a TaskTemplate> {
    let domain = domain.trim().to_ascii_lowercase();
    if domain.is_empty() {
        return None;
    }
    let mut sorted: Vec<&TaskTemplate> = templates.iter().filter(|t| t.enabled).collect();
    // 按 priority 升序；priority 相同时按 name 升序作为稳定兜底
    sorted.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| a.name.cmp(&b.name))
    });
    for template in sorted {
        if domain_matches(&domain, &template.domain_pattern) {
            return Some(template);
        }
    }
    None
}

/// 域名匹配：支持精确域名与通配符子域。
///
/// - 精确 `example.com` 仅匹配 `example.com` 本身
/// - 通配符 `*.example.com` 同时匹配 `example.com` 与其任意子域（如 `api.example.com`）
/// - 大小写不敏感（pattern 与 host 都已转小写）
/// - 必须在域名边界处匹配，不能 `evilgithub.com` 匹配 `github.com`
/// - 空 pattern 不匹配任何域名
fn domain_matches(host: &str, pattern: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    if pattern.is_empty() {
        return false;
    }
    if host == pattern {
        return true;
    }
    // 通配符：`*.example.com` → 提取 `example.com`，匹配 host == example.com
    // 或 host.ends_with(".example.com")
    if let Some(suffix) = pattern.strip_prefix("*.") {
        if suffix.is_empty() {
            return false;
        }
        if host == suffix || host.ends_with(&format!(".{suffix}")) {
            return true;
        }
        return false;
    }
    // 非通配符：仅精确匹配（已在上方 return）
    false
}

/// 从 URL 提取小写主机名。URL 解析失败返回 `None`。
pub fn extract_domain(url: &str) -> Option<String> {
    Url::parse(url.trim())
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
}

/// 测试给定 URL 是否命中任意模板，返回 [`TaskTemplateTestResult`]。
///
/// 供 `task_template_test` 命令使用，便于前端在新建任务对话框展示
/// "已匹配模板：xxx" 提示。URL 解析失败或无模板命中时返回
/// `matched = false`。
pub fn test_task_template(url: &str, templates: &[TaskTemplate]) -> TaskTemplateTestResult {
    let Some(domain) = extract_domain(url) else {
        return TaskTemplateTestResult::default();
    };
    match match_template(&domain, templates) {
        Some(tpl) => TaskTemplateTestResult {
            matched: true,
            matched_template_id: Some(tpl.id.clone()),
            matched_template_name: Some(tpl.name.clone()),
        },
        None => TaskTemplateTestResult::default(),
    }
}

/// 把模板字段套用到 [`NewTaskRequest`]。
///
/// 仅在用户未显式设置对应字段时套用（详见模块文档）。
/// `connections` 必须为 1/2/4/8/16/32 之一，否则跳过该字段。
pub fn apply_template_to_request(template: &TaskTemplate, request: &mut NewTaskRequest) {
    // 连接数：仅在用户未指定时套用
    if request.connection_count.is_none() {
        if let Some(conn) = template.connections {
            if ALLOWED_CONNECTIONS.contains(&conn) {
                request.connection_count = Some(conn);
            }
        }
    }
    // 限速：仅在用户未指定（=0）时套用
    if request.per_task_speed_limit == 0 {
        if let Some(limit) = template.speed_limit {
            request.per_task_speed_limit = limit;
        }
    }
    // 请求头：合并，请求已存在的键不被覆盖
    if let Some(template_headers) = &template.headers {
        for (key, value) in template_headers {
            request
                .headers
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }
    // 保存目录：仅在用户未指定（None 或空字符串）时套用
    if request
        .destination
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_none()
    {
        if let Some(dest) = &template.destination {
            let dest = dest.trim();
            if !dest.is_empty() {
                request.destination = Some(dest.to_string());
            }
        }
    }
    // 完成动作：仅在用户未显式设置（=None 默认值）时套用
    if matches!(request.completion_action, CompletionAction::None) {
        if let Some(action) = &template.completion_action {
            request.completion_action = action.clone();
        }
    }
}

// ===== 单元测试 =====
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CollisionPolicy;
    use std::collections::HashMap;

    fn make_template(id: &str, pattern: &str, enabled: bool, priority: i32) -> TaskTemplate {
        TaskTemplate {
            id: id.into(),
            name: format!("tpl-{id}"),
            domain_pattern: pattern.into(),
            connections: None,
            speed_limit: None,
            headers: None,
            destination: None,
            completion_action: None,
            enabled,
            priority,
        }
    }

    fn make_request() -> NewTaskRequest {
        NewTaskRequest {
            url: "https://example.com/file.zip".into(),
            file_name: None,
            destination: None,
            headers: HashMap::new(),
            scheduled_at: None,
            priority: 0,
            expected_checksum: None,
            source: None,
            per_task_speed_limit: 0,
            collision_policy: CollisionPolicy::Rename,
            completion_action: CompletionAction::None,
            media: None,
            connection_count: None,
            start_paused: false,
            user_edited_file_name: false,
        }
    }

    // ---- match_template 域名匹配 ----

    #[test]
    fn match_template_exact_domain() {
        let templates = vec![
            make_template("a", "example.com", true, 0),
            make_template("b", "other.com", true, 0),
        ];
        let matched = match_template("example.com", &templates);
        assert_eq!(matched.map(|t| t.id.as_str()), Some("a"));
    }

    #[test]
    fn match_template_wildcard_subdomain() {
        // *.example.com 应同时匹配 example.com 与 api.example.com
        let templates = vec![make_template("w", "*.example.com", true, 0)];
        assert!(match_template("example.com", &templates).is_some());
        assert!(match_template("api.example.com", &templates).is_some());
        assert!(match_template("sub.api.example.com", &templates).is_some());
        // 不匹配无关域名
        assert!(match_template("other.com", &templates).is_none());
        // 不匹配部分后缀（必须在域名边界）
        assert!(match_template("evilexample.com", &templates).is_none());
    }

    #[test]
    fn match_template_priority_order() {
        // 两条模板都命中，priority 数字小的优先
        let templates = vec![
            make_template("low", "example.com", true, 10),
            make_template("high", "example.com", true, 1),
        ];
        let matched = match_template("example.com", &templates);
        assert_eq!(matched.map(|t| t.id.as_str()), Some("high"));
    }

    // ---- apply_template_to_request ----

    #[test]
    fn apply_template_to_request_connections() {
        let mut template = make_template("c", "example.com", true, 0);
        template.connections = Some(8);
        let mut request = make_request();
        apply_template_to_request(&template, &mut request);
        assert_eq!(request.connection_count, Some(8));
    }

    #[test]
    fn apply_template_to_request_speed_limit() {
        let mut template = make_template("s", "example.com", true, 0);
        template.speed_limit = Some(1024 * 1024);
        let mut request = make_request();
        apply_template_to_request(&template, &mut request);
        assert_eq!(request.per_task_speed_limit, 1024 * 1024);
    }

    #[test]
    fn apply_template_to_request_headers() {
        let mut template = make_template("h", "example.com", true, 0);
        let mut headers = HashMap::new();
        headers.insert("Authorization".into(), "Bearer token".into());
        headers.insert("User-Agent".into(), "MaobuFetch".into());
        template.headers = Some(headers);
        let mut request = make_request();
        // 用户已设置 User-Agent
        request
            .headers
            .insert("User-Agent".into(), "CustomUA".into());
        apply_template_to_request(&template, &mut request);
        // 模板的 Authorization 应被合并
        assert_eq!(
            request.headers.get("Authorization"),
            Some(&"Bearer token".to_string())
        );
        // 用户的 User-Agent 不应被覆盖
        assert_eq!(
            request.headers.get("User-Agent"),
            Some(&"CustomUA".to_string())
        );
    }

    #[test]
    fn apply_template_does_not_override_user_set_fields() {
        let mut template = make_template("u", "example.com", true, 0);
        template.connections = Some(16);
        template.speed_limit = Some(2048);
        template.destination = Some("/template/dir".into());
        template.completion_action = Some(CompletionAction::Quit);

        let mut request = make_request();
        // 用户显式设置了所有字段
        request.connection_count = Some(4);
        request.per_task_speed_limit = 512;
        request.destination = Some("/user/dir".into());
        request.completion_action = CompletionAction::OpenFolder;

        apply_template_to_request(&template, &mut request);
        // 模板不应覆盖任何字段
        assert_eq!(request.connection_count, Some(4));
        assert_eq!(request.per_task_speed_limit, 512);
        assert_eq!(request.destination.as_deref(), Some("/user/dir"));
        assert!(matches!(
            request.completion_action,
            CompletionAction::OpenFolder
        ));
    }

    // ---- 边界情况 ----

    #[test]
    fn match_template_skips_disabled() {
        let templates = vec![
            make_template("disabled", "example.com", false, 0),
            make_template("enabled", "other.com", true, 1),
        ];
        // disabled 模板即使匹配也不应被选中
        let matched = match_template("example.com", &templates);
        assert!(matched.is_none());
    }

    #[test]
    fn match_template_empty_domain_returns_none() {
        let templates = vec![make_template("a", "example.com", true, 0)];
        assert!(match_template("", &templates).is_none());
        assert!(match_template("   ", &templates).is_none());
    }

    #[test]
    fn match_template_empty_pattern_never_matches() {
        let templates = vec![make_template("a", "", true, 0)];
        assert!(match_template("example.com", &templates).is_none());
    }

    #[test]
    fn match_template_case_insensitive() {
        // 精确域名匹配：host 与 pattern 都被转为小写后比较
        let templates = vec![make_template("a", "EXAMPLE.COM", true, 0)];
        assert!(match_template("example.com", &templates).is_some());
        assert!(match_template("Example.COM", &templates).is_some());
        // 通配符模式：大小写混合也支持子域匹配
        let wildcard_templates = vec![make_template("w", "*.EXAMPLE.COM", true, 0)];
        assert!(match_template("API.Example.COM", &wildcard_templates).is_some());
    }

    #[test]
    fn apply_template_invalid_connections_skipped() {
        let mut template = make_template("ic", "example.com", true, 0);
        template.connections = Some(3); // 3 不是 1/2/4/8/16/32
        let mut request = make_request();
        apply_template_to_request(&template, &mut request);
        assert_eq!(request.connection_count, None);
    }

    #[test]
    fn apply_template_empty_destination_skipped() {
        let mut template = make_template("ed", "example.com", true, 0);
        template.destination = Some("   ".into());
        let mut request = make_request();
        apply_template_to_request(&template, &mut request);
        assert_eq!(request.destination, None);
    }

    #[test]
    fn extract_domain_strips_query_and_fragment() {
        assert_eq!(
            extract_domain("https://api.example.com/path?query=1#frag"),
            Some("api.example.com".into())
        );
        assert_eq!(extract_domain("not-a-url"), None);
    }
}
