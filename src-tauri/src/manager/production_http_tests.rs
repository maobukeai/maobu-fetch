//! Windows native-context tests of the actual manager, database and HTTP workers.
//! No windows, single-instance plugin, bridge listener or real user data are created.
use super::*;
mod bridge_cases;
mod bt_bandwidth;
mod creation_cases;
mod fixture;
mod harness;
mod lifecycle_cases;
mod protocol_cases;
mod publication_cases;
mod recovery_cases;
mod recording_fixture;
mod recording_cases;
mod scenarios;
mod site_cases;
mod startup_cases;
mod cloud_file_case;
mod cloud_content_cases;
mod cloud_content_fixture;
mod support;
mod transfer_cases;

#[test]
fn native_manager_http_downloads_are_integral_resumable_and_cancellable() {
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier = format!("app.maobu.test.{}", Uuid::new_v4().simple());
    let app = tauri::Builder::default()
        .any_thread()
        .build(context)
        .expect("build isolated Wry context without any visible window");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    // Keep the aggregate on the heap instead of moving its large, ungrouped
    // future through block_on's test-thread stack. The runtime/thread stack
    // configuration and every scenario remain unchanged.
    let scenarios = Box::pin(scenarios::run(app.handle().clone()));
    eprintln!(
        "NATIVE_MANAGER_FUTURE ungrouped_bytes={}",
        std::mem::size_of_val(scenarios.as_ref().get_ref())
    );
    if std::env::var("MAOBU_TEST_NATIVE_ONLY").as_deref() == Ok("future-size") {
        eprintln!("NATIVE_MANAGER_FUTURE diagnostic_only=true scenarios_executed=false");
        return;
    }
    runtime.block_on(scenarios);
}
