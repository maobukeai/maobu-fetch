//! 暂停、删除与关闭：错误必须传播，数据清理另走归属验证。
use super::BtEngine;

impl BtEngine {
    /// 仅供确认桌面端离线的 CLI 使用：不创建 Session，也不访问下载内容。
    pub async fn remove_task_offline(
        &self,
        task: &crate::models::DownloadTask,
    ) -> Result<(), String> {
        if self.session.lock().await.is_some() {
            return Err("BT 会话正在运行，请使用在线任务删除入口".into());
        }
        let directory = self.resolve_data_dir();
        if !tokio::fs::try_exists(directory.join("session.json"))
            .await
            .map_err(|e| format!("读取 BT 会话状态失败：{e}"))?
        {
            return Ok(());
        }
        let hash = if let Some(hash) = task
            .bt_meta
            .as_ref()
            .map(|m| m.info_hash.clone())
            .filter(|h| !h.is_empty())
        {
            hash
        } else if let Ok(magnet) = super::magnet::parse_magnet(&task.url) {
            magnet.info_hash
        } else if let Some(encoded) = task
            .bt_meta
            .as_ref()
            .and_then(|m| m.torrent_data_base64.as_deref())
        {
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|e| format!("读取种子内容失败：{e}"))?;
            super::inspect_torrent_bytes(&bytes)?.info_hash
        } else {
            return Err(
                "旧 BT 任务尚无种子身份，无法安全移除持久会话；请启动桌面端恢复元数据后再试".into(),
            );
        };
        let hash = hash.parse().map_err(|e| format!("BT 种子身份无效：{e}"))?;
        librqbit::Session::remove_persisted_torrent(directory, hash)
            .await
            .map_err(|e| format!("移除 BT 持久会话失败，已保留任务记录：{e}"))
    }

    /// 记录删除也要清理重启后尚未绑定的持久 torrent；不要求旧源/数据文件仍存在。
    pub async fn remove_task_for(&self, task: &crate::models::DownloadTask) -> Result<(), String> {
        self.removal_guard().await.remove_task_for(task).await
    }

    pub(super) async fn remove_task_for_unlocked(
        &self,
        task: &crate::models::DownloadTask,
    ) -> Result<(), String> {
        let _registration = self.registration.lock().await;
        let cold = self.session.lock().await.is_none();
        if cold {
            return self.remove_task_offline(task).await;
        }
        if !self.bindings.lock().await.contains_key(&task.id) {
            let session = self.get_or_init_session(None).await?;
            if let Some(handle) = self.restored_handle(&session, task) {
                let mut bindings = self.bindings.lock().await;
                if bindings
                    .iter()
                    .any(|(id, other)| id != &task.id && other.info_hash() == handle.info_hash())
                {
                    return Err("相同种子仍属于另一条活动任务，请先停止原任务".into());
                }
                bindings.insert(task.id.clone(), handle);
            }
        }
        self.remove_task_inner(&task.id).await
    }

    pub async fn pause_task(&self, task_id: &str) -> Result<(), String> {
        self.cancel_seed_preparation(Some(task_id));
        self.pause_task_for_preparation(task_id).await
    }

    pub(super) async fn pause_task_for_preparation(&self, task_id: &str) -> Result<(), String> {
        let _registration = self.registration.lock().await;
        self.pause_task_inner(task_id).await
    }

    async fn pause_task_inner(&self, task_id: &str) -> Result<(), String> {
        let monitor = self.seed_monitors.lock().await.get(task_id).cloned();
        let session = self.session.lock().await.clone();
        let handle = self.bindings.lock().await.get(task_id).cloned();
        if let (Some(session), Some(handle)) = (session, handle) {
            let live = handle.live();
            session
                .pause(&handle)
                .await
                .map_err(|e| format!("暂停 BT 或保存恢复信息失败：{e}"))?;
            if let (Some(live), Some(sample)) =
                (live, self.speed_samples.lock().await.get_mut(task_id))
            {
                sample.total_uploaded = sample.total_uploaded.saturating_add(
                    live.get_uploaded_bytes()
                        .saturating_sub(sample.last_uploaded),
                );
                sample.last_uploaded = 0;
            }
        }
        if let Some((generation, token)) = monitor {
            let mut monitors = self.seed_monitors.lock().await;
            if monitors
                .get(task_id)
                .map(|(current, _)| current == &generation)
                .unwrap_or(false)
            {
                monitors.remove(task_id);
                token.cancel();
            }
        }
        Ok(())
    }

    pub async fn remove_task(&self, task_id: &str) -> Result<(), String> {
        self.removal_guard().await.remove_task(task_id).await
    }

    pub(super) async fn remove_task_unlocked(&self, task_id: &str) -> Result<(), String> {
        let _registration = self.registration.lock().await;
        self.remove_task_inner(task_id).await
    }

    async fn remove_task_inner(&self, task_id: &str) -> Result<(), String> {
        self.pause_task_inner(task_id).await?;
        let session = self.session.lock().await.clone();
        let handle = self.bindings.lock().await.get(task_id).cloned();
        if let (Some(session), Some(handle)) = (session, handle) {
            session
                .delete(handle.info_hash().into(), false)
                .await
                .map_err(|e| format!("移除 BT 引擎任务失败：{e}"))?;
        }
        self.bindings.lock().await.remove(task_id);
        self.speed_samples.lock().await.remove(task_id);
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<(), String> {
        let _preparation = self.removal_guard().await;
        let session = self.session.lock().await.clone();
        if session.is_none() {
            return Ok(());
        }
        let mut failures = Vec::new();
        if let Some(session) = session {
            let ids = self
                .bindings
                .lock()
                .await
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            for id in ids {
                if let Err(error) = self.pause_task(&id).await {
                    failures.push(error);
                }
            }
            let initializing = session.with_torrents(|items| {
                items
                    .filter_map(|(_, handle)| {
                        matches!(
                            handle.stats().state,
                            librqbit::TorrentStatsState::Initializing { .. }
                        )
                        .then(|| handle.clone())
                    })
                    .collect::<Vec<_>>()
            });
            for handle in initializing {
                if let Err(error) = session.pause(&handle).await {
                    failures.push(format!("停止 BT 初始校验失败：{error}"));
                }
            }
            session.stop().await;
            self.session.lock().await.take();
            self.bindings.lock().await.clear();
            self.speed_samples.lock().await.clear();
        }
        if failures.is_empty() {
            use tokio::io::AsyncWriteExt;
            let mut marker =
                tokio::fs::File::create(self.resolve_data_dir().join("maobu-clean-shutdown"))
                    .await
                    .map_err(|e| format!("BT 已停止，但保存正常退出标记失败：{e}"))?;
            marker
                .write_all(b"1")
                .await
                .map_err(|e| format!("写入 BT 退出标记失败：{e}"))?;
            marker
                .sync_all()
                .await
                .map_err(|e| format!("刷新 BT 退出标记失败：{e}"))?;
            Ok(())
        } else {
            Err(format!(
                "BT 已停止，但部分恢复信息保存失败：{}",
                failures.join("；")
            ))
        }
    }
}
