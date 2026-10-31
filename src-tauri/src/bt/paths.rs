//! BT 的物理输出归属。显示名不能反推文件位置，删除仅覆盖记录中的实际文件。
use crate::manager::safe_name;
use crate::models::{CollisionPolicy, DownloadTask};
use std::path::{Component, Path, PathBuf};

pub(super) fn relative_path(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains(':')
        || value.contains('\\')
        || path
            .components()
            .filter_map(|component| match component {
                Component::Normal(name) => name.to_str(),
                _ => None,
            })
            .any(|name| unsafe_windows_component(name))
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("种子包含不安全的文件路径，已停止操作".into());
    }
    Ok(path.to_path_buf())
}

fn unsafe_windows_component(name: &str) -> bool {
    let device = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    name.ends_with([' ', '.'])
        || name
            .chars()
            .any(|c| c <= '\u{1f}' || ['"', '<', '>', '|', '?', '*'].contains(&c))
        || ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&device.as_str())
        || (device.len() == 4
            && (device.starts_with("COM") || device.starts_with("LPT"))
            && matches!(device.as_bytes()[3], b'1'..=b'9'))
}

fn output_name(task: &DownloadTask) -> String {
    let display = safe_name(&task.file_name);
    if display.is_empty() {
        format!("BT-{}", safe_name(&task.id))
    } else {
        display
    }
}

pub async fn prepare_task_output(task: &mut DownloadTask) -> Result<(), String> {
    if task
        .bt_meta
        .as_ref()
        .and_then(|m| m.output_folder.as_ref())
        .is_some()
    {
        return validate_output(task).await.map(|_| ());
    }
    let destination = Path::new(&task.destination);
    if !destination.is_absolute() {
        return Err("BT 保存目录必须是绝对路径".into());
    }
    tokio::fs::create_dir_all(destination)
        .await
        .map_err(|e| format!("创建 BT 保存目录失败：{e}"))?;
    let destination = tokio::fs::canonicalize(destination)
        .await
        .map_err(|e| format!("读取 BT 保存目录失败：{e}"))?;
    let name = output_name(task);
    for suffix in 0..10_000u32 {
        let candidate = destination.join(if suffix == 0 {
            name.clone()
        } else {
            format!("{name} ({suffix})")
        });
        let owned = match tokio::fs::create_dir(&candidate).await {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                match task.collision_policy {
                    CollisionPolicy::Rename => continue,
                    CollisionPolicy::Skip => {
                        return Err("BT 目标已存在，已按跳过策略保留原文件".into())
                    }
                    CollisionPolicy::Overwrite => false,
                }
            }
            Err(e) => return Err(format!("创建 BT 任务目录失败：{e}")),
        };
        let canonical = tokio::fs::canonicalize(&candidate)
            .await
            .map_err(|e| format!("校验 BT 目录失败：{e}"))?;
        if !canonical.starts_with(&destination) || canonical == destination || !canonical.is_dir() {
            return Err("BT 输出目录超出指定保存范围或不是文件夹".into());
        }
        let meta = task.bt_meta.as_mut().ok_or("BT 任务缺少元数据")?;
        meta.output_folder = Some(canonical.to_string_lossy().into_owned());
        meta.output_owned = owned;
        return Ok(());
    }
    Err("同名 BT 目录过多，请选择其他保存位置".into())
}

pub(super) async fn validate_output(task: &DownloadTask) -> Result<PathBuf, String> {
    let root = task
        .bt_meta
        .as_ref()
        .and_then(|m| m.output_folder.as_ref())
        .ok_or("BT 尚未记录实际输出路径")?;
    let root_metadata = tokio::fs::symlink_metadata(root)
        .await
        .map_err(|e| format!("无法读取 BT 输出目录属性：{e}"))?;
    if is_redirect(&root_metadata) {
        return Err("BT 输出目录已变为链接或重解析点，未继续文件操作".into());
    }
    let destination = tokio::fs::canonicalize(&task.destination)
        .await
        .map_err(|e| format!("无法读取下载目录：{e}"))?;
    let root = tokio::fs::canonicalize(root)
        .await
        .map_err(|e| format!("无法读取 BT 实际输出目录：{e}"))?;
    let owned = task
        .bt_meta
        .as_ref()
        .map(|m| m.output_owned)
        .unwrap_or(false);
    if !root.is_dir() || !root.starts_with(&destination) || (owned && root == destination) {
        return Err("BT 实际输出路径不在任务专属保存范围内".into());
    }
    Ok(root)
}

fn is_redirect(metadata: &std::fs::Metadata) -> bool {
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

pub(super) async fn validate_file_layout(root: &Path, files: &[String]) -> Result<(), String> {
    for file in files {
        let relative = relative_path(file)?;
        let mut candidate = root.to_path_buf();
        for part in relative.components() {
            candidate.push(part);
            match tokio::fs::symlink_metadata(&candidate).await {
                Ok(metadata) if is_redirect(&metadata) => {
                    return Err("BT 文件路径包含链接或重解析点，已停止操作".into())
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("无法校验 BT 文件路径属性：{e}")),
            }
        }
    }
    Ok(())
}

pub async fn task_output_folder(task: &DownloadTask) -> Result<PathBuf, String> {
    validate_output(task).await
}

pub async fn task_content_path(
    task: &DownloadTask,
    relative: Option<&str>,
) -> Result<PathBuf, String> {
    let root = validate_output(task).await?;
    let meta = task.bt_meta.as_ref().ok_or("BT 任务缺少文件清单")?;
    let requested = match relative.map(str::trim).filter(|p| !p.is_empty()) {
        Some(path) if meta.owned_files.iter().any(|owned| owned == path) => path,
        Some(_) => return Err("该文件不属于当前 BT 任务，已拒绝打开".into()),
        None if meta.owned_files.len() == 1 => &meta.owned_files[0],
        None if meta.owned_files.is_empty() => {
            return Err("BT_METADATA_PENDING: 文件清单尚待获取".into())
        }
        None => return Ok(root),
    };
    validate_file_layout(&root, &[requested.to_owned()]).await?;
    let file = tokio::fs::canonicalize(root.join(relative_path(requested)?))
        .await
        .map_err(|e| format!("BT 文件尚未下载或已被移动，请检查后重试：{e}"))?;
    if file == root || !file.starts_with(&root) || !file.is_file() {
        return Err("BT 文件路径越界或不是普通文件".into());
    }
    Ok(file)
}

pub(super) async fn adopt_legacy_output(
    task: &mut DownloadTask,
    files: &[String],
    torrent_name: Option<&str>,
) -> Result<(), String> {
    if task
        .bt_meta
        .as_ref()
        .and_then(|m| m.output_folder.as_ref())
        .is_some()
    {
        return Ok(());
    }
    let base = Path::new(&task.destination);
    // 旧实现从原始磁力 dn / 种子源文件 stem 创建目录；显示名随后可能被元数据覆盖。
    let original_name = if task.url.trim().to_ascii_lowercase().starts_with("magnet:") {
        super::magnet::parse_magnet(&task.url)?
            .display_name
            .unwrap_or_default()
    } else {
        Path::new(&task.url)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("种子任务")
            .to_owned()
    };
    let original_name = safe_name(&original_name);
    let original_root = if original_name.is_empty()
        || ["未命名种子", "magnet", "download"].contains(&original_name.as_str())
    {
        base.to_path_buf()
    } else {
        base.join(&original_name)
    };
    let mut candidates = vec![original_root, base.join(safe_name(&task.file_name))];
    if let Some(name) = torrent_name {
        candidates.push(base.join(safe_name(name)));
    }
    candidates.push(base.to_path_buf());
    let mut ambiguous = false;
    for (index, candidate) in candidates.into_iter().enumerate() {
        if !candidate.is_dir() {
            continue;
        }
        let has_file = files
            .iter()
            .filter_map(|p| relative_path(p).ok())
            .any(|p| candidate.join(p).is_file());
        if !has_file {
            continue;
        }
        // 展示名或元数据名不能证明旧物理归属；原始 URL 对应的目录才是确定证据。
        if index > 0 {
            ambiguous = true;
            continue;
        }
        let base = tokio::fs::canonicalize(base)
            .await
            .map_err(|e| format!("读取旧 BT 保存目录失败：{e}"))?;
        let candidate = tokio::fs::canonicalize(candidate)
            .await
            .map_err(|e| format!("读取旧 BT 输出目录失败：{e}"))?;
        if !candidate.starts_with(&base) {
            return Err("旧 BT 输出目录超出保存范围".into());
        }
        let meta = task.bt_meta.as_mut().ok_or("BT 任务缺少元数据")?;
        meta.output_folder = Some(candidate.to_string_lossy().into_owned());
        meta.output_owned = false;
        meta.owned_files = files.to_vec();
        return Ok(());
    }
    if task.downloaded_bytes > 0 || ambiguous {
        return Err("旧 BT 任务的原输出目录尚未确认，已保留全部数据。请定位已有下载目录后继续，或明确新建任务".into());
    }
    prepare_task_output(task).await
}

pub async fn delete_task_files(task: &DownloadTask) -> Result<Vec<String>, String> {
    let Some(meta) = task.bt_meta.as_ref() else {
        return Err("BT 任务缺少文件归属记录，未删除文件".into());
    };
    if meta.output_folder.is_none() || (meta.owned_files.is_empty() && !meta.output_owned) {
        return Err(
            "旧 BT 任务尚未确认实际文件清单，请先恢复元数据后再删除文件；也可仅删除记录".into(),
        );
    }
    let root = validate_output(task).await?;
    if meta.owned_files.is_empty() {
        return tokio::fs::remove_dir(&root)
            .await
            .map(|_| Vec::new())
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::DirectoryNotEmpty {
                    "元数据尚未就绪且目录中存在未确认文件，已保留全部内容；可选择仅删除记录".into()
                } else {
                    format!("清理未开始的 BT 空目录失败：{e}")
                }
            });
    }
    validate_file_layout(&root, &meta.owned_files).await?;
    let mut removed = Vec::new();
    let mut directories = Vec::new();
    for value in &meta.owned_files {
        let relative = relative_path(value)?;
        let path = root.join(&relative);
        let canonical = match tokio::fs::canonicalize(&path).await {
            Ok(path) => path,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("无法确认 BT 文件路径，未继续删除：{e}")),
        };
        if !canonical.starts_with(&root) || canonical == root || !canonical.is_file() {
            return Err("BT 文件路径越界或类型不符，未继续删除".into());
        }
        tokio::fs::remove_file(&canonical)
            .await
            .map_err(|e| format!("删除 BT 文件失败，请关闭占用程序后重试：{e}"))?;
        removed.push(canonical.to_string_lossy().into_owned());
        let mut parent = path.parent();
        while let Some(dir) = parent.filter(|dir| dir.starts_with(&root) && *dir != root) {
            directories.push(dir.to_path_buf());
            parent = dir.parent();
        }
    }
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    directories.dedup();
    if meta.output_owned {
        directories.push(root);
    }
    for directory in directories {
        match tokio::fs::remove_dir(&directory).await {
            Ok(()) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(e) => return Err(format!("文件已删除，但清理 BT 空目录失败：{e}")),
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bt::tests::bt_task_for_test;
    #[test]
    fn relative_paths_reject_escape_and_windows_prefixes() {
        for path in [
            "../secret",
            "/etc/passwd",
            "C:/secret",
            "a\\..\\secret",
            "",
            "./a",
            "CON.txt",
            "folder/aux",
            "folder/name.",
            "folder/name ",
            "folder/file?",
            "folder/com1.mp4",
        ] {
            assert!(relative_path(path).is_err(), "{path}");
        }
        assert!(relative_path("folder/video.mp4").is_ok());
    }

    #[tokio::test]
    async fn old_magnet_dn_recovers_original_root_after_display_name_changed() {
        let temp = tempfile::tempdir().unwrap();
        let mut task = bt_task_for_test();
        task.url.push_str("&dn=Original%20Name");
        task.file_name = "metadata-renamed.bin".into();
        task.downloaded_bytes = 4;
        task.destination = temp.path().join("downloads").to_string_lossy().into_owned();
        let original = Path::new(&task.destination).join("Original Name");
        tokio::fs::create_dir_all(&original).await.unwrap();
        tokio::fs::write(original.join("payload.bin"), b"data")
            .await
            .unwrap();
        adopt_legacy_output(
            &mut task,
            &["payload.bin".into()],
            Some("Actual Torrent Name"),
        )
        .await
        .unwrap();
        assert_eq!(
            task_output_folder(&task).await.unwrap(),
            original.canonicalize().unwrap()
        );
        assert!(!task.bt_meta.as_ref().unwrap().output_owned);
        assert_eq!(
            tokio::fs::read(original.join("payload.bin")).await.unwrap(),
            b"data"
        );
    }

    #[tokio::test]
    async fn old_torrent_source_stem_and_empty_dn_keep_legacy_layout() {
        let temp = tempfile::tempdir().unwrap();
        for (source, directory) in [
            ("C:/Old/Source Name.torrent", "Source Name"),
            (
                "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
                "",
            ),
        ] {
            let mut task = bt_task_for_test();
            task.url = source.into();
            task.destination = temp
                .path()
                .join(if directory.is_empty() {
                    "bare"
                } else {
                    "named"
                })
                .to_string_lossy()
                .into_owned();
            task.file_name = "changed-name".into();
            task.downloaded_bytes = 4;
            let original = Path::new(&task.destination).join(directory);
            tokio::fs::create_dir_all(&original).await.unwrap();
            tokio::fs::write(original.join("payload.bin"), b"data")
                .await
                .unwrap();
            adopt_legacy_output(&mut task, &["payload.bin".into()], None)
                .await
                .unwrap();
            assert_eq!(
                task_output_folder(&task).await.unwrap(),
                original.canonicalize().unwrap()
            );
        }
    }

    #[tokio::test]
    async fn missing_legacy_partial_data_is_not_silently_retargeted() {
        let temp = tempfile::tempdir().unwrap();
        let mut task = bt_task_for_test();
        task.destination = temp.path().to_string_lossy().into_owned();
        task.downloaded_bytes = 123;
        assert!(
            adopt_legacy_output(&mut task, &["missing.bin".into()], None)
                .await
                .is_err()
        );
        assert!(task.bt_meta.unwrap().output_folder.is_none());
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }
}
