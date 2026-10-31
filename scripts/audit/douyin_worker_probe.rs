//! Same native candidates, actual worker redirect/header adapter, explicit existing proxy vs direct.
#[path = "../../src-tauri/src/media_cookies/scope.rs"]
mod media_cookies;
mod manager {
    use std::collections::{HashMap, HashSet};
    use tokio_util::sync::CancellationToken;
    use url::Url;
    #[path = "../../src-tauri/src/manager/request_headers.rs"]
    mod request_headers;
    #[path = "../../src-tauri/src/manager/http_redirect.rs"]
    mod http_redirect;
    pub async fn send(client: &reqwest::Client, request: reqwest::RequestBuilder, source: &str)
        -> Result<reqwest::Response, String> {
        let headers = HashMap::from([
            ("User-Agent".into(), super::UA.into()),
            ("Referer".into(), "https://www.douyin.com/".into()),
        ]);
        http_redirect::send(client, request, &headers, source, &CancellationToken::new()).await
    }
}
use futures_util::{stream, StreamExt};
use reqwest::{Client, Method};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, time::{Duration, Instant}};
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const LIMIT: usize = 65_536;

async fn check(client: Client, route: &'static str, id: String, url: String, range: Option<&'static str>) -> Value {
    let begin=Instant::now();
    let method=if range.is_some(){Method::GET}else{Method::HEAD};
    let mut request=client.request(method.clone(),&url).header("Accept-Encoding","identity");
    if let Some(range)=range{request=request.header("Range",range);}
    let mut result=json!({"id":id,"configured_route":route,"method":method.as_str(),"range":range,
        "adapter":"production http_redirect::send/request_headers","worker_headers":"injected Browser UA + Douyin root Referer; no Cookie",
        "source_url_sha256":hex::encode(Sha256::digest(url.as_bytes()))});
    match manager::send(&client,request,&format!("https://www.douyin.com/video/{id}")).await {
        Ok(mut response)=>{
            result["status"]=json!(response.status().as_u16());
            result["final_host"]=json!(response.url().host_str());
            for name in ["Content-Type","Content-Length","Content-Range"] {
                result[name]=json!(response.headers().get(name).and_then(|value|value.to_str().ok()));
            }
            if method==Method::GET {
                let mut bytes=Vec::new();
                while bytes.len()<LIMIT {
                    match response.chunk().await {
                        Ok(Some(chunk))=>{let length=(LIMIT-bytes.len()).min(chunk.len());bytes.extend_from_slice(&chunk[..length]);},
                        Ok(None)=>break,
                        Err(error)=>{result["body_error"]=json!(if error.is_timeout(){"timeout"}else{"body"});break;}
                    }
                }
                result["read_bytes"]=json!(bytes.len());
                result["prefix_sha256"]=json!(hex::encode(Sha256::digest(&bytes)));
            }
        },
        Err(error)=>result["error"]=json!(error),
    }
    result["elapsed_ms"]=json!(begin.elapsed().as_millis());
    result
}
fn client(proxy: Option<&str>) -> Result<Client,String> {
    let mut builder=Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(8)).connect_timeout(Duration::from_secs(5))
        .no_gzip().no_brotli().no_deflate();
    if let Some(proxy)=proxy{builder=builder.proxy(reqwest::Proxy::all(proxy).map_err(|_|"Existing proxy invalid")?);}
    builder.build().map_err(|_|"Create diagnostic client failed".into())
}
async fn run(input: PathBuf) -> Result<Value,String> {
    let (proxy_key,proxy)=["HTTPS_PROXY","HTTP_PROXY","ALL_PROXY"].iter()
        .find_map(|key|std::env::var(key).ok().filter(|s|!s.trim().is_empty()).map(|value|(*key,value)))
        .ok_or("No existing environment proxy; refusing to invent one")?;
    let direct=client(None)?;
    let proxied=client(Some(&proxy))?;
    let mut cases=Vec::new();
    for id in ["7694589931460594953","7408129200156101907"] {
        let private=std::fs::read(input.join(format!("{id}-private.json"))).map_err(|_|"Read TEMP native evidence failed")?;
        let metadata:Value=serde_json::from_slice(&private).map_err(|_|"TEMP native JSON invalid")?;
        let url=metadata.pointer("/converted/formats/0/url").and_then(Value::as_str).ok_or("Missing actual native candidate")?;
        for (route,client) in [("direct-no-proxy",direct.clone()),("explicit-existing-env-proxy",proxied.clone())] {
            for range in [None,Some("bytes=0-0"),Some("bytes=0-65535")] {
                cases.push((client.clone(),route,id.to_owned(),url.to_owned(),range));
            }
        }
    }
    let results=stream::iter(cases).map(|(client,route,id,url,range)|check(client,route,id,url,range))
        .buffer_unordered(2).collect::<Vec<_>>().await;
    Ok(json!({"kind":"REAL_WORKER_ADAPTER_DOUYIN_RANGE_ROUTE_DIAGNOSTIC","proxy_source_key":proxy_key,
        "personal_cookies":false,"prefix_limit":LIMIT,"not_complete_download":true,
        "candidate_source":"same TEMP URL minted by original default-environment native fetch; no direct-route token regeneration",
        "intermediate_redirect_hops_not_instrumented":true,"results":results}))
}
#[tokio::main]
async fn main() {
    let mut args=std::env::args_os().skip(1);
    let (Some(input),Some(output))=(args.next().map(PathBuf::from),args.next().map(PathBuf::from)) else{eprintln!("TEMP input/output required");std::process::exit(1)};
    if !input.starts_with(std::env::temp_dir()) || !output.starts_with(std::env::temp_dir()) || output.exists(){eprintln!("Use a new TEMP output");std::process::exit(1)};
    let report=match tokio::time::timeout(Duration::from_secs(120),run(input)).await {
        Ok(Ok(report))=>report,Ok(Err(error))=>json!({"error":error}),Err(_)=>json!({"error":"120s deadline"}),
    };
    if std::fs::create_dir_all(&output).and_then(|_|std::fs::write(output.join("sanitized-worker-results.json"),report.to_string())).is_err(){eprintln!("Write TEMP report failed");std::process::exit(1)};
    println!("{}",report);
}
