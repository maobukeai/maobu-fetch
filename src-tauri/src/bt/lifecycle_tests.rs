//! 在唯一隔离 Wry context 内运行真实 manager/数据库/BT worker。
use super::{swarm_tests::local_swarm, *};
use crate::manager::SharedManager;
use crate::models::{BtNewTaskRequest, DownloadTask, TaskStatus};
use sha2::{Digest, Sha256};
use std::{future::Future, path::Path, time::Duration};

pub(crate) async fn run<F, Fut>(manager: &SharedManager, root: &Path, dispatch: F) -> String
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    tokio::fs::create_dir_all(root).await.unwrap();
    let swarm = local_swarm(root).await;
    use base64::Engine as _;
    let task = manager
        .add_bt(BtNewTaskRequest {
            source: "lifecycle.torrent".into(),
            source_data_base64: Some(
                base64::engine::general_purpose::STANDARD.encode(&swarm.torrent),
            ),
            destination: None,
            selected_files: vec![],
            start_paused: false,
            source_tag: Some("native-test".into()),
            streaming_priority: false,
        })
        .await
        .unwrap();
    dispatch().await.unwrap();
    let complete = wait_row(manager, &task.id, |row| row.status == TaskStatus::Completed).await;
    assert!(
        !manager
            .bt
            .status_of(&task.id, &complete.destination)
            .await
            .unwrap()
            .seeder,
        "默认关闭做种"
    );
    assert!(!complete.bt_meta.as_ref().unwrap().seeding_requested);
    let file = task_content_path(&complete, None).await.unwrap();
    assert_eq!(
        Sha256::digest(tokio::fs::read(file).await.unwrap()),
        Sha256::digest(&swarm.expected)
    );
    assert_eq!(manager.bt.active_downloads().await, 0);
    enable_and_toggle_seed(manager, &task.id).await;
    check_requested_only_restart(manager, &task.id).await;
    swarm.seeder.stop().await;
    // 返回时无限分享率且没有 leecher，父测试用同 manager 验证下个 HTTP 任务可完成。
    task.id
}

async fn enable_and_toggle_seed(manager: &SharedManager, id: &str) {
    let mut settings = manager.settings().await;
    settings.bt_seed_enabled = true;
    settings.bt_seed_ratio = 0.0;
    manager.save_settings(settings.clone()).await.unwrap();
    sync_seeding_policy(manager).await.unwrap();
    assert!(
        !manager.bt.status_of(id, "").await.unwrap().seeder,
        "开启全局开关不能静默上传未请求的历史任务"
    );
    request_seeding(manager, id, true).await.unwrap();
    let row = wait_seeding(manager, id, true).await;
    assert_eq!(row.status, TaskStatus::Completed);
    assert_eq!(manager.bt.active_downloads().await, 0);
    settings.bt_seed_enabled = false;
    manager.save_settings(settings.clone()).await.unwrap();
    let stopped = wait_seeding(manager, id, false).await;
    assert_eq!(stopped.status, TaskStatus::Completed);
    assert!(stopped.bt_meta.unwrap().seeding_requested);
    settings.bt_seed_enabled = true;
    manager.save_settings(settings).await.unwrap();
    wait_seeding(manager, id, true).await;
}

async fn check_requested_only_restart(manager: &SharedManager, id: &str) {
    let mut historical = manager.store.get_task(id).await.unwrap().unwrap();
    historical.id = "history-without-seed-request".into();
    historical.bt_meta.as_mut().unwrap().seeding_requested = false;
    historical.bt_runtime = None;
    manager.store.upsert_task(&historical).await.unwrap();
    manager.bt.shutdown().await.unwrap();
    sync_seeding_policy(manager).await.unwrap();
    let restored = wait_seeding(manager, id, true).await;
    assert_eq!(restored.status, TaskStatus::Completed);
    assert!(
        manager
            .bt
            .status_of(id, &restored.destination)
            .await
            .unwrap()
            .seeder
    );
    assert!(!manager
        .bt
        .bindings
        .lock()
        .await
        .contains_key(&historical.id));
    assert_eq!(manager.bt.active_downloads().await, 0);
}

async fn wait_seeding(manager: &SharedManager, id: &str, expected: bool) -> DownloadTask {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let row = manager.store.get_task(id).await.unwrap().unwrap();
            assert_eq!(row.status, TaskStatus::Completed);
            if manager
                .bt
                .status_of(id, &row.destination)
                .await
                .unwrap()
                .seeder
                == expected
            {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("真实原生做种状态必须响应策略变化")
}

async fn wait_row(
    manager: &SharedManager,
    id: &str,
    ready: impl Fn(&DownloadTask) -> bool,
) -> DownloadTask {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let row = manager.store.get_task(id).await.unwrap().unwrap();
            assert_ne!(row.status, TaskStatus::Failed, "{:?}", row.error);
            if ready(&row) {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("真实 BT manager 必须在有限时间内提交状态")
}
