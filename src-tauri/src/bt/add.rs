//! 元数据解析与稳定任务绑定，所有下载使用同一原生会话。
use super::limits::task_limits;
use super::selection::selected_indices;
use super::BtEngine;
use super::DEFAULT_PUBLIC_TRACKERS;
use crate::models::{AppSettings, DownloadTask};
use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, ManagedTorrent, Session};
use std::{path::Path, sync::Arc};

impl BtEngine {
    pub async fn add_task(
        &self,
        app: &tauri::AppHandle,
        settings: &AppSettings,
        task: &mut DownloadTask,
    ) -> Result<String, String> {
        let _ = app;
        self.add_task_inner(settings, task).await
    }

    pub(super) async fn add_task_inner(
        &self,
        settings: &AppSettings,
        task: &mut DownloadTask,
    ) -> Result<String, String> {
        self.prepare_task(settings, task, true).await
    }

    /// 仅恢复元数据和校验，不允许在 Completed 任务上悄悄补下载。
    pub(super) async fn prepare_task(
        &self,
        settings: &AppSettings,
        task: &mut DownloadTask,
        start: bool,
    ) -> Result<String, String> {
        let session = self.get_or_init_session(Some(settings)).await?;
        let binding_guard = self.registration.lock().await;
        let existing = self.bindings.lock().await.get(&task.id).cloned();
        if let Some(handle) = existing {
            self.configure_handle(&session, &handle, task, start)
                .await?;
            return Ok(handle.info_hash().as_string());
        }
        // 原 .torrent 文件可能已经被用户移走；持久会话才是续传的元数据来源。
        if let Some(handle) = self.restored_handle(&session, task) {
            self.bind_existing(&session, handle.clone(), task, start)
                .await?;
            return Ok(handle.info_hash().as_string());
        }
        drop(binding_guard);
        let source = self.task_source(task, settings).await?;
        let preview = session
            .add_torrent(
                source,
                Some(AddTorrentOptions {
                    list_only: true,
                    paused: true,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| format!("解析 BT 元数据失败：{e}"))?;
        let (torrent_bytes, info_hash, files, name) = match preview {
            AddTorrentResponse::ListOnly(preview) => {
                let files = preview
                    .info
                    .iter_file_details()
                    .map(|f| f.filename.to_string())
                    .collect::<Vec<_>>();
                (
                    preview.torrent_bytes,
                    preview.info_hash,
                    files,
                    preview.info.name().map(|n| n.into_owned()),
                )
            }
            AddTorrentResponse::AlreadyManaged(_, handle) => {
                let _registration = self.registration.lock().await;
                self.bind_existing(&session, handle.clone(), task, start)
                    .await?;
                return Ok(handle.info_hash().as_string());
            }
            _ => return Err("预解析 BT 元数据返回了意外状态".into()),
        };
        if task.bt_meta.as_ref().is_some_and(|meta| {
            !meta.info_hash.is_empty()
                && !meta.info_hash.eq_ignore_ascii_case(&info_hash.as_string())
        }) {
            return Err(
                "种子源内容与原任务身份不一致，已保留原下载数据；请为新种子新建任务".into(),
            );
        }
        for file in &files {
            super::paths::relative_path(file)?;
        }
        // JSON persistence 的 next_id/read/store 不是原子分配；只串行注册，元数据获取仍并发。
        let registration = self.registration.lock().await;
        super::paths::adopt_legacy_output(task, &files, name.as_deref()).await?;
        let output = super::paths::validate_output(task).await?;
        super::paths::validate_file_layout(&output, &files).await?;
        let meta = task.bt_meta.as_ref().ok_or("BT 任务缺少元数据")?;
        // 独占根属于本任务；一次原生初始化/持久化失败可能已预分配部分文件，重试须可重开。
        let allow_resume = meta.output_owned
            || !meta.owned_files.is_empty()
            || task.collision_policy == crate::models::CollisionPolicy::Overwrite;
        let options = AddTorrentOptions {
            output_folder: Some(output.to_string_lossy().into_owned()),
            paused: true,
            overwrite: allow_resume,
            only_files: selected_indices(&meta.selected_files, files.len())?,
            ratelimits: task_limits(task),
            ..Default::default()
        };
        // 已取得可信元数据；即使随后持久化失败，也保留目标清单供安全重试/删除。
        task.bt_meta
            .as_mut()
            .ok_or("BT 任务缺少元数据")?
            .owned_files = files;
        let response = session
            .add_torrent(AddTorrent::from_bytes(torrent_bytes), Some(options))
            .await
            .map_err(|e| format!("添加 BT 任务失败：{e}"))?;
        let handle = response
            .into_handle()
            .ok_or("添加 BT 任务没有返回有效句柄")?;
        if handle.info_hash() != info_hash {
            return Err("BT 种子身份发生变化，已停止下载".into());
        }
        self.bind_existing(&session, handle, task, start).await?;
        drop(registration);
        Ok(info_hash.as_string())
    }

    pub(super) fn restored_handle(
        &self,
        session: &Session,
        task: &DownloadTask,
    ) -> Option<Arc<ManagedTorrent>> {
        let hash = task
            .bt_meta
            .as_ref()
            .map(|m| m.info_hash.clone())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                super::magnet::parse_magnet(&task.url)
                    .ok()
                    .map(|m| m.info_hash)
            })?;
        session.with_torrents(|torrents| {
            for (_, handle) in torrents {
                if handle.info_hash().as_string().eq_ignore_ascii_case(&hash) {
                    return Some(handle.clone());
                }
            }
            None
        })
    }

    async fn task_source(
        &self,
        task: &DownloadTask,
        settings: &AppSettings,
    ) -> Result<AddTorrent<'static>, String> {
        if task.url.trim().to_ascii_lowercase().starts_with("magnet:") {
            return Ok(AddTorrent::from_url(Self::enrich_magnet_url(
                task.url.trim(),
                settings,
            )));
        }
        let bytes = match task
            .bt_meta
            .as_ref()
            .and_then(|m| m.torrent_data_base64.as_deref())
        {
            Some(encoded) => {
                use base64::Engine as _;
                base64::engine::general_purpose::STANDARD
                    .decode(encoded.trim())
                    .map_err(|e| format!("种子内容解码失败：{e}"))?
            }
            None => super::metainfo::read_torrent_file_async(Path::new(&task.url)).await?,
        };
        super::validate_torrent_bytes(&bytes)?;
        Ok(AddTorrent::from_bytes(bytes))
    }

    async fn bind_existing(
        &self,
        session: &Arc<Session>,
        handle: Arc<ManagedTorrent>,
        task: &mut DownloadTask,
        start: bool,
    ) -> Result<(), String> {
        let bindings = self.bindings.lock().await;
        if bindings
            .iter()
            .any(|(id, other)| id != &task.id && other.info_hash() == handle.info_hash())
        {
            return Err("相同种子已在另一个任务中管理，请使用原任务".into());
        }
        drop(bindings);
        if task.bt_meta.as_ref().is_some_and(|meta| {
            !meta.info_hash.is_empty()
                && !meta
                    .info_hash
                    .eq_ignore_ascii_case(&handle.info_hash().as_string())
        }) {
            return Err("恢复种子身份与原任务不一致，已停止操作".into());
        }
        let files = handle
            .with_metadata(|m| {
                m.info
                    .iter_file_details()
                    .map(|f| f.filename.to_string())
                    .collect::<Vec<_>>()
            })
            .map_err(|e| format!("读取 BT 文件清单失败：{e}"))?;
        let actual = tokio::fs::canonicalize(handle.output_folder())
            .await
            .map_err(|e| format!("读取 BT 输出目录失败：{e}"))?;
        if task
            .bt_meta
            .as_ref()
            .and_then(|m| m.output_folder.as_ref())
            .is_none()
        {
            let meta = task.bt_meta.as_mut().ok_or("BT 元数据缺失")?;
            meta.output_folder = Some(actual.to_string_lossy().into_owned());
            meta.output_owned = false;
        }
        let expected = super::paths::validate_output(task).await?;
        if actual != expected {
            return Err("该种子已绑定另一个输出目录，未改变原有文件".into());
        }
        let meta = task.bt_meta.as_mut().ok_or("BT 元数据缺失")?;
        meta.info_hash = handle.info_hash().as_string();
        meta.owned_files = files;
        self.register_sample(task, &handle).await;
        self.bindings
            .lock()
            .await
            .insert(task.id.clone(), handle.clone());
        self.configure_handle(session, &handle, task, start).await?;
        Ok(())
    }

    pub(super) async fn configure_handle(
        &self,
        session: &Arc<Session>,
        handle: &Arc<ManagedTorrent>,
        task: &DownloadTask,
        start: bool,
    ) -> Result<(), String> {
        handle.set_streaming_priority(
            task.bt_meta
                .as_ref()
                .is_some_and(|meta| meta.streaming_priority),
        );
        let limits = task_limits(task);
        handle.ratelimits().set_download_bps(limits.download_bps);
        handle.ratelimits().set_upload_bps(limits.upload_bps);
        let count = handle
            .with_metadata(|m| m.file_infos.len())
            .map_err(|e| format!("读取种子文件清单失败：{e}"))?;
        let indices = selected_indices(
            &task
                .bt_meta
                .as_ref()
                .map(|m| m.selected_files.clone())
                .unwrap_or_default(),
            count,
        )?
        .unwrap_or_else(|| (0..count).collect());
        let current = handle.only_files().unwrap_or_else(|| (0..count).collect());
        if current
            .into_iter()
            .collect::<std::collections::HashSet<_>>()
            != indices.iter().copied().collect()
        {
            session
                .update_only_files(handle, &indices.into_iter().collect())
                .await
                .map_err(|e| format!("更新 BT 文件选择失败：{e}"))?;
        }
        if start
            && (handle.is_paused()
                || matches!(handle.stats().state, librqbit::TorrentStatsState::Paused))
        {
            session
                .unpause(handle)
                .await
                .map_err(|e| format!("恢复 BT 任务失败：{e}"))?;
        }
        Ok(())
    }

    pub(super) fn enrich_magnet_url(original: &str, settings: &AppSettings) -> String {
        let mut url = original.to_string();
        let mut existing_lower = url.to_ascii_lowercase();

        // 追加内置公共 Trackers
        for tracker in DEFAULT_PUBLIC_TRACKERS {
            let encoded_tr: String =
                url::form_urlencoded::byte_serialize(tracker.as_bytes()).collect();
            let check_fragment = format!("tr={}", encoded_tr).to_ascii_lowercase();
            if !existing_lower.contains(&check_fragment)
                && !existing_lower.contains(&tracker.to_ascii_lowercase())
            {
                url.push_str("&tr=");
                url.push_str(&encoded_tr);
                existing_lower.push_str(&check_fragment);
            }
        }

        // 追加用户自定义 Trackers
        for line in settings.bt_extra_trackers.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                let encoded_tr: String =
                    url::form_urlencoded::byte_serialize(trimmed.as_bytes()).collect();
                url.push_str("&tr=");
                url.push_str(&encoded_tr);
            }
        }

        url
    }
}
