// 猫步下载器 · 局域网互联 (LanDisk) 单任务分桶并发断点续传 ZIP 引擎
//
// 核心解决痛点：
//   1. 保持单一 ZIP 任务：不在任务列表生成上千个散碎子任务、不刷屏通知、不向下载目录倾倒散落文件；
//   2. 真正支持随时暂停与断点续传：通过 2 层浅层目录分片（耗时 < 1.5s），将大目录/多文件拆分为
//      固定保存在 `_maobu_tmp/<task_id>/` 下的多个分桶（大文件采用支持 HTTP 206 Range 字节级续传的
//      `SingleFileChunk`，子目录与小文件采用并发 `SubBatch` 压缩包分桶）；
//   3. 暂停后点继续绝不归零：已落盘的 `.raw` 字节分片与已完成的 `.zip` 分桶在恢复时 100% 复用；
//   4. 16 路并发加速：突破单路 `POST /api/download/batch` 的单核串行 I/O 与单 TCP 流吞吐瓶颈；
//   5. 本地零重压缩极速合流：利用 `zip::ZipWriter::raw_copy_file_rename` 直接拼接 Deflate 压缩流。

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File as StdFile;
use std::io::{BufReader, BufWriter as StdBufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;
use futures_util::StreamExt;
use reqwest::header::{ACCEPT_ENCODING, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use serde::{Deserialize, Serialize};
use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use url::Url;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::manager::{
    ensure_task_temp_dir, friendly_body_error, friendly_reqwest, DownloadManager, RateLimiter,
    RuntimeTaskOptions,
};
use crate::models::{DownloadSegment, DownloadTask, TaskStatus};

/// 单个大文件超过该阈值（512 KB）即走 HTTP Range 字节级可续传通道
const SINGLE_FILE_RANGE_THRESHOLD: u64 = 512 * 1024;
/// 单个 Range 分片的最大字节数（8 MB），保证大文件可被 16 路并发拉取且秒级落盘
const MAX_RANGE_CHUNK_BYTES: u64 = 8 * 1024 * 1024;
/// 每个 SubBatch 压缩分桶最多包含的子目录数
const MAX_DIRS_PER_SUBBATCH: usize = 8;
/// 每个 SubBatch 压缩分桶最多包含的小文件数
const MAX_SMALL_FILES_PER_SUBBATCH: usize = 32;
/// 未展开子目录的初始保守估计体积（2 MB），用于在下载初期提供平滑总大小与进度百分比
const ESTIMATED_SUBDIR_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BucketKind {
    /// 支持 HTTP 206 `Range: bytes=offset-end` 字节级精确断点续传的文件切片
    SingleFileChunk {
        remote_path: String,
        zip_entry_name: String,
        chunk_index: usize,
        offset: u64,
        length: u64,
        total_file_size: u64,
    },
    /// 同一父目录下的小文件或子目录集合，通过 `POST /api/download/batch` 压缩传输
    SubBatch {
        remote_paths: Vec<String>,
        zip_prefix: String,
        estimated_bytes: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanDiskBucket {
    pub index: usize,
    pub kind: BucketKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanDiskManifest {
    pub base_origin: String,
    pub folder_name: String,
    pub pin: Option<String>,
    pub qr_token: Option<String>,
    pub buckets: Vec<LanDiskBucket>,
}

#[derive(Debug, Deserialize)]
struct ApiFilesResponse {
    #[serde(default)]
    files: Vec<ApiFileEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiFileEntry {
    name: String,
    path: String,
    #[serde(default)]
    size: u64,
    #[serde(rename = "isDirectory", default)]
    is_directory: bool,
}

/// 判断任务是否为局域网互联 (LanDisk) 批量/目录打包下载任务
pub fn is_landisk_batch_task(task: &DownloadTask) -> bool {
    task.url.contains("/api/download/batch")
}

/// 解析 `/api/download/batch` 任务的请求参数（兼容 JSON、x-www-form-urlencoded 与 URL query）
pub fn parse_batch_request(task: &DownloadTask) -> Result<(String, String, Vec<String>, Option<String>, Option<String>), String> {
    let parsed_url = Url::parse(&task.url).map_err(|e| format!("解析任务 URL 失败: {e}"))?;
    let base_origin = format!(
        "{}://{}{}",
        parsed_url.scheme(),
        parsed_url.host_str().unwrap_or("127.0.0.1"),
        parsed_url
            .port()
            .map(|p| format!(":{p}"))
            .unwrap_or_default()
    );

    let mut folder_name = String::new();
    let mut files: Vec<String> = Vec::new();
    let mut pin: Option<String> = None;
    let mut qr_token: Option<String> = None;

    for (k, v) in parsed_url.query_pairs() {
        match k.as_ref() {
            "pin" if !v.trim().is_empty() => pin = Some(v.trim().to_string()),
            "token" if !v.trim().is_empty() => qr_token = Some(v.trim().to_string()),
            "folderName" if !v.trim().is_empty() => folder_name = v.trim().to_string(),
            "files" | "files[]" | "path" if !v.trim().is_empty() => {
                files.push(v.trim().to_string());
            }
            _ => {}
        }
    }

    if let Some(h_pin) = task.headers.get("x-pin").or_else(|| task.headers.get("X-Pin")) {
        if !h_pin.trim().is_empty() {
            pin = Some(h_pin.trim().to_string());
        }
    }
    if let Some(h_tok) = task
        .headers
        .get("x-qr-token")
        .or_else(|| task.headers.get("X-Qr-Token"))
    {
        if !h_tok.trim().is_empty() {
            qr_token = Some(h_tok.trim().to_string());
        }
    }

    if let Some(body) = task.body.as_deref() {
        let trimmed = body.trim();
        if trimmed.starts_with('{') {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(f_name) = val.get("folderName").and_then(|v| v.as_str()) {
                    if !f_name.trim().is_empty() {
                        folder_name = f_name.trim().to_string();
                    }
                }
                if let Some(p) = val.get("pin").and_then(|v| v.as_str()) {
                    if !p.trim().is_empty() {
                        pin = Some(p.trim().to_string());
                    }
                }
                if let Some(arr) = val.get("files").and_then(|v| v.as_array()) {
                    for item in arr {
                        if let Some(s) = item.as_str() {
                            if !s.trim().is_empty() {
                                files.push(s.trim().to_string());
                            }
                        }
                    }
                } else if let Some(s) = val.get("files").and_then(|v| v.as_str()) {
                    if !s.trim().is_empty() {
                        files.push(s.trim().to_string());
                    }
                }
            }
        } else if !trimmed.is_empty() {
            for (k, v) in url::form_urlencoded::parse(trimmed.as_bytes()) {
                match k.as_ref() {
                    "folderName" if !v.trim().is_empty() => folder_name = v.trim().to_string(),
                    "pin" if !v.trim().is_empty() => pin = Some(v.trim().to_string()),
                    "token" if !v.trim().is_empty() => qr_token = Some(v.trim().to_string()),
                    "files" | "files[]" if !v.trim().is_empty() => {
                        let s = v.trim();
                        if s.starts_with('[') {
                            if let Ok(parsed_arr) = serde_json::from_str::<Vec<String>>(s) {
                                for p in parsed_arr {
                                    if !p.trim().is_empty() {
                                        files.push(p.trim().to_string());
                                    }
                                }
                                continue;
                            }
                        }
                        files.push(s.to_string());
                    }
                    _ => {}
                }
            }
        }
    }

    // 去重保序
    let mut seen = HashSet::new();
    files.retain(|f| seen.insert(f.clone()));

    if files.is_empty() {
        return Err("局域网互联打包下载未指定任何文件或目录路径".to_string());
    }

    if folder_name.is_empty() || folder_name == "batch_download" {
        if files.len() == 1 {
            folder_name = basename_of_remote_path(&files[0]);
        } else {
            folder_name = format!("batch_download_{}_items", files.len());
        }
    }

    Ok((base_origin, folder_name, files, pin, qr_token))
}

fn basename_of_remote_path(remote_path: &str) -> String {
    remote_path
        .split(['\\', '/'])
        .filter(|s| !s.is_empty() && !s.ends_with(':'))
        .next_back()
        .unwrap_or("batch_download")
        .to_string()
}

fn join_zip_prefix(prefix: &str, name: &str) -> String {
    let clean_name = name.trim_matches(['/', '\\']).replace('\\', "/");
    if prefix.is_empty() {
        clean_name
    } else {
        format!("{}/{}", prefix.trim_matches(['/', '\\']).replace('\\', "/"), clean_name)
    }
}

/// 构造带认证参数与请求头的 GET 请求
fn build_authed_get(
    client: &reqwest::Client,
    url: &str,
    headers: &HashMap<String, String>,
    pin: Option<&str>,
    qr_token: Option<&str>,
) -> reqwest::RequestBuilder {
    let mut req = client.get(url).header(ACCEPT_ENCODING, "identity");
    for (k, v) in headers {
        if !k.eq_ignore_ascii_case("content-type") && !k.eq_ignore_ascii_case("content-length") {
            req = req.header(k, v);
        }
    }
    if let Some(p) = pin {
        req = req.header("x-pin", p);
    }
    if let Some(t) = qr_token {
        req = req.header("x-qr-token", t);
    }
    req
}

/// 调用 `GET /api/files?path=...` 获取单层目录列表（带 4 秒超时保护）
async fn fetch_dir_entries(
    client: &reqwest::Client,
    base_origin: &str,
    remote_path: &str,
    headers: &HashMap<String, String>,
    pin: Option<&str>,
    qr_token: Option<&str>,
) -> Option<Vec<ApiFileEntry>> {
    let mut url = Url::parse(&format!("{base_origin}/api/files")).ok()?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("path", remote_path);
        if let Some(p) = pin {
            qp.append_pair("pin", p);
        }
        if let Some(t) = qr_token {
            qp.append_pair("token", t);
        }
    }
    let req = build_authed_get(client, url.as_str(), headers, pin, qr_token)
        .timeout(Duration::from_secs(4));
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let data: ApiFilesResponse = resp.json().await.ok()?;
    Some(data.files)
}

/// 探测单个远程文件的字节大小（发送 `Range: bytes=0-0` 探针）
async fn probe_remote_file_size(
    client: &reqwest::Client,
    base_origin: &str,
    remote_path: &str,
    headers: &HashMap<String, String>,
    pin: Option<&str>,
    qr_token: Option<&str>,
) -> Option<u64> {
    let mut url = Url::parse(&format!("{base_origin}/api/download")).ok()?;
    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("path", remote_path);
        if let Some(p) = pin {
            qp.append_pair("pin", p);
        }
        if let Some(t) = qr_token {
            qp.append_pair("token", t);
        }
    }
    let req = build_authed_get(client, url.as_str(), headers, pin, qr_token)
        .header(RANGE, "bytes=0-0")
        .timeout(Duration::from_secs(4));
    let resp = req.send().await.ok()?;
    if resp.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        if let Some(cr) = resp.headers().get(CONTENT_RANGE).and_then(|v| v.to_str().ok()) {
            if let Some(total_str) = cr.split('/').nth(1) {
                if let Ok(total) = total_str.trim().parse::<u64>() {
                    return Some(total);
                }
            }
        }
    } else if resp.status().is_success() {
        return resp.content_length();
    }
    None
}

/// 将单个文件拆分为 1 个或多个 `SingleFileChunk` 分桶
fn push_file_chunks(
    buckets: &mut Vec<LanDiskBucket>,
    remote_path: &str,
    zip_entry_name: &str,
    size: u64,
) {
    if size == 0 {
        let index = buckets.len();
        buckets.push(LanDiskBucket {
            index,
            kind: BucketKind::SingleFileChunk {
                remote_path: remote_path.to_string(),
                zip_entry_name: zip_entry_name.to_string(),
                chunk_index: 0,
                offset: 0,
                length: 0,
                total_file_size: 0,
            },
        });
        return;
    }

    let mut offset = 0u64;
    let mut chunk_index = 0usize;
    while offset < size {
        let length = (size - offset).min(MAX_RANGE_CHUNK_BYTES);
        let index = buckets.len();
        buckets.push(LanDiskBucket {
            index,
            kind: BucketKind::SingleFileChunk {
                remote_path: remote_path.to_string(),
                zip_entry_name: zip_entry_name.to_string(),
                chunk_index,
                offset,
                length,
                total_file_size: size,
            },
        });
        offset += length;
        chunk_index += 1;
    }
}

/// 将同一父目录（相同 `zip_prefix`）下的一组目录或小文件切分为多个 `SubBatch` 分桶
fn push_grouped_subbatches(
    buckets: &mut Vec<LanDiskBucket>,
    entries: &[ApiFileEntry],
    zip_prefix: &str,
) {
    let mut small_files: Vec<&ApiFileEntry> = Vec::new();
    let mut subdirs: Vec<&ApiFileEntry> = Vec::new();

    for entry in entries {
        if entry.is_directory {
            subdirs.push(entry);
        } else if entry.size >= SINGLE_FILE_RANGE_THRESHOLD {
            let entry_zip_name = join_zip_prefix(zip_prefix, &entry.name);
            push_file_chunks(buckets, &entry.path, &entry_zip_name, entry.size);
        } else {
            small_files.push(entry);
        }
    }

    for chunk in small_files.chunks(MAX_SMALL_FILES_PER_SUBBATCH) {
        let paths = chunk.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
        let est_bytes: u64 = chunk.iter().map(|e| e.size.max(512)).sum();
        let index = buckets.len();
        buckets.push(LanDiskBucket {
            index,
            kind: BucketKind::SubBatch {
                remote_paths: paths,
                zip_prefix: zip_prefix.to_string(),
                estimated_bytes: est_bytes.max(4096),
            },
        });
    }

    for chunk in subdirs.chunks(MAX_DIRS_PER_SUBBATCH) {
        let paths = chunk.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
        let est_bytes = (paths.len() as u64) * ESTIMATED_SUBDIR_BYTES;
        let index = buckets.len();
        buckets.push(LanDiskBucket {
            index,
            kind: BucketKind::SubBatch {
                remote_paths: paths,
                zip_prefix: zip_prefix.to_string(),
                estimated_bytes: est_bytes,
            },
        });
    }
}

/// 构建或从磁盘恢复 `LanDiskManifest`（浅层 2~3 级并发探查，总耗时 < 2 秒）
pub async fn build_or_load_manifest(
    client: &reqwest::Client,
    task: &DownloadTask,
    temp_dir: &Path,
) -> Result<LanDiskManifest, String> {
    let manifest_path = temp_dir.join("landisk_manifest.json");
    if manifest_path.exists() {
        if let Ok(content) = fs::read_to_string(&manifest_path).await {
            if let Ok(existing) = serde_json::from_str::<LanDiskManifest>(&content) {
                if !existing.buckets.is_empty() {
                    return Ok(existing);
                }
            }
        }
    }

    let (base_origin, folder_name, root_paths, pin, qr_token) = parse_batch_request(task)?;
    let mut buckets: Vec<LanDiskBucket> = Vec::new();
    let mut root_fallback_paths: Vec<String> = Vec::new();

    for root_path in &root_paths {
        let root_name = basename_of_remote_path(root_path);
        match fetch_dir_entries(
            client,
            &base_origin,
            root_path,
            &task.headers,
            pin.as_deref(),
            qr_token.as_deref(),
        )
        .await
        {
            Some(level0_entries) => {
                // root_path 是目录：在 ZIP 内部顶层前缀为 `root_name`
                let mut l0_files_and_small = Vec::new();
                let mut l0_dirs = Vec::new();
                for entry in level0_entries {
                    if entry.is_directory {
                        l0_dirs.push(entry);
                    } else {
                        l0_files_and_small.push(entry);
                    }
                }
                push_grouped_subbatches(&mut buckets, &l0_files_and_small, &root_name);

                // 并发拉取 Level 1 子目录（上限 16 个）
                let (expand_l1, remain_l1) = if l0_dirs.len() > 16 {
                    l0_dirs.split_at(16)
                } else {
                    (l0_dirs.as_slice(), &[][..])
                };
                if !remain_l1.is_empty() {
                    push_grouped_subbatches(&mut buckets, remain_l1, &root_name);
                }

                let l1_futs = expand_l1.iter().map(|d| {
                    let prefix = join_zip_prefix(&root_name, &d.name);
                    let path = d.path.clone();
                    let name = d.name.clone();
                    let base = base_origin.clone();
                    let headers = task.headers.clone();
                    let pin_c = pin.clone();
                    let tok_c = qr_token.clone();
                    async move {
                        let entries = fetch_dir_entries(
                            client,
                            &base,
                            &path,
                            &headers,
                            pin_c.as_deref(),
                            tok_c.as_deref(),
                        )
                        .await;
                        (name, path, prefix, entries)
                    }
                });
                let l1_results = join_all(l1_futs).await;

                let mut l2_expand_targets: Vec<(String, String)> = Vec::new();
                for (_dir_name, dir_path, dir_prefix, entries_opt) in l1_results {
                    match entries_opt {
                        Some(l1_entries) => {
                            let mut l1_non_expand = Vec::new();
                            for item in l1_entries {
                                // 对包含大量子包或分片的目录（如 Library/PackageCache、Library/Artifacts 等）再展开一层
                                let should_expand_l2 = item.is_directory
                                    && l2_expand_targets.len() < 16
                                    && matches!(
                                        item.name.as_str(),
                                        "PackageCache"
                                            | "Artifacts"
                                            | "ScriptAssemblies"
                                            | "Bee"
                                            | "ShaderCache"
                                            | "BurstCache"
                                            | "Search"
                                    );
                                if should_expand_l2 {
                                    let sub_prefix = join_zip_prefix(&dir_prefix, &item.name);
                                    l2_expand_targets.push((item.path, sub_prefix));
                                } else {
                                    l1_non_expand.push(item);
                                }
                            }
                            push_grouped_subbatches(&mut buckets, &l1_non_expand, &dir_prefix);
                        }
                        None => {
                            let idx = buckets.len();
                            buckets.push(LanDiskBucket {
                                index: idx,
                                kind: BucketKind::SubBatch {
                                    remote_paths: vec![dir_path],
                                    zip_prefix: root_name.clone(),
                                    estimated_bytes: ESTIMATED_SUBDIR_BYTES,
                                },
                            });
                        }
                    }
                }

                // 并发拉取重点 Level 2 目录（如 PackageCache、Artifacts、Bee 等）
                if !l2_expand_targets.is_empty() {
                    let l2_futs = l2_expand_targets.into_iter().map(|(path, prefix)| {
                        let base = base_origin.clone();
                        let headers = task.headers.clone();
                        let pin_c = pin.clone();
                        let tok_c = qr_token.clone();
                        async move {
                            let entries = fetch_dir_entries(
                                client,
                                &base,
                                &path,
                                &headers,
                                pin_c.as_deref(),
                                tok_c.as_deref(),
                            )
                            .await;
                            (path, prefix, entries)
                        }
                    });
                    for (path, prefix, entries_opt) in join_all(l2_futs).await {
                        match entries_opt {
                            Some(l2_entries) => {
                                push_grouped_subbatches(&mut buckets, &l2_entries, &prefix);
                            }
                            None => {
                                let parent_prefix = prefix
                                    .rsplit_once('/')
                                    .map(|(p, _)| p.to_string())
                                    .unwrap_or_default();
                                let idx = buckets.len();
                                buckets.push(LanDiskBucket {
                                    index: idx,
                                    kind: BucketKind::SubBatch {
                                        remote_paths: vec![path],
                                        zip_prefix: parent_prefix,
                                        estimated_bytes: ESTIMATED_SUBDIR_BYTES,
                                    },
                                });
                            }
                        }
                    }
                }
            }
            None => {
                // root_path 可能是单个文件：尝试探测其精确字节大小以启用 Range 并发切片
                if let Some(file_size) = probe_remote_file_size(
                    client,
                    &base_origin,
                    root_path,
                    &task.headers,
                    pin.as_deref(),
                    qr_token.as_deref(),
                )
                .await
                {
                    push_file_chunks(&mut buckets, root_path, &root_name, file_size);
                } else {
                    root_fallback_paths.push(root_path.clone());
                }
            }
        }
    }

    if !root_fallback_paths.is_empty() {
        for chunk in root_fallback_paths.chunks(MAX_SMALL_FILES_PER_SUBBATCH) {
            let idx = buckets.len();
            buckets.push(LanDiskBucket {
                index: idx,
                kind: BucketKind::SubBatch {
                    remote_paths: chunk.to_vec(),
                    zip_prefix: String::new(),
                    estimated_bytes: (chunk.len() as u64) * ESTIMATED_SUBDIR_BYTES,
                },
            });
        }
    }

    // 重排索引：让 SingleFileChunk（支持字节级 Range 实时落盘）与小 SubBatch 交替执行，
    // 确保无论用户在第 1 秒还是第 30 秒点击暂停，磁盘上都已有字节级落盘数据。
    let mut chunk_buckets = VecDeque::new();
    let mut subbatch_buckets = VecDeque::new();
    for b in buckets {
        match &b.kind {
            BucketKind::SingleFileChunk { .. } => chunk_buckets.push_back(b),
            BucketKind::SubBatch { .. } => subbatch_buckets.push_back(b),
        }
    }
    let mut interleaved = Vec::with_capacity(chunk_buckets.len() + subbatch_buckets.len());
    while !chunk_buckets.is_empty() || !subbatch_buckets.is_empty() {
        for _ in 0..2 {
            if let Some(cb) = chunk_buckets.pop_front() {
                interleaved.push(cb);
            }
        }
        if let Some(sb) = subbatch_buckets.pop_front() {
            interleaved.push(sb);
        }
    }
    let mut buckets = interleaved;
    for (idx, b) in buckets.iter_mut().enumerate() {
        b.index = idx;
    }

    let manifest = LanDiskManifest {
        base_origin,
        folder_name,
        pin,
        qr_token,
        buckets,
    };

    if let Ok(json_str) = serde_json::to_string_pretty(&manifest) {
        let _ = fs::write(&manifest_path, json_str).await;
    }

    Ok(manifest)
}

fn bucket_raw_path(temp_dir: &Path, index: usize) -> PathBuf {
    temp_dir.join(format!("landisk_bucket_{index}.raw"))
}

fn bucket_zip_path(temp_dir: &Path, index: usize) -> PathBuf {
    temp_dir.join(format!("landisk_bucket_{index}.zip"))
}

fn bucket_zip_part_path(temp_dir: &Path, index: usize) -> PathBuf {
    temp_dir.join(format!("landisk_bucket_{index}.zip.part"))
}

/// 校验磁盘上已存在的 SubBatch `.zip` 分桶是否完整可读（含 Central Directory）
fn is_valid_zip_file(path: &Path) -> Option<u64> {
    let file = StdFile::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len < 22 {
        return None;
    }
    let reader = BufReader::new(file);
    let _archive = ZipArchive::new(reader).ok()?;
    Some(len)
}

/// 执行局域网互联单任务分桶并发断点续传下载，并在完成后组装为单个标准 `.zip` 文件
pub async fn download_landisk_zip_task(
    manager: &DownloadManager,
    mut task: DownloadTask,
    client: &reqwest::Client,
    temp_path: &Path,
    token: CancellationToken,
    task_limiter: Arc<RateLimiter>,
) -> Result<DownloadTask, String> {
    let temp_dir = ensure_task_temp_dir(&task.destination, &task.id).await?;

    let manifest = build_or_load_manifest(client, &task, &temp_dir).await?;
    if token.is_cancelled() {
        return Err("任务已暂停".to_string());
    }
    if manifest.buckets.is_empty() {
        return Err("未找到任何可下载的分桶条目".to_string());
    }

    let connections = if task.connection_count <= 1 {
        16
    } else {
        task.connection_count.clamp(4, 32)
    };
    task.connection_count = connections;
    let runtime_options = Arc::new(RuntimeTaskOptions::new(&task));
    manager
        .task_runtime
        .write()
        .await
        .insert(task.id.clone(), runtime_options.clone());

    // 扫描每个分桶在磁盘上的已完成/已落盘字节数（实现 100% 真实物理续传）
    let mut pending_buckets: VecDeque<LanDiskBucket> = VecDeque::new();
    let mut initial_persisted_bytes: u64 = 0;
    let mut initial_total_bytes: u64 = 0;
    let bucket_count = manifest.buckets.len();
    let mut segments: Vec<DownloadSegment> = Vec::with_capacity(bucket_count);

    for bucket in &manifest.buckets {
        match &bucket.kind {
            BucketKind::SingleFileChunk { offset, length, .. } => {
                let raw_path = bucket_raw_path(&temp_dir, bucket.index);
                let existing_len = fs::metadata(&raw_path)
                    .await
                    .map(|m| m.len().min(*length))
                    .unwrap_or(0);
                initial_persisted_bytes = initial_persisted_bytes.saturating_add(existing_len);
                initial_total_bytes = initial_total_bytes.saturating_add(*length);

                let done = existing_len >= *length;
                segments.push(DownloadSegment {
                    index: (bucket.index.min(u8::MAX as usize)) as u8,
                    start_byte: *offset,
                    end_byte: offset.saturating_add(*length),
                    downloaded_bytes: existing_len,
                    status: if done {
                        "completed".into()
                    } else if existing_len > 0 {
                        "paused".into()
                    } else {
                        "pending".into()
                    },
                });
                if !done {
                    pending_buckets.push_back(bucket.clone());
                }
            }
            BucketKind::SubBatch {
                estimated_bytes, ..
            } => {
                let zip_path = bucket_zip_path(&temp_dir, bucket.index);
                if let Some(valid_len) = is_valid_zip_file(&zip_path) {
                    initial_persisted_bytes = initial_persisted_bytes.saturating_add(valid_len);
                    initial_total_bytes = initial_total_bytes.saturating_add(valid_len);
                    segments.push(DownloadSegment {
                        index: (bucket.index.min(u8::MAX as usize)) as u8,
                        start_byte: 0,
                        end_byte: valid_len,
                        downloaded_bytes: valid_len,
                        status: "completed".into(),
                    });
                } else {
                    let _ = fs::remove_file(&zip_path).await;
                    let _ = fs::remove_file(bucket_zip_part_path(&temp_dir, bucket.index)).await;
                    let est = (*estimated_bytes).max(4096);
                    initial_total_bytes = initial_total_bytes.saturating_add(est);
                    segments.push(DownloadSegment {
                        index: (bucket.index.min(u8::MAX as usize)) as u8,
                        start_byte: 0,
                        end_byte: est,
                        downloaded_bytes: 0,
                        status: "pending".into(),
                    });
                    pending_buckets.push_back(bucket.clone());
                }
            }
        }
    }

    // 保持 UI 进度单调不回退：若上次暂停时记录了含在途分桶的进度水位线，保留水位线直至真实进度超越
    let high_water_mark = task.downloaded_bytes.max(initial_persisted_bytes);
    let progress_bytes = Arc::new(AtomicU64::new(initial_persisted_bytes));
    let dynamic_total_bytes = Arc::new(AtomicU64::new(
        initial_total_bytes.max(high_water_mark.saturating_add(1024)),
    ));
    let shared_segments = Arc::new(Mutex::new(segments));

    let active_workers = connections.min(pending_buckets.len().max(1) as u8);
    task.status = TaskStatus::Downloading;
    task.accepts_ranges = Some(true);
    task.active_connections = active_workers;
    task.downloaded_bytes = high_water_mark;
    task.total_bytes = dynamic_total_bytes.load(Ordering::Relaxed);
    task.segments = shared_segments.lock().await.clone();
    let _ = manager.store.upsert_task(&task).await;
    manager.emit_task("updated", &task);

    // 若所有分桶均已完成，直接进入本地 ZIP 合流组装阶段
    if !pending_buckets.is_empty() {
        let queue = Arc::new(Mutex::new(pending_buckets));
        let reporter_token = token.clone();
        let reporter_progress = progress_bytes.clone();
        let reporter_total = dynamic_total_bytes.clone();
        let reporter_segments = shared_segments.clone();
        let reporter_store = manager.store.clone();
        let reporter_app = manager.app.clone();
        let mut reporter_task = task.clone();
        let reporter_runtime_opts = runtime_options.clone();

        let reporter_handle = tokio::spawn(async move {
            let mut last_raw_bytes = reporter_progress.load(Ordering::Relaxed);
            let mut smoothed_speed: u64 = 0;
            let mut interval = tokio::time::interval(Duration::from_millis(350));
            loop {
                tokio::select! {
                    _ = reporter_token.cancelled() => break,
                    _ = interval.tick() => {
                        if reporter_token.is_cancelled() {
                            break;
                        }
                        let raw_now = reporter_progress.load(Ordering::Relaxed);
                        let delta = raw_now.saturating_sub(last_raw_bytes);
                        last_raw_bytes = raw_now;
                        let instant_speed = (delta * 1000) / 350;
                        smoothed_speed = if smoothed_speed == 0 {
                            instant_speed
                        } else {
                            (smoothed_speed * 6 + instant_speed * 4) / 10
                        };

                        let display_dl = raw_now.max(high_water_mark);
                        let display_total = reporter_total
                            .load(Ordering::Relaxed)
                            .max(display_dl.saturating_add(4096));

                        reporter_task.downloaded_bytes = display_dl;
                        reporter_task.total_bytes = display_total;
                        reporter_task.speed = smoothed_speed;
                        reporter_task.eta_seconds = if smoothed_speed > 0 {
                            Some(display_total.saturating_sub(display_dl) / smoothed_speed)
                        } else {
                            None
                        };
                        reporter_task.status = TaskStatus::Downloading;
                        reporter_task.active_connections = active_workers;
                        reporter_task.segments = reporter_segments.lock().await.clone();
                        reporter_runtime_opts.apply(&mut reporter_task).await;

                        if !reporter_token.is_cancelled() {
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
            }
        });

        let mut worker_handles = Vec::with_capacity(active_workers as usize);
        let first_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        for _ in 0..active_workers {
            let w_queue = queue.clone();
            let w_token = token.clone();
            let w_client = client.clone();
            let w_temp_dir = temp_dir.clone();
            let w_headers = task.headers.clone();
            let w_base = manifest.base_origin.clone();
            let w_pin = manifest.pin.clone();
            let w_tok = manifest.qr_token.clone();
            let w_progress = progress_bytes.clone();
            let w_total = dynamic_total_bytes.clone();
            let w_segments = shared_segments.clone();
            let w_err = first_error.clone();
            let w_limiter = task_limiter.clone();
            let w_runtime = runtime_options.clone();
            let w_bw = manager.bandwidth_scheduler.clone();
            let w_task_id = task.id.clone();

            worker_handles.push(tokio::spawn(async move {
                loop {
                    if w_token.is_cancelled() {
                        break;
                    }
                    let next_bucket = {
                        let mut g = w_queue.lock().await;
                        g.pop_front()
                    };
                    let Some(bucket) = next_bucket else {
                        break;
                    };

                    {
                        let mut segs = w_segments.lock().await;
                        if let Some(s) = segs.get_mut(bucket.index) {
                            s.status = "downloading".into();
                        }
                    }

                    let res = execute_bucket_download(
                        &w_client,
                        &w_base,
                        &bucket,
                        &w_temp_dir,
                        &w_headers,
                        w_pin.as_deref(),
                        w_tok.as_deref(),
                        &w_token,
                        &w_progress,
                        &w_total,
                        &w_segments,
                        &w_limiter,
                        &w_runtime,
                        &w_bw,
                        &w_task_id,
                    )
                    .await;

                    if let Err(err_msg) = res {
                        if !w_token.is_cancelled() {
                            let mut err_guard = w_err.lock().await;
                            if err_guard.is_none() {
                                *err_guard = Some(err_msg);
                            }
                            w_token.cancel();
                        }
                        break;
                    }
                }
            }));
        }

        for h in worker_handles {
            let _ = h.await;
        }
        reporter_handle.abort();
        let _ = reporter_handle.await;

        if let Some(err_msg) = first_error.lock().await.take() {
            return Err(err_msg);
        }

        if token.is_cancelled() {
            // 用户点击了暂停：将已精确落盘的进度与分片状态写回数据库，确保不覆盖 Paused 状态且进度绝不丢失
            let final_raw = progress_bytes.load(Ordering::Relaxed);
            let saved_dl = final_raw.max(high_water_mark);
            let saved_total = dynamic_total_bytes
                .load(Ordering::Relaxed)
                .max(saved_dl.saturating_add(4096));
            let mut segs = shared_segments.lock().await.clone();
            for s in &mut segs {
                if s.status == "downloading" {
                    s.status = "paused".into();
                }
            }
            if let Ok(Some(mut stored_task)) = manager.store.get_task(&task.id).await {
                if matches!(stored_task.status, TaskStatus::Downloading | TaskStatus::Queued) {
                    stored_task.status = TaskStatus::Paused;
                }
                stored_task.downloaded_bytes = saved_dl;
                stored_task.total_bytes = saved_total;
                stored_task.accepts_ranges = Some(true);
                stored_task.speed = 0;
                stored_task.eta_seconds = None;
                stored_task.active_connections = 0;
                stored_task.segments = segs;
                let _ = manager.store.upsert_task(&stored_task).await;
                manager.emit_task("updated", &stored_task);
            }
            return Err("任务已暂停".to_string());
        }
    }

    // 所有分桶已落盘完成：在本地执行零重压缩 ZIP 合流组装
    task.status = TaskStatus::Verifying;
    task.speed = 0;
    task.active_connections = 0;
    let _ = manager.store.upsert_task(&task).await;
    manager.emit_task("updated", &task);

    let temp_dir_clone = temp_dir.clone();
    let temp_path_buf = temp_path.to_path_buf();
    let manifest_clone = manifest.clone();
    let final_zip_bytes = tokio::task::spawn_blocking(move || {
        assemble_final_zip(&manifest_clone, &temp_dir_clone, &temp_path_buf)
    })
    .await
    .map_err(|e| format!("ZIP 合流线程异常: {e}"))??;

    let mut final_segments = shared_segments.lock().await.clone();
    for s in &mut final_segments {
        s.status = "completed".into();
        s.downloaded_bytes = s.end_byte.saturating_sub(s.start_byte);
    }

    task.downloaded_bytes = final_zip_bytes;
    task.total_bytes = final_zip_bytes;
    task.segments = final_segments;
    Ok(task)
}

#[allow(clippy::too_many_arguments)]
async fn execute_bucket_download(
    client: &reqwest::Client,
    base_origin: &str,
    bucket: &LanDiskBucket,
    temp_dir: &Path,
    headers: &HashMap<String, String>,
    pin: Option<&str>,
    qr_token: Option<&str>,
    token: &CancellationToken,
    progress_bytes: &Arc<AtomicU64>,
    dynamic_total_bytes: &Arc<AtomicU64>,
    shared_segments: &Arc<Mutex<Vec<DownloadSegment>>>,
    task_limiter: &Arc<RateLimiter>,
    runtime_options: &Arc<RuntimeTaskOptions>,
    bandwidth_scheduler: &crate::manager::bandwidth::BandwidthScheduler,
    task_id: &str,
) -> Result<(), String> {
    match &bucket.kind {
        BucketKind::SingleFileChunk {
            remote_path,
            offset,
            length,
            ..
        } => {
            if *length == 0 {
                let raw_path = bucket_raw_path(temp_dir, bucket.index);
                let _ = StdFile::create(&raw_path);
                let mut segs = shared_segments.lock().await;
                if let Some(s) = segs.get_mut(bucket.index) {
                    s.status = "completed".into();
                }
                return Ok(());
            }

            let raw_path = bucket_raw_path(temp_dir, bucket.index);
            let existing = fs::metadata(&raw_path)
                .await
                .map(|m| m.len().min(*length))
                .unwrap_or(0);
            if existing >= *length {
                let mut segs = shared_segments.lock().await;
                if let Some(s) = segs.get_mut(bucket.index) {
                    s.downloaded_bytes = *length;
                    s.status = "completed".into();
                }
                return Ok(());
            }

            let range_start = offset + existing;
            let range_end = offset + length - 1;
            let mut url =
                Url::parse(&format!("{base_origin}/api/download")).map_err(|e| e.to_string())?;
            {
                let mut qp = url.query_pairs_mut();
                qp.append_pair("path", remote_path);
                if let Some(p) = pin {
                    qp.append_pair("pin", p);
                }
                if let Some(t) = qr_token {
                    qp.append_pair("token", t);
                }
            }

            let req = build_authed_get(client, url.as_str(), headers, pin, qr_token)
                .header(RANGE, format!("bytes={range_start}-{range_end}"));
            let resp = req.send().await.map_err(friendly_reqwest)?;
            let is_partial = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
            if !resp.status().is_success() && !is_partial {
                return Err(format!(
                    "拉取文件切片失败 ({}): HTTP {}",
                    remote_path,
                    resp.status()
                ));
            }

            // 若服务端未返回 206（极少数回退情况且 offset == 0），从头写入
            let append_mode = existing > 0 && is_partial;
            if existing > 0 && !is_partial {
                progress_bytes.fetch_sub(existing, Ordering::Relaxed);
            }

            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .append(append_mode)
                .truncate(!append_mode)
                .open(&raw_path)
                .await
                .map_err(|e| format!("打开分片临时文件失败: {e}"))?;
            let mut writer = BufWriter::with_capacity(256 * 1024, file);
            let mut stream = resp.bytes_stream();
            let mut written_in_chunk = if append_mode { existing } else { 0 };

            while let Some(chunk_res) = tokio::select! {
                _ = token.cancelled() => None,
                item = stream.next() => item,
            } {
                let chunk = chunk_res.map_err(friendly_body_error)?;
                let remain_allow = (*length).saturating_sub(written_in_chunk) as usize;
                let slice = &chunk[..chunk.len().min(remain_allow)];
                if slice.is_empty() {
                    break;
                }

                let len_u64 = slice.len() as u64;
                let per_limit = runtime_options.speed_limit.load(Ordering::Relaxed);
                if per_limit > 0 {
                    task_limiter.acquire(len_u64, per_limit).await;
                }
                bandwidth_scheduler.acquire(task_id, len_u64, 0, token).await;

                writer
                    .write_all(slice)
                    .await
                    .map_err(|e| format!("写入分片数据失败: {e}"))?;
                written_in_chunk += len_u64;
                progress_bytes.fetch_add(len_u64, Ordering::Relaxed);

                {
                    let mut segs = shared_segments.lock().await;
                    if let Some(s) = segs.get_mut(bucket.index) {
                        s.downloaded_bytes = written_in_chunk;
                    }
                }

                if written_in_chunk >= *length {
                    break;
                }
            }

            let _ = writer.flush().await;
            if token.is_cancelled() {
                return Err("任务已暂停".to_string());
            }
            if written_in_chunk < *length {
                return Err(format!(
                    "文件分片未完整传输 ({written_in_chunk} / {length} 字节)"
                ));
            }

            {
                let mut segs = shared_segments.lock().await;
                if let Some(s) = segs.get_mut(bucket.index) {
                    s.downloaded_bytes = *length;
                    s.status = "completed".into();
                }
            }
            Ok(())
        }
        BucketKind::SubBatch {
            remote_paths,
            estimated_bytes,
            ..
        } => {
            let zip_path = bucket_zip_path(temp_dir, bucket.index);
            if is_valid_zip_file(&zip_path).is_some() {
                return Ok(());
            }
            let part_path = bucket_zip_part_path(temp_dir, bucket.index);

            let mut url = Url::parse(&format!("{base_origin}/api/download/batch"))
                .map_err(|e| e.to_string())?;
            {
                let mut qp = url.query_pairs_mut();
                if let Some(p) = pin {
                    qp.append_pair("pin", p);
                }
                if let Some(t) = qr_token {
                    qp.append_pair("token", t);
                }
            }

            let body_json = serde_json::json!({
                "folderName": format!("bucket_{}", bucket.index),
                "files": remote_paths,
            })
            .to_string();

            let mut req = client
                .post(url.as_str())
                .header(ACCEPT_ENCODING, "identity")
                .header(CONTENT_TYPE, "application/json")
                .body(body_json);
            for (k, v) in headers {
                if !k.eq_ignore_ascii_case("content-type")
                    && !k.eq_ignore_ascii_case("content-length")
                {
                    req = req.header(k, v);
                }
            }
            if let Some(p) = pin {
                req = req.header("x-pin", p);
            }
            if let Some(t) = qr_token {
                req = req.header("x-qr-token", t);
            }

            let resp = req.send().await.map_err(friendly_reqwest)?;
            if !resp.status().is_success() {
                return Err(format!("拉取压缩分桶失败: HTTP {}", resp.status()));
            }

            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&part_path)
                .await
                .map_err(|e| format!("创建压缩分桶临时文件失败: {e}"))?;
            let mut writer = BufWriter::with_capacity(256 * 1024, file);
            let mut stream = resp.bytes_stream();
            let mut received: u64 = 0;
            let est = (*estimated_bytes).max(4096);

            while let Some(chunk_res) = tokio::select! {
                _ = token.cancelled() => None,
                item = stream.next() => item,
            } {
                let chunk = chunk_res.map_err(friendly_body_error)?;
                if chunk.is_empty() {
                    continue;
                }
                let len_u64 = chunk.len() as u64;
                let per_limit = runtime_options.speed_limit.load(Ordering::Relaxed);
                if per_limit > 0 {
                    task_limiter.acquire(len_u64, per_limit).await;
                }
                bandwidth_scheduler.acquire(task_id, len_u64, 0, token).await;

                writer
                    .write_all(&chunk)
                    .await
                    .map_err(|e| format!("写入压缩分桶失败: {e}"))?;

                let prev_received = received;
                received += len_u64;
                progress_bytes.fetch_add(len_u64, Ordering::Relaxed);
                if received > est {
                    let extra = received - prev_received.max(est);
                    dynamic_total_bytes.fetch_add(extra, Ordering::Relaxed);
                }

                {
                    let mut segs = shared_segments.lock().await;
                    if let Some(s) = segs.get_mut(bucket.index) {
                        s.downloaded_bytes = received;
                        if received > s.end_byte {
                            s.end_byte = received;
                        }
                    }
                }
            }

            let _ = writer.flush().await;
            drop(writer);

            if token.is_cancelled() {
                // 未完成的流式分桶不作为有效 zip，回退其计入的字节数（UI 水位线会防止界面数字跳变）
                progress_bytes.fetch_sub(received, Ordering::Relaxed);
                let _ = fs::remove_file(&part_path).await;
                return Err("任务已暂停".to_string());
            }

            if received <est {
                let diff = est - received;
                let cur_tot = dynamic_total_bytes.load(Ordering::Relaxed);
                let cur_dl = progress_bytes.load(Ordering::Relaxed);
                if cur_tot > cur_dl + diff {
                    dynamic_total_bytes.fetch_sub(diff, Ordering::Relaxed);
                }
            }

            fs::rename(&part_path, &zip_path)
                .await
                .map_err(|e| format!("重命名完成分桶失败: {e}"))?;

            {
                let mut segs = shared_segments.lock().await;
                if let Some(s) = segs.get_mut(bucket.index) {
                    s.end_byte = received;
                    s.downloaded_bytes = received;
                    s.status = "completed".into();
                }
            }
            Ok(())
        }
    }
}

/// 将所有已完成的分桶（`SingleFileChunk` `.raw` 与 `SubBatch` `.zip`）无损合并为单个目标 `.zip`
pub fn assemble_final_zip(
    manifest: &LanDiskManifest,
    temp_dir: &Path,
    output_temp_path: &Path,
) -> Result<u64, String> {
    if let Some(parent) = output_temp_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let out_file =
        StdFile::create(output_temp_path).map_err(|e| format!("创建目标压缩包失败: {e}"))?;
    let buf_writer = StdBufWriter::with_capacity(1024 * 1024, out_file);
    let mut zip_writer = ZipWriter::new(buf_writer);
    let mut seen_entries: HashSet<String> = HashSet::new();

    // 1. 按 `zip_entry_name` 聚合所有 SingleFileChunk 分桶并按 chunk_index 顺序写入
    let mut single_files_map: HashMap<String, Vec<(usize, usize, u64)>> = HashMap::new();
    let mut single_files_order: Vec<String> = Vec::new();

    for bucket in &manifest.buckets {
        if let BucketKind::SingleFileChunk {
            zip_entry_name,
            chunk_index,
            total_file_size,
            ..
        } = &bucket.kind
        {
            if !single_files_map.contains_key(zip_entry_name) {
                single_files_order.push(zip_entry_name.clone());
            }
            single_files_map.entry(zip_entry_name.clone()).or_default().push((
                *chunk_index,
                bucket.index,
                *total_file_size,
            ));
        }
    }

    let mut copy_buf = vec![0u8; 256 * 1024];
    for entry_name in single_files_order {
        let clean_name = entry_name.trim_matches('/').replace('\\', "/");
        if clean_name.is_empty() || !seen_entries.insert(clean_name.clone()) {
            continue;
        }
        let Some(mut chunks) = single_files_map.remove(&entry_name) else {
            continue;
        };
        chunks.sort_by_key(|(chunk_idx, _, _)| *chunk_idx);
        let total_size = chunks.first().map(|(_, _, sz)| *sz).unwrap_or(0);

        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .large_file(total_size >= 0xFFFF_FFFF);
        zip_writer
            .start_file(&clean_name, options)
            .map_err(|e| format!("写入 ZIP 条目头失败 ({clean_name}): {e}"))?;

        for (_chunk_idx, bucket_index, _) in chunks {
            let raw_path = bucket_raw_path(temp_dir, bucket_index);
            if total_size == 0 && !raw_path.exists() {
                continue;
            }
            let mut raw_file = StdFile::open(&raw_path)
                .map_err(|e| format!("读取已完成分片失败 ({:?}): {e}", raw_path))?;
            loop {
                let n = raw_file
                    .read(&mut copy_buf)
                    .map_err(|e| format!("读取分片字节失败: {e}"))?;
                if n == 0 {
                    break;
                }
                zip_writer
                    .write_all(&copy_buf[..n])
                    .map_err(|e| format!("写入目标 ZIP 失败: {e}"))?;
            }
        }
    }

    // 2. 合并所有 SubBatch `.zip` 分桶（使用 raw_copy_file_rename 直接拷贝压缩块，0 CPU 重压缩开销）
    for bucket in &manifest.buckets {
        if let BucketKind::SubBatch { zip_prefix, .. } = &bucket.kind {
            let zip_path = bucket_zip_path(temp_dir, bucket.index);
            let file = StdFile::open(&zip_path)
                .map_err(|e| format!("打开分桶压缩包失败 ({:?}): {e}", zip_path))?;
            let mut archive = ZipArchive::new(BufReader::new(file))
                .map_err(|e| format!("解析分桶压缩包失败 ({:?}): {e}", zip_path))?;

            for i in 0..archive.len() {
                let entry = archive
                    .by_index(i)
                    .map_err(|e| format!("读取分桶压缩包条目失败: {e}"))?;
                let raw_entry_name = entry.name().to_string();
                let is_dir = entry.is_dir();
                let target_name = join_zip_prefix(zip_prefix, &raw_entry_name);
                let normalized_name = if is_dir && !target_name.ends_with('/') {
                    format!("{target_name}/")
                } else {
                    target_name
                };

                if normalized_name.trim_matches('/').is_empty()
                    || !seen_entries.insert(normalized_name.clone())
                {
                    continue;
                }

                if is_dir {
                    drop(entry);
                    let _ = zip_writer.add_directory(&normalized_name, SimpleFileOptions::default());
                } else {
                    zip_writer
                        .raw_copy_file_rename(entry, &normalized_name)
                        .map_err(|e| format!("零拷贝合流 ZIP 条目失败 ({normalized_name}): {e}"))?;
                }
            }
        }
    }

    let inner_writer = zip_writer
        .finish()
        .map_err(|e| format!("完成目标 ZIP 归档失败: {e}"))?;
    let out_file = inner_writer
        .into_inner()
        .map_err(|e| format!("刷新目标 ZIP 缓冲区失败: {e}"))?;
    let final_len = out_file
        .metadata()
        .map(|m| m.len())
        .unwrap_or(0);
    Ok(final_len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn test_parse_batch_request_urlencoded_and_json() {
        let mut task = DownloadTask {
            id: "t1".into(),
            url: "http://100.126.152.101:3928/api/download/batch?pin=888888".into(),
            file_name: "My project.zip".into(),
            destination: "D:\\Downloads".into(),
            total_bytes: 0,
            downloaded_bytes: 0,
            speed: 0,
            eta_seconds: None,
            status: TaskStatus::Queued,
            error: None,
            created_at: 0,
            completed_at: None,
            scheduled_at: None,
            category: "zip".into(),
            queue_position: 0,
            priority: 0,
            retry_count: 0,
            max_retries: 3,
            checksum_sha256: None,
            expected_checksum: None,
            source: "browser".into(),
            etag: None,
            last_modified: None,
            final_url: None,
            response_status: None,
            content_type: None,
            accepts_ranges: None,
            headers: HashMap::new(),
            media: None,
            per_task_speed_limit: 0,
            collision_policy: Default::default(),
            completion_action: Default::default(),
            connection_count: 16,
            active_connections: 0,
            segments: Vec::new(),
            retry_policy_override: None,
            proxy_override: None,
            proxy_auth: None,
            task_kind: Default::default(),
            bt_meta: None,
            bt_runtime: None,
            cloud_refresh: None,
            method: Some("POST".into()),
            body: Some("folderName=My+project&files=C%3A%5CUnity%5CMy+project&pin=888888".into()),
        };

        let (origin, folder, files, pin, _) = parse_batch_request(&task).unwrap();
        assert_eq!(origin, "http://100.126.152.101:3928");
        assert_eq!(folder, "My project");
        assert_eq!(files, vec![r"C:\Unity\My project".to_string()]);
        assert_eq!(pin.as_deref(), Some("888888"));

        task.body = Some(
            serde_json::json!({
                "folderName": "CustomFolder",
                "files": [r"D:\Data\A", r"D:\Data\B"]
            })
            .to_string(),
        );
        let (_, folder2, files2, _, _) = parse_batch_request(&task).unwrap();
        assert_eq!(folder2, "CustomFolder");
        assert_eq!(files2.len(), 2);
    }

    #[test]
    fn test_assemble_final_zip_merges_raw_chunks_and_subbatch_zips() {
        let dir = tempfile::tempdir().unwrap();
        let temp_dir = dir.path();

        // 构造 2 个 SingleFileChunk 分片（模拟一个大文件的 2 个 Range 切片）
        std::fs::write(bucket_raw_path(temp_dir, 0), b"Hello, ").unwrap();
        std::fs::write(bucket_raw_path(temp_dir, 1), b"Resumable World!").unwrap();

        // 构造 1 个 SubBatch .zip 分桶（模拟服务端 /api/download/batch 返回的子目录压缩包）
        {
            let sub_zip_file = StdFile::create(bucket_zip_path(temp_dir, 2)).unwrap();
            let mut sub_writer = ZipWriter::new(sub_zip_file);
            sub_writer
                .start_file(
                    "PackageCache/pkg.json",
                    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
                )
                .unwrap();
            sub_writer.write_all(b"{\"name\":\"unity.pkg\"}").unwrap();
            sub_writer.finish().unwrap();
        }

        let manifest = LanDiskManifest {
            base_origin: "http://127.0.0.1:3928".into(),
            folder_name: "My project".into(),
            pin: None,
            qr_token: None,
            buckets: vec![
                LanDiskBucket {
                    index: 0,
                    kind: BucketKind::SingleFileChunk {
                        remote_path: r"C:\Unity\My project\Library\ArtifactDB".into(),
                        zip_entry_name: "My project/Library/ArtifactDB".into(),
                        chunk_index: 0,
                        offset: 0,
                        length: 7,
                        total_file_size: 23,
                    },
                },
                LanDiskBucket {
                    index: 1,
                    kind: BucketKind::SingleFileChunk {
                        remote_path: r"C:\Unity\My project\Library\ArtifactDB".into(),
                        zip_entry_name: "My project/Library/ArtifactDB".into(),
                        chunk_index: 1,
                        offset: 7,
                        length: 16,
                        total_file_size: 23,
                    },
                },
                LanDiskBucket {
                    index: 2,
                    kind: BucketKind::SubBatch {
                        remote_paths: vec![r"C:\Unity\My project\Library\PackageCache".into()],
                        zip_prefix: "My project/Library".into(),
                        estimated_bytes: 100,
                    },
                },
            ],
        };

        let out_zip = temp_dir.join("My project.zip.lumaget");
        let size = assemble_final_zip(&manifest, temp_dir, &out_zip).unwrap();
        assert!(size > 0);

        let reader = StdFile::open(&out_zip).unwrap();
        let mut archive = ZipArchive::new(reader).unwrap();
        assert_eq!(archive.len(), 2);

        let mut f1 = archive.by_name("My project/Library/ArtifactDB").unwrap();
        let mut s1 = String::new();
        f1.read_to_string(&mut s1).unwrap();
        drop(f1);
        assert_eq!(s1, "Hello, Resumable World!");

        let mut f2 = archive
            .by_name("My project/Library/PackageCache/pkg.json")
            .unwrap();
        let mut s2 = String::new();
        f2.read_to_string(&mut s2).unwrap();
        assert_eq!(s2, "{\"name\":\"unity.pkg\"}");
    }

    #[test]
    fn test_persisted_progress_bytes_and_resume_state() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().to_string_lossy().into_owned();
        let task_id = "task-resume-check";
        let t_dir = crate::manager::task_temp_dir(&dest, task_id);
        std::fs::create_dir_all(&t_dir).unwrap();

        // 写入 1 个已下载一半的 SingleFileChunk (5000 bytes / 10000 bytes)
        std::fs::write(bucket_raw_path(&t_dir, 0), vec![b'A'; 5000]).unwrap();
        // 写入 1 个已完成的 SubBatch .zip
        let sub_zip_path = bucket_zip_path(&t_dir, 1);
        {
            let f = StdFile::create(&sub_zip_path).unwrap();
            let mut zw = ZipWriter::new(f);
            zw.start_file("sub/a.txt", SimpleFileOptions::default()).unwrap();
            zw.write_all(b"1234567890").unwrap();
            zw.finish().unwrap();
        }
        let zip_len = is_valid_zip_file(&sub_zip_path).expect("valid zip bucket");
        assert!(zip_len > 22);

        let raw_len = std::fs::metadata(bucket_raw_path(&t_dir, 0)).unwrap().len();
        assert_eq!(raw_len, 5000);

        let task = DownloadTask {
            id: task_id.into(),
            url: "http://127.0.0.1:3928/api/download/batch".into(),
            file_name: "My project.zip".into(),
            destination: dest,
            total_bytes: 0,
            downloaded_bytes: 0,
            speed: 0,
            eta_seconds: None,
            status: TaskStatus::Downloading,
            error: None,
            created_at: 0,
            completed_at: None,
            scheduled_at: None,
            category: "archives".into(),
            queue_position: 0,
            priority: 0,
            retry_count: 0,
            max_retries: 3,
            checksum_sha256: None,
            expected_checksum: None,
            source: "extension".into(),
            etag: None,
            last_modified: None,
            final_url: None,
            response_status: None,
            content_type: None,
            accepts_ranges: None,
            headers: HashMap::new(),
            media: None,
            per_task_speed_limit: 0,
            collision_policy: Default::default(),
            completion_action: Default::default(),
            connection_count: 16,
            active_connections: 0,
            segments: Vec::new(),
            retry_policy_override: None,
            proxy_override: None,
            proxy_auth: None,
            task_kind: Default::default(),
            bt_meta: None,
            bt_runtime: None,
            cloud_refresh: None,
            method: Some("POST".into()),
            body: Some("files=%5B%22C%3A%5C%5CMy%20project%22%5D".into()),
        };

        assert!(is_landisk_batch_task(&task));
    }
}
