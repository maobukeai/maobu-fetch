//! 恢复文件名固定属于任务，最终目标重命名/变更封装后仍能找到旧分片。
use super::*;

const LAYOUT: &str = "temp-layout.json";

pub(super) fn redirected(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Layout {
    version: u32,
    task_id: String,
    source_name: String,
    #[serde(default)]
    legacy_root: bool,
}

pub(super) async fn validate_ancestors(task: &DownloadTask) -> Result<(), String> {
    if !is_safe_path_component(&task.id) {
        return Err("任务恢复目录标识无效".into());
    }
    let root = PathBuf::from(&task.destination).join("_maobu_tmp");
    for path in [&root, &root.join(&task.id)] {
        match fs::symlink_metadata(path).await {
            Ok(metadata) if !metadata.is_dir() || redirected(&metadata) => {
                return Err("任务恢复目录已变为链接或不是普通目录，已保留文件并停止读写".into())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("检查任务恢复目录失败：{error}")),
        }
    }
    Ok(())
}

pub(super) async fn resolve(task: &DownloadTask) -> Result<PathBuf, String> {
    if !is_safe_path_component(&task.id) || !is_safe_path_component(&task.file_name) {
        return Err("任务标识或文件名不是有效路径组件".into());
    }
    validate_ancestors(task).await?;
    let directory = task_temp_dir(&task.destination, &task.id);
    let file = directory.join(LAYOUT);
    let layout = match fs::symlink_metadata(&file).await {
        Ok(metadata) if !metadata.is_file() || redirected(&metadata) || metadata.len() > 4096 => {
            return Err("任务恢复布局文件类型或长度异常，已保留全部分片".into());
        }
        Ok(_) => Some(
            serde_json::from_slice::<Layout>(
                &fs::read(&file)
                    .await
                    .map_err(|e| format!("读取任务恢复布局失败：{e}"))?,
            )
            .map_err(|_| "任务恢复布局损坏，已保留全部分片")?,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("读取恢复布局属性失败：{error}")),
    };
    let (name, legacy) = match layout {
        Some(layout)
            if layout.version == 1
                && layout.task_id == task.id
                && is_safe_path_component(&layout.source_name) =>
        {
            (layout.source_name, layout.legacy_root)
        }
        Some(_) => return Err("任务恢复布局身份或路径不匹配，未继续下载".into()),
        None => (task.file_name.clone(), false),
    };
    Ok(if legacy {
        PathBuf::from(&task.destination)
    } else {
        directory
    }
    .join(format!("{name}.lumaget")))
}

pub(super) async fn ensure(task: &DownloadTask) -> Result<PathBuf, String> {
    let source = resolve(task).await?;
    let directory = ensure_task_temp_dir(&task.destination, &task.id).await?;
    let layout = directory.join(LAYOUT);
    write_layout(task, &layout, false).await?;
    Ok(source)
}

async fn write_layout(task: &DownloadTask, layout: &Path, legacy_root: bool) -> Result<(), String> {
    if !fs::try_exists(layout)
        .await
        .map_err(|e| format!("检查任务恢复布局失败：{e}"))?
    {
        let data = serde_json::to_vec(&Layout {
            version: 1,
            task_id: task.id.clone(),
            source_name: task.file_name.clone(),
            legacy_root,
        })
        .map_err(|_| "任务恢复布局序列化失败")?;
        let staging = layout
            .parent()
            .ok_or("恢复布局目录无效")?
            .join(format!(".layout-{}", Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&staging)
            .await
            .map_err(|e| format!("创建任务恢复布局失败：{e}"))?;
        file.write_all(&data)
            .await
            .map_err(|e| format!("写入任务恢复布局失败：{e}"))?;
        file.sync_all()
            .await
            .map_err(|e| format!("任务恢复布局落盘失败：{e}"))?;
        drop(file);
        publish_file_atomically(&staging, layout, false).await?;
    }
    Ok(())
}

/// An old flat layout is adopted only when the database establishes a unique owner.
pub(super) async fn ensure_owned(task: &DownloadTask, store: &Store) -> Result<PathBuf, String> {
    resolve(task).await?;
    let private = task_temp_dir(&task.destination, &task.id);
    let existing_layout = private.join(LAYOUT);
    if fs::try_exists(&existing_layout)
        .await
        .map_err(|e| format!("检查恢复布局失败：{e}"))?
    {
        let source = resolve(task).await?;
        if source.parent() == Some(Path::new(&task.destination)) {
            unique_legacy_owner(
                task,
                store,
                source
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| name.strip_suffix(".lumaget"))
                    .ok_or("旧分片名称无效")?,
            )
            .await?;
        }
        return Ok(source);
    }
    // New tasks have explicit intent metadata and never borrow a flat shard with the same filename.
    if store.get_task_name_intent(&task.id).await?.is_some() || has_private_payload(task).await? {
        return ensure(task).await;
    }
    if legacy_files(task).await?.is_empty() {
        return ensure(task).await;
    }
    unique_legacy_owner(task, store, &task.file_name).await?;
    let directory = ensure_task_temp_dir(&task.destination, &task.id).await?;
    write_layout(task, &directory.join(LAYOUT), true).await?;
    resolve(task).await
}

async fn unique_legacy_owner(
    task: &DownloadTask,
    store: &Store,
    source_name: &str,
) -> Result<(), String> {
    let target = path_key(&PathBuf::from(&task.destination).join(source_name));
    let directory = fs::canonicalize(&task.destination)
        .await
        .map_err(|e| format!("读取旧分片保存目录失败：{e}"))?;
    for other in store.list_tasks().await? {
        if other.id == task.id || other.task_kind != TaskKind::Http {
            continue;
        }
        match fs::canonicalize(&other.destination).await {
            Ok(other_directory) if other_directory == directory => {}
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "另一任务保存目录不可读取，暂不能确认旧分片归属：{error}"
                ))
            }
        }
        let source = resolve(&other).await?;
        let adopted = source.parent() == Some(Path::new(&other.destination));
        let unclaimed_old = !task_temp_dir(&other.destination, &other.id)
            .join(LAYOUT)
            .exists()
            && store.get_task_name_intent(&other.id).await?.is_none()
            && !has_private_payload(&other).await?;
        let claimed = if adopted {
            source
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".lumaget"))
        } else if unclaimed_old {
            Some(other.file_name.as_str())
        } else {
            None
        };
        if claimed
            .is_some_and(|name| path_key(&PathBuf::from(&other.destination).join(name)) == target)
        {
            return Err("旧格式分片可能被多个同名任务使用，无法确认归属；已保留全部文件，请先选择明确的保存位置".into());
        }
    }
    Ok(())
}

async fn has_private_payload(task: &DownloadTask) -> Result<bool, String> {
    let path = task_temp_dir(&task.destination, &task.id);
    let mut entries = match fs::read_dir(path).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("检查任务恢复目录失败：{error}")),
    };
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("读取任务恢复目录失败：{e}"))?
    {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name != LAYOUT && !name.starts_with(".layout-") && !name.starts_with(".publication-") {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) async fn legacy_files(task: &DownloadTask) -> Result<Vec<PathBuf>, String> {
    if !is_safe_path_component(&task.file_name) {
        return Err("任务文件名无效，未访问旧分片".into());
    }
    legacy_files_at(&PathBuf::from(&task.destination).join(format!("{}.lumaget", task.file_name)))
        .await
}

pub(super) async fn legacy_files_at(source: &Path) -> Result<Vec<PathBuf>, String> {
    let base = source
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| is_safe_path_component(name))
        .ok_or("旧分片路径无效")?
        .to_owned();
    let prefix = format!("{base}.part");
    let mut entries = match fs::read_dir(source.parent().ok_or("旧分片目录无效")?).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("读取旧分片目录失败：{error}")),
    };
    let mut files = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("读取旧分片失败：{e}"))?
    {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let shard = name
            .strip_prefix(&prefix)
            .is_some_and(|index| !index.is_empty() && index.bytes().all(|c| c.is_ascii_digit()));
        if name == base
            || name == format!("{base}.merge")
            || shard
            || is_window_part_name(&name, &prefix)
        {
            let metadata = entry
                .metadata()
                .await
                .map_err(|e| format!("检查旧分片失败：{e}"))?;
            if !metadata.is_file()
                || entry
                    .file_type()
                    .await
                    .map_err(|e| e.to_string())?
                    .is_symlink()
            {
                return Err("旧分片路径不是普通文件，已保留全部数据".into());
            }
            files.push(entry.path());
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests;
