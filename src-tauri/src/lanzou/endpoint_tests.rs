use super::*;
use axum::{
    body::Body,
    extract::State,
    http::{Response, Uri},
    Router,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
#[test]
fn static_literal_variables_and_official_default_branch_keep_the_actual_endpoint() {
    let base = "https://author.lanzoul.com/fn?synthetic";
    let vars = HashMap::from([
        ("simple".into(), "/ajaxm.php?file=7".into()),
        (
            "domain1".into(),
            "https://apifile.woozooo.com/ajaxfile.php?file=7".into(),
        ),
        (
            "domain2".into(),
            "https://apifile.lanzouw.com/ajaxfile.php?file=7".into(),
        ),
    ]);
    assert!(ajax_url("url:'/ajaxm.php?file=7'", &vars, base)
        .unwrap()
        .contains("/ajaxm.php?file=7"));
    assert!(ajax_url(
        "url:'/ajaxm.php';data:{action:'downprocess',sign:nonce}",
        &vars,
        base
    )
    .unwrap()
    .ends_with("/ajaxm.php"));
    assert!(ajax_url("url:simple", &vars, base)
        .unwrap()
        .contains("/ajaxm.php?file=7"));
    let html="if (typeof(killdnsweb)=='undefined'){dom_ajaxs = domain2;}else{dom_ajaxs=domain1;}\nurl : dom_ajaxs,";
    assert!(ajax_url(html, &vars, base)
        .unwrap()
        .starts_with("https://apifile.lanzouw.com/ajaxfile.php"));
    for html in [
        "url:unresolved,",
        "url:'https://not-official.invalid/ajaxm.php'",
        "url:'https://user:synthetic-secret@apifile.lanzouw.com/ajaxm.php'",
        "url:'javascript:alert(1)'",
        "url:'/ajaxm.php'+unknown",
    ] {
        let error = ajax_url(html, &vars, base).unwrap_err();
        assert!(!error.contains("synthetic-secret"));
    }
}
#[derive(Clone)]
struct StateData {
    base: String,
    api: Arc<AtomicUsize>,
}
async fn respond(State(state): State<StateData>, uri: Uri) -> Response<Body> {
    let text = if uri.path().ends_with("/actual-file") {
        "<iframe src='/fn?synthetic'></iframe>".into()
    } else if uri.path().ends_with("/fn") {
        "<script>var domain1='https://apifile.woozooo.com/ajaxfile.php?file=7'; var domain2='https://apifile.lanzouw.com/ajaxfile.php?file=7';var nonce='synthetic';if (typeof(killdnsweb)=='undefined'){dom_ajaxs = domain2;}else{dom_ajaxs=domain1;}\n$.ajax({url : dom_ajaxs,\ndata : {'sign':nonce,'kd':kdns}});</script>".into()
    } else if uri.path() == "/apifile.lanzouw.com/ajaxfile.php" {
        state.api.fetch_add(1, Ordering::Relaxed);
        assert_eq!(uri.query(), Some("file=7"));
        serde_json::json!({"zt":1,"dom":state.base,"url":"actual.bin"}).to_string()
    } else {
        assert!(
            uri.path().ends_with("/file/actual.bin"),
            "unexpected endpoint {}",
            uri.path()
        );
        "actual file".into()
    };
    Response::new(Body::from(text))
}
#[tokio::test]
async fn production_iframe_resolver_posts_to_declared_official_endpoint_over_real_http() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let api = Arc::new(AtomicUsize::new(0));
    let state = StateData {
        base: base.clone(),
        api: api.clone(),
    };
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().fallback(respond).with_state(state))
            .await
            .unwrap();
    });
    let network = crate::cloud_request::CloudRequestContext::new(
        reqwest::Client::new(),
        reqwest::Client::new(),
        tokio_util::sync::CancellationToken::new(),
    )
    .with_test_endpoint(&base);
    let link = super::super::resolve_lanzou_file_with_context(
        "https://author.lanzoul.com/s/collection",
        "actual-file",
        None,
        &network,
    )
    .await
    .unwrap();
    assert_eq!(api.load(Ordering::Relaxed), 1);
    assert!(link.url.ends_with("/file/actual.bin"));
    assert_eq!(
        reqwest::get(&link.url)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
        "actual file"
    );
    server.abort();
}
