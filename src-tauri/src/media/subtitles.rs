use super::{
    output::{OutputManifest, Workspace},
    *,
};

pub(super) async fn validate(
    workspace: &Workspace,
    manifest: &OutputManifest,
) -> Result<(), String> {
    for (language, path) in &manifest.subtitles {
        if language.is_empty()
            || language.len() > 64
            || !language
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err("媒体字幕语言标识无效，原文件已保留".into());
        }
        workspace.validate_file(path).await?;
        if !matches!(
            path.extension().and_then(|value| value.to_str()),
            Some(
                "srt"
                    | "vtt"
                    | "ass"
                    | "ssa"
                    | "ttml"
                    | "dfxp"
                    | "json3"
                    | "srv1"
                    | "srv2"
                    | "srv3"
                    | "lrc"
                    | "xml"
            )
        ) {
            return Err("媒体字幕格式无法确认，原文件已保留".into());
        }
    }
    Ok(())
}

pub(super) async fn publish(
    workspace: &Workspace,
    manifest: &OutputManifest,
    task: &DownloadTask,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let stem = Path::new(&task.file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or("媒体字幕目标文件名无效")?;
    for (language, path) in &manifest.subtitles {
        if cancel.is_cancelled() {
            return Err("任务已暂停，未发布的字幕已保留".into());
        }
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .ok_or("字幕格式缺失")?;
        let mut subtitle = task.clone();
        subtitle.file_name = format!("{stem}.{language}.{extension}");
        workspace
            .publish_sidecar(path, &mut subtitle, cancel)
            .await
            .map_err(|_| {
                "字幕发布失败，媒体原始暂存文件已保留。请检查重名策略、目录权限和磁盘空间后重试"
            })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn subtitle_manifest_publishes_selected_language_without_touching_existing_user_file() {
        let root = tempfile::tempdir().unwrap();
        let mut task = test_support::task();
        task.destination = root.path().to_string_lossy().into_owned();
        task.collision_policy = crate::models::CollisionPolicy::Rename;
        let workspace = Workspace::open(&task, "best").await.unwrap();
        let path = workspace.root.join("payload.zh-Hans.vtt");
        tokio::fs::write(&path, b"WEBVTT\n\n").await.unwrap();
        let mut manifest = OutputManifest::default();
        manifest
            .record(&format!(
                "MAOBU_OUTPUT:{}",
                serde_json::json!({"subtitles":{"zh-Hans":{"filepath":path}}})
            ))
            .unwrap();
        validate(&workspace, &manifest).await.unwrap();
        let existing = root.path().join("video.zh-Hans.vtt");
        tokio::fs::write(&existing, b"user subtitle").await.unwrap();
        publish(&workspace, &manifest, &task, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(tokio::fs::read(existing).await.unwrap(), b"user subtitle");
        assert_eq!(
            tokio::fs::read(root.path().join("video.zh-Hans (1).vtt"))
                .await
                .unwrap(),
            b"WEBVTT\n\n"
        );
        manifest.subtitles = vec![("../../escape".into(), path)];
        assert!(validate(&workspace, &manifest).await.is_err());
    }
}
