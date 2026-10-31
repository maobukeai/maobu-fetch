use super::*;
use axum::{
    body::Body,
    extract::{Form, State},
    http::StatusCode,
    response::Response,
    routing::post,
    Router,
};
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
#[derive(Clone, Copy)]
enum Case {
    Complete,
    Duplicate,
    Repeated,
    Empty,
    EmptySuccess,
    Error,
    Malformed,
    Missing,
    FalseTerminal,
    Cancelled,
    WrongPassword,
    NeedsPassword,
    PublicNoFileTerminal,
}
struct Fixture {
    url: String,
    hits: Arc<AtomicUsize>,
    handle: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn start(case: Case) -> Self {
        let hits = Arc::new(AtomicUsize::new(0));
        let router = Router::new()
            .route("/list", post(respond))
            .with_state((case, hits.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            url: format!("http://{address}/list"),
            hits,
            handle,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.handle.abort();
    }
}
async fn respond(
    State((case, hits)): State<(Case, Arc<AtomicUsize>)>,
    Form(params): Form<HashMap<String, String>>,
) -> Response {
    hits.fetch_add(1, Ordering::AcqRel);
    let page = params.get("pg").unwrap().parse::<usize>().unwrap();
    if matches!(case, Case::Cancelled) {
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    if matches!(case, Case::NeedsPassword | Case::WrongPassword) {
        return Response::new(Body::from(
            json!({"zt":3,"text":"password rejected"}).to_string(),
        ));
    }
    if page == 2 && matches!(case, Case::Error) {
        return Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .body(Body::from("synthetic failure"))
            .unwrap();
    }
    if page == 2 && matches!(case, Case::Malformed) {
        return Response::new(Body::from("not json"));
    }
    if page == 2 && matches!(case, Case::Missing) {
        return Response::new(Body::from(json!({"zt":1}).to_string()));
    }
    if page == 2 && matches!(case, Case::FalseTerminal) {
        return Response::new(Body::from(
            json!({"zt":2,"text":"access denied"}).to_string(),
        ));
    }
    if page == 2 && matches!(case, Case::PublicNoFileTerminal) {
        return Response::new(Body::from(
            json!({"zt":2,"text":"no file","info":"legacy encoded terminal message"}).to_string(),
        ));
    }
    if matches!(case, Case::EmptySuccess) {
        return Response::new(Body::from(json!({"zt":1,"text":[]}).to_string()));
    }
    if matches!(case, Case::Empty) || page > 2 {
        return Response::new(Body::from(json!({"zt":2,"text":[]}).to_string()));
    }
    let start = if page == 2 && matches!(case, Case::Repeated) {
        0
    } else {
        (page - 1) * 100
    };
    let end = if matches!(case, Case::PublicNoFileTerminal) {
        2
    } else {
        (start + 100).min(101)
    };
    let mut files:Vec<_>=(start..end).map(|id|json!({"id":format!("item-{id}"),"name_all":format!("entry-{id}.bin"),"size":"1 B","time":"synthetic"})).collect();
    if matches!(case, Case::Duplicate) {
        files.push(files[0].clone());
    }
    Response::new(Body::from(json!({"zt":1,"text":files}).to_string()))
}
#[tokio::test]
async fn actual_public_no_file_terminal_preserves_first_page_without_a_third_request() {
    let fixture = Fixture::start(Case::PublicNoFileTerminal).await;
    let listing = collect_folder(
        &context(CancellationToken::new()),
        &fixture.url,
        &HashMap::new(),
        "https://fixture.invalid",
        None,
        "fixture.invalid",
        false,
        false,
    )
    .await
    .unwrap();
    assert_eq!(listing.files.len(), 2);
    assert_eq!(fixture.hits.load(Ordering::Acquire), 2);
    assert!(!listing.requires_password);
}
fn context(cancel: CancellationToken) -> CloudRequestContext {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    CloudRequestContext::new(client.clone(), client, cancel)
}
#[tokio::test]
async fn folder_101_files_multiple_pages_and_duplicates_are_complete() {
    for case in [
        Case::Complete,
        Case::Duplicate,
        Case::Empty,
        Case::EmptySuccess,
    ] {
        let fixture = Fixture::start(case).await;
        let listing = collect_folder(
            &context(CancellationToken::new()),
            &fixture.url,
            &HashMap::from([("pg".into(), "99".into())]),
            "https://fixture.invalid",
            None,
            "fixture.invalid",
            false,
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            listing.files.len(),
            if matches!(case, Case::Empty | Case::EmptySuccess) {
                0
            } else {
                101
            }
        );
        assert!(!listing.requires_password);
        assert_eq!(
            fixture.hits.load(Ordering::Acquire),
            if matches!(case, Case::Empty | Case::EmptySuccess) {
                1
            } else {
                3
            }
        );
    }
}
#[tokio::test]
async fn folder_page_two_failures_and_repeated_pages_never_return_partial_success() {
    for case in [
        Case::Error,
        Case::Malformed,
        Case::Missing,
        Case::Repeated,
        Case::FalseTerminal,
    ] {
        let fixture = Fixture::start(case).await;
        let result = collect_folder(
            &context(CancellationToken::new()),
            &fixture.url,
            &HashMap::new(),
            "https://fixture.invalid",
            None,
            "fixture.invalid",
            false,
            false,
        )
        .await;
        assert!(result.err().unwrap().contains("未完整"));
        assert!(fixture.hits.load(Ordering::Acquire) <= 2);
    }
}
#[tokio::test]
async fn folder_request_cancellation_and_password_states_are_explicit() {
    let fixture = Fixture::start(Case::Cancelled).await;
    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        stopper.cancel();
    });
    let started = Instant::now();
    let result = collect_folder(
        &context(cancel),
        &fixture.url,
        &HashMap::new(),
        "https://fixture.invalid",
        None,
        "fixture.invalid",
        false,
        false,
    )
    .await;
    assert!(result.err().unwrap().contains("取消"));
    assert!(started.elapsed() < Duration::from_millis(500));
    let fixture = Fixture::start(Case::NeedsPassword).await;
    let context = context(CancellationToken::new());
    assert!(
        collect_folder(
            &context,
            &fixture.url,
            &HashMap::new(),
            "https://fixture.invalid",
            None,
            "fixture.invalid",
            true,
            false
        )
        .await
        .unwrap()
        .requires_password
    );
    let fixture = Fixture::start(Case::WrongPassword).await;
    assert!(collect_folder(
        &context,
        &fixture.url,
        &HashMap::new(),
        "https://fixture.invalid",
        None,
        "fixture.invalid",
        true,
        true
    )
    .await
    .is_err());
}

#[tokio::test]
async fn folder_protection_limits_are_explicit_errors_not_truncated_lists() {
    for kind in 0..3 {
        let fixture = Fixture::start(Case::Complete).await;
        let network = context(CancellationToken::new());
        let params = HashMap::new();
        let request = FolderRequest {
            api_url: &fixture.url,
            params: &params,
            referer: "https://fixture.invalid",
            cookie: None,
            host: "fixture.invalid",
            requires_password: false,
            has_password: false,
        };
        let mut limits = Limits::default();
        match kind {
            0 => limits.pages = 1,
            1 => limits.entries = 50,
            _ => limits.metadata_bytes = 50,
        };
        let result = network
            .run("蓝奏云", collect(&network, &request, limits))
            .await;
        assert!(result.err().unwrap().contains("未完整"));
        assert_eq!(fixture.hits.load(Ordering::Acquire), 1);
    }
}
