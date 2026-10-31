//! 新建任务先持久化真实种子身份和内容，不依赖下一次轮询或原种子路径。
use super::*;
use base64::Engine as _;

#[derive(Debug)]
struct BtSource { url: String, name: String, meta: BtTaskMeta, total_bytes: u64 }

async fn parse_bt_source(request: &BtNewTaskRequest) -> Result<BtSource, String> {
    let source = request.source.trim();
    if source.is_empty() { return Err("请输入 magnet: 磁力链接或选择 .torrent 种子文件".into()); }
    if request.selected_files.contains(&0) { return Err("BT 文件索引从1开始，请重新选择文件".into()); }
    let inline = request.source_data_base64.as_deref().map(str::trim).filter(|data| !data.is_empty());
    if inline.is_none() && source.to_ascii_lowercase().starts_with("magnet:") {
        let info = crate::bt::magnet::parse_magnet(source)?;
        return Ok(BtSource { url: source.to_owned(), name: info.display_name.clone().unwrap_or_default(), total_bytes: 0,
            meta: BtTaskMeta { info_hash: info.info_hash, display_name: info.display_name,
                selected_files: request.selected_files.clone(), streaming_priority: request.streaming_priority,
                ..Default::default() } });
    }
    let bytes = read_torrent_bytes(source, inline).await?;
    let inspected = crate::bt::inspect_torrent_bytes(&bytes)?;
    let selected: HashSet<_> = request.selected_files.iter().copied().collect();
    if selected.iter().any(|index| *index as usize > inspected.files.len()) {
        return Err("所选 BT 文件索引超出种子清单范围".into());
    }
    let total_bytes = inspected.files.iter().filter(|file| selected.is_empty() || selected.contains(&file.index))
        .try_fold(0u64, |sum,file| sum.checked_add(file.length_bytes).ok_or("BT 文件总大小超过可支持范围"))?;
    Ok(BtSource { url: source.to_owned(), name: inspected.name.clone(), total_bytes,
        meta: BtTaskMeta { info_hash: inspected.info_hash, display_name: Some(inspected.name), metadata_ready: true,
            selected_files: request.selected_files.clone(), streaming_priority: request.streaming_priority,
            torrent_data_base64: Some(base64::engine::general_purpose::STANDARD.encode(bytes)), ..Default::default() } })
}

async fn read_torrent_bytes(source: &str, inline: Option<&str>) -> Result<Vec<u8>, String> {
    const MAX: usize = 20 * 1024 * 1024;
    let bytes = if let Some(data) = inline {
        if data.len() > (MAX + 2) / 3 * 4 { return Err("种子文件超过20MiB限制".into()); }
        base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| "种子文件内容无效（base64解码失败）")?
    } else {
        let file = fs::File::open(source).await.map_err(|e| format!("读取种子文件失败：{e}"))?;
        let mut bytes = Vec::new();
        file.take((MAX + 1) as u64).read_to_end(&mut bytes).await.map_err(|e| format!("读取种子文件失败：{e}"))?;
        bytes
    };
    if bytes.len() > MAX { return Err("种子文件超过20MiB限制".into()); }
    Ok(bytes)
}

impl DownloadManager {
    pub async fn add_bt(&self, request: BtNewTaskRequest) -> Result<DownloadTask, String> {
        let settings = self.settings().await;
        let source = parse_bt_source(&request).await?;
        let BtSource { url, name: initial_name, meta, total_bytes } = source;
        let default_dir = normalize_directory(&settings.download_dir);
        let default_dir = if default_dir.trim().is_empty() {
            crate::system_dirs::system_download_dir().to_string_lossy().to_string()
        } else {
            default_dir
        };
        let destination = match request.destination.as_deref().map(str::trim) {
            Some(dir) if !dir.is_empty() => {
                let p = Path::new(dir);
                if p.is_absolute() {
                    normalize_directory(dir)
                } else {
                    let joined = Path::new(&default_dir).join(p);
                    normalize_directory(&joined.to_string_lossy())
                }
            }
            _ => default_dir,
        };
        let display_name_for_file = safe_name(&initial_name);
        let mut task = DownloadTask {
            id: Uuid::new_v4().to_string(),
            url,
            file_name: display_name_for_file,
            destination,
            total_bytes,
            downloaded_bytes: 0,
            speed: 0,
            eta_seconds: None,
            status: if request.start_paused {
                TaskStatus::Paused
            } else {
                TaskStatus::Queued
            },
            error: None,
            created_at: now(),
            completed_at: None,
            scheduled_at: None,
            category: category(&initial_name),
            queue_position: self.store.next_queue_position().await?,
            priority: 0,
            retry_count: 0,
            max_retries: settings.max_retries,
            checksum_sha256: None,
            expected_checksum: None,
            source: request.source_tag.unwrap_or_else(|| "desktop".into()),
            etag: None,
            last_modified: None,
            final_url: None,
            response_status: None,
            content_type: None,
            accepts_ranges: None,
            headers: HashMap::new(),
            media: None,
            per_task_speed_limit: 0,
            collision_policy: CollisionPolicy::Rename,
            completion_action: CompletionAction::None,
            connection_count: 1,
            active_connections: 0,
            segments: Vec::new(),
            retry_policy_override: None,
            proxy_override: None,
            proxy_auth: None,
            task_kind: TaskKind::Bt,
            bt_meta: Some(meta),
            bt_runtime: None,
            cloud_refresh: None,
            method: None,
            body: None,
        };
        crate::bt::prepare_task_output(&mut task).await?;
        self.store.upsert_task(&task).await?;
        self.register_power_action_target(&task.id).await;
        self.emit_task("created", &task);
        self.dispatcher.notify_waiters();
        Ok(task)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    fn torrent() -> Vec<u8> {
        use sha1::Digest;
        let mut data = b"d4:infod6:lengthi4e4:name8:file.bin12:piece lengthi4e6:pieces20:".to_vec();
        data.extend_from_slice(&sha1::Sha1::digest(b"DATA"));
        data.extend_from_slice(b"ee");
        data
    }
    #[tokio::test]
    async fn known_torrent_identity_and_content_are_persistable_before_a_worker_starts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.torrent");
        let bytes = torrent();
        fs::write(&path, &bytes).await.unwrap();
        let local = parse_bt_source(&BtNewTaskRequest { source: path.to_string_lossy().into_owned(), ..Default::default() }).await.unwrap();
        assert_eq!(local.meta.info_hash.len(), 40);
        assert_eq!(local.name, "file.bin");
        assert_eq!(local.total_bytes, 4);
        fs::remove_file(path).await.unwrap();
        let restored = parse_bt_source(&BtNewTaskRequest { source: "moved.torrent".into(),
            source_data_base64: local.meta.torrent_data_base64.clone(), ..Default::default() }).await.unwrap();
        assert_eq!(restored.meta.info_hash, local.meta.info_hash);
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(restored.meta.torrent_data_base64.unwrap()).unwrap(), bytes);
    }
    #[tokio::test]
    async fn invalid_torrent_or_nonexistent_selected_file_is_rejected_before_creation() {
        let mut request = BtNewTaskRequest { source:"source.torrent".into(),
            source_data_base64:Some(base64::engine::general_purpose::STANDARD.encode(torrent())),
            selected_files:vec![2], ..Default::default() };
        assert!(parse_bt_source(&request).await.unwrap_err().contains("索引"));
        request.selected_files.clear();
        request.source_data_base64 = Some(base64::engine::general_purpose::STANDARD.encode(b"d invalid torrent data"));
        assert!(parse_bt_source(&request).await.is_err());
    }
}
