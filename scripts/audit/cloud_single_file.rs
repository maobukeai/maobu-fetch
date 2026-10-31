//! One designated file: no folder rescan, no request retry, no full executable download.
use super::*;
pub(super) async fn run(sample: &Sample, root: &Path) -> Value {
    let start = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut result = json!({"platform":sample.platform,"sample":sample.url,"source":sample.source,"mode":"single designated file; production resolve once, HEAD and at most 64KiB in-memory Range; no listing/retry/full download","time_unix_seconds":start,"account_cookie":"none","production_resolve":false,"reachability":null});
    if sample.get_only {
        result["mode"]=json!("single designated file; production resolve once, GET Range only; no HEAD/listing/retry/full download");
        result["method"] = json!("GET");
        result["range"] = json!("bytes=0-65535");
        result["route"] = json!("default-existing-env-proxy");
    }
    let outcome=tokio::time::timeout(Duration::from_secs(90),async {
        let direct=lanzou::resolve_lanzou_file(&sample.url,sample.file_id.as_deref().ok_or("指定文件ID缺失")?,sample.password.as_deref()).await?;
        let parsed=url::Url::parse(&direct.url).map_err(|_|"生产解析未返回有效HTTP地址")?;
        if !matches!(parsed.scheme(),"http"|"https")||parsed.host_str().is_none()||!parsed.username().is_empty()||parsed.password().is_some(){return Err("生产取链地址不安全".to_owned())}
        result["production_resolve"]=json!(true);
        tokio::fs::write(root.join("lanzou-private-download.json"),serde_json::to_vec(&json!({"url":direct.url,"headers":direct.headers,"file_name":sample.file_name,"size":null,"expected_bytes":null,"public_share_url":sample.url,"public_source":sample.source,"route":"default-existing-env-proxy","reachability":null})).unwrap()).await.map_err(|_|"写入探测前隔离TEMP参数失败")?;
        let reach=if sample.get_only {probe_get(&direct.url,&direct.headers).await?} else {probe(&direct.url,&direct.headers).await?};
        tokio::fs::write(root.join("lanzou-private-download.json"),serde_json::to_vec(&json!({"url":direct.url,"headers":direct.headers,"file_name":sample.file_name,"size":reach["entity_total"],"expected_bytes":reach["entity_total"],"public_share_url":sample.url,"public_source":sample.source,"reachability":reach})).unwrap()).await.map_err(|_|"写入隔离TEMP参数失败")?;
        result["reachability"]=reach;
        Ok::<_,String>(())
    }).await;
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            result["failure_phase"] = json!(if error.starts_with("GET_") {
                error.split(':').next().unwrap_or("GET")
            } else {
                "production_resolve_or_manifest"
            });
            result["error"] = json!(clean(&error));
        }
        Err(_) => result["error"] = json!("单次指定文件探针90秒超时，已停止，无重试"),
    };
    result["elapsed_seconds"] = json!(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - start
    );
    result
}
async fn probe_get(url: &str, headers: &HashMap<String, String>) -> Result<Value, String> {
    use futures_util::StreamExt;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "GET_BUILD:创建有界请求失败")?;
    let mut request = client.get(url).header("Range", "bytes=0-65535");
    for (key, value) in headers {
        request = request.header(key, value);
    }
    let response = request.send().await.map_err(|e| {
        let timeout = e.is_timeout();
        let connect = e.is_connect();
        let builder = e.is_builder();
        format!(
            "GET_SEND:实体GET发送失败，timeout={timeout}, connect={connect}, builder={builder}：{}",
            e.without_url()
        )
    })?;
    let status = response.status().as_u16();
    if status != 206 {
        return Err(format!(
            "GET_HEADERS:HTTP{status}，已立即drop且不读取body，无重试"
        ));
    }
    let content_range = response
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let total = content_range
        .strip_prefix("bytes 0-65535/")
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|n| *n >= 65536)
        .ok_or("GET_HEADERS:Content-Range边界无效，未读取body")?;
    if response.content_length().is_some_and(|n| n != 65536) {
        return Err("GET_HEADERS:206 Content-Length不是65536，未读取body".into());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("GET_BODY:读取失败：{}", e.without_url()))?;
        if bytes.len() + chunk.len() > 65536 {
            return Err("GET_BODY:响应超过64KiB，已停止".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() != 65536 {
        return Err("GET_BODY:实体前64KiB不完整".into());
    }
    Ok(
        json!({"method":"GET","route":"default-existing-env-proxy","range_status":status,"range_bytes":bytes.len(),"entity_total":total,"range_sha256":hex::encode(Sha256::digest(&bytes)),"full_download":false,"saved_entity_to_disk":false}),
    )
}
async fn probe(url: &str, headers: &HashMap<String, String>) -> Result<Value, String> {
    use futures_util::StreamExt;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "创建有界请求失败")?;
    let mut head = client.head(url);
    let mut range = client.get(url).header("Range", "bytes=0-65535");
    for (key, value) in headers {
        head = head.header(key, value);
        range = range.header(key, value);
    }
    let response = head
        .send()
        .await
        .map_err(|e| format!("实体HEAD失败：{}", e.without_url()))?;
    let status = response.status().as_u16();
    let length = response.content_length();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    drop(response);
    if status != 200 && status != 206 && status != 405 {
        return Err(format!("实体HEAD被拒绝：HTTP{status}"));
    }
    if content_type.contains("text/html") {
        return Err("实体HEAD是HTML，未读取文件".into());
    }
    let response = range
        .send()
        .await
        .map_err(|e| format!("实体Range失败：{}", e.without_url()))?;
    let range_status = response.status().as_u16();
    if range_status != 206 {
        return Err(format!(
            "实体Range返回HTTP{range_status}，未读取body，无重试"
        ));
    }
    let content_range = response
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let total = content_range
        .strip_prefix("bytes 0-65535/")
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|n| *n >= 65536)
        .ok_or("实体Range边界不符，未读取body")?;
    if status == 200 && length.is_some_and(|n| n != total) {
        return Err("HEAD与Range实体总长不一致".into());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Range读取失败：{}", e.without_url()))?;
        if bytes.len() + chunk.len() > 65536 {
            return Err("Range超过64KiB限制，已停止".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() != 65536 {
        return Err("实体Range不完整".into());
    }
    Ok(
        json!({"head_status":status,"head_length":length,"content_type":content_type,"range_status":range_status,"range_bytes":bytes.len(),"entity_total":total,"range_sha256":hex::encode(Sha256::digest(&bytes)),"full_download":false,"saved_entity_to_disk":false}),
    )
}
