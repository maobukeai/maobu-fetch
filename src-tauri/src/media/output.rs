use crate::models::{CollisionPolicy, DownloadTask};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub(super) fn work_directory(task: &DownloadTask) -> PathBuf {
    let id = hex::encode(Sha256::digest(task.id.as_bytes()));
    Path::new(&task.destination).join(format!(".maobu-media-{}.lumaget", &id[..24]))
}

pub(super) struct Workspace {
    pub root: PathBuf,
}

impl Workspace {
    pub async fn record_metadata(&self, metadata: &Value) -> Result<(), String> {
        use tokio::io::AsyncWriteExt;
        let mut value = serde_json::json!({"is_live": metadata.get("is_live").and_then(Value::as_bool) == Some(true),
            "live_status": metadata.get("live_status").and_then(Value::as_str).filter(|status| status.len() <= 32)});
        for key in [
            "title",
            "uploader",
            "channel",
            "uploader_id",
            "upload_date",
            "id",
            "display_id",
            "extractor_key",
            "maobu_platform",
        ] {
            if let Some(text) = metadata
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| text.len() <= 4096)
            {
                value[key] = text.into();
            }
        }
        let staged = self
            .root
            .join(format!("metadata-{}.json", uuid::Uuid::new_v4().simple()));
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&staged)
            .await
            .map_err(|_| "创建媒体运行状态失败")?;
        file.write_all(value.to_string().as_bytes())
            .await
            .map_err(|_| "写入媒体运行状态失败")?;
        file.sync_all().await.map_err(|_| "保存媒体运行状态失败")?;
        drop(file);
        crate::manager::publish_file_atomically(&staged, &self.root.join("runtime.json"), true)
            .await
    }
    pub async fn open(task: &DownloadTask, format: &str) -> Result<Self, String> {
        let root = work_directory(task);
        tokio::fs::create_dir_all(&task.destination)
            .await
            .map_err(|_| "创建媒体目标目录失败，请检查权限与磁盘空间")?;
        match tokio::fs::symlink_metadata(&root).await {
            Ok(meta) if !meta.is_dir() || super::path_safety::is_link(&meta) => {
                return Err("媒体任务临时路径不安全，已保留现有文件".into())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tokio::fs::create_dir(&root)
                    .await
                    .map_err(|_| "创建媒体任务隔离目录失败")?
            }
            Err(_) => return Err("无法检查媒体任务隔离目录".into()),
        }
        let binding = hex::encode(Sha256::digest(task.id.as_bytes()));
        let marker = root.join("owner.json");
        if marker.exists() {
            let saved: Value = serde_json::from_slice(
                &tokio::fs::read(&marker)
                    .await
                    .map_err(|_| "读取媒体恢复信息失败")?,
            )
            .map_err(|_| "媒体恢复信息损坏，原文件已保留")?;
            if saved.get("binding").and_then(Value::as_str) != Some(&binding) {
                return Err("媒体工作目录不属于当前任务，原文件已保留".into());
            }
        } else {
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(marker)
                .await
                .map_err(|_| "创建媒体恢复标记失败")?;
            file.write_all(
                serde_json::json!({"binding":binding})
                    .to_string()
                    .as_bytes(),
            )
            .await
            .map_err(|_| "写入媒体恢复标记失败")?;
            file.sync_all().await.map_err(|_| "保存媒体恢复标记失败")?;
        }
        if !format.is_empty() {
            let transfer = root.join("transfer.json");
            let binding = hex::encode(Sha256::digest(format!("{}\n{format}", task.url).as_bytes()));
            if transfer.exists() {
                let old: Value = serde_json::from_slice(
                    &tokio::fs::read(&transfer)
                        .await
                        .map_err(|_| "读取媒体传输恢复信息失败")?,
                )
                .map_err(|_| "媒体传输恢复信息损坏")?;
                if old.get("binding").and_then(Value::as_str) != Some(&binding) {
                    return Err("媒体来源或格式已经变化，不能混用原恢复文件".into());
                }
            } else {
                use tokio::io::AsyncWriteExt;
                let mut file = tokio::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(transfer)
                    .await
                    .map_err(|_| "创建媒体传输恢复信息失败")?;
                file.write_all(
                    serde_json::json!({"binding":binding})
                        .to_string()
                        .as_bytes(),
                )
                .await
                .map_err(|_| "保存媒体传输恢复信息失败")?;
                file.sync_all()
                    .await
                    .map_err(|_| "刷新媒体传输恢复信息失败")?;
            }
        }
        Ok(Self {
            root: tokio::fs::canonicalize(root)
                .await
                .map_err(|_| "规范化媒体任务目录失败")?,
        })
    }

    pub async fn validate_file(&self, file: &Path) -> Result<PathBuf, String> {
        let file = if file.is_absolute() {
            file.to_owned()
        } else {
            self.root.join(file)
        };
        let metadata = tokio::fs::symlink_metadata(&file)
            .await
            .map_err(|_| "媒体输出文件缺失，恢复数据已保留")?;
        if !metadata.is_file() || super::path_safety::is_link(&metadata) || metadata.len() == 0 {
            return Err("媒体输出不是有效的非空普通文件".into());
        }
        let normalized = tokio::fs::canonicalize(file)
            .await
            .map_err(|_| "规范化媒体输出路径失败")?;
        if normalized.parent() != Some(self.root.as_path()) {
            return Err("媒体输出超出当前任务隔离目录，已拒绝发布".into());
        }
        Ok(normalized)
    }

    pub async fn publish(
        &self,
        source: &Path,
        task: &mut DownloadTask,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<u64, String> {
        self.publish_candidate(source, task, cancel, true, None)
            .await
    }

    pub async fn publish_with_subtitles(
        &self,
        source: &Path,
        task: &mut DownloadTask,
        cancel: &tokio_util::sync::CancellationToken,
        manifest: &OutputManifest,
    ) -> Result<u64, String> {
        self.publish_candidate(source, task, cancel, true, Some(manifest))
            .await
    }

    pub async fn publish_sidecar(
        &self,
        source: &Path,
        task: &mut DownloadTask,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<u64, String> {
        // Sidecars reuse atomic target handling without recursively expanding a bundle future.
        Box::pin(self.publish_candidate(source, task, cancel, false, None)).await
    }

    async fn publish_candidate(
        &self,
        source: &Path,
        task: &mut DownloadTask,
        cancel: &tokio_util::sync::CancellationToken,
        primary: bool,
        manifest: Option<&OutputManifest>,
    ) -> Result<u64, String> {
        if cancel.is_cancelled() {
            return Err("任务已暂停，媒体暂存文件已保留".into());
        }
        let source = self.validate_file(source).await?;
        if Path::new(&task.file_name)
            .file_name()
            .and_then(|name| name.to_str())
            != Some(&task.file_name)
        {
            return Err("媒体目标文件名无效".into());
        }
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .ok_or("媒体输出缺少实际容器扩展名")?;
        if extension == "part" {
            return Err("媒体半成品不能直接发布，请先结束并验证录制容器".into());
        }
        let mut name = PathBuf::from(&task.file_name);
        name.set_extension(extension);
        let directory = Path::new(&task.destination);
        let mut target = directory.join(&name);
        if task.collision_policy == CollisionPolicy::Rename {
            let stem = name
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or("媒体目标文件名无效")?;
            let mut suffix = 1u32;
            while target.exists() {
                if suffix > 10000 {
                    return Err("重名文件过多，请选择新文件名".into());
                }
                target = directory.join(format!("{stem} ({suffix}).{extension}"));
                suffix += 1;
            }
        } else if task.collision_policy == CollisionPolicy::Skip && target.exists() {
            return Err("目标文件已存在，已按跳过策略保留现有文件和媒体临时文件".into());
        }
        if let Some(manifest) = manifest {
            let mut planned = task.clone();
            planned.file_name = target
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or("媒体完成文件名无效")?
                .to_owned();
            // Finish sidecars first: a recoverable main-file journal must never hide missing subtitles.
            super::subtitles::publish(self, manifest, &planned, cancel).await?;
        }
        let length = tokio::fs::metadata(&source)
            .await
            .map_err(|_| "读取媒体完成长度失败")?
            .len();
        if primary {
            crate::manager::record_task_publication(task, &source, &target, cancel).await?;
        }
        if cancel.is_cancelled() {
            return Err("任务已暂停，媒体暂存文件已保留".into());
        }
        crate::manager::publish_file_atomically(
            &source,
            &target,
            task.collision_policy == CollisionPolicy::Overwrite,
        )
        .await?;
        let actual = tokio::fs::metadata(&target)
            .await
            .map_err(|_| "媒体发布后的文件无法读取")?;
        if !actual.is_file() || actual.len() != length {
            return Err("媒体发布长度不一致，不能报告下载完成".into());
        }
        task.file_name = target
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("媒体完成文件名无效")?
            .into();
        Ok(length)
    }
}

pub(super) fn read_metadata(task: &DownloadTask) -> Option<Value> {
    use std::io::Read;
    let root = work_directory(task);
    let directory = std::fs::symlink_metadata(&root).ok()?;
    if !directory.is_dir() || super::path_safety::is_link(&directory) {
        return None;
    }
    let path = root.join("runtime.json");
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || super::path_safety::is_link(&metadata) || metadata.len() > 64 * 1024 {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(64 * 1024).read_to_end(&mut bytes).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(super) async fn record_probe_metadata(
    task: &DownloadTask,
    metadata: &Value,
    platform: crate::media_platforms::MediaPlatform,
) -> Result<(), String> {
    let workspace = Workspace::open(task, "").await?;
    let mut minimal = serde_json::json!({"maobu_platform":platform.as_str()});
    for key in [
        "title",
        "uploader",
        "channel",
        "uploader_id",
        "upload_date",
        "id",
        "display_id",
        "extractor_key",
        "is_live",
        "live_status",
    ] {
        if let Some(value) = metadata.get(key) {
            minimal[key] = value.clone();
        }
    }
    workspace.record_metadata(&minimal).await
}

#[derive(Default)]
pub(super) struct OutputManifest {
    pub file: Option<PathBuf>,
    pub tracks: Vec<PathBuf>,
    pub metadata: Option<Value>,
    pub subtitles: Vec<(String, PathBuf)>,
}

impl OutputManifest {
    pub fn record(&mut self, line: &str) -> Result<bool, String> {
        if let Some(text) = line.strip_prefix("MAOBU_META:") {
            self.metadata = Some(serde_json::from_str(text).map_err(|_| "媒体命名元数据无效")?);
            return Ok(true);
        }
        if let Some(text) = line.strip_prefix("MAOBU_OUTPUT:") {
            let value: Value = serde_json::from_str(text).map_err(|_| "媒体输出清单无效")?;
            self.file = value
                .get("file")
                .and_then(Value::as_str)
                .filter(|path| *path != "NA")
                .map(PathBuf::from);
            if let Some(formats) = value.get("tracks").and_then(Value::as_array) {
                for format in formats {
                    super::drm::ensure_unprotected(format)?;
                    if let Some(path) = format.get("filepath").and_then(Value::as_str) {
                        self.tracks.push(path.into());
                    }
                }
            }
            if let Some(subtitles) = value.get("subtitles").and_then(Value::as_object) {
                if subtitles.len() > 20 {
                    return Err("媒体字幕清单超过已支持的数量，请减少字幕语言后重试".into());
                }
                for (language, subtitle) in subtitles {
                    if let Some(path) = subtitle.get("filepath").and_then(Value::as_str) {
                        self.subtitles.push((language.clone(), PathBuf::from(path)));
                    }
                }
            }
            return Ok(true);
        }
        Ok(false)
    }
}
