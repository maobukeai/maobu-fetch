//! 从原生已校验分片读取状态，绝不从本地文件长度伪造进度。
use super::{engine::SpeedSample, status, BtEngine};
use crate::models::{BtFileEntry, DownloadTask};
use librqbit::ManagedTorrent;
use std::sync::Arc;

impl BtEngine {
    pub(super) async fn register_sample(&self, task: &DownloadTask, handle: &Arc<ManagedTorrent>) {
        let stats = handle.stats();
        let history = task.bt_meta.as_ref().map(|m| m.uploaded_bytes).unwrap_or(0);
        self.speed_samples
            .lock()
            .await
            .entry(task.id.clone())
            .and_modify(|sample| sample.total_uploaded = sample.total_uploaded.max(history))
            .or_insert(SpeedSample {
                last_uploaded: stats.uploaded_bytes,
                total_uploaded: history.saturating_add(stats.uploaded_bytes),
            });
    }

    pub async fn status_of(
        &self,
        task_id: &str,
        _base_dir: &str,
    ) -> Result<status::BtProgress, String> {
        let handle = {
            let bindings = self.bindings.lock().await;
            bindings.get(task_id).cloned()
        }
        .ok_or_else(|| "BT 任务尚未注册".to_string())?;

        let stats = handle.stats();
        let info_hash = handle.info_hash().as_string();
        let metadata_fetching = handle.metadata.load().is_none();
        // initializing.progress_bytes 表示已检查字节，并非通过哈希的下载进度。
        let verified_progress = if matches!(
            stats.state,
            librqbit::TorrentStatsState::Initializing { .. }
        ) {
            0
        } else {
            stats.progress_bytes
        };

        let num_peers = stats
            .live
            .as_ref()
            .map(|l| l.snapshot.peer_stats.live)
            .unwrap_or(0);
        // 该版本公开统计不区分完整 bitfield peers，未知不可伪造。
        let num_seeds = 0;
        // 上传累计跨暂停/恢复保留；速度直接取真实网络估计器，重检不计为网络传输。
        let mut samples = self.speed_samples.lock().await;
        let sample = samples
            .entry(task_id.to_string())
            .or_insert_with(|| SpeedSample {
                last_uploaded: stats.uploaded_bytes,
                total_uploaded: stats.uploaded_bytes,
            });
        if stats.live.is_some() {
            let up_diff = if stats.uploaded_bytes >= sample.last_uploaded {
                stats.uploaded_bytes - sample.last_uploaded
            } else {
                stats.uploaded_bytes
            };
            sample.total_uploaded = sample.total_uploaded.saturating_add(up_diff);
            sample.last_uploaded = stats.uploaded_bytes;
        }
        let download_speed = if !stats.finished {
            stats
                .live
                .as_ref()
                .map(|live| live.download_speed.as_bytes())
                .unwrap_or(0)
        } else {
            0
        };
        let upload_speed = stats
            .live
            .as_ref()
            .map(|live| live.upload_speed.as_bytes())
            .unwrap_or(0);
        let uploaded_bytes = sample.total_uploaded;

        let only_files = handle.only_files();
        let (display_name, files) = match handle.with_metadata(|metadata| {
            let files = metadata
                .info
                .iter_file_details()
                .enumerate()
                .map(|(index, file)| BtFileEntry {
                    index: (index + 1) as u32,
                    path: file.filename.to_string(),
                    length_bytes: file.len,
                    selected: only_files
                        .as_ref()
                        .map(|selected| selected.contains(&index))
                        .unwrap_or(true),
                    downloaded_bytes: stats
                        .file_progress
                        .get(index)
                        .copied()
                        .unwrap_or(0)
                        .min(file.len),
                })
                .collect::<Vec<_>>();
            (metadata.info.name().map(|n| n.into_owned()), files)
        }) {
            Ok(value) => value,
            Err(_) if metadata_fetching => (None, Vec::new()),
            Err(error) => return Err(format!("读取 BT 元数据失败：{error}")),
        };
        let is_error = matches!(stats.state, librqbit::TorrentStatsState::Error);
        let err_msg = stats.error.clone();
        let progress = status::BtProgress {
            state: if is_error {
                "error".into()
            } else if stats.finished {
                "complete".into()
            } else if matches!(
                stats.state,
                librqbit::TorrentStatsState::Paused
                    | librqbit::TorrentStatsState::Initializing { paused: true }
            ) {
                "paused".into()
            } else {
                "active".into()
            },
            total_bytes: stats.total_bytes,
            downloaded_bytes: verified_progress,
            upload_speed,
            download_speed,
            num_seeds,
            seeds_known: false,
            num_peers,
            info_hash,
            display_name,
            seeder: stats.finished && matches!(stats.state, librqbit::TorrentStatsState::Live),
            metadata_fetching,
            uploaded_bytes,
            error: err_msg,
            files,
            output_folder: Some(handle.output_folder().to_string_lossy().into_owned()),
        };

        Ok(progress)
    }

    pub async fn files_of(
        &self,
        task_id: &str,
        base_dir: &str,
    ) -> Result<Vec<BtFileEntry>, String> {
        let progress = self.status_of(task_id, base_dir).await?;
        if progress.metadata_fetching {
            return Err("BT_METADATA_PENDING: 磁力元数据尚未获取，请稍后再试".into());
        }
        Ok(progress.files)
    }

    pub async fn files_for_task(&self, task: &DownloadTask) -> Result<Vec<BtFileEntry>, String> {
        self.resolve_task_paths_inner(task, true).await?;
        self.files_of(&task.id, &task.destination).await
    }

    /// 仅从已恢复的暂停会话补齐可信归属，不读取旧种子源、不启动数据连接。
    pub async fn resolve_task_paths(&self, task: &DownloadTask) -> Result<DownloadTask, String> {
        self.resolve_task_paths_inner(task, false).await
    }

    async fn resolve_task_paths_inner(
        &self,
        task: &DownloadTask,
        bind: bool,
    ) -> Result<DownloadTask, String> {
        if !bind {
            if let Some(meta) = task
                .bt_meta
                .as_ref()
                .filter(|meta| meta.output_folder.is_some())
            {
                let root = super::paths::validate_output(task).await?;
                super::paths::validate_file_layout(&root, &meta.owned_files).await?;
                if !meta.owned_files.is_empty() {
                    return Ok(task.clone());
                }
                if meta.output_owned {
                    let mut entries = tokio::fs::read_dir(root)
                        .await
                        .map_err(|e| format!("读取 BT 输出目录失败：{e}"))?;
                    if entries
                        .next_entry()
                        .await
                        .map_err(|e| format!("读取 BT 文件清单失败：{e}"))?
                        .is_none()
                    {
                        return Ok(task.clone());
                    }
                }
            }
        }
        let existing = self.bindings.lock().await.get(&task.id).cloned();
        let handle = match existing {
            Some(handle) => handle,
            None => {
                let session = self.get_or_init_session(None).await?;
                self.restored_handle(&session, task)
                    .ok_or("BT_METADATA_PENDING: 恢复会话中尚无该种子，请先继续任务获取元数据")?
            }
        };
        let root = tokio::fs::canonicalize(handle.output_folder())
            .await
            .map_err(|e| format!("读取 BT 输出根失败：{e}"))?;
        let mut resolved = task.clone();
        let meta = resolved.bt_meta.as_mut().ok_or("BT 任务缺少元数据")?;
        let hash = handle.info_hash().as_string();
        if !meta.info_hash.is_empty() && !meta.info_hash.eq_ignore_ascii_case(&hash) {
            return Err("恢复会话的种子身份与任务不一致".into());
        }
        if meta.output_folder.is_none() {
            meta.output_folder = Some(root.to_string_lossy().into_owned());
            meta.output_owned = false;
        }
        meta.info_hash = hash;
        meta.owned_files = handle
            .with_metadata(|m| {
                m.info
                    .iter_file_details()
                    .map(|file| file.filename.to_string())
                    .collect()
            })
            .map_err(|e| format!("读取种子文件清单失败：{e}"))?;
        if super::paths::validate_output(&resolved).await? != root {
            return Err("BT 会话与任务的实际输出根不一致".into());
        }
        super::paths::validate_file_layout(
            &root,
            &resolved
                .bt_meta
                .as_ref()
                .ok_or("BT 元数据缺失")?
                .owned_files,
        )
        .await?;
        self.register_sample(&resolved, &handle).await;
        let mut bindings = self.bindings.lock().await;
        if bindings
            .iter()
            .any(|(id, h)| id != &task.id && h.info_hash() == handle.info_hash())
        {
            return Err("该种子已属于另一条任务，不能绑定其文件".into());
        }
        bindings.insert(task.id.clone(), handle);
        Ok(resolved)
    }
}
