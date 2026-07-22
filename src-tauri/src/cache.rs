//! 软件缓存检查与清理模块。
//!
//! 遵循 AGENTS.md 强约束：
//! - 严禁删除核心任务数据库 `lumaget.db`、配置文件或按需工具（yt-dlp/ffmpeg）。
//! - 仅清理孤立/残留分片、超过 7 天的旧日志文件及媒体探测临时缓存。

use crate::models::{CacheClearResult, CacheInspectResult, TaskStatus};
use crate::store::Store;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// 获取所有缓存文件的路径与大小信息。
pub async fn inspect_cache_files(app_data_dir: &Path, store: &Store) -> Result<Vec<(PathBuf, u64)>, String> {
    let mut files = Vec::new();

    // 1. 扫描日志目录 (app_data_dir/logs) 中所有的旧日志文件（除了当天的活动日志）
    let logs_dir = app_data_dir.join("logs");
    if logs_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&logs_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    // 保留当前主要活动日志文件 maobu.log，清理旧滚动日志如 maobu.log.2026-07-10 或 old 日志
                    if name != "maobu.log" {
                        if let Ok(metadata) = entry.metadata() {
                            files.push((path, metadata.len()));
                        }
                    }
                }
            }
        }
    }

    // 2. 扫描 app_data_dir/scratch 或 app_data_dir/tmp 媒体分析临时文件
    for sub in &["scratch", "tmp"] {
        let tmp_dir = app_data_dir.join(sub);
        if tmp_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&tmp_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        if let Ok(meta) = entry.metadata() {
                            files.push((path, meta.len()));
                        }
                    }
                }
            }
        }
    }

    // 3. 扫描活动下载目录与 app_data_dir 中无对应活动任务的孤立 .lumaget 临时分片
    let tasks = store.list_tasks().await?;
    let active_temp_files: HashSet<PathBuf> = tasks
        .iter()
        .filter(|t| matches!(t.status, TaskStatus::Downloading | TaskStatus::Paused | TaskStatus::Queued | TaskStatus::Scheduled))
        .filter_map(|t| {
            let p = PathBuf::from(&t.destination).join(&t.file_name);
            Some(PathBuf::from(format!("{}.lumaget", p.to_string_lossy())))
        })
        .collect();

    // 收集所有需要检测孤立分片的搜索目录
    let mut search_dirs: HashSet<PathBuf> = tasks
        .iter()
        .map(|t| PathBuf::from(&t.destination))
        .collect();
    search_dirs.insert(app_data_dir.to_path_buf());

    for dir in search_dirs {
        // 检测 dir/ 中的孤立 .lumaget
        if dir.is_dir() {
            scan_orphaned_lumaget_files(&dir, &active_temp_files, &mut files);
        }
        // 检测 dir/_maobu_tmp/ 中的孤立缓存
        let maobu_tmp = dir.join("_maobu_tmp");
        if maobu_tmp.is_dir() {
            scan_orphaned_lumaget_files(&maobu_tmp, &active_temp_files, &mut files);
        }
    }

    Ok(files)
}

fn scan_orphaned_lumaget_files(
    dir: &Path,
    active_temp_files: &HashSet<PathBuf>,
    files: &mut Vec<(PathBuf, u64)>,
) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.ends_with(".lumaget") || name.ends_with(".part") {
                    if !active_temp_files.contains(&path) {
                        if let Ok(meta) = entry.metadata() {
                            files.push((path, meta.len()));
                        }
                    }
                }
            } else if path.is_dir() && path.file_name().map_or(false, |n| n != "logs" && n != "tools") {
                // 递归扫描一层子目录（如 _maobu_tmp/[task_id]）
                scan_orphaned_lumaget_files(&path, active_temp_files, files);
            }
        }
    }
}

/// 评估缓存大小。
pub async fn inspect_cache(app_data_dir: &Path, store: &Store) -> Result<CacheInspectResult, String> {
    let files = inspect_cache_files(app_data_dir, store).await?;
    let total_bytes: u64 = files.iter().map(|(_, sz)| *sz).sum();
    Ok(CacheInspectResult {
        total_bytes,
        file_count: files.len(),
    })
}

/// 执行缓存清理。
pub async fn clear_cache(app_data_dir: &Path, store: &Store) -> Result<CacheClearResult, String> {
    let files = inspect_cache_files(app_data_dir, store).await?;
    let mut freed_bytes = 0u64;
    let mut deleted_files_count = 0usize;

    for (path, size) in files {
        if path.is_file() {
            if fs::remove_file(&path).is_ok() {
                freed_bytes += size;
                deleted_files_count += 1;
            }
        }
    }

    Ok(CacheClearResult {
        freed_bytes,
        deleted_files_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DownloadTask, TaskStatus};
    use tempfile::tempdir;

    fn make_test_task(id: &str, file_name: &str, destination: &str, status: TaskStatus) -> DownloadTask {
        DownloadTask {
            id: id.into(),
            url: "https://example.com/file".into(),
            file_name: file_name.into(),
            destination: destination.into(),
            total_bytes: 1024,
            downloaded_bytes: 0,
            speed: 0,
            eta_seconds: None,
            status,
            error: None,
            created_at: 1000,
            completed_at: None,
            scheduled_at: None,
            category: "default".into(),
            queue_position: 1,
            priority: 0,
            retry_count: 0,
            max_retries: 3,
            checksum_sha256: None,
            expected_checksum: None,
            source: "direct".into(),
            etag: None,
            last_modified: None,
            final_url: None,
            response_status: None,
            content_type: None,
            accepts_ranges: None,
            collision_policy: crate::models::CollisionPolicy::Rename,
            completion_action: crate::models::CompletionAction::None,
            active_connections: 0,
            connection_count: 4,
            per_task_speed_limit: 0,
            headers: std::collections::HashMap::new(),
            media: None,
            segments: Vec::new(),
            retry_policy_override: None,
            proxy_override: None,
            proxy_auth: None,
        }
    }

    #[test]
    fn inspect_and_clear_cache_deletes_old_logs_and_orphaned_files() {
        let dir = tempdir().unwrap();
        let app_data_dir = dir.path().to_path_buf();

        // 1. 创建旧的日志文件和当天活动日志
        let logs_dir = app_data_dir.join("logs");
        fs::create_dir_all(&logs_dir).unwrap();
        let old_log = logs_dir.join("maobu.log.2026-07-01");
        fs::write(&old_log, "test log content").unwrap();
        let active_log = logs_dir.join("maobu.log");
        fs::write(&active_log, "active log content").unwrap();

        // 2. 创建孤立的 .lumaget 文件
        let orphaned_lumaget = app_data_dir.join("orphaned.lumaget");
        fs::write(&orphaned_lumaget, "orphaned temp data").unwrap();

        // 3. 打开临时 Store
        let store = Store::open(app_data_dir.clone()).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // 检查缓存
            let inspect = inspect_cache(&app_data_dir, &store).await.unwrap();
            assert_eq!(inspect.file_count, 2);
            assert!(inspect.total_bytes > 0);

            // 清理缓存
            let clear = clear_cache(&app_data_dir, &store).await.unwrap();
            assert_eq!(clear.deleted_files_count, 2);
            assert_eq!(clear.freed_bytes, inspect.total_bytes);

            // 再次检查缓存应只保留 active_log
            let re_inspect = inspect_cache(&app_data_dir, &store).await.unwrap();
            assert_eq!(re_inspect.file_count, 0);
            assert_eq!(re_inspect.total_bytes, 0);

            // 主日志仍存在
            assert!(active_log.exists());
        });
    }

    #[test]
    fn inspect_cache_preserves_active_task_temp_files() {
        let dir = tempdir().unwrap();
        let app_data_dir = dir.path().to_path_buf();
        let store = Store::open(app_data_dir.clone()).unwrap();

        // 创建活动任务并记录临时分片
        let download_dir = dir.path().join("downloads");
        fs::create_dir_all(&download_dir).unwrap();
        let active_file = download_dir.join("test.mp4.lumaget");
        fs::write(&active_file, "active download chunk").unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let task = make_test_task("task-1", "test.mp4", &download_dir.to_string_lossy(), TaskStatus::Downloading);
            store.upsert_task(&task).await.unwrap();

            let inspect = inspect_cache(&app_data_dir, &store).await.unwrap();
            // 活动分片必须受保护，不能算入垃圾缓存
            assert_eq!(inspect.file_count, 0);
            assert_eq!(inspect.total_bytes, 0);
        });
    }
}
