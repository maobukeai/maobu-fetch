//! Hold reconciliation exclusion until the caller commits its record removal or archive transition.
use super::BtEngine;
use crate::models::DownloadTask;
use tokio::sync::MutexGuard;

pub struct BtRemovalGuard<'a> {
    engine: &'a BtEngine,
    _reconciliation: MutexGuard<'a, ()>,
}

impl BtEngine {
    /// Lock order: task-operation (manager), reconciliation, torrent registration.
    /// Keep this lease through the caller's database commit, not merely the native delete call.
    pub async fn removal_guard(&self) -> BtRemovalGuard<'_> {
        let waiting = self.seed_sync.lock();
        tokio::pin!(waiting);
        loop {
            self.cancel_seed_preparation(None);
            tokio::select! {
                biased;
                lease = &mut waiting => return BtRemovalGuard { engine: self, _reconciliation: lease },
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
            }
        }
    }
}

impl BtRemovalGuard<'_> {
    pub async fn remove_task_for(&self, task: &DownloadTask) -> Result<(), String> {
        self.engine.remove_task_for_unlocked(task).await
    }
    pub async fn remove_task(&self, task_id: &str) -> Result<(), String> {
        self.engine.remove_task_unlocked(task_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, time::Duration};
    #[tokio::test]
    async fn removal_lease_excludes_reconciliation_through_the_callers_database_window() {
        let engine = Arc::new(BtEngine::new());
        let lease = engine.removal_guard().await;
        let second = engine.clone();
        let waiting = tokio::spawn(async move {
            let _reconcile = second.seed_sync.lock().await;
        });
        tokio::task::yield_now().await;
        // Even a no-op native removal must not release the caller's record-mutation lease.
        lease.remove_task("not-registered").await.unwrap();
        assert!(!waiting.is_finished());
        drop(lease);
        tokio::time::timeout(Duration::from_millis(100), waiting)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn prepared_paused_binding_can_be_cleaned_inside_reconciliation_without_relocking() {
        use base64::Engine as _;
        use sha1::Digest;
        let root = tempfile::tempdir().unwrap();
        let mut task = super::super::tests::bt_task_for_test();
        task.destination = root.path().join("downloads").to_string_lossy().into_owned();
        task.url = "isolated-removal.torrent".into();
        let mut bytes =
            b"d4:infod6:lengthi4e4:name8:file.bin12:piece lengthi4e6:pieces20:".to_vec();
        bytes.extend_from_slice(&sha1::Sha1::digest(b"DATA"));
        bytes.extend_from_slice(b"ee");
        let meta = task.bt_meta.as_mut().unwrap();
        meta.info_hash = crate::bt::inspect_torrent_bytes(&bytes).unwrap().info_hash;
        meta.torrent_data_base64 = Some(base64::engine::general_purpose::STANDARD.encode(bytes));
        crate::bt::prepare_task_output(&mut task).await.unwrap();
        let engine = BtEngine::with_data_dir(root.path().join("session"));
        engine
            .prepare_task(&Default::default(), &mut task, false)
            .await
            .unwrap();
        let native = engine.bindings.lock().await.get(&task.id).unwrap().clone();
        tokio::time::timeout(Duration::from_secs(5), native.wait_until_initialized())
            .await
            .unwrap()
            .unwrap();
        assert!(native.is_paused());
        let _reconciliation = engine.seed_sync.lock().await;
        tokio::time::timeout(
            Duration::from_secs(2),
            engine.remove_task_for_unlocked(&task),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(engine.bindings.lock().await.is_empty());
        assert!(engine
            .restored_handle(&engine.session.lock().await.clone().unwrap(), &task)
            .is_none());
        drop(_reconciliation);
        engine.shutdown().await.unwrap();
    }
}
