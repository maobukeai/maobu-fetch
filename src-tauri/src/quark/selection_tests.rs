use super::*;
use axum::{body::Body, extract::State, http::Uri, response::Response, Router};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn selected_subdirectory_pagination_never_expands_to_the_share_root() {
    for kind in 0..3 {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let router = Router::new()
            .fallback(reply)
            .with_state((kind, seen.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let network = crate::cloud_request::CloudRequestContext::new(
            client.clone(),
            client,
            CancellationToken::new(),
        )
        .with_test_endpoint(&format!("http://{address}"));
        let result = inspect_quark_share_with_context(
            "https://pan.quark.cn/s/synthetic/selected",
            None,
            None,
            &network,
        )
        .await;
        if kind == 2 {
            assert!(result.unwrap_err().contains("未完整"));
        } else {
            let listing = result.unwrap();
            let count = if kind == 0 { 301 } else { 0 };
            assert_eq!(listing.file_count, count);
            assert_eq!(listing.files.len(), count);
            let unique: std::collections::HashSet<_> =
                listing.files.iter().map(|item| &item.id).collect();
            assert_eq!(unique.len(), count);
        }
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), if kind == 0 { 4 } else { 1 });
        assert!(seen.iter().all(|folder| folder == "selected"));
        server.abort();
    }
}

async fn reply(State((kind, seen)): State<(i32, Arc<Mutex<Vec<String>>>)>, uri: Uri) -> Response {
    if uri.path().ends_with("/token") {
        return Response::new(Body::from(
            json!({"code":0,"data":{"stoken":"synthetic"}}).to_string(),
        ));
    }
    let query: HashMap<String, String> =
        url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let folder = query.get("pdir_fid").cloned().unwrap_or_default();
    seen.lock().unwrap().push(folder.clone());
    if kind == 2 || folder != "selected" {
        return Response::new(Body::from(json!({"code":404,"data":{}}).to_string()));
    }
    let total = if kind == 0 { 301 } else { 0 };
    let page: usize = query.get("_page").unwrap().parse().unwrap();
    let list:Vec<_>=((page-1)*100..(page*100).min(total)).map(|index|
        json!({"fid":format!("id-{index}"),"file_name":format!("fixture-{index}.bin"),"size":1,"file":true,"dir":false})).collect();
    Response::new(Body::from(
        json!({"code":0,"data":{"total":total,"list":list}}).to_string(),
    ))
}
