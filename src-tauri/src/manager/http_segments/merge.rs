use super::*;
#[cfg(test)]
#[path = "merge_tests.rs"]
mod tests;

struct MergeInput<'a> {
    plan: &'a plan::Plan,
    total: u64,
    token: &'a CancellationToken,
}

pub(super) async fn complete(
    manager: &DownloadManager,
    task: &mut DownloadTask,
    temp: &Path,
    session: &Session,
) -> Result<(), String> {
    check_cancel(&session.token)?;
    ensure_space(manager, task, session.total).await?;
    let merge = PathBuf::from(format!("{}.merge", temp.to_string_lossy()));
    let buffer_size = if manager.settings().await.low_memory_mode {
        64 * 1024
    } else {
        1024 * 1024
    };
    let input = MergeInput {
        plan: &session.plan,
        total: session.total,
        token: &session.token,
    };
    let result = write_merge(&merge, temp, &input, buffer_size).await;
    if result.is_err() {
        let _ = fs::remove_file(&merge).await;
    }
    result
}

async fn ensure_space(
    manager: &DownloadManager,
    task: &mut DownloadTask,
    required: u64,
) -> Result<(), String> {
    let available = query_available_space_for_destination(&task.destination);
    if available >= required {
        return Ok(());
    }
    task.status = TaskStatus::PausedByLowDisk;
    task.speed = 0;
    task.eta_seconds = None;
    task.active_connections = 0;
    task.error = Some(format!(
        "合并所需空间不足（可用 {} 字节，需要 {} 字节），请清理或更换目录",
        available, required
    ));
    manager.store.upsert_task(task).await?;
    manager.emit_task("updated", task);
    let _ = manager.app.emit(
        "merge-blocked-by-low-disk",
        LowDiskPayload {
            task_id: task.id.clone(),
            available_bytes: available,
            required_bytes: required,
        },
    );
    Err(format!(
        "{LOW_DISK_PREFIX}合并所需空间不足（可用 {} 字节，需要 {} 字节）",
        available, required
    ))
}

async fn write_merge(
    merge: &Path,
    temp: &Path,
    input: &MergeInput<'_>,
    buffer_size: usize,
) -> Result<(), String> {
    let mut output = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(merge)
        .await
        .map_err(|error| error.to_string())?;
    let mut buffer = vec![0; buffer_size];
    for &(index, start, end) in &input.plan.ranges {
        check_cancel(input.token)?;
        merge_segment(&mut output, temp, input, index, start, end, &mut buffer).await?;
    }
    output.flush().await.map_err(|error| error.to_string())?;
    output
        .sync_all()
        .await
        .map_err(|error| format!("同步合并文件失败：{error}"))?;
    let actual = output
        .metadata()
        .await
        .map_err(|error| error.to_string())?
        .len();
    if actual != input.total {
        return Err(format!(
            "合并长度不完整（应为 {} 字节，实际 {actual} 字节）",
            input.total
        ));
    }
    drop(output);
    check_cancel(input.token)?;
    // Source parts survive this internal merge and are removed only after the caller publishes the final file.
    replace_file_atomically(merge, temp).await
}

async fn merge_segment(
    output: &mut fs::File,
    temp: &Path,
    input: &MergeInput<'_>,
    index: u8,
    start: u64,
    end: u64,
    buffer: &mut [u8],
) -> Result<(), String> {
    let expected = end - start + 1;
    let legacy = PathBuf::from(format!("{}.part{index}", temp.to_string_lossy()));
    let prefix = plan::existing_length(&legacy).await?;
    let mut merged = 0u64;
    if prefix > 0 {
        append(output, &legacy, prefix, buffer, input.token).await?;
        merged = prefix;
    }
    if prefix < expected {
        let ordered = input
            .plan
            .coordinator
            .get_ordered_windows_for_segment(index)
            .await;
        if ordered.is_empty() {
            return Err(format!("分片 #{} 缺少切片数据", index + 1));
        }
        let mut cursor = start + prefix;
        for window in ordered {
            if window.start_byte != cursor {
                return Err(format!(
                    "分片 #{} 存在切片间隙：期望偏移 {}，实际窗口偏移 {}",
                    index + 1,
                    cursor,
                    window.start_byte
                ));
            }
            let bytes = window.end_byte - window.start_byte + 1;
            append(output, &window.path, bytes, buffer, input.token).await?;
            merged = merged.saturating_add(bytes);
            cursor = window.end_byte + 1;
        }
    }
    if merged != expected {
        return Err(format!(
            "分片 #{} 大小不完整（应为 {} 字节，实际 {} 字节）",
            index + 1,
            expected,
            merged
        ));
    }
    Ok(())
}

fn check_cancel(token: &CancellationToken) -> Result<(), String> {
    if token.is_cancelled() {
        Err("任务已暂停".into())
    } else {
        Ok(())
    }
}

async fn append(
    output: &mut fs::File,
    path: &Path,
    expected: u64,
    buffer: &mut [u8],
    token: &CancellationToken,
) -> Result<(), String> {
    let mut source = fs::File::open(path)
        .await
        .map_err(|error| error.to_string())?;
    let actual = source
        .metadata()
        .await
        .map_err(|error| error.to_string())?
        .len();
    if actual != expected {
        return Err(format!(
            "续接窗口大小不完整（应为 {expected} 字节，实际 {actual} 字节）"
        ));
    }
    let mut copied = 0u64;
    loop {
        check_cancel(token)?;
        let count = source
            .read(buffer)
            .await
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        copied = copied
            .checked_add(count as u64)
            .filter(|bytes| *bytes <= expected)
            .ok_or("合并期间源分片长度变化，已停止发布")?;
        check_cancel(token)?;
        output
            .write_all(&buffer[..count])
            .await
            .map_err(|error| error.to_string())?;
    }
    if copied != expected {
        return Err("合并期间源分片提前结束，已停止发布".into());
    }
    Ok(())
}
