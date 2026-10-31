use super::super::*;
use super::{
    fixture::{Fixture, Mode},
    harness::Harness,
};

pub(super) async fn bt_lifecycle_and_background_seeding(app: &AppHandle) {
    eprintln!("native manager: real BT lifecycle and background seeding slot isolation");
    let harness = Harness::new(app, None).await;
    let fixture_dir = harness.data.join("bt-fixture");
    let id = crate::bt::lifecycle_tests::run(&harness.manager, &fixture_dir, || {
        harness.manager.dispatch_once()
    })
    .await;
    let fixture = Fixture::new(Mode::Good).await;
    let task = harness.task(&fixture, 1).await;
    harness.check_output(&task.id, &fixture.expected()).await;
    crate::bt::request_seeding(&harness.manager, &id, false)
        .await
        .unwrap();
    harness.manager.shutdown_bt().await.unwrap();
}

pub(super) async fn raw_share_requires_selection_and_never_spawns_children(app: &AppHandle) {
    eprintln!("native manager: five raw cloud shares require explicit selection without background children");
    let fixture = Fixture::new(Mode::Good).await;
    let harness = Harness::new(app, None).await;
    let mut ids = Vec::new();
    for url in [
        "https://pan.baidu.com/s/1synthetic",
        "https://pan.quark.cn/s/synthetic",
        "https://mypikpak.com/s/synthetic",
        "https://wwabc.lanzouq.com/synthetic",
        "https://www.123pan.com/s/synthetic",
    ] {
        let mut task = harness.queued_task(&fixture, 1).await;
        task.url = url.into();
        // If the selection guard regresses, even the attempted cloud request is
        // confined to a local non-forwarding proxy; this test cannot reach the internet.
        task.proxy_override = Some(fixture.url.strip_suffix("/fixture.bin").unwrap().into());
        harness.manager.store.upsert_task(&task).await.unwrap();
        harness.manager.dispatch_once().await.unwrap();
        let row = harness.wait(&task.id, TaskStatus::Failed).await;
        assert!(row
            .error
            .as_deref()
            .is_some_and(|error| error.contains("勾选需要的文件")));
        assert_eq!(row.url, url);
        assert_eq!(row.file_name, task.file_name);
        assert!(!harness.output.join(&row.file_name).exists());
        ids.push(task.id);
    }
    assert_eq!(harness.manager.store.list_tasks().await.unwrap().len(), 5);
    for id in ids {
        harness.manager.remove(&id, false).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(
        harness.manager.store.list_tasks().await.unwrap().is_empty(),
        "removed raw-share parents cannot create late child tasks"
    );
    harness.manager.shutdown_bt().await.unwrap();
}

pub(super) async fn opt_in_external_media(app: &AppHandle) {
    if std::env::var("MAOBU_TEST_EXTERNAL_MEDIA").as_deref() != Ok("1") {
        eprintln!("external media integration: SKIPPED (MAOBU_TEST_EXTERNAL_MEDIA=1 not enabled)");
        return;
    }
    let harness = Harness::new(app, None).await;
    match crate::media::execution_e2e::run(app, &harness.manager, &harness.output)
        .await
        .expect("isolated external media production execution")
    {
        crate::media::execution_e2e::Outcome::Executed(cases) => {
            assert!(!cases.is_empty());
            eprintln!(
                "external media integration executed: {}",
                serde_json::Value::Array(cases)
            );
        }
        crate::media::execution_e2e::Outcome::Skipped(reason) => {
            eprintln!("external media integration: SKIPPED ({reason})")
        }
    }
    harness.manager.shutdown_bt().await.unwrap();
}
