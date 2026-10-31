//! Public-share listing only. No saved cookies/database, no resolve/save/download.
#[path = "../../src-tauri/src/pikpak.rs"] mod pikpak;
#[path = "../../src-tauri/src/quark.rs"] mod quark;
#[path = "../../src-tauri/src/baidupan.rs"] mod baidupan;
#[path = "../../src-tauri/src/lanzou.rs"] mod lanzou;
#[path = "../../src-tauri/src/pan123.rs"] mod pan123;

use serde_json::{json, Value};
use std::time::Duration;

fn record(platform: &str, url: &str, result: Result<Result<Value, String>, tokio::time::error::Elapsed>) -> Value {
    let outcome = match result {
        Ok(Ok(value)) => json!({"ok": true, "summary": value}),
        Ok(Err(error)) => {
            let urls = regex::Regex::new(r"https?://[^\s]+\b").unwrap();
            let clean = urls.replace_all(&error, "[URL]");
            // Avoid echoing arbitrary upstream bodies which may contain tokens.
            let summary = if clean.contains("接口回执异常") {
                "接口回执异常（正文已省略）".to_owned()
            } else { clean.chars().take(300).collect() };
            json!({"ok": false, "error_summary": summary})
        }
        Err(_) => json!({"ok": false, "error_summary": "test harness timeout after 25 seconds"}),
    };
    // Share passwords may be present in fixtures; do not repeat them in results.
    let public_sample = url.split('?').next().unwrap_or(url);
    json!({
        "platform": platform, "public_sample": public_sample, "cookie": "none",
        "mode": "native module share listing only; no resolve/save/download",
        "time_unix_seconds": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
        "result": outcome,
    })
}

async fn pikpak_probe() -> Value {
    let url = "https://mypikpak.com/s/VOveL7ZI01ViAz9VVKGgSWDlo2";
    let device_id = hex::encode(rand::random::<[u8; 16]>());
    let future = async {
        pikpak::inspect_pikpak_share(url, None, &device_id).await.map(|v| json!({
            "listed_items": v.files.len(), "file_count": v.file_count,
            "pass_code_required": v.pass_code_required,
        }))
    };
    record("pikpak", url, tokio::time::timeout(Duration::from_secs(25), future).await)
}

async fn quark_probe() -> Value {
    let url = "https://pan.quark.cn/s/69ba75a686aa#/list/share";
    let future = async {
        quark::inspect_quark_share(url, None, None).await.map(|v| json!({
            "listed_items": v.files.len(), "file_count": v.file_count,
            "pass_code_required": v.pass_code_required,
        }))
    };
    record("quark", url, tokio::time::timeout(Duration::from_secs(25), future).await)
}

async fn baidu_probe() -> Value {
    let url = "https://pan.baidu.com/s/1uqSBOwEkkw-NdAenNndoqA?pwd=fqq4";
    let future = async {
        baidupan::inspect_baidu_share(url, None, None).await.map(|v| json!({
            "listed_items": v.files.len(), "file_count": v.file_count,
            "pass_code_required": v.pass_code_required,
        }))
    };
    record("baidupan", url, tokio::time::timeout(Duration::from_secs(25), future).await)
}

async fn lanzou_probe() -> Value {
    let url = "https://www.lanzoui.com/u/yoyodadada";
    let future = async {
        lanzou::inspect_lanzou_share(url, None).await.map(|v| json!({
            "listed_items": v.files.len(), "requires_password": v.requires_password,
        }))
    };
    record("lanzou", url, tokio::time::timeout(Duration::from_secs(25), future).await)
}

async fn pan123_probe() -> Value {
    let url = "https://1683912.share.123pan.cn/123pan/z3h9-rtFzh?notoken=1";
    let future = async {
        pan123::inspect_pan123_share(url, None).await.map(|v| json!({
            "listed_items": v.files.len(), "requires_password": v.requires_password,
        }))
    };
    record("pan123", url, tokio::time::timeout(Duration::from_secs(25), future).await)
}

#[tokio::main]
async fn main() {
    let (pikpak, quark, baidu, lanzou, pan123) = tokio::join!(
        pikpak_probe(), quark_probe(), baidu_probe(), lanzou_probe(), pan123_probe(),
    );
    println!("{}", json!([pikpak, quark, baidu, lanzou, pan123]));
}
