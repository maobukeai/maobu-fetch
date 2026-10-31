//! Task-owned, hash-verified segment journal. URLs and credentials are not saved.
use super::{
    parser::{MediaPlaylist, MediaSegment},
    request::Validators,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
};

const JOURNAL_VERSION: u32 = 1;
const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SegmentRecord {
    pub index: usize,
    pub identity: String,
    pub sequence: Option<u64>,
    pub length: u64,
    pub sha256: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl SegmentRecord {
    pub fn validators(&self) -> Validators {
        Validators {
            etag: self.etag.clone(),
            last_modified: self.last_modified.clone(),
        }
    }
    pub fn complete(&self) -> bool {
        self.length > 0 && self.sha256.len() == 64
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResumeJournal {
    version: u32,
    source: String,
    media_url: String,
    vod_fingerprint: Option<String>,
    pub live: bool,
    pub entries: Vec<SegmentRecord>,
}

pub fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn segment_identity(segment: &MediaSegment) -> String {
    digest(
        format!(
            "{}\n{}\n{}\n{:?}\n{:?}\n{}\n{:?}",
            segment.url,
            if segment.is_init_segment {
                0
            } else {
                segment.sequence
            },
            segment.discontinuity,
            segment.byte_range,
            segment.key,
            segment.is_init_segment,
            segment
                .initialization
                .as_ref()
                .map(|init| segment_identity(init))
        )
        .as_bytes(),
    )
}

fn playlist_fingerprint(playlist: &MediaPlaylist) -> String {
    let mut hash = Sha256::new();
    for segment in &playlist.segments {
        if let Some(init) = &segment.initialization {
            hash.update(segment_identity(init));
        }
        hash.update(segment_identity(segment));
    }
    hex::encode(hash.finalize())
}

impl ResumeJournal {
    pub async fn open(
        temp: &Path,
        source: &str,
        media_url: &str,
        playlist: &MediaPlaylist,
    ) -> Result<Self, String> {
        let path = journal_path(temp);
        let source_hash = digest(source.as_bytes());
        let media_hash = digest(media_url.as_bytes());
        let loaded = match fs::symlink_metadata(&path).await {
            Ok(meta)
                if !meta.is_file()
                    || meta.file_type().is_symlink()
                    || meta.len() > MAX_JOURNAL_BYTES =>
            {
                return Err("M3U8 恢复清单不是普通文件或体积异常，已保留原有数据".into())
            }
            Ok(_) => Some(
                fs::read(&path)
                    .await
                    .map_err(|error| format!("读取 M3U8 恢复清单失败：{error}"))?,
            ),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => return Err(format!("检查 M3U8 恢复清单失败：{error}")),
        };
        let fingerprint = playlist_fingerprint(playlist);
        let journal: Self = if let Some(bytes) = loaded {
            serde_json::from_slice(&bytes)
                .map_err(|_| "M3U8 恢复清单损坏，已保留分片，请选择重新下载")?
        } else {
            Self {
                version: JOURNAL_VERSION,
                source: source_hash.clone(),
                media_url: media_hash.clone(),
                vod_fingerprint: playlist.is_endlist.then_some(fingerprint.clone()),
                live: !playlist.is_endlist,
                entries: Vec::new(),
            }
        };
        if journal.version != JOURNAL_VERSION
            || journal.source != source_hash
            || journal.media_url != media_hash
            || journal
                .vod_fingerprint
                .as_ref()
                .is_some_and(|saved| saved != &fingerprint)
        {
            return Err(
                "REMOTE_CHANGED:M3U8 播放列表或资源地址已变化，已保留旧分片，请选择重新下载".into(),
            );
        }
        if journal
            .entries
            .iter()
            .enumerate()
            .any(|(index, entry)| entry.index != index)
        {
            return Err("M3U8 恢复清单索引无效，已拒绝读取分片".into());
        }
        Ok(journal)
    }

    pub async fn save(&self, temp: &Path) -> Result<(), String> {
        let data = serde_json::to_vec(self)
            .map_err(|error| format!("序列化 M3U8 恢复清单失败：{error}"))?;
        write_owned_file(&journal_path(temp), &data).await
    }

    pub fn plan(&mut self, playlist: &MediaPlaylist) -> Result<Vec<(usize, MediaSegment)>, String> {
        let mut jobs = Vec::new();
        for segment in &playlist.segments {
            if let Some(saved) = self
                .entries
                .iter()
                .find(|entry| entry.sequence == Some(segment.sequence))
            {
                if saved.identity != segment_identity(segment) {
                    return Err("REMOTE_CHANGED:M3U8 同一媒体序列号的内容地址或加密参数已变化，旧分片已保留".into());
                }
            } else {
                if let Some(previous) = self.entries.iter().rev().find_map(|entry| entry.sequence) {
                    if segment.sequence != previous.checked_add(1).ok_or("M3U8 序列号溢出")? {
                        return Err("M3U8 直播已跳过尚未保存的媒体序列，无法保证完整录制，请停止保存或重新开始".into());
                    }
                }
                if let Some(init) = &segment.initialization {
                    let identity = segment_identity(init);
                    let last_init = self
                        .entries
                        .iter()
                        .rev()
                        .find(|entry| entry.sequence.is_none());
                    if last_init.map_or(true, |entry| entry.identity != identity) {
                        self.push_entry(identity, None);
                    }
                }
                self.push_entry(segment_identity(segment), Some(segment.sequence));
            }
            if let Some(init) = &segment.initialization {
                let identity = segment_identity(init);
                if let Some(record) = self
                    .entries
                    .iter()
                    .rev()
                    .find(|entry| entry.sequence.is_none() && entry.identity == identity)
                {
                    if !jobs.iter().any(|(index, _)| *index == record.index) {
                        jobs.push((record.index, (**init).clone()));
                    }
                }
            }
            let record = self
                .entries
                .iter()
                .find(|entry| entry.sequence == Some(segment.sequence))
                .ok_or("M3U8 切片规划失败")?;
            jobs.push((record.index, segment.clone()));
        }
        Ok(jobs)
    }

    fn push_entry(&mut self, identity: String, sequence: Option<u64>) {
        self.entries.push(SegmentRecord {
            index: self.entries.len(),
            identity,
            sequence,
            length: 0,
            sha256: String::new(),
            etag: None,
            last_modified: None,
        });
    }

    pub fn complete_prefix(&self) -> Vec<SegmentRecord> {
        self.entries
            .iter()
            .take_while(|entry| entry.complete())
            .cloned()
            .collect()
    }
}

pub fn journal_path(temp: &Path) -> PathBuf {
    PathBuf::from(format!("{}.m3u8-state.json", temp.to_string_lossy()))
}
pub fn segment_part_path(temp: &Path, index: usize) -> PathBuf {
    PathBuf::from(format!("{}.seg{index}", temp.to_string_lossy()))
}

pub async fn verify_part(temp: &Path, record: &SegmentRecord) -> Result<bool, String> {
    if !record.complete() {
        return Ok(false);
    }
    let path = segment_part_path(temp, record.index);
    let meta = match fs::symlink_metadata(&path).await {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => meta,
        Ok(_) => return Err("M3U8 分片路径不是安全的普通文件".into()),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("检查 M3U8 分片失败：{error}")),
    };
    if meta.len() != record.length {
        return Ok(false);
    }
    Ok(hash_file(&path).await? == record.sha256)
}

pub async fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .await
        .map_err(|error| format!("读取 M3U8 分片失败：{error}"))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|error| format!("校验 M3U8 分片失败：{error}"))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hex::encode(hash.finalize()))
}

pub async fn write_owned_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let staged = PathBuf::from(format!("{}.writing", path.to_string_lossy()));
    if fs::symlink_metadata(&staged)
        .await
        .is_ok_and(|meta| !meta.is_file() || meta.file_type().is_symlink())
    {
        return Err("M3U8 临时文件路径不安全，已停止写入".into());
    }
    let mut file = fs::File::create(&staged)
        .await
        .map_err(|error| format!("创建 M3U8 临时文件失败：{error}"))?;
    file.write_all(bytes)
        .await
        .map_err(|error| format!("写入 M3U8 临时文件失败：{error}"))?;
    file.sync_all()
        .await
        .map_err(|error| format!("保存 M3U8 临时文件失败：{error}"))?;
    drop(file);
    replace_owned_file(&staged, path).await
}

/// 同目录原子替换；Windows 使用 MoveFileEx，避免“先移开旧文件”的崩溃窗口。
pub async fn replace_owned_file(staged: &Path, target: &Path) -> Result<(), String> {
    if staged.parent() != target.parent() {
        return Err("M3U8 原子替换要求临时文件位于同一目录".into());
    }
    let meta = match fs::symlink_metadata(target).await {
        Ok(meta) => Some(meta),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(format!("检查 M3U8 输出失败：{error}")),
    };
    if let Some(meta) = &meta {
        if !meta.is_file() || meta.file_type().is_symlink() || meta.permissions().readonly() {
            return Err("M3U8 输出被设为只读或不是普通文件，已保留合并文件和分片".into());
        }
    }
    #[cfg(windows)]
    {
        let (staged, target) = (staged.to_owned(), target.to_owned());
        tokio::task::spawn_blocking(move || replace_windows(&staged, &target))
            .await
            .map_err(|error| format!("M3U8 文件提交任务异常：{error}"))?
    }
    #[cfg(not(windows))]
    fs::rename(staged, target)
        .await
        .map_err(|error| format!("保存 M3U8 合并文件失败：{error}，分片和合并数据已保留"))
}

#[cfg(windows)]
fn replace_windows(staged: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let wide_path = |path: &Path| -> Result<Vec<u16>, String> {
        let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
        if wide.contains(&0) {
            return Err("M3U8 文件路径含有无效字符".into());
        }
        wide.push(0);
        Ok(wide)
    };
    let (source, destination) = (wide_path(staged)?, wide_path(target)?);
    // Both buffers remain alive and NUL-terminated during this synchronous Win32 call.
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        return Err(format!(
            "保存 M3U8 合并文件失败：{}，分片和合并数据已保留",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}
