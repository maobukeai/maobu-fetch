use crate::models::{
    CompletionAction, DownloadTask, NewTaskRequest, TaskExportFile, TaskExportItem,
};
use std::path::{Path, PathBuf};
use tokio::fs;
use url::Url;
use uuid::Uuid;

const TASK_EXPORT_SCHEMA_VERSION: u32 = 1;
const MAX_IMPORT_BYTES: u64 = 10 * 1024 * 1024;
const MAX_IMPORT_TASKS: usize = 5_000;

pub async fn export_file(
    path: &str,
    tasks: &[DownloadTask],
    exported_at: u64,
) -> Result<usize, String> {
    let path = json_path(path, "导出")?;
    let payload = TaskExportFile {
        schema_version: TASK_EXPORT_SCHEMA_VERSION,
        exported_at,
        tasks: tasks.iter().map(export_item).collect(),
    };
    let bytes = serde_json::to_vec_pretty(&payload)
        .map_err(|error| format!("生成导出文件失败：{error}"))?;
    atomic_write(&path, &bytes).await?;
    Ok(payload.tasks.len())
}

pub async fn import_requests(path: &str, destination: &str) -> Result<Vec<NewTaskRequest>, String> {
    let path = json_path(path, "导入")?;
    let destination = PathBuf::from(destination);
    if !destination.is_absolute() {
        return Err("请选择有效的绝对下载目录".into());
    }
    fs::create_dir_all(&destination)
        .await
        .map_err(|error| format!("无法创建下载目录：{error}"))?;
    let metadata = fs::metadata(&path)
        .await
        .map_err(|error| format!("无法读取导入文件：{error}"))?;
    if metadata.len() > MAX_IMPORT_BYTES {
        return Err("导入文件不能超过 10 MB".into());
    }
    let bytes = fs::read(&path)
        .await
        .map_err(|error| format!("无法读取导入文件：{error}"))?;
    let payload: TaskExportFile =
        serde_json::from_slice(&bytes).map_err(|error| format!("任务文件格式无效：{error}"))?;
    if payload.schema_version != TASK_EXPORT_SCHEMA_VERSION {
        return Err(format!(
            "不支持任务文件版本 {}，当前支持版本 {}",
            payload.schema_version, TASK_EXPORT_SCHEMA_VERSION
        ));
    }
    if payload.tasks.is_empty() || payload.tasks.len() > MAX_IMPORT_TASKS {
        return Err(format!("导入任务数量必须为 1–{MAX_IMPORT_TASKS}"));
    }
    payload
        .tasks
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            import_request(item, &destination)
                .map_err(|error| format!("第 {} 个任务无效：{error}", index + 1))
        })
        .collect()
}

fn export_item(task: &DownloadTask) -> TaskExportItem {
    TaskExportItem {
        url: sanitized_export_url(&task.url),
        file_name: task.file_name.clone(),
        priority: task.priority,
        scheduled_at: task.scheduled_at,
        expected_checksum: task.expected_checksum.clone(),
        per_task_speed_limit: task.per_task_speed_limit,
        collision_policy: task.collision_policy.clone(),
        completion_action: if task.completion_action == CompletionAction::RunFile {
            CompletionAction::None
        } else {
            task.completion_action.clone()
        },
        media: task.media.clone(),
        connection_count: task.connection_count,
    }
}

fn import_request(item: TaskExportItem, destination: &Path) -> Result<NewTaskRequest, String> {
    let parsed = Url::parse(&item.url).map_err(|_| "URL 无效")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("仅支持 HTTP/HTTPS URL".into());
    }
    Ok(NewTaskRequest {
        url: parsed.to_string(),
        file_name: Some(item.file_name),
        destination: Some(destination.to_string_lossy().into_owned()),
        headers: Default::default(),
        scheduled_at: item.scheduled_at,
        priority: item.priority.clamp(-10, 10),
        expected_checksum: item.expected_checksum,
        source: Some("import".into()),
        per_task_speed_limit: item.per_task_speed_limit,
        collision_policy: item.collision_policy,
        completion_action: if item.completion_action == CompletionAction::RunFile {
            CompletionAction::None
        } else {
            item.completion_action
        },
        media: item.media,
        connection_count: Some(item.connection_count.clamp(1, 32)),
        start_paused: true,
    })
}

fn json_path(value: &str, label: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!("{label}路径必须是绝对路径"));
    }
    let is_json = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
    if !is_json {
        return Err(format!("{label}文件必须使用 .json 扩展名"));
    }
    Ok(path)
}

fn sanitized_export_url(value: &str) -> String {
    let Ok(mut url) = Url::parse(value) else {
        return value.to_string();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    let safe_pairs: Vec<_> = url
        .query_pairs()
        .filter(|(name, _)| !is_sensitive_query_name(name))
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    if !safe_pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(safe_pairs);
    }
    url.to_string()
}

fn is_sensitive_query_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase().replace(['-', '_'], "");
    [
        "token",
        "accesstoken",
        "authorization",
        "auth",
        "signature",
        "sig",
        "apikey",
        "credential",
        "xamzsignature",
        "xamzcredential",
        "policy",
    ]
    .iter()
    .any(|sensitive| name == *sensitive || name.ends_with(sensitive))
}

async fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = target.parent().ok_or("导出路径缺少父目录")?;
    if !parent.is_dir() {
        return Err("导出文件夹不存在".into());
    }
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("导出文件名无效")?;
    let temporary = parent.join(format!(".{name}.{}.tmp", Uuid::new_v4()));
    let backup = parent.join(format!(".{name}.{}.bak", Uuid::new_v4()));
    let mut file = fs::File::create(&temporary)
        .await
        .map_err(|error| format!("无法创建导出临时文件：{error}"))?;
    use tokio::io::AsyncWriteExt;
    file.write_all(bytes)
        .await
        .map_err(|error| format!("写入导出文件失败：{error}"))?;
    file.sync_all()
        .await
        .map_err(|error| format!("同步导出文件失败：{error}"))?;
    drop(file);
    let had_target = target.exists();
    if had_target {
        fs::rename(target, &backup)
            .await
            .map_err(|error| format!("无法替换原导出文件：{error}"))?;
    }
    if let Err(error) = fs::rename(&temporary, target).await {
        if had_target {
            let _ = fs::rename(&backup, target).await;
        }
        let _ = fs::remove_file(&temporary).await;
        return Err(format!("保存导出文件失败：{error}"));
    }
    if had_target {
        let _ = fs::remove_file(&backup).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CollisionPolicy, CompletionAction, DownloadSegment, TaskStatus};
    use std::collections::HashMap;

    fn task() -> DownloadTask {
        DownloadTask {
            id: "task".into(),
            url: "https://user:password@example.com/file?id=42&token=secret&X-Amz-Signature=hidden"
                .into(),
            file_name: "file.bin".into(),
            destination: "C:\\secret\\path".into(),
            total_bytes: 10,
            downloaded_bytes: 5,
            speed: 0,
            eta_seconds: None,
            status: TaskStatus::Paused,
            error: None,
            created_at: 0,
            completed_at: None,
            scheduled_at: None,
            category: "other".into(),
            queue_position: 0,
            priority: 1,
            retry_count: 0,
            max_retries: 3,
            checksum_sha256: None,
            expected_checksum: None,
            source: "desktop".into(),
            etag: None,
            last_modified: None,
            final_url: None,
            response_status: None,
            content_type: None,
            accepts_ranges: None,
            headers: HashMap::from([("Authorization".into(), "Bearer private".into())]),
            media: None,
            per_task_speed_limit: 0,
            collision_policy: CollisionPolicy::Rename,
            completion_action: CompletionAction::RunFile,
            connection_count: 8,
            active_connections: 0,
            segments: vec![DownloadSegment {
                index: 0,
                start_byte: 0,
                end_byte: 9,
                downloaded_bytes: 5,
                status: "paused".into(),
            }],
        }
    }

    #[test]
    fn export_omits_credentials_paths_headers_and_runtime_state() {
        let item = export_item(&task());
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("id=42"));
        for secret in [
            "password",
            "secret",
            "hidden",
            "private",
            "C:\\\\secret",
            "segments",
            "downloaded_bytes",
        ] {
            assert!(!json.contains(secret), "export leaked {secret}");
        }
        assert_eq!(item.completion_action, CompletionAction::None);
    }

    #[test]
    fn imported_tasks_are_paused_and_have_no_headers() {
        let directory = tempfile::tempdir().unwrap();
        let request = import_request(export_item(&task()), directory.path()).unwrap();
        assert!(request.start_paused);
        assert!(request.headers.is_empty());
        assert_eq!(request.source.as_deref(), Some("import"));
    }

    #[tokio::test]
    async fn exported_json_round_trips_from_disk() {
        let directory = tempfile::tempdir().unwrap();
        let export_path = directory.path().join("tasks.json");
        let destination = directory.path().join("downloads");
        let count = export_file(export_path.to_str().unwrap(), &[task()], 123)
            .await
            .unwrap();
        let requests =
            import_requests(export_path.to_str().unwrap(), destination.to_str().unwrap())
                .await
                .unwrap();
        assert_eq!(count, 1);
        assert_eq!(requests.len(), 1);
        assert!(requests[0].start_paused);
        assert_eq!(requests[0].destination.as_deref(), destination.to_str());
    }
}
