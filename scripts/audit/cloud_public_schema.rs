//! Sanitized shape diagnostics only: never serialize raw URLs, token values or API bodies.
use super::*;

pub(super) async fn pikpak(
    sample: &Candidate,
    share: &Value,
    device: &str,
    root: &Path,
) -> Result<Value, String> {
    let network = cloud_request::CloudRequestContext::defaults(
        pikpak::PIKPAK_USER_AGENT,
        20,
        tokio_util::sync::CancellationToken::new(),
    )?;
    let (captcha, signature) = pikpak::get_captcha_and_sign_with_context(device, &network).await?;
    let mut query = vec![
        ("share_id", string(share, "share_id").unwrap_or("")),
        ("file_id", sample.id.as_str()),
    ];
    if let Some(token) = string(share, "pass_token").filter(|s| !s.is_empty()) {
        query.push(("pass_code_token", token));
    }
    let mut request = network
        .get(format!(
            "{}/drive/v1/share/file_info",
            pikpak::PIKPAK_API_HOST
        ))
        .query(&query)
        .header("X-Client-Id", pikpak::PIKPAK_CLIENT_ID)
        .header("X-Client-Version", pikpak::PIKPAK_CLIENT_VERSION)
        .header("X-Device-Id", device)
        .header("X-Device-Sign", signature)
        .header("Referer", "https://mypikpak.com/");
    if !captcha.is_empty() {
        request = request.header("X-Captcha-Token", captcha);
    }
    let response = request
        .send()
        .await
        .map_err(|e| clean(&format!("诊断请求失败：{e}")))?;
    let status = response.status().as_u16();
    let text = network.text(response).await?;
    let json: Value = serde_json::from_str(&text).map_err(|_| "诊断响应不是JSON")?;
    let keys = |value: &Value| {
        value
            .as_object()
            .map(|map| map.keys().take(60).cloned().collect::<Vec<_>>())
    };
    let mut business = serde_json::Map::new();
    for key in [
        "code",
        "error_code",
        "message",
        "error",
        "error_description",
        "status",
        "share_status",
        "share_status_text",
    ] {
        if let Some(value) = json.get(key) {
            if value.is_number() || value.is_boolean() {
                business.insert(key.into(), value.clone());
            } else if let Some(value) = value.as_str() {
                business.insert(key.into(), json!(clean(value)));
            }
        }
    }
    let mut candidates = Vec::new();
    scan(&json, "", 0, &mut candidates);
    let apps=json["file_info"]["apps"].as_array().map(|apps|apps.iter().take(8).map(|app|json!({"id":app.get("id").and_then(Value::as_str).map(clean),"name":app.get("name").and_then(Value::as_str).map(clean),"keys":keys(app)})).collect::<Vec<_>>());
    let mut result = json!({"http_status":status,"top_keys":keys(&json),"file_info_keys":keys(&json["file_info"]),"file_keys":keys(&json["file"]),"data_keys":keys(&json["data"]),"data_file_info_keys":keys(&json["data"]["file_info"]),"business":business,"url_candidates":candidates,"medias_count":json["file_info"]["medias"].as_array().map(Vec::len),"links_keys":keys(&json["file_info"]["links"]),"apps":apps});
    if let Some(link) = json["file_info"]["apps"]
        .get(0)
        .and_then(|app| app.get("link"))
        .and_then(Value::as_str)
        .filter(|link| link.starts_with("https://"))
    {
        let headers = HashMap::from([
            (
                "User-Agent".to_owned(),
                pikpak::PIKPAK_USER_AGENT.to_owned(),
            ),
            ("Referer".to_owned(), "https://mypikpak.com/".to_owned()),
        ]);
        match super::reachability(link, &headers, sample.size).await {
            Ok(value) => {
                result["app_candidate_reachability"] = value;
                tokio::fs::write(root.join("pikpak-app-candidate-private-download.json"),serde_json::to_vec(&json!({"url":link,"headers":headers,"file_name":sample.name,"size":sample.size,"expected_bytes":sample.size,"public_share_url":format!("https://mypikpak.com/s/{}",string(share,"share_id").unwrap_or("")),"diagnostic_only_not_production_selection":true})).unwrap()).await.map_err(|_|"写入有界候选下载参数失败")?;
            }
            Err(error) => result["app_candidate_reachability"] = json!({"error":clean(&error)}),
        }
    }
    Ok(result)
}
fn scan(value: &Value, path: &str, depth: usize, found: &mut Vec<Value>) {
    if depth > 7 || found.len() > 80 {
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, value) in map.iter().take(60) {
                if key.to_ascii_lowercase().contains("token")
                    || key.to_ascii_lowercase().contains("sign")
                {
                    continue;
                }
                let next = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                if let Some(text) = value.as_str() {
                    let is_link = key.contains("url") || key.contains("link");
                    if is_link || text.starts_with("http:") || text.starts_with("https:") {
                        let parsed = url::Url::parse(text.trim()).ok();
                        found.push(json!({"path":next,"characters":text.chars().count(),"empty":text.trim().is_empty(),"http_url":parsed.as_ref().is_some_and(|u|matches!(u.scheme(),"http"|"https")&&u.host_str().is_some()),"embedded_credentials":parsed.as_ref().is_some_and(|u|!u.username().is_empty()||u.password().is_some())}));
                    }
                } else {
                    scan(value, &next, depth + 1, found);
                }
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().take(20).enumerate() {
                scan(value, &format!("{path}[{index}]"), depth + 1, found);
            }
        }
        _ => {}
    }
}
