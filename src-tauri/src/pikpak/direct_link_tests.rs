use super::*;
use axum::{
    body::Body,
    extract::State,
    http::{Response, StatusCode, Uri},
    Router,
};
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[test]
fn blank_media_uses_trimmed_original_without_changing_valid_media_precedence() {
    let original = "https://download.example.invalid/original.txt";
    for empty in ["", " \t "] {
        let json = serde_json::json!({"file_info":{"medias":[{"link":{"url":empty}}],"web_content_link":format!("  {original}  ")}});
        assert_eq!(select(&json).unwrap(), original);
    }
    assert_eq!(
        select(
            &serde_json::json!({"file_info":{"web_content_link":""},"web_content_link":original})
        )
        .unwrap(),
        original
    );
    let json = serde_json::json!({"file_info":{"medias":[{"link":{"url":"https://download.example.invalid/media.mp4"}}],"web_content_link":original}});
    assert!(select(&json).unwrap().ends_with("media.mp4"));
    let json = serde_json::json!({"file_info":{"medias":[{"link":{"url":" "}},{"link":{"url":"https://download.example.invalid/second.mp4"}}],"web_content_link":original}});
    assert!(select(&json).unwrap().ends_with("second.mp4"));
    let json = serde_json::json!({"file_info":null,"file":{"web_content_link":original}});
    assert_eq!(select(&json).unwrap(), original);
}

#[test]
fn missing_blank_and_unsafe_addresses_are_actionable_errors_without_echoing_secrets() {
    for json in [
        serde_json::json!({}),
        serde_json::json!({"file_info":{"medias":[{"link":{"url":" "}}],"web_content_link":""},"web_content_link":"\t"}),
    ] {
        assert!(select(&json).unwrap_err().contains("重新解析"));
    }
    for address in [
        "javascript:alert(1)",
        "file:///C:/private.txt",
        "ftp://example.invalid/file",
        "https://user:synthetic-secret@example.invalid/file",
        "https://example.invalid/file\r\nheader",
    ] {
        let error = select(&serde_json::json!({"file":{"web_content_link":address}})).unwrap_err();
        assert!(error.contains("重新解析"));
        assert!(!error.contains("synthetic-secret"));
    }
    let json = serde_json::json!({"file_info":{"medias":[{"link":{"url":"javascript:alert(1)"}},{"link":{"url":"https://download.example.invalid/later.mp4"}}]}});
    assert!(select(&json).unwrap_err().contains("不安全"));
}

struct Fixture {
    base: String,
    mode: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
}
#[derive(Clone)]
struct Api {
    base: String,
    mode: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
}
impl Fixture {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mode = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(AtomicUsize::new(0));
        let state = Api {
            base: base.clone(),
            mode: mode.clone(),
            requests: requests.clone(),
        };
        let server = tokio::spawn(async move {
            axum::serve(listener, Router::new().fallback(serve).with_state(state))
                .await
                .unwrap();
        });
        Self {
            base,
            mode,
            requests,
            server,
        }
    }
    fn network(&self) -> crate::cloud_request::CloudRequestContext {
        crate::cloud_request::CloudRequestContext::new(
            reqwest::Client::new(),
            reqwest::Client::new(),
            tokio_util::sync::CancellationToken::new(),
        )
        .with_test_endpoint(&self.base)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
async fn serve(State(state): State<Api>, uri: Uri) -> Response<Body> {
    if uri.path().ends_with("/captcha/init") {
        return Response::new(Body::from(r#"{"captcha_token":"synthetic"}"#));
    }
    if uri.path().ends_with("/share/file_info") {
        state.requests.fetch_add(1, Ordering::Relaxed);
        let link = match state.mode.load(Ordering::Acquire) {
            0 => format!("{}/expired", state.base),
            1 => format!("{}/fresh", state.base),
            _ => "  ".into(),
        };
        return Response::new(Body::from(serde_json::json!({"file_info":{"medias":[{"link":{"url":""}}],"web_content_link":link}}).to_string()));
    }
    if uri.path() == "/expired" {
        return Response::builder()
            .status(StatusCode::GONE)
            .body(Body::empty())
            .unwrap();
    }
    Response::new(Body::from("actual local original file\n"))
}

#[tokio::test]
async fn production_resolver_falls_back_and_requeries_expired_link_over_real_http() {
    let fixture = Fixture::start().await;
    let network = fixture.network();
    let resolve = || {
        super::super::resolve_pikpak_file_with_context(
            "synthetic-share",
            "same-file",
            None,
            "synthetic-device",
            &network,
        )
    };
    let first = resolve().await.unwrap();
    assert_eq!(
        reqwest::get(&first.url).await.unwrap().status(),
        StatusCode::GONE
    );
    fixture.mode.store(1, Ordering::Release);
    let fresh = resolve().await.unwrap();
    assert_ne!(first.url, fresh.url);
    let body = reqwest::get(&fresh.url)
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(
        Sha256::digest(&body),
        Sha256::digest(b"actual local original file\n")
    );
    fixture.mode.store(2, Ordering::Release);
    assert!(resolve().await.unwrap_err().contains("有效下载直链"));
    assert_eq!(fixture.requests.load(Ordering::Relaxed), 3);
}
