//! 重复任务检测模块（Task 10）。
//!
//! 在创建新任务前，比对当前 URL、目标路径与已有任务，识别四类冲突：
//! - `SameUrl`：完全相同 URL（剥离跟踪参数后比对）
//! - `SameFinalUrl`：重定向后相同 URL（剥离跟踪参数后比对）
//! - `SameTargetPath`：相同目标文件路径（规范化后比对）
//! - `SameChecksum`：已完成文件大小 + SHA-256 相同
//!
//! 跟踪参数白名单仅剥离 utm_*、fbclid、gclid、mc_*、igshid 等已知跟踪参数，
//! 保留 sign、auth、token、X-Amz-Signature、access_token 等业务参数。
//!
//! 安全约束：
//! - 不使用 `unwrap()` / `expect()` 处理可恢复错误
//! - URL 解析失败时原样返回输入字符串，不阻断流程
//! - 已 Cancelled 的任务不参与比对（用户已主动放弃）

use crate::models::{
    DownloadTask, DuplicateCheckResult, DuplicateMatch, DuplicateType, TaskStatus,
};
use std::path::PathBuf;
use url::Url;

use super::DownloadManager;

/// 跟踪参数白名单（大小写不敏感匹配）。
///
/// 仅这些参数会在 URL 比对前被剥离。其他参数（如 sign、token、auth、
/// X-Amz-Signature、access_token）必须保留，因为它们可能是业务必需的。
const TRACKING_PARAMS: &[&str] = &[
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "fbclid",
    "gclid",
    "mc_cid",
    "mc_eid",
    "igshid",
    "msclkid",
    "yclid",
    "_hsenc",
    "_hsmi",
    "icid",
    "vero_id",
    "oly_enc_id",
    "oly_anon_id",
];

/// `ref` 参数仅在值为 tracking 性质时剥离。
///
/// Tracking 性质的判定：值长度 > 10 且仅含字母、数字、'-'、'_'。
/// 短值如 "menu"、"sidebar" 视为导航引用，保留。
fn is_tracking_ref_value(value: &str) -> bool {
    value.len() > 10
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 判断参数是否为跟踪参数（参数名大小写不敏感）。
fn is_tracking_param(name_lower: &str, value: &str) -> bool {
    if TRACKING_PARAMS.iter().any(|p| *p == name_lower) {
        return true;
    }
    if name_lower == "ref" && is_tracking_ref_value(value) {
        return true;
    }
    false
}

/// 剥离 URL 中的跟踪参数，保留业务参数。
///
/// 使用 `url::Url` 解析，仅修改 query，保留 fragment 和其他部分。
/// 大小写不敏感匹配参数名（`UTM_SOURCE` 与 `utm_source` 等价）。
/// 解析失败时原样返回输入字符串。
pub fn strip_tracking_params(input: &str) -> String {
    let Ok(mut url) = Url::parse(input) else {
        return input.to_string();
    };

    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    if pairs.is_empty() {
        return url.to_string();
    }

    let kept: Vec<(String, String)> = pairs
        .into_iter()
        .filter(|(key, value)| {
            let key_lower = key.to_lowercase();
            !is_tracking_param(&key_lower, value)
        })
        .collect();

    // 重建 query，保留未被剥离的参数
    {
        let mut query = url.query_pairs_mut();
        query.clear();
        for (k, v) in &kept {
            query.append_pair(k, v);
        }
    }

    // 如果所有参数都被剥离，移除 '?'
    if kept.is_empty() {
        url.set_query(None);
    }

    url.to_string()
}

/// 规范化目标文件路径（用于比较，大小写不敏感）。
///
/// 统一为小写，去除尾部路径分隔符。使用 PathBuf 拼接以统一分隔符。
pub(crate) fn normalize_target_path(directory: &str, file_name: &str) -> String {
    let dir = directory.trim_end_matches(['/', '\\']);
    if dir.is_empty() || file_name.is_empty() {
        return String::new();
    }
    PathBuf::from(dir)
        .join(file_name)
        .to_string_lossy()
        .to_lowercase()
}

/// 规范化完整路径（用于比较，大小写不敏感）。
fn normalize_full_path(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return String::new();
    }
    PathBuf::from(trimmed).to_string_lossy().to_lowercase()
}

/// 为任务构建一个简短显示标签（文件名优先，否则用 URL）。
fn task_label(task: &DownloadTask) -> String {
    if !task.file_name.is_empty() {
        task.file_name.clone()
    } else {
        task.url.clone()
    }
}

/// 从任务列表中查找重复匹配（纯函数，便于单元测试）。
///
/// 比对优先级：SameUrl > SameFinalUrl > SameTargetPath > SameChecksum。
/// 已 Cancelled 的任务不参与比对。URL 比对前会先剥离跟踪参数。
/// 每个任务最多命中一种冲突类型（优先级最高的）。
pub(crate) fn find_duplicate_matches(
    tasks: &[DownloadTask],
    url: &str,
    target_path: &str,
    file_size: Option<u64>,
    sha256: Option<&str>,
) -> Vec<DuplicateMatch> {
    let normalized_url = strip_tracking_params(url);
    let normalized_target = normalize_full_path(target_path);

    let mut matches = Vec::new();
    for task in tasks {
        if matches!(task.status, TaskStatus::Cancelled) {
            continue;
        }

        // SameUrl：剥离跟踪参数后比较
        if !normalized_url.is_empty() {
            let task_url_normalized = strip_tracking_params(&task.url);
            if task_url_normalized == normalized_url {
                matches.push(DuplicateMatch {
                    duplicate_type: DuplicateType::SameUrl,
                    existing_task_id: task.id.clone(),
                    existing_task_label: task_label(task),
                    existing_task_status: task.status.as_str().to_string(),
                });
                continue;
            }
        }

        // SameFinalUrl：如果任务有 final_url，剥离跟踪参数后比较
        if !normalized_url.is_empty() {
            if let Some(task_final) = task.final_url.as_deref() {
                if !task_final.is_empty() {
                    let task_final_normalized = strip_tracking_params(task_final);
                    if task_final_normalized == normalized_url {
                        matches.push(DuplicateMatch {
                            duplicate_type: DuplicateType::SameFinalUrl,
                            existing_task_id: task.id.clone(),
                            existing_task_label: task_label(task),
                            existing_task_status: task.status.as_str().to_string(),
                        });
                        continue;
                    }
                }
            }
        }

        // SameTargetPath：规范化后比较
        if !normalized_target.is_empty() {
            let task_target = normalize_target_path(&task.destination, &task.file_name);
            if !task_target.is_empty() && task_target == normalized_target {
                matches.push(DuplicateMatch {
                    duplicate_type: DuplicateType::SameTargetPath,
                    existing_task_id: task.id.clone(),
                    existing_task_label: task_label(task),
                    existing_task_status: task.status.as_str().to_string(),
                });
                continue;
            }
        }

        // SameChecksum：已完成任务的 file_size + sha256 匹配
        if matches!(task.status, TaskStatus::Completed) {
            if let (Some(new_size), Some(new_sha)) = (file_size, sha256) {
                if task.total_bytes == new_size {
                    if let Some(task_sha) = task.checksum_sha256.as_deref() {
                        if !task_sha.is_empty() && task_sha.eq_ignore_ascii_case(new_sha) {
                            matches.push(DuplicateMatch {
                                duplicate_type: DuplicateType::SameChecksum,
                                existing_task_id: task.id.clone(),
                                existing_task_label: task_label(task),
                                existing_task_status: task.status.as_str().to_string(),
                            });
                            continue;
                        }
                    }
                }
            }
        }
    }

    matches
}

impl DownloadManager {
    /// 检测新任务是否与已有任务重复。
    ///
    /// 比对四类冲突：`SameUrl`、`SameFinalUrl`、`SameTargetPath`、`SameChecksum`。
    /// 已 Cancelled 的任务不参与比对。URL 比对前会先剥离跟踪参数（白名单方式）。
    ///
    /// `file_size` 和 `sha256` 来自预检结果（可选），用于 `SameChecksum` 检测。
    /// 任一为 `None` 时跳过 `SameChecksum` 比对。
    pub async fn check_duplicate(
        &self,
        url: &str,
        target_path: &str,
        file_size: Option<u64>,
        sha256: Option<&str>,
    ) -> Result<DuplicateCheckResult, String> {
        let tasks = self.store.list_tasks().await?;
        Ok(DuplicateCheckResult {
            matches: find_duplicate_matches(&tasks, url, target_path, file_size, sha256),
        })
    }
}

// ===== 单元测试 =====
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CollisionPolicy, CompletionAction, MediaSelection};
    use std::collections::HashMap;

    // ---- strip_tracking_params：白名单内剥离 ----

    #[test]
    fn strip_removes_single_utm_source() {
        let result = strip_tracking_params("https://example.com/file.zip?utm_source=x");
        assert_eq!(result, "https://example.com/file.zip");
    }

    #[test]
    fn strip_removes_multiple_utm_params() {
        let result = strip_tracking_params(
            "https://example.com/file.zip?utm_source=x&utm_medium=y&utm_campaign=z",
        );
        assert_eq!(result, "https://example.com/file.zip");
    }

    #[test]
    fn strip_removes_fbclid() {
        let result = strip_tracking_params("https://example.com/file.zip?fbclid=abc");
        assert_eq!(result, "https://example.com/file.zip");
    }

    #[test]
    fn strip_removes_gclid() {
        let result = strip_tracking_params("https://example.com/file.zip?gclid=xyz");
        assert_eq!(result, "https://example.com/file.zip");
    }

    #[test]
    fn strip_removes_mc_cid_and_mc_eid() {
        let result = strip_tracking_params("https://example.com/file.zip?mc_cid=abc&mc_eid=def");
        assert_eq!(result, "https://example.com/file.zip");
    }

    #[test]
    fn strip_removes_igshid_msclkid_yclid() {
        let result =
            strip_tracking_params("https://example.com/file.zip?igshid=a&msclkid=b&yclid=c");
        assert_eq!(result, "https://example.com/file.zip");
    }

    #[test]
    fn strip_removes_hsenc_hsmi() {
        let result = strip_tracking_params("https://example.com/file.zip?_hsenc=1&_hsmi=2");
        assert_eq!(result, "https://example.com/file.zip");
    }

    #[test]
    fn strip_removes_icid_vero_id_oly_ids() {
        let result = strip_tracking_params(
            "https://example.com/file.zip?icid=1&vero_id=2&oly_enc_id=3&oly_anon_id=4",
        );
        assert_eq!(result, "https://example.com/file.zip");
    }

    // ---- 关键测试用例：白名单外参数必须保留 ----

    #[test]
    fn strip_preserves_sign_param_with_utm_source() {
        // 关键测试用例 1: ?utm_source=x&sign=abc → 仅剥离 utm_source
        let result = strip_tracking_params("https://example.com/file.zip?utm_source=x&sign=abc");
        assert_eq!(result, "https://example.com/file.zip?sign=abc");
    }

    #[test]
    fn strip_preserves_token_param_with_fbclid() {
        // 关键测试用例 2: ?token=secret&fbclid=xxx → 仅剥离 fbclid
        let result = strip_tracking_params("https://example.com/file.zip?token=secret&fbclid=xxx");
        assert_eq!(result, "https://example.com/file.zip?token=secret");
    }

    #[test]
    fn strip_preserves_x_amz_signature_with_utm_campaign() {
        // 关键测试用例 3: ?X-Amz-Signature=abc&utm_campaign=y → 仅剥离 utm_campaign
        let result = strip_tracking_params(
            "https://example.com/file.zip?X-Amz-Signature=abc&utm_campaign=y",
        );
        assert_eq!(result, "https://example.com/file.zip?X-Amz-Signature=abc");
    }

    #[test]
    fn strip_preserves_access_token_and_auth() {
        let result = strip_tracking_params(
            "https://example.com/file.zip?access_token=secret&auth=basic&signature=abc",
        );
        assert_eq!(
            result,
            "https://example.com/file.zip?access_token=secret&auth=basic&signature=abc"
        );
    }

    // ---- 混合大小写测试 ----

    #[test]
    fn strip_matches_case_insensitive() {
        // 大小写混合：UTM_SOURCE、Fbclid、GCLID 都应被剥离
        let result =
            strip_tracking_params("https://example.com/file.zip?UTM_SOURCE=x&Fbclid=y&GCLID=z");
        assert_eq!(result, "https://example.com/file.zip");
    }

    #[test]
    fn strip_preserves_original_case_of_kept_params() {
        // 被保留的参数名应保持原大小写
        let result = strip_tracking_params("https://example.com/file.zip?utm_source=x&Sign=abc");
        assert_eq!(result, "https://example.com/file.zip?Sign=abc");
    }

    // ---- fragment 保留 ----

    #[test]
    fn strip_preserves_fragment() {
        let result = strip_tracking_params("https://example.com/file.zip?utm_source=x#section-1");
        assert_eq!(result, "https://example.com/file.zip#section-1");
    }

    #[test]
    fn strip_preserves_fragment_with_kept_params() {
        let result =
            strip_tracking_params("https://example.com/file.zip?utm_source=x&sign=abc#frag");
        assert_eq!(result, "https://example.com/file.zip?sign=abc#frag");
    }

    // ---- 边界情况 ----

    #[test]
    fn strip_url_without_query_returns_unchanged() {
        let result = strip_tracking_params("https://example.com/file.zip");
        assert_eq!(result, "https://example.com/file.zip");
    }

    #[test]
    fn strip_invalid_url_returns_input_unchanged() {
        let input = "not a url at all";
        let result = strip_tracking_params(input);
        assert_eq!(result, input);
    }

    #[test]
    fn strip_all_params_removed_drops_question_mark() {
        let result = strip_tracking_params("https://example.com/file.zip?utm_source=x");
        assert!(!result.contains('?'));
    }

    #[test]
    fn strip_ref_short_value_preserved() {
        // 短值 "menu" 是导航引用，应保留
        let result = strip_tracking_params("https://example.com/file.zip?ref=menu");
        assert_eq!(result, "https://example.com/file.zip?ref=menu");
    }

    #[test]
    fn strip_ref_long_alphanumeric_value_removed() {
        // 长值 "tfNjX3vJ9kLp" 是 tracking ID，应剥离
        let result = strip_tracking_params("https://example.com/file.zip?ref=tfNjX3vJ9kLp");
        assert_eq!(result, "https://example.com/file.zip");
    }

    #[test]
    fn strip_preserves_multiple_kept_params_order() {
        let result = strip_tracking_params("https://example.com/file.zip?a=1&utm_source=x&b=2&c=3");
        assert_eq!(result, "https://example.com/file.zip?a=1&b=2&c=3");
    }

    // ---- 路径规范化 ----

    #[test]
    fn normalize_target_path_lowercases() {
        let a = normalize_target_path("C:\\Downloads", "FILE.ZIP");
        let b = normalize_target_path("c:\\downloads", "file.zip");
        assert_eq!(a, b);
    }

    #[test]
    fn normalize_target_path_trims_trailing_separators() {
        let a = normalize_target_path("C:\\Downloads\\", "file.zip");
        let b = normalize_target_path("C:\\Downloads", "file.zip");
        assert_eq!(a, b);
    }

    #[test]
    fn normalize_target_path_returns_empty_for_empty_inputs() {
        assert_eq!(normalize_target_path("", "file.zip"), "");
        assert_eq!(normalize_target_path("C:\\Dir", ""), "");
    }

    #[test]
    fn normalize_full_path_lowercases_and_trims() {
        let a = normalize_full_path("C:\\Downloads\\FILE.ZIP");
        let b = normalize_full_path("c:\\downloads\\file.zip");
        assert_eq!(a, b);
    }

    // ---- find_duplicate_matches ----

    fn make_task(
        id: &str,
        url: &str,
        final_url: Option<&str>,
        dest: &str,
        name: &str,
        status: TaskStatus,
    ) -> DownloadTask {
        DownloadTask {
            id: id.to_string(),
            url: url.to_string(),
            file_name: name.to_string(),
            destination: dest.to_string(),
            total_bytes: 0,
            downloaded_bytes: 0,
            speed: 0,
            eta_seconds: None,
            status,
            error: None,
            created_at: 0,
            completed_at: None,
            scheduled_at: None,
            category: String::new(),
            queue_position: 0,
            priority: 0,
            retry_count: 0,
            max_retries: 0,
            checksum_sha256: None,
            expected_checksum: None,
            source: String::new(),
            etag: None,
            last_modified: None,
            final_url: final_url.map(|s| s.to_string()),
            response_status: None,
            content_type: None,
            accepts_ranges: None,
            headers: HashMap::new(),
            media: None,
            per_task_speed_limit: 0,
            collision_policy: CollisionPolicy::default(),
            completion_action: CompletionAction::default(),
            connection_count: 1,
            active_connections: 0,
            segments: Vec::new(),
            retry_policy_override: None,
            proxy_override: None,
            proxy_auth: None,
        }
    }

    fn make_completed_task(
        id: &str,
        url: &str,
        dest: &str,
        name: &str,
        size: u64,
        sha256: Option<&str>,
    ) -> DownloadTask {
        let mut task = make_task(id, url, None, dest, name, TaskStatus::Completed);
        task.total_bytes = size;
        task.checksum_sha256 = sha256.map(|s| s.to_string());
        task
    }

    #[test]
    fn find_no_matches_for_empty_task_list() {
        let result = find_duplicate_matches(
            &[],
            "https://example.com/file.zip",
            "C:\\Downloads\\file.zip",
            None,
            None,
        );
        assert!(result.is_empty());
    }

    #[test]
    fn find_same_url_match() {
        let task = make_task(
            "t1",
            "https://example.com/file.zip",
            None,
            "C:\\DL",
            "a.zip",
            TaskStatus::Queued,
        );
        let result = find_duplicate_matches(
            &[task],
            "https://example.com/file.zip",
            "C:\\Other\\different.zip",
            None,
            None,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].duplicate_type, DuplicateType::SameUrl);
        assert_eq!(result[0].existing_task_id, "t1");
        assert_eq!(result[0].existing_task_label, "a.zip");
        assert_eq!(result[0].existing_task_status, "queued");
    }

    #[test]
    fn find_same_url_match_with_tracking_params_stripped() {
        // 已有任务带 utm_source，新 URL 不带 → 剥离后应匹配 SameUrl
        let task = make_task(
            "t1",
            "https://example.com/file.zip?utm_source=fb",
            None,
            "C:\\DL",
            "a.zip",
            TaskStatus::Queued,
        );
        let result = find_duplicate_matches(
            &[task],
            "https://example.com/file.zip",
            "C:\\Other\\different.zip",
            None,
            None,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].duplicate_type, DuplicateType::SameUrl);
    }

    #[test]
    fn find_same_final_url_match() {
        let task = make_task(
            "t1",
            "https://shortener.com/abc",
            Some("https://example.com/real.zip"),
            "C:\\DL",
            "real.zip",
            TaskStatus::Queued,
        );
        let result = find_duplicate_matches(
            &[task],
            "https://example.com/real.zip",
            "C:\\Other\\different.zip",
            None,
            None,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].duplicate_type, DuplicateType::SameFinalUrl);
    }

    #[test]
    fn find_same_target_path_match() {
        let task = make_task(
            "t1",
            "https://other.com/x",
            None,
            "C:\\Downloads",
            "video.mp4",
            TaskStatus::Queued,
        );
        let result = find_duplicate_matches(
            &[task],
            "https://different.com/y",
            "C:\\Downloads\\video.mp4",
            None,
            None,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].duplicate_type, DuplicateType::SameTargetPath);
    }

    #[test]
    fn find_same_checksum_match() {
        let task = make_completed_task(
            "t1",
            "https://other.com/x",
            "C:\\DL",
            "old.zip",
            1024,
            Some("abc123def456"),
        );
        let result = find_duplicate_matches(
            &[task],
            "https://new.com/y",
            "C:\\Other\\new.zip",
            Some(1024),
            Some("abc123def456"),
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].duplicate_type, DuplicateType::SameChecksum);
        assert_eq!(result[0].existing_task_status, "completed");
    }

    #[test]
    fn find_checksum_case_insensitive() {
        let task = make_completed_task(
            "t1",
            "https://other.com/x",
            "C:\\DL",
            "old.zip",
            1024,
            Some("ABC123DEF456"),
        );
        let result = find_duplicate_matches(
            &[task],
            "https://new.com/y",
            "C:\\Other\\new.zip",
            Some(1024),
            Some("abc123def456"),
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].duplicate_type, DuplicateType::SameChecksum);
    }

    #[test]
    fn find_checksum_no_match_when_size_differs() {
        let task = make_completed_task(
            "t1",
            "https://other.com/x",
            "C:\\DL",
            "old.zip",
            1024,
            Some("abc123"),
        );
        let result = find_duplicate_matches(
            &[task],
            "https://new.com/y",
            "C:\\Other\\new.zip",
            Some(2048), // 不同大小
            Some("abc123"),
        );
        assert!(result.is_empty());
    }

    #[test]
    fn find_checksum_no_match_when_sha_missing() {
        let task = make_completed_task(
            "t1",
            "https://other.com/x",
            "C:\\DL",
            "old.zip",
            1024,
            None, // 任务没有 sha256
        );
        let result = find_duplicate_matches(
            &[task],
            "https://new.com/y",
            "C:\\Other\\new.zip",
            Some(1024),
            Some("abc123"),
        );
        assert!(result.is_empty());
    }

    #[test]
    fn find_checksum_skipped_when_new_sha_none() {
        let task = make_completed_task(
            "t1",
            "https://other.com/x",
            "C:\\DL",
            "old.zip",
            1024,
            Some("abc123"),
        );
        let result = find_duplicate_matches(
            &[task],
            "https://new.com/y",
            "C:\\Other\\new.zip",
            Some(1024),
            None, // 新任务没有 sha256
        );
        assert!(result.is_empty());
    }

    #[test]
    fn find_skips_cancelled_tasks() {
        let cancelled = make_task(
            "t1",
            "https://example.com/file.zip",
            None,
            "C:\\DL",
            "a.zip",
            TaskStatus::Cancelled,
        );
        let result = find_duplicate_matches(
            &[cancelled],
            "https://example.com/file.zip",
            "C:\\DL\\a.zip",
            None,
            None,
        );
        assert!(result.is_empty());
    }

    #[test]
    fn find_url_takes_precedence_over_final_url() {
        // 任务 URL 和 final_url 都匹配，应优先返回 SameUrl
        let task = make_task(
            "t1",
            "https://example.com/same",
            Some("https://final.example.com/same"),
            "C:\\DL",
            "a.zip",
            TaskStatus::Queued,
        );
        let result = find_duplicate_matches(
            &[task],
            "https://example.com/same",
            "C:\\Other\\different.zip",
            None,
            None,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].duplicate_type, DuplicateType::SameUrl);
    }

    #[test]
    fn find_returns_one_match_per_task_at_most() {
        // 同一任务同时命中 URL 和 target_path，应只返回 SameUrl
        let task = make_task(
            "t1",
            "https://example.com/file.zip",
            None,
            "C:\\Downloads",
            "file.zip",
            TaskStatus::Queued,
        );
        let result = find_duplicate_matches(
            &[task],
            "https://example.com/file.zip",
            "C:\\Downloads\\file.zip",
            None,
            None,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].duplicate_type, DuplicateType::SameUrl);
    }

    #[test]
    fn find_multiple_matches_across_tasks() {
        let task1 = make_task(
            "t1",
            "https://example.com/a.zip",
            None,
            "C:\\DL1",
            "a.zip",
            TaskStatus::Queued,
        );
        let task2 = make_task(
            "t2",
            "https://other.com/b.zip",
            None,
            "C:\\Downloads",
            "shared.zip",
            TaskStatus::Completed,
        );
        let result = find_duplicate_matches(
            &[task1, task2],
            "https://example.com/a.zip",
            "C:\\Downloads\\shared.zip",
            None,
            None,
        );
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].duplicate_type, DuplicateType::SameUrl);
        assert_eq!(result[1].duplicate_type, DuplicateType::SameTargetPath);
    }

    #[test]
    fn find_empty_url_skips_url_comparisons() {
        let task = make_task(
            "t1",
            "https://example.com/file.zip",
            None,
            "C:\\DL",
            "a.zip",
            TaskStatus::Queued,
        );
        let result = find_duplicate_matches(
            &[task],
            "", // 空 URL
            "C:\\DL\\a.zip",
            None,
            None,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].duplicate_type, DuplicateType::SameTargetPath);
    }

    #[test]
    fn find_empty_target_path_skips_path_comparison() {
        let task = make_task(
            "t1",
            "https://example.com/file.zip",
            None,
            "C:\\DL",
            "a.zip",
            TaskStatus::Queued,
        );
        let result = find_duplicate_matches(
            &[task],
            "https://different.com/y",
            "", // 空路径
            None,
            None,
        );
        assert!(result.is_empty());
    }
}
