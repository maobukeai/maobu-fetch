use super::*;
use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::Request,
    response::Response,
    Router,
};
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn cloud_redirects_replay_exact_post_bytes_only_for_307_308() {
    for status in [301, 302, 303, 307, 308] {
        let seen = Arc::new(Mutex::new(Vec::<(String, Vec<u8>)>::new()));
        let router = Router::new()
            .fallback(reply)
            .with_state((status, seen.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let network = CloudRequestContext::new(client.clone(), client, CancellationToken::new());
        network
            .post(format!("http://{address}/post"))
            .body(b"synthetic-post-body".to_vec())
            .send()
            .await
            .unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].1, b"synthetic-post-body");
        if status <= 303 {
            assert_eq!(seen[1].0, "GET");
            assert!(seen[1].1.is_empty());
        } else {
            assert_eq!(seen[1].0, "POST");
            assert_eq!(seen[1].1, b"synthetic-post-body");
        }
        server.abort();
    }
}
async fn reply(
    State((status, seen)): State<(u16, Arc<Mutex<Vec<(String, Vec<u8>)>>>)>,
    request: Request<Body>,
) -> Response {
    let (head, body) = request.into_parts();
    let bytes = to_bytes(body, 1024).await.unwrap().to_vec();
    seen.lock().unwrap().push((head.method.to_string(), bytes));
    if head.uri.path() == "/post" {
        Response::builder()
            .status(status)
            .header("Location", "/done")
            .body(Body::empty())
            .unwrap()
    } else {
        Response::new(Body::empty())
    }
}
