//! Isolated release smoke must never revoke a user's existing browser pairing.
use axum::{
    body::{to_bytes, Body},
    http::{Method, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Json, Router,
};

pub(super) fn enabled() -> bool {
    std::env::var("MAOBU_FETCH_ISOLATED_TEST").as_deref() == Ok("1")
}
pub(super) fn apply(router: Router) -> Router {
    router.layer(middleware::from_fn(guard))
}

async fn guard(request: Request<Body>, next: Next) -> Response {
    if request.method() != Method::GET || request.uri().path() != "/v1/health" {
        return unavailable();
    }
    let response = next.run(request).await;
    let (parts, body) = response.into_parts();
    let Ok(bytes) = to_bytes(body, 4096).await else {
        return unavailable();
    };
    let Ok(mut health) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return unavailable();
    };
    if !health.is_object() {
        return unavailable();
    }
    health["takeover_enabled"] = false.into();
    health["bt_magnet_enabled"] = false.into();
    (parts.status, Json(health)).into_response()
}
fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "隔离启动验收中，本地桥写操作暂不可用；浏览器凭证保持不变",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    #[tokio::test]
    async fn actual_http_smoke_allows_only_health_and_never_mutates_pairing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_owned();
        let store = Arc::new(
            tokio::task::spawn_blocking(move || crate::store::Store::open(path))
                .await
                .unwrap()
                .unwrap(),
        );
        store
            .save_pairing("synthetic-existing-extension", "synthetic-existing-hash")
            .await
            .unwrap();
        let before = store.get_pairing().await.unwrap();
        let side_effects = Arc::new(AtomicUsize::new(0));
        let observed = side_effects.clone();
        let mutations = store.clone();
        let router=Router::new().route("/v1/health",axum::routing::get(||async {Json(serde_json::json!({"name":"Maobu Fetch","api_version":1,"version":crate::updater::APP_VERSION,"ready":true,"takeover_enabled":true,"bt_magnet_enabled":true}))}))
            .fallback(move||{let mutations=mutations.clone();let observed=observed.clone();async move {observed.fetch_add(1,Ordering::SeqCst);mutations.clear_pairing().await.unwrap();StatusCode::OK}});
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, apply(router)).await.unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
        let base = format!("http://{address}");
        let health = client
            .get(format!("{base}/v1/health"))
            .send()
            .await
            .unwrap();
        assert_eq!(health.status(), 200);
        let health: serde_json::Value = health.json().await.unwrap();
        assert_eq!(health["version"], crate::updater::APP_VERSION);
        assert_eq!(health["takeover_enabled"], false);
        assert_eq!(health["bt_magnet_enabled"], false);
        for (method, path) in [
            (Method::POST, "/v1/pair"),
            (Method::GET, "/v1/tasks/recent"),
            (Method::POST, "/v1/tasks"),
            (Method::POST, "/v1/tasks/example/action"),
            (Method::POST, "/v1/media/probe"),
            (Method::POST, "/v1/media/credentials/sync"),
            (Method::POST, "/v1/focus"),
            (Method::OPTIONS, "/v1/health"),
            (Method::HEAD, "/v1/health"),
            (Method::GET, "/unknown"),
        ] {
            assert_eq!(
                client
                    .request(method, format!("{base}{path}"))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                503,
                "{path}"
            );
        }
        assert_eq!(side_effects.load(Ordering::SeqCst), 0);
        assert_eq!(store.get_pairing().await.unwrap(), before);
        server.abort();
        let _ = server.await;
    }
}
