//! Bounded anonymous CDN diagnostics, importing the production native fetch and converter.
//! Signed URLs and metadata remain only under the explicitly supplied TEMP directory.
#[path = "../../src-tauri/src/media_platforms.rs"]
mod media_platforms;
#[path = "../../src-tauri/src/media_cookies/scope.rs"]
mod media_cookies;

// Unused presentation-only types required to compile the production platform module.
// No fetch, conversion, URL selection, cookie scope or transport implementation is replaced.
mod manager {
    pub mod diagnose {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum MediaPlatformError {
            LoginExpired, RegionBlocked, LinkExpired, DrmProtected, Unsupported, Unknown,
        }
    }
    pub mod duplicate {
        pub fn strip_tracking_params(url: &str) -> String { url.to_owned() }
    }
}
mod models {
    pub struct MediaCredentialCheckResult {
        pub domain: String, pub valid: bool, pub message: String, pub tested_at: String,
    }
}
fn now_iso8601_utc() -> String { String::new() } // Unused credential-presentation path.

use futures_util::{stream, StreamExt};
use reqwest::{Client, Method};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, path::{Path, PathBuf}, time::{Duration, Instant}};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const LIMIT: usize = 65_536;

#[derive(Clone)]
struct Case {
    candidate: usize,
    url: String,
    method: Method,
    mode: &'static str,
    browser_ua: bool,
    referer: Option<String>,
}
fn error_kind(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() { "timeout" } else if error.is_connect() { "connect" }
    else if error.is_body() { "body" } else { "request" }
}
fn clean(error: &str) -> String {
    let urls = regex::Regex::new(r"https?://[^\s]+" );
    match urls {
        Ok(pattern) => pattern.replace_all(error, "[URL]").chars().take(200).collect(),
        Err(_) => "native metadata request failed".into(),
    }
}
fn hash(bytes: &[u8]) -> String { hex::encode(Sha256::digest(bytes)) }
fn header(response: &reqwest::Response, name: &str) -> Option<String> {
    response.headers().get(name).and_then(|value| value.to_str().ok()).map(|s|s.chars().take(120).collect())
}
fn hint(bytes: &[u8]) -> &'static str {
    if bytes.get(4..8) == Some(b"ftyp") { "mp4-ftyp-prefix-only" }
    else if bytes.starts_with(b"FLV") { "flv-prefix-only" }
    else if bytes.first() == Some(&0x47) && bytes.get(188) == Some(&0x47) { "ts-prefix-only" }
    else { "unconfirmed-prefix" }
}
async fn prefix(response: &mut reqwest::Response) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    while bytes.len() < LIMIT {
        let Some(chunk) = response.chunk().await.map_err(|error|error_kind(&error).to_owned())? else { break };
        let length = (LIMIT - bytes.len()).min(chunk.len());
        bytes.extend_from_slice(&chunk[..length]);
    }
    Ok(bytes)
}
async fn request(client: &Client, case: Case) -> Value {
    let start = Instant::now();
    let mut result = json!({"candidate":case.candidate,"url_sha256":hash(case.url.as_bytes()),
        "method":case.method.as_str(),"headers_mode":case.mode,"browser_ua":case.browser_ua,
        "referer_present":case.referer.is_some(),"requested_range":if case.method==Method::GET {Some("bytes=0-65535")}else{None},"hops":[]});
    let mut current = case.url.clone();
    let mut visited = HashSet::new();
    let mut hops = Vec::new();
    for _ in 0..6 {
        if !visited.insert(current.clone()) { result["error"] = json!("redirect-loop"); break; }
        let parsed = match url::Url::parse(&current) {
            Ok(url) if matches!(url.scheme(), "https" | "http") => url,
            _ => { result["error"] = json!("invalid-redirect-protocol"); break; }
        };
        let mut builder = client.request(case.method.clone(), parsed.clone());
        if case.browser_ua { builder=builder.header("User-Agent",UA); }
        if let Some(referer)=&case.referer { builder=builder.header("Referer",referer); }
        if case.method==Method::GET { builder=builder.header("Range","bytes=0-65535"); }
        builder=builder.header("Accept-Encoding","identity");
        let mut response=match builder.send().await {
            Ok(response)=>response,
            Err(error)=>{result["error"]=json!(error_kind(&error));break;}
        };
        hops.push(json!({"host":parsed.host_str(),"scheme":parsed.scheme(),"status":response.status().as_u16()}));
        if response.status().is_redirection() {
            let next=response.headers().get("Location").and_then(|value|value.to_str().ok()).and_then(|location|parsed.join(location).ok());
            match next { Some(next)=>{current=next.to_string();continue;},None=>{result["error"]=json!("invalid-location");break;} }
        }
        result["status"]=json!(response.status().as_u16());
        result["content_type"]=json!(header(&response,"Content-Type"));
        result["content_length"]=json!(header(&response,"Content-Length"));
        result["content_range"]=json!(header(&response,"Content-Range"));
        if case.method==Method::GET {
            match prefix(&mut response).await {
                Ok(bytes)=>{result["read_bytes"]=json!(bytes.len());result["prefix_sha256"]=json!(hash(&bytes));result["container_hint"]=json!(hint(&bytes));},
                Err(error)=>result["body_error"]=json!(error),
            }
        }
        break;
    }
    if !result.get("status").is_some() && result.get("error").is_none() { result["error"]=json!("redirect-limit"); }
    result["hops"]=json!(hops);
    result["elapsed_ms"]=json!(start.elapsed().as_millis());
    result
}
fn cases(id: &str, urls: &[String]) -> Vec<Case> {
    let mut cases=Vec::new();
    for (candidate,url) in urls.iter().take(3).enumerate() {
        let variants=if candidate==0 {vec![
            ("default-no-referer",false,None),
            ("browser-no-referer",true,None),
            ("browser-site-referer",true,Some("https://www.douyin.com/".to_owned())),
            ("browser-video-referer",true,Some(format!("https://www.douyin.com/video/{id}"))),
        ]} else {vec![("browser-site-referer",true,Some("https://www.douyin.com/".to_owned()))]};
        for (mode,browser_ua,referer) in variants {
            for method in [Method::HEAD,Method::GET] {
                cases.push(Case{candidate,url:url.clone(),method,mode,browser_ua,referer:referer.clone()});
            }
        }
    }
    cases
}
async fn sample(id: &str, root: &Path, metadata_client: &Client, cdn_client: &Client) -> Value {
    let fetched=tokio::time::timeout(Duration::from_secs(35),media_platforms::fetch_douyin_aweme_detail_with_client(id,None,None,Some(UA),metadata_client)).await;
    let detail=match fetched {
        Ok(Ok(detail))=>detail,
        Ok(Err(error))=>return json!({"id":id,"stage":"native-fetch","error":clean(&error)}),
        Err(_)=>return json!({"id":id,"stage":"native-fetch","error":"35s timeout"}),
    };
    let converted=media_platforms::convert_douyin_aweme_to_yt_dlp_json(&detail);
    let selected=converted.pointer("/formats/0/url").and_then(Value::as_str);
    let mut urls=Vec::new();
    if let Some(url)=selected { urls.push(url.to_owned()); }
    if let Some(all)=detail.pointer("/aweme_detail/video/play_addr/url_list").and_then(Value::as_array) {
        for url in all.iter().filter_map(Value::as_str) {
            if !urls.iter().any(|existing|existing==url) { urls.push(url.to_owned()); }
        }
    }
    if let Err(error)=std::fs::write(root.join(format!("{id}-private.json")),json!({"detail":detail,"converted":converted,"candidate_urls":urls}).to_string()) {
        return json!({"id":id,"stage":"private-evidence-write","error":error.kind().to_string()});
    }
    let tests=stream::iter(cases(id,&urls)).map(|case|request(cdn_client,case)).buffer_unordered(2).collect::<Vec<_>>().await;
    json!({"id":id,"stage":"cdn-matrix","metadata_duration":converted.get("duration"),"format_count":converted["formats"].as_array().map(Vec::len),"candidate_count":urls.len(),"tests":tests})
}
#[tokio::main]
async fn main() {
    let Some(output)=std::env::args_os().nth(1).map(PathBuf::from) else {eprintln!("TEMP output argument required");std::process::exit(1)};
    let temporary=std::env::temp_dir();
    if !output.starts_with(&temporary) || output.exists() {eprintln!("Use a new TEMP output directory");std::process::exit(1)};
    if std::fs::create_dir_all(&output).is_err() {eprintln!("Create TEMP evidence failed");std::process::exit(1)};
    let metadata_client=Client::builder().timeout(Duration::from_secs(10)).connect_timeout(Duration::from_secs(5)).redirect(reqwest::redirect::Policy::limited(5)).build();
    let cdn_client=Client::builder().timeout(Duration::from_secs(8)).connect_timeout(Duration::from_secs(5)).redirect(reqwest::redirect::Policy::none()).no_gzip().no_brotli().no_deflate().build();
    let (Ok(metadata_client),Ok(cdn_client))=(metadata_client,cdn_client) else {eprintln!("Create unchanged-route clients failed");std::process::exit(1)};
    let operation=async {
        stream::iter(["7694589931460594953","7408129200156101907"])
            .map(|id|sample(id,&output,&metadata_client,&cdn_client)).buffer_unordered(2).collect::<Vec<_>>().await
    };
    let results=match tokio::time::timeout(Duration::from_secs(180),operation).await {
        Ok(results)=>results,
        Err(_)=>vec![json!({"stage":"overall","error":"180s diagnostic deadline"})],
    };
    let report=json!({"kind":"ANONYMOUS_DOUYIN_CDN_DIAGNOSTIC_ONLY","personal_cookies":false,"route":"existing default environment; unchanged","body_prefix_limit":LIMIT,"max_cases":24,"max_redirects":5,"results":results,"not_complete_download":true});
    if std::fs::write(output.join("sanitized-results.json"),report.to_string()).is_err() {eprintln!("Write sanitized evidence failed");std::process::exit(1)};
    println!("{}",report);
}
