//! Read-only form/schema diagnostics. No page JavaScript is executed.
use super::*;
pub(super) async fn inspect(sample: &Sample) -> Result<Value, String> {
    let network = cloud_request::CloudRequestContext::defaults(
        lanzou::LANZOU_USER_AGENT,
        20,
        tokio_util::sync::CancellationToken::new(),
    )?;
    let response = network
        .get(&sample.url)
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .send()
        .await
        .map_err(|e| clean(&format!("页面诊断失败：{e}")))?;
    let html = network.text(response).await?;
    if html.contains("var arg1") || html.contains("验证码") {
        return Ok(json!({"page_challenge":true,"form_requests_not_sent":true}));
    }
    let vars = regex::Regex::new(r#"var\s+([a-zA-Z0-9_]+)\s*=\s*['"]([^'"]+)['"]"#)
        .unwrap()
        .captures_iter(&html)
        .map(|c| (c[1].to_owned(), c[2].to_owned()))
        .collect::<HashMap<_, _>>();
    let data_re = regex::Regex::new(r#"data\s*:\s*\{([^}]+)\}"#).unwrap();
    let block = data_re.captures(&html).ok_or("页面没有目录表单")?;
    let pairs = regex::Regex::new(r#"['"]?([a-zA-Z0-9_]+)['"]?\s*:\s*([^,\n\r]+)"#).unwrap();
    let mut params = HashMap::new();
    for cap in pairs.captures_iter(&block[1]) {
        let raw = cap[2].trim().trim_matches('\'').trim_matches('"');
        let mut value = vars.get(raw).cloned().unwrap_or_else(|| raw.to_owned());
        if value == "pgs" || value == "pg" {
            value = "1".into()
        }
        params.insert(cap[1].to_owned(), value);
    }
    if let Some(password) = &sample.password {
        params.insert("pwd".into(), password.clone());
    }
    for (name, value) in [("lx", "1"), ("pg", "1"), ("rep", "0")] {
        params.entry(name.into()).or_insert_with(|| value.into());
    }
    let path_re = regex::Regex::new(r#"url\s*:\s*['"](/filemoreajax\.php[^'"]*)['"]"#).unwrap();
    let path = path_re
        .captures(&html)
        .map(|c| c[1].to_owned())
        .unwrap_or_else(|| "/filemoreajax.php".into());
    let host = url::Url::parse(&sample.url)
        .unwrap()
        .host_str()
        .unwrap()
        .to_owned();
    let api = format!("https://{host}{path}");
    let mut pages = Vec::new();
    for page in 1..=2 {
        params.insert("pg".into(), page.to_string());
        let response = network
            .post(&api)
            .form(&params)
            .header("Referer", &sample.url)
            .header("Accept", "application/json, text/javascript, */*")
            .send()
            .await
            .map_err(|e| clean(&format!("目录诊断失败：{e}")))?;
        let text = network.text(response).await?;
        let value: Value = serde_json::from_str(&text).map_err(|_| "目录诊断非JSON")?;
        let mut message = json!({});
        for key in ["text", "info"] {
            if let Some(text) = value.get(key).and_then(Value::as_str) {
                message[key] = json!(clean(text));
            }
        }
        pages.push(json!({"page":page,"zt":value.get("zt"),"items":value.get("text").and_then(Value::as_array).map(Vec::len),"message":message}));
    }
    let mut keys = params.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    Ok(json!({"page_challenge":false,"param_keys":keys,"pages":pages}))
}
