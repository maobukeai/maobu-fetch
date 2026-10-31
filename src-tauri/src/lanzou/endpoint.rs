//! Read only the AJAX address declaration; never execute a sharing page's JavaScript.
use regex::Regex;
use std::collections::HashMap;

pub(super) fn ajax_url(
    html: &str,
    variables: &HashMap<String, String>,
    base: &str,
) -> Result<String, String> {
    let address = Regex::new(r#"url\s*:\s*([^,\r\n]+)"#).map_err(|_| "蓝奏云解析规则无效")?;
    let expression = address.captures(html).and_then(|cap| cap.get(1));
    let expression = expression.map(|value| value.as_str().trim());
    let value = match expression {
        None => "/ajaxm.php".to_owned(),
        Some(expression) if expression.starts_with(['\'', '"']) => {
            let quote = expression.as_bytes()[0] as char;
            let end = expression[1..]
                .find(quote)
                .ok_or("蓝奏云直链接口声明不完整，请重新解析分享")?
                + 1;
            let tail = expression[end + 1..].trim();
            if !tail.is_empty() && !tail.starts_with([';', '}', ')']) {
                return Err("蓝奏云直链接口包含未识别的动态地址，请在浏览器中检查分享".into());
            }
            expression[1..end].to_owned()
        }
        Some(name) => variables
            .get(name)
            .cloned()
            .or_else(|| default_ajax_variable(html, name, variables))
            .ok_or("蓝奏云直链接口变量无法静态识别，请重新解析分享")?,
    };
    let base = url::Url::parse(base).map_err(|_| "蓝奏云页面地址无效")?;
    let target = base.join(&value).map_err(|_| "蓝奏云直链接口地址无效")?;
    let official_api = matches!(
        target.host_str(),
        Some("apifile.lanzouw.com" | "apifile.woozooo.com")
    ) && target.scheme() == "https";
    if !matches!(target.scheme(), "https" | "http")
        || !target.username().is_empty()
        || target.password().is_some()
        || value.chars().any(char::is_control)
        || !target.path().starts_with("/ajax")
        || !(target.origin() == base.origin() || official_api)
    {
        return Err("蓝奏云直链接口地址不安全或不属于支持的服务，请在浏览器中检查分享".into());
    }
    Ok(target.to_string())
}

fn default_ajax_variable(
    html: &str,
    name: &str,
    variables: &HashMap<String, String>,
) -> Option<String> {
    // Current official pages use this explicit default when the optional browser DNS helper is absent.
    let pattern = format!(
        r#"if\s*\(\s*typeof\s*\(\s*killdnsweb\s*\)\s*==\s*['"]undefined['"]\s*\)\s*\{{\s*{}\s*=\s*([a-zA-Z_$][a-zA-Z0-9_$]*)\s*;"#,
        regex::escape(name)
    );
    let selected = Regex::new(&pattern).ok()?.captures(html)?;
    variables.get(selected.get(1)?.as_str()).cloned()
}

#[cfg(test)]
#[path = "endpoint_tests.rs"]
mod tests;
