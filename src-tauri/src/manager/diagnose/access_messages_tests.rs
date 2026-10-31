use super::*;

fn rendered(platform: MediaPlatform, evidence: &str) -> String {
    let category = classify_platform_error(platform, evidence);
    platform_error_to_chinese_with_evidence(category, platform, evidence)
}

#[test]
fn fresh_cookies_do_not_imply_an_expired_account_session() {
    for (platform, error) in [
        (
            MediaPlatform::Douyin,
            "Fresh cookies (not necessarily logged in) are needed",
        ),
        (
            MediaPlatform::Unknown,
            "ERROR: [Ixigua] Cookies (not necessarily logged in) are needed",
        ),
    ] {
        let message = rendered(platform, error);
        assert!(message.contains("不一定需要登录"));
        assert!(!message.contains("已失效"));
        assert!(!message.contains("过期"));
        assert!(!message.contains("明确提示此内容要求登录"));
    }
}

#[test]
fn explicit_account_requirement_is_distinct_from_fresh_cookie_requirement() {
    let message = rendered(
        MediaPlatform::Twitter,
        "Login required to view this content",
    );
    assert!(message.contains("明确提示此内容要求登录或相应账号权限"));
    assert!(!message.contains("不一定需要登录"));
    assert!(!message.contains("已失效"));
}

#[test]
fn plain_forbidden_response_has_no_unobserved_login_or_po_token_claim() {
    for platform in [
        MediaPlatform::YouTube,
        MediaPlatform::Douyin,
        MediaPlatform::Unknown,
    ] {
        let message = rendered(platform, "HTTP Error 403: Forbidden");
        assert!(message.contains("认证或访问校验未通过"));
        for unsupported in [
            "已失效",
            "过期",
            "PO Token",
            "不是软件",
            "robots.txt",
            "轮换",
            "12 小时",
        ] {
            assert!(!message.contains(unsupported), "{message}");
        }
    }
}

#[test]
fn youtube_bot_prompt_does_not_claim_a_specific_cause_or_recovery_time() {
    let message = rendered(MediaPlatform::YouTube,
        "Sign in to confirm you're not a bot. Use --cookies-from-browser or --cookies for the authentication.");
    assert!(message.contains("进一步的访问校验"));
    assert!(message.contains("自己的合法凭据"));
    assert!(message.contains("当前媒体工具版本"));
    for unsupported in [
        "PO Token",
        "不是软件",
        "robots.txt",
        "轮换",
        "12 小时",
        "自动恢复",
    ] {
        assert!(!message.contains(unsupported), "{message}");
    }
}

#[test]
fn drm_refusal_remains_higher_priority_than_cookie_hints() {
    assert_eq!(
        rendered(
            MediaPlatform::YouTube,
            "DRM protected; fresh cookies needed"
        ),
        "该内容受 DRM 保护，无法下载"
    );
}

#[test]
fn http_auth_diagnosis_is_neutral_and_keeps_redaction() {
    let context =
        ErrorContext::from_task_fields("https://example.test/file".into(), None, false, None);
    let diagnosis = classify_error(
        "HTTP 403 https://example.test/file?token=synthetic-secret",
        Some(403),
        &context,
    );
    assert_eq!(diagnosis.category, ErrorCategory::AuthExpired);
    assert!(diagnosis.title.contains("认证或访问校验未通过"));
    assert!(!diagnosis.title.contains("过期"));
    assert!(!diagnosis.description.contains("登录已失效"));
    assert!(!diagnosis.raw_error_redacted.contains("synthetic-secret"));
}

#[test]
fn legacy_error_identifiers_remain_wire_compatible() {
    assert_eq!(
        serde_json::to_value(MediaPlatformError::LoginExpired).unwrap(),
        "login-expired"
    );
    assert_eq!(
        serde_json::to_value(ErrorCategory::AuthExpired).unwrap(),
        "auth-expired"
    );
}
