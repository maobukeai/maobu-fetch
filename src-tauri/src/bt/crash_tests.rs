//! 子进程被实际终止后的恢复，不能用 graceful stop 冒充崩溃。
use super::{swarm_tests::local_swarm, tests::bt_task_for_test, BtEngine};
use crate::models::{AppSettings, DownloadTask};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};

const CHILD_ENV: &str = "MAOBU_BT_CRASH_CHILD";
const TEST_NAME: &str =
    "bt::crash_tests::real_process_crash_rechecks_partial_data_without_progress_loss";

struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn run_child(root: &Path) {
    let root = root.canonicalize().unwrap();
    assert!(root.starts_with(std::env::temp_dir().canonicalize().unwrap()));
    let mut task: DownloadTask =
        serde_json::from_slice(&tokio::fs::read(root.join("child-job.json")).await.unwrap())
            .unwrap();
    assert!(Path::new(&task.destination)
        .canonicalize()
        .unwrap()
        .starts_with(&root));
    assert!(task.bt_meta.as_ref().unwrap().torrent_data_base64.is_some());
    let engine = BtEngine::with_data_dir(root.join("session"));
    engine
        .add_task_inner(&AppSettings::default(), &mut task)
        .await
        .unwrap();
    let handle = engine.bindings.lock().await.get(&task.id).unwrap().clone();
    let checkpoint = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let stats = handle.stats();
            if stats.progress_bytes > 0 && stats.progress_bytes < stats.total_bytes {
                break stats.progress_bytes;
            }
            assert!(
                !matches!(stats.state, librqbit::TorrentStatsState::Error),
                "{:?}",
                stats.error
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    tokio::fs::write(
        root.join("checkpoint-task.json"),
        serde_json::to_vec(&task).unwrap(),
    )
    .await
    .unwrap();
    tokio::fs::write(root.join("ready"), checkpoint.to_string())
        .await
        .unwrap();
    // 父测试会强制终止本进程；这里不运行 Drop/暂停/stop。
    std::future::pending::<()>().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_process_crash_rechecks_partial_data_without_progress_loss() {
    if let Some(root) = std::env::var_os(CHILD_ENV) {
        run_child(Path::new(&root)).await;
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let swarm = local_swarm(temp.path()).await;
    let mut task = bt_task_for_test();
    task.id = "crash-test".into();
    task.url = "crash-source.torrent".into();
    task.destination = temp.path().join("downloads").to_string_lossy().into_owned();
    task.file_name = "stable-crash-output".into();
    task.per_task_speed_limit = 64 * 1024;
    use base64::Engine as _;
    task.bt_meta.as_mut().unwrap().torrent_data_base64 =
        Some(base64::engine::general_purpose::STANDARD.encode(&swarm.torrent));
    task.bt_meta.as_mut().unwrap().info_hash = super::inspect_torrent_bytes(&swarm.torrent)
        .unwrap()
        .info_hash;
    super::prepare_task_output(&mut task).await.unwrap();
    tokio::fs::write(
        temp.path().join("child-job.json"),
        serde_json::to_vec(&task).unwrap(),
    )
    .await
    .unwrap();
    let stderr = std::fs::File::create(temp.path().join("child-error.log")).unwrap();
    let mut child = KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(CHILD_ENV, temp.path())
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .unwrap(),
    );
    tokio::time::timeout(Duration::from_secs(25), async {
        while !temp.path().join("ready").is_file() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "child failed: {}",
                std::fs::read_to_string(temp.path().join("child-error.log")).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("子进程应完成部分真实下载并写出检查点");
    let checkpoint = tokio::fs::read_to_string(temp.path().join("ready"))
        .await
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert!(!temp.path().join("session/maobu-clean-shutdown").exists());
    child.0.kill().unwrap();
    assert!(!child.0.wait().unwrap().success());
    let mut task: DownloadTask = serde_json::from_slice(
        &tokio::fs::read(temp.path().join("checkpoint-task.json"))
            .await
            .unwrap(),
    )
    .unwrap();
    let engine = BtEngine::with_data_dir(temp.path().join("session"));
    let session = engine.get_or_init_session(None).await.unwrap();
    let handles = session.with_torrents(|items| items.map(|(_, h)| h.clone()).collect::<Vec<_>>());
    assert_eq!(handles.len(), 1);
    assert!(handles[0].is_paused());
    tokio::time::timeout(Duration::from_secs(10), handles[0].wait_until_initialized())
        .await
        .unwrap()
        .unwrap();
    assert!(
        handles[0].stats().progress_bytes >= checkpoint,
        "异常退出必须重检磁盘，不能只信任较旧bitv导致进度回退"
    );
    engine
        .add_task_inner(&AppSettings::default(), &mut task)
        .await
        .unwrap();
    engine
        .set_task_limits(&task.id, 1024 * 1024, 0)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), handles[0].wait_until_completed())
        .await
        .unwrap()
        .unwrap();
    engine.pause_task(&task.id).await.unwrap();
    let actual = tokio::fs::read(super::task_content_path(&task, None).await.unwrap())
        .await
        .unwrap();
    assert_eq!(actual.len(), swarm.expected.len());
    assert_eq!(Sha256::digest(actual), Sha256::digest(&swarm.expected));
    engine.shutdown().await.unwrap();
    assert!(temp.path().join("session/maobu-clean-shutdown").is_file());
    swarm.seeder.stop().await;
}
