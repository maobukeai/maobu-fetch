//! 补充结构化回执、代理URL及签名字段的脱敏。
use regex::Regex;
use std::sync::OnceLock;

const KEYS: &str = r"(?:password|proxy_password|pass(?:_code)?(?:_token)?|pwd|(?:access_|refresh_|share_|security_|session_)?token|stoken|authorization|proxy-authorization|cookie|set-cookie|x-api-key|x-luma-token|x-luma-signature|(?:x-amz-|x-goog-)?signature|sign|auth|randsk|seckey|credential|wssecret|pr)";

pub(super) fn structured(text: &str) -> String {
    static JSON: OnceLock<Regex> = OnceLock::new();
    static FORM: OnceLock<Regex> = OnceLock::new();
    static URL: OnceLock<Regex> = OnceLock::new();
    let json = JSON.get_or_init(|| Regex::new(&format!(r#"(?i)(["']{KEYS}["']\s*:\s*)["'](?:\\.|[^"'\\])*["']"#))
        .expect("static structured credential pattern"));
    let form = FORM.get_or_init(|| Regex::new(&format!(r"(?i)\b({KEYS})=([^&;\s,#]*)"))
        .expect("static credential key pattern"));
    let url = URL.get_or_init(|| Regex::new(r"(?i)\b((?:https?|socks5h?)://)[^/@\s]+@")
        .expect("static proxy userinfo pattern"));
    let redacted = json.replace_all(text, "${1}\"***\"");
    let redacted = form.replace_all(&redacted, "${1}=***");
    url.replace_all(&redacted, "${1}***@").into_owned()
}

#[cfg(test)]
mod tests {
    use super::super::redact_sensitive;
    #[test]
    fn structured_auth_and_proxy_userinfo_are_never_left_in_errors() {
        let source = r#"failure {"access_token":"synthetic-access","Authorization":"Bearer synthetic-auth","pass_code_token":"synthetic-share"} proxy=https://tester:synthetic-password@proxy.invalid:8080"#;
        let result = redact_sensitive(source);
        for value in ["synthetic-access","synthetic-auth","synthetic-share","synthetic-password","tester:"] {
            assert!(!result.contains(value));
        }
        assert!(result.contains("failure"));
        assert!(result.contains("proxy.invalid"));
    }
    #[test]
    fn signed_download_parameters_and_bridge_headers_are_masked() {
        let result = redact_sensitive("X-Luma-Token: synthetic-bridge\nhttps://cdn.invalid/file?pr=synthetic-opaque&X-Amz-Signature=synthetic-sign&other=public");
        assert!(!result.contains("synthetic"));
        assert!(result.contains("other=public"));
    }
}
