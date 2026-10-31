use super::super::*;
use super::{
    fixture::{Fixture, Mode},
    harness::Harness,
};

pub async fn run(app: &AppHandle) {
    if std::env::var("MAOBU_TEST_BRIDGE_BROWSER").as_deref() != Ok("1") {
        return;
    }
    let harness = Harness::new(app, None).await;
    // Keep the real stalled transfer observable longer than the browser's 12s lifecycle wait.
    let mut settings = harness.manager.settings().await;
    settings.default_retry_policy.connection_timeout_secs = 30;
    let client = build_client(&settings).unwrap();
    harness
        .manager
        .store
        .save_settings(&settings)
        .await
        .unwrap();
    *harness.manager.settings.write().await = settings;
    *harness.manager.client.write().await = client;
    let complete = Fixture::new(Mode::Good).await;
    let slow = Fixture::new(Mode::SilentBody).await;
    let scheduler = tokio::spawn(harness.manager.clone().scheduler_loop());
    let result = crate::bridge::browser_fixture::run(
        app,
        &harness.manager,
        &harness.output,
        &complete.url,
        &slow.url,
        &hex::encode(Sha256::digest(complete.expected())),
    )
    .await;
    scheduler.abort();
    let _ = scheduler.await;
    harness.manager.shutdown_bt().await.unwrap();
    eprintln!(
        "real browser bridge integration: {}",
        result.expect("isolated Chrome/Edge -> real Rust bridge must pass")
    );
}
