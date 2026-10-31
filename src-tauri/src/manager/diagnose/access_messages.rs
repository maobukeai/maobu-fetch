use crate::media_platforms::MediaPlatform;

/// Only describe evidence in the tool error; the legacy category does not prove an expired login.
pub(super) fn message(platform: MediaPlatform, evidence: Option<&str>) -> String {
    let prefix = if platform == MediaPlatform::Unknown {
        String::new()
    } else {
        format!("{}：", platform.display_name())
    };
    let lower = evidence.unwrap_or_default().to_ascii_lowercase();
    let detail = if lower.contains("fresh cookies")
        || lower.contains("cookies (not necessarily logged in) are needed")
    {
        "媒体工具提示需要较新的站点 Cookie（不一定需要登录）。"
    } else if lower.contains("not a bot")
        || lower.contains("bot verification")
        || lower.contains("captcha")
    {
        "站点要求进一步的访问校验，具体条件尚未确认。"
    } else if [
        "login required",
        "login is required",
        "requires login",
        "please log in",
        "must log in",
        "log in to",
        "sign in to view",
        "sign in to confirm your age",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
    {
        "媒体工具明确提示此内容要求登录或相应账号权限。"
    } else {
        "本次请求未通过，可能涉及站点访问验证或凭据条件。"
    };
    format!("{prefix}认证或访问校验未通过。{detail}请确认浏览器中有权访问，检查当前媒体工具版本与网络设置；需要 Cookie 时，请在「设置 → 媒体凭证」配置自己的合法凭据。")
}
