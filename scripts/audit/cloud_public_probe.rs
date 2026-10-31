//! Read-only public-share audit. Imports current production request/list/resolve implementations.
//! No app setup, database, user cookies, account login, save/transfer, or executable content execution.
#[path = "../../src-tauri/src/baidupan.rs"]
mod baidupan;
#[path = "../../src-tauri/src/cloud_pagination.rs"]
mod cloud_pagination;
#[path = "../../src-tauri/src/cloud_request.rs"]
mod cloud_request;
#[path = "../../src-tauri/src/lanzou.rs"]
mod lanzou;
#[path = "../../src-tauri/src/media_cookies/scope.rs"]
mod media_cookies;
#[path = "../../src-tauri/src/pan123.rs"]
mod pan123;
#[path = "../../src-tauri/src/pikpak.rs"]
mod pikpak;
#[path = "../../src-tauri/src/quark.rs"]
mod quark;
// Only the application's presentation redactor is replaced; no network/parser behavior is mocked.
mod manager {
    pub fn redact_sensitive(error: &str) -> String {
        super::clean(error)
    }
}
#[path = "cloud_lanzou_schema.rs"]
mod lanzou_schema;
#[path = "cloud_public_schema.rs"]
mod schema;
#[path = "cloud_single_file.rs"]
mod single_file;

use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_DOWNLOAD: u64 = 8 * 1024 * 1024;
#[derive(Deserialize)]
struct Sample {
    platform: String,
    url: String,
    source: String,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    file_id: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    get_only: bool,
}
struct Candidate {
    id: String,
    name: String,
    size: u64,
    metadata: Value,
}
fn clean(error: &str) -> String {
    let urls = regex::Regex::new(r"https?://[^\s]+").unwrap();
    // Production errors can include upstream messages. Never persist bodies or temporary links.
    let without = urls.replace_all(error, "[URL]");
    if without.contains("回执异常") || without.contains("接口返回异常") {
        return "上游接口异常（正文省略）".into();
    }
    without.chars().take(240).collect()
}
fn summary(files: &[Candidate], password: bool) -> Value {
    json!({"listed_items":files.len(),"password_required":password,"smallest_files":files.iter().filter(|f|f.size>0).take(8).map(|f|json!({"name":f.name,"size":f.size})).collect::<Vec<_>>()})
}
fn candidate<T: serde::Serialize>(id: String, name: String, size: u64, metadata: &T) -> Candidate {
    Candidate {
        id,
        name,
        size,
        metadata: serde_json::to_value(metadata).unwrap(),
    }
}
async fn inspect(sample: &Sample, device: &str) -> Result<(Vec<Candidate>, bool, Value), String> {
    let password = sample.password.as_deref();
    match sample.platform.as_str() {
        "pikpak" => {
            let v =
                pikpak::inspect_pikpak_share(&sample.url, sample.password.clone(), device).await?;
            let files = v
                .files
                .iter()
                .filter(|f| !f.kind.contains("folder"))
                .map(|f| candidate(f.id.clone(), f.name.clone(), f.size, f))
                .collect();
            Ok((
                files,
                v.pass_code_required,
                json!({"share_id":v.share_id,"pass_token":v.pass_code_token}),
            ))
        }
        "quark" => {
            let v = quark::inspect_quark_share(&sample.url, sample.password.clone(), None).await?;
            let files = v
                .files
                .iter()
                .filter(|f| !f.kind.contains("folder"))
                .map(|f| candidate(f.id.clone(), f.name.clone(), f.size, f))
                .collect();
            Ok((
                files,
                v.pass_code_required,
                json!({"share_id":v.pwd_id,"stoken":v.stoken}),
            ))
        }
        "baidu" => {
            let v = baidupan::inspect_baidu_share(&sample.url, password, None).await?;
            let files = v
                .files
                .iter()
                .filter(|f| !f.kind.contains("folder"))
                .map(|f| candidate(f.id.clone(), f.name.clone(), f.size, f))
                .collect();
            let details = serde_json::to_value(&v).unwrap();
            Ok((files, v.pass_code_required, details))
        }
        "123" => {
            let v = pan123::inspect_pan123_share(&sample.url, password).await?;
            let files = v
                .files
                .iter()
                .filter(|f| f.kind == "file")
                .map(|f| candidate(f.id.to_string(), f.name.clone(), f.size, f))
                .collect();
            Ok((files, v.requires_password, json!({"share_id":v.share_key})))
        }
        "lanzou" => {
            let v = lanzou::inspect_lanzou_share(&sample.url, password).await?;
            let files = v
                .files
                .iter()
                .filter(|f| f.kind == "file")
                .map(|f| candidate(f.id.clone(), f.name.clone(), f.size, f))
                .collect();
            Ok((files, v.requires_password, json!({})))
        }
        _ => Err("未知平台".into()),
    }
}
fn string<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}
async fn resolve(
    sample: &Sample,
    file: &Candidate,
    share: &Value,
    device: &str,
) -> Result<(String, HashMap<String, String>), String> {
    let password = sample.password.as_deref();
    let share_id = string(share, "share_id").unwrap_or("");
    match sample.platform.as_str() {
        "pikpak" => {
            let v = pikpak::resolve_pikpak_file(
                share_id,
                &file.id,
                string(share, "pass_token"),
                device,
            )
            .await?;
            Ok((v.url, v.headers))
        }
        "quark" => {
            let v = quark::resolve_quark_file(
                share_id,
                &file.id,
                string(&file.metadata, "shareFidToken"),
                string(share, "stoken"),
                None,
            )
            .await?;
            Ok((v.url, v.headers))
        }
        "baidu" => {
            let v = baidupan::resolve_baidu_file(
                string(share, "surl").unwrap_or(""),
                &file.id,
                string(share, "shareId"),
                string(share, "uk"),
                string(share, "sign"),
                share.get("timestamp").and_then(Value::as_i64),
                string(share, "seckey"),
                string(share, "randsk"),
                None,
            )
            .await?;
            Ok((v.url, v.headers))
        }
        "123" => {
            let v = pan123::resolve_pan123_file(
                share_id,
                file.id.parse().map_err(|_| "文件ID无效")?,
                string(&file.metadata, "s3_key_flag").unwrap_or(""),
                file.size,
                string(&file.metadata, "etag").unwrap_or(""),
                password,
                None,
            )
            .await?;
            Ok((v.url, v.headers))
        }
        "lanzou" => {
            let v = lanzou::resolve_lanzou_file(&sample.url, &file.id, password).await?;
            Ok((v.url, v.headers))
        }
        _ => Err("未知平台".into()),
    }
}
async fn download(
    url: &str,
    headers: &HashMap<String, String>,
    size: u64,
    path: &Path,
) -> Result<Value, String> {
    use futures_util::StreamExt;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|_| "创建检查客户端失败")?;
    let mut request = client.get(url);
    for (key, value) in headers {
        request = request.header(key, value);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("实体请求失败：{}", e.without_url()))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(format!("实体HTTP状态{status}"));
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|x| x.to_str().ok())
        .unwrap_or("")
        .to_owned();
    if content_type.contains("text/html") {
        return Err("实体返回HTML，拒绝当作文件成功".into());
    }
    if response.content_length().is_some_and(|n| n > MAX_DOWNLOAD) {
        return Err("实体超过8MiB保护上限".into());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("实体读取失败：{}", e.without_url()))?;
        if bytes.len() as u64 + chunk.len() as u64 > MAX_DOWNLOAD {
            return Err("实体超过8MiB保护上限".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() as u64 != size {
        return Err(format!("列表/实体长度不一致：{size}/{}", bytes.len()));
    }
    if bytes.is_empty() {
        return Err("实体为空".into());
    }
    let hash = hex::encode(Sha256::digest(&bytes));
    tokio::fs::write(path, &bytes)
        .await
        .map_err(|_| "写入隔离TEMP实体失败")?;
    Ok(
        json!({"http_status":status,"content_type":content_type,"bytes":bytes.len(),"sha256":hash,"saved_in_temp":true,"mode":"independent HTTP integrity probe; not DownloadManager E2E"}),
    )
}
async fn run(sample: &Sample, root: &Path) -> Value {
    if sample.file_id.is_some() {
        return single_file::run(sample, root).await;
    }
    let device = hex::encode(rand::random::<[u8; 16]>());
    let start = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut result = json!({"platform":sample.platform,"sample":sample.url.split('?').next().unwrap_or(&sample.url),"source":sample.source,"public_password_supplied":sample.password.is_some(),"account_cookie":"none","time_unix_seconds":start,"production_list":false,"production_resolve":false,"download":null});
    let outcome=tokio::time::timeout(Duration::from_secs(90),async {
  let inspected=inspect(sample,&device).await;
  if sample.platform=="lanzou" {result["sanitized_api_schema"]=match lanzou_schema::inspect(sample).await {Ok(value)=>value,Err(error)=>json!({"diagnostic_error":clean(&error)})};}
  let (mut files,requires,share)=inspected?;
  files.sort_by_key(|f|f.size);result["production_list"]=json!(true);result["listing"]=summary(&files,requires);
  let Some(file)=files.iter().find(|f|f.size>0) else{return Ok::<_,String>(())};
  result["selected"]=json!({"name":file.name,"size":file.size});
  let resolved=resolve(sample,file,&share,&device).await;
  if sample.platform=="pikpak" {result["sanitized_api_schema"]=match schema::pikpak(file,&share,&device,root).await {Ok(value)=>value,Err(error)=>json!({"diagnostic_error":clean(&error)})};}
  let (url,headers)=resolved?;
  result["production_resolve_returned_ok"]=json!(true);
  let parsed=url::Url::parse(&url).map_err(|_|"生产取链接口返回Ok但下载地址为空或无效，不能记为有效直链")?;
  if !matches!(parsed.scheme(),"https"|"http") || parsed.host_str().is_none() {return Err("生产取链接口没有返回有效HTTP地址".into())}
  result["production_resolve"]=json!(true);
  // Private temp manifest lets the root reuse the real manager without publishing signed URLs.
  let manifest=root.join(format!("{}-private-download.json",sample.platform));
  tokio::fs::write(manifest,serde_json::to_vec(&json!({"url":url,"headers":headers,"file_name":file.name,"expected_bytes":file.size,"size":file.size,"public_share_url":sample.url})).unwrap()).await.map_err(|_|"写入隔离下载参数失败")?;
  if file.size>MAX_DOWNLOAD {result["reachability"]=reachability(&url,&headers,file.size).await?;result["download"]=json!({"skipped":"full entity delegated to real DownloadManager; bounded reachability only"});return Ok(())}
  result["download"]=download(&url,&headers,file.size,&root.join(format!("{}-payload.bin",sample.platform))).await?;
  Ok(())
 }).await;
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(error)) => result["error"] = json!(clean(&error)),
        Err(_) => result["error"] = json!("独立probe总时限90秒，已停止等待"),
    }
    result["elapsed_seconds"] = json!(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - start
    );
    result
}
async fn reachability(
    url: &str,
    headers: &HashMap<String, String>,
    size: u64,
) -> Result<Value, String> {
    use futures_util::StreamExt;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "创建有界实体检查客户端失败")?;
    let mut head = client.head(url);
    let mut range = client.get(url).header("Range", "bytes=0-65535");
    for (key, value) in headers {
        head = head.header(key, value);
        range = range.header(key, value);
    }
    let head = head
        .send()
        .await
        .map_err(|e| format!("HEAD检查失败：{}", e.without_url()))?;
    let head_status = head.status().as_u16();
    let head_length = head.content_length();
    drop(head);
    let response = range
        .send()
        .await
        .map_err(|e| format!("Range检查失败：{}", e.without_url()))?;
    let status = response.status().as_u16();
    if status != 206 {
        return Err(format!("有界实体Range HTTP状态{status}；未读取大文件body"));
    }
    let content_range = response
        .headers()
        .get("content-range")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_owned();
    if content_range != format!("bytes 0-65535/{size}") {
        return Err("有界Range Content-Range与列表大小不一致".into());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Range读取失败：{}", e.without_url()))?;
        if bytes.len() + chunk.len() > 65536 {
            return Err("有界Range返回过量字节".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() != 65536 {
        return Err("有界Range不完整".into());
    }
    Ok(
        json!({"head_status":head_status,"head_length":head_length,"range_status":status,"range_bytes":bytes.len(),"range_sha256":hex::encode(Sha256::digest(&bytes)),"full_download":false}),
    )
}
#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args().collect();
    let input = std::fs::read(&args[1]).unwrap();
    let samples: Vec<Sample> = serde_json::from_slice(&input).unwrap();
    let root = Path::new(&args[2]);
    std::fs::create_dir_all(root).unwrap();
    let results =
        futures_util::future::join_all(samples.iter().map(|sample| run(sample, root))).await;
    for result in &results {
        println!("{}", result);
    }
    std::fs::write(
        root.join("results.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
}
