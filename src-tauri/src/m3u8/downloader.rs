// HLS M3U8 原生轻量下载引擎（纯 Rust，并发切片、AES-128 原生解密、流式合并与断点续传）

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{ACCEPT_ENCODING, RANGE};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::manager::{
    friendly_body_error, friendly_reqwest, DownloadManager, RateLimiter,
    RuntimeTaskOptions,
};
use crate::models::{DownloadTask, TaskStatus};

use super::crypto::{decrypt_aes_128, derive_iv_from_sequence};
use super::parser::{
    parse_m3u8, select_best_variant, EncryptionMethod, ParsedPlaylist,
};

/// M3U8 原生下载执行入口
pub async fn download_m3u8_task(
    manager: &DownloadManager,
    mut task: DownloadTask,
    client: &reqwest::Client,
    temp_path: &Path,
    token: CancellationToken,
    task_limiter: Arc<RateLimiter>,
) -> Result<DownloadTask, String> {
    // 1. 抓取并解析 M3U8 播放列表
    let mut current_url = task.url.clone();
    let media_playlist = loop {
        if token.is_cancelled() {
            return Err("任务已暂停".to_string());
        }

        let mut req = client.get(&current_url).header(ACCEPT_ENCODING, "identity");
        for (name, value) in &task.headers {
            req = req.header(name, value);
        }

        let resp = req.send().await.map_err(friendly_reqwest)?;
        if !resp.status().is_success() {
            return Err(format!(
                "获取 M3U8 播放列表失败：HTTP {}",
                resp.status()
            ));
        }

        let final_resp_url = resp.url().to_string();
        let text = resp.text().await.map_err(friendly_body_error)?;

        match parse_m3u8(&text, &final_resp_url)? {
            ParsedPlaylist::Master(variants) => {
                let best_url = select_best_variant(&variants)
                    .ok_or_else(|| "未找到可用的 Master Playlist 变体流".to_string())?;
                current_url = best_url;
            }
            ParsedPlaylist::Media(media) => {
                break media;
            }
        }
    };

    if media_playlist.segments.is_empty() {
        return Err("M3U8 播放列表中没有任何媒体切片".to_string());
    }

    // 确保临时目录存在
    if let Some(parent) = temp_path.parent() {
        let _ = fs::create_dir_all(parent).await;
    }

    // 2. 初始化并发与限速选项
    let connections = task.connection_count.clamp(1, 32);
    task.active_connections = connections;
    let runtime_options = Arc::new(RuntimeTaskOptions::new(&task));
    manager
        .task_runtime
        .write()
        .await
        .insert(task.id.clone(), runtime_options.clone());

    let total_segments = media_playlist.segments.len();

    // 统计已存在的分片（断点续传）
    let progress_bytes = Arc::new(AtomicU64::new(0));
    let completed_segments_count = Arc::new(AtomicU64::new(0));

    for seg in &media_playlist.segments {
        let seg_path = segment_part_path(temp_path, seg.index);
        if let Ok(meta) = fs::metadata(&seg_path).await {
            if meta.len() > 0 {
                progress_bytes.fetch_add(meta.len(), Ordering::Relaxed);
                completed_segments_count.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    // 3. 密钥缓存池（避免重复请求相同的 16 字节解密 Key）
    let key_cache = Arc::new(Mutex::new(HashMap::<String, Vec<u8>>::new()));
    let pending_queue = Arc::new(Mutex::new(VecDeque::from(media_playlist.segments.clone())));

    task.status = TaskStatus::Downloading;
    task.active_connections = connections;
    let _ = manager.store.upsert_task(&task).await;
    manager.emit_task("updated", &task);

    // 4. 启动后台进度上报与状态循环
    let reporter_token = token.clone();
    let reporter_progress = progress_bytes.clone();
    let reporter_store = manager.store.clone();
    let reporter_app = manager.app.clone();
    let mut reporter_task = task.clone();
    let reporter_runtime_opts = runtime_options.clone();

    let reporter_handle = tokio::spawn(async move {
        let mut last_bytes = reporter_progress.load(Ordering::Relaxed);
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                _ = reporter_token.cancelled() => break,
                _ = interval.tick() => {
                    let current_bytes = reporter_progress.load(Ordering::Relaxed);
                    let speed = current_bytes.saturating_sub(last_bytes) * 2;
                    last_bytes = current_bytes;

                    reporter_task.downloaded_bytes = current_bytes;
                    reporter_task.speed = speed;
                    reporter_task.status = TaskStatus::Downloading;
                    reporter_task.active_connections = connections;
                    reporter_runtime_opts.apply(&mut reporter_task).await;

                    let _ = reporter_store.upsert_task(&reporter_task).await;
                    let _ = tauri::Emitter::emit(
                        &reporter_app,
                        "task-updated",
                        crate::models::TaskProgressEvent {
                            task: reporter_task.clone(),
                            event: "updated".into(),
                        },
                    );
                }
            }
        }
    });

    // 5. 启动 Worker 协程池执行分片抓取
    let mut worker_handles = Vec::with_capacity(connections as usize);

    for _worker_idx in 0..connections {
        let queue = pending_queue.clone();
        let client = client.clone();
        let task_headers = task.headers.clone();
        let token = token.clone();
        let key_cache = key_cache.clone();
        let progress = progress_bytes.clone();
        let completed_counter = completed_segments_count.clone();
        let limiter = task_limiter.clone();
        let runtime_options = runtime_options.clone();
        let bandwidth = manager.bandwidth_scheduler.clone();
        let task_id = task.id.clone();
        let temp_prefix = temp_path.to_path_buf();

        worker_handles.push(tokio::spawn(async move {
            loop {
                if token.is_cancelled() {
                    return Err("任务已暂停".to_string());
                }

                let seg = {
                    let mut q = queue.lock().await;
                    match q.pop_front() {
                        Some(item) => item,
                        None => return Ok(()),
                    }
                };

                let seg_path = segment_part_path(&temp_prefix, seg.index);
                // 若分片已存在且有内容，跳过下载
                if let Ok(meta) = fs::metadata(&seg_path).await {
                    if meta.len() > 0 {
                        continue;
                    }
                }

                // 抓取并准备解密密钥（若有）
                let key_bytes = if let Some(key_info) = &seg.key {
                    if key_info.method == EncryptionMethod::Aes128 {
                        let mut cache = key_cache.lock().await;
                        if let Some(cached) = cache.get(&key_info.uri) {
                            Some(cached.clone())
                        } else {
                            let mut key_req = client.get(&key_info.uri);
                            for (name, val) in &task_headers {
                                key_req = key_req.header(name, val);
                            }
                            let key_resp = key_req.send().await.map_err(friendly_reqwest)?;
                            if !key_resp.status().is_success() {
                                return Err(format!(
                                    "获取 M3U8 AES-128 解密密钥失败：HTTP {}",
                                    key_resp.status()
                                ));
                            }
                            let k_bytes = key_resp.bytes().await.map_err(friendly_body_error)?;
                            if k_bytes.len() != 16 {
                                return Err(format!(
                                    "M3U8 解密密钥长度异常：期望 16 字节，实际 {} 字节",
                                    k_bytes.len()
                                ));
                            }
                            let vec_k = k_bytes.to_vec();
                            cache.insert(key_info.uri.clone(), vec_k.clone());
                            Some(vec_k)
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                // 下载切片数据（支持重试）
                let mut retries = 0u32;
                let seg_data = loop {
                    if token.is_cancelled() {
                        return Err("任务已暂停".to_string());
                    }

                    let mut seg_req = client.get(&seg.url).header(ACCEPT_ENCODING, "identity");
                    for (name, val) in &task_headers {
                        seg_req = seg_req.header(name, val);
                    }
                    if let Some((len, opt_offset)) = seg.byte_range {
                        let start = opt_offset.unwrap_or(0);
                        let end = start.saturating_add(len).saturating_sub(1);
                        seg_req = seg_req.header(RANGE, format!("bytes={start}-{end}"));
                    }

                    match seg_req.send().await {
                        Ok(resp) if resp.status().is_success() || resp.status() == 206 => {
                            let mut stream = resp.bytes_stream();
                            let mut raw_buf = Vec::new();
                            let mut stream_err = None;

                            while let Some(chunk_res) = stream.next().await {
                                if token.is_cancelled() {
                                    return Err("任务已暂停".to_string());
                                }
                                match chunk_res {
                                    Ok(chunk) => {
                                        let len = chunk.len() as u64;
                                        bandwidth
                                            .acquire(
                                                &task_id,
                                                len,
                                                runtime_options.priority.load(Ordering::Relaxed),
                                                &token,
                                            )
                                            .await;
                                        limiter
                                            .acquire_with_cancel(
                                                len,
                                                runtime_options.speed_limit.load(Ordering::Relaxed),
                                                &token,
                                            )
                                            .await;

                                        raw_buf.extend_from_slice(&chunk);
                                        progress.fetch_add(len, Ordering::Relaxed);
                                    }
                                    Err(e) => {
                                        stream_err = Some(friendly_body_error(e));
                                        break;
                                    }
                                }
                            }

                            if let Some(err) = stream_err {
                                if retries < 3 {
                                    retries += 1;
                                    tokio::time::sleep(Duration::from_millis(500 * retries as u64)).await;
                                    continue;
                                }
                                return Err(err);
                            }

                            break raw_buf;
                        }
                        Ok(resp) => {
                            if retries < 3 {
                                retries += 1;
                                tokio::time::sleep(Duration::from_millis(500 * retries as u64)).await;
                                continue;
                            }
                            return Err(format!(
                                "下载切片 #{} 失败：HTTP {}",
                                seg.index + 1,
                                resp.status()
                            ));
                        }
                        Err(e) => {
                            if retries < 3 {
                                retries += 1;
                                tokio::time::sleep(Duration::from_millis(500 * retries as u64)).await;
                                continue;
                            }
                            return Err(friendly_reqwest(e));
                        }
                    }
                };

                // 执行解密（若有 AES-128）
                let final_data = if let Some(k) = &key_bytes {
                    let iv = if let Some(key_info) = &seg.key {
                        key_info.iv.unwrap_or_else(|| derive_iv_from_sequence(seg.sequence))
                    } else {
                        derive_iv_from_sequence(seg.sequence)
                    };
                    decrypt_aes_128(k, &iv, &seg_data)?
                } else {
                    seg_data
                };

                // 写入分片临时文件
                let tmp_seg_path = format!("{}.tmp", seg_path.to_string_lossy());
                let mut f = File::create(&tmp_seg_path)
                    .await
                    .map_err(|e| format!("创建分片文件失败: {e}"))?;
                f.write_all(&final_data)
                    .await
                    .map_err(|e| format!("写入分片数据失败: {e}"))?;
                f.flush().await.map_err(|e| e.to_string())?;
                drop(f);

                let _ = fs::rename(&tmp_seg_path, &seg_path).await;
                completed_counter.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }

    // 等待所有 Worker 完成
    let mut worker_error = None;
    for handle in worker_handles {
        match handle.await {
            Ok(Err(e)) => {
                if worker_error.is_none() {
                    worker_error = Some(e);
                    token.cancel();
                }
            }
            Err(join_err) => {
                if worker_error.is_none() {
                    worker_error = Some(format!("Worker 异常退出: {join_err}"));
                    token.cancel();
                }
            }
            Ok(Ok(())) => {}
        }
    }

    reporter_handle.abort();

    if let Some(err) = worker_error {
        if token.is_cancelled() {
            task.status = TaskStatus::Paused;
            task.speed = 0;
            task.eta_seconds = None;
            task.active_connections = 0;
            let _ = manager.store.upsert_task(&task).await;
            manager.emit_task("updated", &task);
            return Err("任务已暂停".to_string());
        }
        task.status = TaskStatus::Failed;
        task.error = Some(err.clone());
        let _ = manager.store.upsert_task(&task).await;
        manager.emit_task("updated", &task);
        return Err(err);
    }

    if token.is_cancelled() {
        task.status = TaskStatus::Paused;
        task.speed = 0;
        task.eta_seconds = None;
        task.active_connections = 0;
        let _ = manager.store.upsert_task(&task).await;
        manager.emit_task("updated", &task);
        return Err("任务已暂停".to_string());
    }

    // 6. 流式合并所有切片为目标文件
    let merge_path = PathBuf::from(format!("{}.merge", temp_path.to_string_lossy()));
    let output_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&merge_path)
        .await
        .map_err(|e| format!("创建合并输出文件失败: {e}"))?;

    let mut buf_writer = BufWriter::with_capacity(512 * 1024, output_file);
    let mut total_merged_bytes = 0u64;
    let mut seg_cleanup_list = Vec::new();
    let mut copy_buf = vec![0u8; 256 * 1024];

    for i in 0..total_segments {
        let seg_path = segment_part_path(temp_path, i);
        if !seg_path.exists() {
            let _ = fs::remove_file(&merge_path).await;
            return Err(format!("切片 #{} 数据丢失，无法完成合并", i + 1));
        }

        let mut seg_f = File::open(&seg_path)
            .await
            .map_err(|e| format!("读取切片 #{} 失败: {e}", i + 1))?;
        loop {
            let n = seg_f
                .read(&mut copy_buf)
                .await
                .map_err(|e| format!("读取切片 #{} 流失败: {e}", i + 1))?;
            if n == 0 {
                break;
            }
            buf_writer
                .write_all(&copy_buf[..n])
                .await
                .map_err(|e| format!("写入合并流失败: {e}"))?;
            total_merged_bytes += n as u64;
        }

        seg_cleanup_list.push(seg_path);
    }

    buf_writer.flush().await.map_err(|e| e.to_string())?;
    drop(buf_writer);

    // 原子替换为最终临时文件
    if let Err(_e) = fs::rename(&merge_path, temp_path).await {
        let _ = fs::copy(&merge_path, temp_path).await;
        let _ = fs::remove_file(&merge_path).await;
    }

    // 清理已合并的切片碎片文件
    for part in seg_cleanup_list {
        let _ = fs::remove_file(part).await;
    }

    task.downloaded_bytes = total_merged_bytes;
    task.total_bytes = total_merged_bytes;
    task.speed = 0;
    task.eta_seconds = None;
    task.active_connections = 0;

    Ok(task)
}

/// 生成 M3U8 切片临时文件路径
pub fn segment_part_path(temp: &Path, index: usize) -> PathBuf {
    PathBuf::from(format!("{}.seg{index}", temp.to_string_lossy()))
}
