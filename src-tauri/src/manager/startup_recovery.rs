//! SQLite 检查点可能滞后文件落盘；恢复前只核验布局，不因长度不同删除合法前缀。
use super::*;

/// A SQLite checkpoint may still say zero while a durable prefix already exists.
/// Checking both layouts is conservative: ambiguous old data is preserved for an explicit decision.
pub(super) async fn has_existing_bytes(task: &DownloadTask, store: &Store) -> Result<bool, String> {
    let current = temp_layout::resolve(task).await?;
    let legacy = PathBuf::from(&task.destination).join(format!("{}.lumaget", task.file_name));
    let scan_legacy = !task_temp_dir(&task.destination, &task.id)
        .join("temp-layout.json")
        .exists()
        && store.get_task_name_intent(&task.id).await?.is_none();
    for path in [&current, &legacy]
        .into_iter()
        .take(if scan_legacy { 2 } else { 1 })
    {
        if optional_length(path)
            .await?
            .is_some_and(|length| length > 0)
        {
            return Ok(true);
        }
        let Some(parent) = path.parent() else {
            return Err("任务恢复路径无效".into());
        };
        let prefix = format!(
            "{}.part",
            path.file_name()
                .ok_or("任务恢复文件名无效")?
                .to_string_lossy()
        );
        let mut entries = match fs::read_dir(parent).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("检查已有分片失败：{error}")),
        };
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| format!("检查已有分片失败：{e}"))?
        {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let shard = name.strip_prefix(&prefix).is_some_and(|index| {
                !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
            });
            if (shard || is_window_part_name(&name, &prefix))
                && optional_length(&entry.path())
                    .await?
                    .is_some_and(|length| length > 0)
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub(super) async fn execute_selfcheck(store: &Store) -> Result<SelfcheckReport, String> {
    let mut report = SelfcheckReport::default();
    for mut task in store.list_tasks().await? {
        if task.status == TaskStatus::Verifying {
            task.status = TaskStatus::Failed;
            task.speed = 0;
            task.eta_seconds = None;
            task.active_connections = 0;
            task.error =
                Some("上次文件校验意外中断，已有文件未改动；请在任务菜单重新校验文件".into());
            store.upsert_task(&task).await?;
            report.interrupted_count += 1;
            continue;
        }
        if task.status != TaskStatus::Downloading {
            continue;
        }
        task.status = TaskStatus::Interrupted;
        task.speed = 0;
        task.eta_seconds = None;
        task.active_connections = 0;
        if let Err(error) = reconcile_http_bytes(&mut task, store).await {
            task.status = TaskStatus::Failed;
            task.error = Some(format!(
                "启动恢复检查失败，原分片已全部保留，请检查保存位置或明确重新下载：{error}"
            ));
        }
        store.upsert_task(&task).await?;
        report.interrupted_count += 1;
        if task.status == TaskStatus::Interrupted {
            report.recovered_tasks.push(task.id);
        }
    }
    Ok(report)
}

async fn reconcile_http_bytes(task: &mut DownloadTask, store: &Store) -> Result<(), String> {
    for segment in &mut task.segments {
        if segment.status == "downloading" {
            segment.status = "pending".into();
        }
    }
    // 这些引擎各自核验真实快照/哈希/清单，不能套用普通HTTP分片命名。
    if task.task_kind == TaskKind::Bt
        || (task.media.is_some() && task.segments.is_empty())
        || task.url.contains(".m3u8")
        || task
            .content_type
            .as_deref()
            .is_some_and(|mime| mime.contains("mpegurl"))
        || task_temp_dir(&task.destination, &task.id)
            .join("landisk_manifest.json")
            .is_file()
    {
        return Ok(());
    }
    temp_layout::validate_ancestors(task).await?;
    if task_temp_dir(&task.destination, &task.id)
        .join("task-publication.json")
        .exists()
    {
        return Ok(());
    }
    let path = temp_layout::ensure_owned(task, store).await?;
    let (first, has_shards) = segment_bytes(&path, 0).await?;
    let single = task.segments.len() == 1 && task.segments[0].index == 0 && !has_shards;
    let mut total = 0u64;
    for segment in &mut task.segments {
        let actual = if single {
            optional_length(&path).await?.unwrap_or(0)
        } else if segment.index == 0 {
            first
        } else {
            segment_bytes(&path, segment.index).await?.0
        };
        let expected = segment
            .end_byte
            .checked_sub(segment.start_byte)
            .and_then(|length| length.checked_add(1))
            .ok_or("分片布局无效")?;
        if task.total_bytes > 0 && actual > expected {
            return Err(format!(
                "分片{}长度超出原布局，未自动删除或截断",
                segment.index + 1
            ));
        }
        segment.downloaded_bytes = actual;
        segment.status = "pending".into();
        total = total.checked_add(actual).ok_or("分片进度超过可支持范围")?;
    }
    if !task.segments.is_empty() {
        task.downloaded_bytes = total;
    }
    Ok(())
}

async fn optional_length(path: &Path) -> Result<Option<u64>, String> {
    match fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            Ok(Some(metadata.len()))
        }
        Ok(_) => Err("分片路径不是普通文件，未继续文件操作".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("读取分片属性失败：{error}")),
    }
}

async fn segment_bytes(temp: &Path, index: u8) -> Result<(u64, bool), String> {
    let legacy = PathBuf::from(format!("{}.part{index}", temp.to_string_lossy()));
    let length = optional_length(&legacy).await?;
    let mut total = length.unwrap_or(0);
    let mut present = length.is_some();
    let Some(parent) = temp.parent() else {
        return Err("分片目录无效".into());
    };
    let prefix = format!(
        "{}.part{index}.w",
        temp.file_name().ok_or("分片名无效")?.to_string_lossy()
    );
    let mut entries = match fs::read_dir(parent).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((total, present)),
        Err(error) => return Err(format!("读取分片目录失败：{error}")),
    };
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("读取分片清单失败：{e}"))?
    {
        let name = entry.file_name();
        if name
            .to_string_lossy()
            .strip_prefix(&prefix)
            .is_some_and(|offset| !offset.is_empty() && offset.bytes().all(|b| b.is_ascii_digit()))
        {
            total = total
                .checked_add(optional_length(&entry.path()).await?.unwrap_or(0))
                .ok_or("分片长度溢出")?;
            present = true;
        }
    }
    Ok((total, present))
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn fixture() -> (tempfile::TempDir, Store, DownloadTask) {
        let directory = tempfile::tempdir().unwrap();
        let data = directory.path().join("data");
        let store = tokio::task::spawn_blocking(move || Store::open(data))
            .await
            .unwrap()
            .unwrap();
        let mut task = crate::bt::tests::bt_task_for_test();
        task.task_kind = TaskKind::Http;
        task.destination = directory.path().to_string_lossy().into_owned();
        task.file_name = "download.bin".into();
        task.status = TaskStatus::Downloading;
        task.total_bytes = 100;
        task.downloaded_bytes = 10;
        task.segments = vec![DownloadSegment {
            index: 0,
            start_byte: 0,
            end_byte: 99,
            downloaded_bytes: 10,
            status: "downloading".into(),
        }];
        (directory, store, task)
    }
    #[tokio::test]
    async fn disk_ahead_of_sqlite_and_renamed_output_keep_the_actual_prefix() {
        let (_directory, store, mut task) = fixture().await;
        let source = temp_layout::ensure(&task).await.unwrap();
        fs::write(&source, vec![7u8; 60]).await.unwrap();
        task.file_name = "renamed.bin".into();
        store.upsert_task(&task).await.unwrap();
        execute_selfcheck(&store).await.unwrap();
        let restored = store.get_task(&task.id).await.unwrap().unwrap();
        assert_eq!(restored.status, TaskStatus::Interrupted);
        assert_eq!(restored.downloaded_bytes, 60);
        assert_eq!(fs::read(source).await.unwrap(), vec![7u8; 60]);
    }
    #[tokio::test]
    async fn oversized_shard_is_flagged_without_destroying_the_evidence() {
        let (_directory, store, task) = fixture().await;
        let source = temp_layout::ensure(&task).await.unwrap();
        fs::write(&source, vec![9u8; 101]).await.unwrap();
        store.upsert_task(&task).await.unwrap();
        execute_selfcheck(&store).await.unwrap();
        let restored = store.get_task(&task.id).await.unwrap().unwrap();
        assert_eq!(restored.status, TaskStatus::Failed);
        assert!(restored.error.unwrap().contains("保留"));
        assert_eq!(fs::read(source).await.unwrap(), vec![9u8; 101]);
    }
    #[tokio::test]
    async fn bt_verified_progress_is_not_reinterpreted_as_empty_http_shards() {
        let (_directory, store, mut task) = fixture().await;
        task.task_kind = TaskKind::Bt;
        task.downloaded_bytes = 42;
        task.segments.clear();
        store.upsert_task(&task).await.unwrap();
        execute_selfcheck(&store).await.unwrap();
        assert_eq!(
            store
                .get_task(&task.id)
                .await
                .unwrap()
                .unwrap()
                .downloaded_bytes,
            42
        );
    }
    #[tokio::test]
    async fn interrupted_verification_preserves_completed_file_and_never_auto_queues_a_download() {
        let (_root, store, mut task) = fixture().await;
        task.status = TaskStatus::Verifying;
        let output = PathBuf::from(&task.destination).join(&task.file_name);
        fs::write(&output, b"completed user file").await.unwrap();
        store.upsert_task(&task).await.unwrap();
        execute_selfcheck(&store).await.unwrap();
        let restored = store.get_task(&task.id).await.unwrap().unwrap();
        assert_eq!(restored.status, TaskStatus::Failed);
        assert!(restored.error.unwrap().contains("重新校验"));
        assert_eq!(fs::read(output).await.unwrap(), b"completed user file");
    }
}
