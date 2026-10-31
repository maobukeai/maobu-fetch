use super::*;

pub(super) struct Plan {
    pub ranges: Vec<(u8, u64, u64)>,
    pub initial: u64,
    pub coordinator: Arc<WorkStealingCoordinator>,
    pub runtimes: Arc<Vec<SegmentRuntime>>,
}

/// Keep the persisted logical layout and resume each physical window at its exact file length.
pub(super) async fn prepare(
    task: &DownloadTask,
    temp: &Path,
    total: u64,
    connections: u8,
) -> Result<Plan, String> {
    let ranges = planned_segment_ranges(task, total, connections);
    let mut initial = 0u64;
    let mut initial_windows = Vec::new();
    let mut window_id_counter = 1u64;
    let mut runtimes = Vec::with_capacity(ranges.len());

    for &(index, start, end) in &ranges {
        let legacy_part = PathBuf::from(format!("{}.part{index}", temp.to_string_lossy()));
        let expected = end - start + 1;
        let prefix_bytes = existing_length(&legacy_part).await?;
        if prefix_bytes > expected {
            return Err(format!(
                "分片 #{} 超出原规划长度，已保留原始数据，请重新下载或检查分片",
                index + 1
            ));
        }
        let mut downloaded = prefix_bytes;
        if prefix_bytes < expected {
            let layout =
                select_window_layout(temp, index, start + prefix_bytes, end, prefix_bytes).await?;
            for &(ordinal, window_start, window_end) in &layout {
                let path = window_part_path(temp, index, window_start);
                quarantine::ensure_usable(&path).await?;
                let expected_window = window_end - window_start + 1;
                let existing_bytes = existing_length(&path).await?;
                if existing_bytes > expected_window {
                    return Err(format!(
                        "续接窗口 #{} 超出原规划长度，已保留原始数据，请重新下载或检查分片",
                        index + 1
                    ));
                }
                let existing_bytes =
                    pending::resume(task, &path, index, window_start, window_end, existing_bytes)
                        .await?;
                downloaded = downloaded.saturating_add(existing_bytes);
                let is_completed = existing_bytes >= expected_window;
                initial_windows.push(RangeWindow {
                    id: window_id_counter,
                    segment_index: index,
                    ordinal,
                    start_byte: window_start,
                    end_byte: window_end,
                    existing_bytes,
                    path,
                    status: if is_completed {
                        WindowStatus::Completed
                    } else {
                        WindowStatus::Pending
                    },
                });
                window_id_counter += 1;
            }
        }
        initial = initial.saturating_add(downloaded);
        let status = if downloaded == expected {
            SEGMENT_COMPLETED
        } else {
            SEGMENT_PENDING
        };
        runtimes.push(SegmentRuntime::new(index, start, end, downloaded, status));
    }
    if initial > 0 && http_response::if_range_validator(task).is_none() {
        return Err(format!("{REMOTE_CHANGED_PREFIX}服务器没有可核验的资源验证器，无法安全拼接旧分片；已保留旧数据，请选择重新下载"));
    }
    initial_windows.sort_by_key(|w| (w.ordinal, w.segment_index));
    let coordinator = Arc::new(WorkStealingCoordinator::new(temp, initial_windows));
    Ok(Plan {
        ranges,
        initial,
        coordinator,
        runtimes: Arc::new(runtimes),
    })
}

pub(super) async fn existing_length(path: &Path) -> Result<u64, String> {
    let metadata = match fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("无法读取已有分片属性，未修改原始数据：{error}")),
    };
    let mut redirected = metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        redirected |= metadata.file_attributes() & 0x400 != 0;
    }
    if redirected || !metadata.is_file() {
        return Err("已有分片是链接或不是普通文件，未修改原始数据".into());
    }
    Ok(metadata.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn oversized_legacy_and_window_data_are_errors_and_are_never_deleted() {
        for window in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let temp = directory.path().join("fixture.lumaget");
            let task = crate::manager::tests::test_task(
                directory.path(),
                "fixture.bin",
                CollisionPolicy::Rename,
            );
            let path = if window {
                window_part_path(&temp, 0, 0)
            } else {
                PathBuf::from(format!("{}.part0", temp.display()))
            };
            let bytes = b"oversized source evidence";
            fs::write(&path, bytes).await.unwrap();
            assert!(prepare(&task, &temp, 8, 2).await.is_err());
            assert_eq!(fs::read(&path).await.unwrap(), bytes);
        }
    }
    #[tokio::test]
    async fn only_missing_file_is_zero_and_non_regular_sources_are_explicit_errors() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            existing_length(&directory.path().join("missing"))
                .await
                .unwrap(),
            0
        );
        assert!(existing_length(directory.path()).await.is_err());
        let path = directory.path().join("regular");
        fs::write(&path, b"source").await.unwrap();
        assert!(existing_window_starts(&path.join("not-a-directory"), 0)
            .await
            .is_err());
        assert_eq!(fs::read(path).await.unwrap(), b"source");
    }
}
