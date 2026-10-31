//! HLS VOD and live lifecycle, independent from Tauri and the task database.
use super::{
    crypto::{decrypt_aes_128_owned, derive_iv_from_sequence},
    merge::{merge_recording, merge_segments},
    parser::{parse_m3u8, select_best_variant, MediaPlaylist, MediaSegment, ParsedPlaylist},
    request::{HlsClient, PAUSED},
    resume::{digest, segment_part_path, verify_part, write_owned_file, ResumeJournal},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{fs, sync::Mutex, task::JoinSet};
use tokio_util::sync::CancellationToken;

const MAX_PLAYLIST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SEGMENT_BYTES: u64 = 128 * 1024 * 1024;

pub struct HlsOptions {
    pub http: HlsClient,
    pub user_cancel: CancellationToken,
    pub finalize_live: Arc<AtomicBool>,
    pub concurrency: usize,
    pub live_idle_timeout: Duration,
}

#[derive(Debug)]
pub struct HlsOutcome {
    pub bytes: u64,
    pub stopped_live: bool,
}

pub async fn download(options: &HlsOptions, temp: &Path) -> Result<HlsOutcome, String> {
    if !temp.is_absolute() {
        return Err("M3U8 临时文件必须使用明确的绝对路径".into());
    }
    if let Some(parent) = temp.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("创建 M3U8 临时目录失败：{error}"))?;
    }
    let (mut playlist, media_url) = load_playlist(&options.http, &options.http.source_url).await?;
    let journal =
        ResumeJournal::open(temp, &options.http.source_url, &media_url, &playlist).await?;
    let journal = Arc::new(Mutex::new(journal));
    restore_verified_progress(&options.http, temp, &journal).await?;
    let was_live = journal.lock().await.live;
    let mut last_new_segment = Instant::now();
    let mut validated = HashSet::new();
    loop {
        let previous_count = journal.lock().await.entries.len();
        let jobs = {
            let mut journal = journal.lock().await;
            let jobs = journal.plan(&playlist)?;
            journal.save(temp).await?;
            jobs
        };
        if journal.lock().await.entries.len() > previous_count {
            last_new_segment = Instant::now();
        }
        let jobs: Vec<_> = jobs
            .into_iter()
            .filter(|(index, _)| !validated.contains(index))
            .collect();
        let job_indices: Vec<_> = jobs.iter().map(|(index, _)| *index).collect();
        let batch = download_batch(options, temp, journal.clone(), jobs).await;
        if options.http.cancel.is_cancelled() {
            return finish_cancelled_live(options, temp, &journal, was_live).await;
        }
        batch?;
        validated.extend(job_indices);
        if playlist.is_endlist {
            let entries = journal.lock().await.complete_prefix();
            if entries.len() != journal.lock().await.entries.len() {
                return Err("M3U8 切片不完整，已拒绝合并".into());
            }
            let bytes = merge_segments(temp, &entries, &options.http.cancel).await?;
            return Ok(HlsOutcome {
                bytes,
                stopped_live: false,
            });
        }
        if last_new_segment.elapsed() > options.live_idle_timeout {
            return Err(
                "直播播放列表长时间未更新，录制已中断，已保存切片可继续恢复或停止保存".into(),
            );
        }
        let wait = playlist
            .target_duration
            .filter(|duration| duration.is_finite() && *duration > 0.0)
            .map(|duration| Duration::from_secs_f64((duration / 2.0).clamp(0.25, 10.0)))
            .unwrap_or(Duration::from_secs(2));
        tokio::select! {
            biased;
            _ = options.http.cancel.cancelled() => return finish_cancelled_live(options, temp, &journal, was_live).await,
            _ = tokio::time::sleep(wait) => {}
        }
        match load_playlist(&options.http, &media_url).await {
            Ok((next, _)) => playlist = next,
            Err(_) if options.http.cancel.is_cancelled() => {
                return finish_cancelled_live(options, temp, &journal, was_live).await
            }
            Err(error) => return Err(error),
        }
    }
}

async fn load_playlist(http: &HlsClient, source: &str) -> Result<(MediaPlaylist, String), String> {
    let mut current = source.to_owned();
    let mut visited = HashSet::new();
    for _ in 0..8 {
        if !visited.insert(current.clone()) {
            return Err("M3U8 主播放列表发生循环跳转".into());
        }
        let response = http.fetch(&current, None, MAX_PLAYLIST_BYTES).await?;
        let text = std::str::from_utf8(&response.bytes)
            .map_err(|_| "M3U8 播放列表不是有效的 UTF-8 文本")?;
        match parse_m3u8(text, &response.final_url)? {
            ParsedPlaylist::Master(variants) => {
                current = select_best_variant(&variants).ok_or("M3U8 没有可用的变体流")?
            }
            ParsedPlaylist::Media(media) => {
                if !media.is_endlist {
                    http.progress.is_live.store(true, Ordering::Release);
                }
                return Ok((media, response.final_url));
            }
        }
    }
    Err("M3U8 主播放列表跳转层数过多".into())
}

async fn restore_verified_progress(
    http: &HlsClient,
    temp: &Path,
    journal: &Mutex<ResumeJournal>,
) -> Result<(), String> {
    let mut journal = journal.lock().await;
    let mut saved = 0;
    for entry in &mut journal.entries {
        if verify_part(temp, entry).await? {
            saved += entry.length;
        } else {
            entry.length = 0;
            entry.sha256.clear();
        }
    }
    http.progress.saved_bytes.store(saved, Ordering::Relaxed);
    Ok(())
}

async fn finish_cancelled_live(
    options: &HlsOptions,
    temp: &Path,
    journal: &Mutex<ResumeJournal>,
    was_live: bool,
) -> Result<HlsOutcome, String> {
    if !was_live
        || !options.user_cancel.is_cancelled()
        || !options.finalize_live.load(Ordering::Acquire)
    {
        return Err(PAUSED.into());
    }
    let entries = journal.lock().await.complete_prefix();
    let bytes = merge_recording(temp, &entries, &options.finalize_live).await?;
    Ok(HlsOutcome {
        bytes,
        stopped_live: true,
    })
}

async fn download_batch(
    options: &HlsOptions,
    temp: &Path,
    journal: Arc<Mutex<ResumeJournal>>,
    jobs: Vec<(usize, MediaSegment)>,
) -> Result<(), String> {
    let queue = Arc::new(Mutex::new(VecDeque::from(jobs)));
    let keys = Arc::new(Mutex::new(HashMap::new()));
    let cancel = options.http.cancel.child_token();
    let mut workers = JoinSet::new();
    for _ in 0..options.concurrency.clamp(1, 32) {
        let (queue, keys, journal, temp) = (
            queue.clone(),
            keys.clone(),
            journal.clone(),
            temp.to_owned(),
        );
        let mut http = options.http.clone();
        http.cancel = cancel.clone();
        workers.spawn(async move {
            loop {
                if http.cancel.is_cancelled() {
                    return Err(PAUSED.to_owned());
                }
                let Some((index, segment)) = queue.lock().await.pop_front() else {
                    return Ok(());
                };
                download_segment(&http, &temp, &journal, &keys, index, &segment).await?;
            }
        });
    }
    let mut first_error = None;
    while let Some(result) = workers.join_next().await {
        let error = match result {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(error) => Some(format!("M3U8 连接异常退出：{error}")),
        };
        if first_error.is_none() && error.is_some() {
            first_error = error;
            cancel.cancel();
        }
    }
    first_error.map_or(Ok(()), Err)
}

async fn download_segment(
    http: &HlsClient,
    temp: &Path,
    journal: &Mutex<ResumeJournal>,
    keys: &Mutex<HashMap<String, Vec<u8>>>,
    index: usize,
    segment: &MediaSegment,
) -> Result<(), String> {
    let saved = journal
        .lock()
        .await
        .entries
        .get(index)
        .cloned()
        .ok_or("M3U8 分片索引无效")?;
    if verify_part(temp, &saved).await?
        && http
            .cached_validators_match(&segment.url, &saved.validators())
            .await?
    {
        return Ok(());
    }
    // Fetch the small key before reserving a media-body buffer, avoiding a
    // semaphore deadlock when multiple unknown-length encrypted bodies fill the pool.
    let decryption = if let Some(key) = &segment.key {
        Some((
            get_key(http, keys, &key.uri).await?,
            key.iv
                .unwrap_or_else(|| derive_iv_from_sequence(segment.sequence)),
        ))
    } else {
        None
    };
    let response = http
        .fetch(&segment.url, segment.byte_range, MAX_SEGMENT_BYTES)
        .await?;
    if saved.complete()
        && saved.validators().usable()
        && !saved.validators().safely_matches(&response.validators)
    {
        return Err("REMOTE_CHANGED:M3U8 媒体分片验证器已变化，旧分片已保留".into());
    }
    let data = if let Some((key_bytes, iv)) = decryption {
        decrypt_aes_128_owned(&key_bytes, &iv, response.bytes)?
    } else {
        response.bytes
    };
    if http.cancel.is_cancelled() {
        return Err(PAUSED.into());
    }
    write_owned_file(&segment_part_path(temp, index), &data).await?;
    let mut journal = journal.lock().await;
    let record = journal.entries.get_mut(index).ok_or("M3U8 分片索引无效")?;
    let previous_length = record.length;
    record.length = data.len() as u64;
    record.sha256 = digest(&data);
    record.etag = response.validators.etag;
    record.last_modified = response.validators.last_modified;
    journal.save(temp).await?;
    http.progress
        .saved_bytes
        .fetch_sub(previous_length, Ordering::Relaxed);
    http.progress
        .saved_bytes
        .fetch_add(data.len() as u64, Ordering::Relaxed);
    Ok(())
}

async fn get_key(
    http: &HlsClient,
    keys: &Mutex<HashMap<String, Vec<u8>>>,
    uri: &str,
) -> Result<Vec<u8>, String> {
    if let Some(key) = keys.lock().await.get(uri).cloned() {
        return Ok(key);
    }
    let response = http.fetch(uri, None, 16).await?;
    if response.bytes.len() != 16 {
        return Err("M3U8 AES-128 密钥长度必须为 16 字节".into());
    }
    keys.lock()
        .await
        .insert(uri.to_owned(), response.bytes.clone());
    Ok(response.bytes)
}
