use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

/// Short-lived private files; neither credentials nor signed target URLs appear in argv.
pub(super) struct PrivateConfig {
    paths: Vec<PathBuf>,
    config: PathBuf,
}

pub(crate) struct AuthFiles {
    cookie: crate::media_cookies::CookieFileGuard,
    _config: PrivateConfig,
}

impl AuthFiles {
    pub(super) fn new(
        cookie: crate::media_cookies::CookieFileGuard,
        config: PrivateConfig,
    ) -> Self {
        Self {
            cookie,
            _config: config,
        }
    }
    pub fn path(&self) -> Option<&Path> {
        self.cookie.path()
    }
    pub async fn consume(self) {
        self.cookie.consume().await;
    }
}

impl PrivateConfig {
    pub async fn create(
        base: &Path,
        options: &[(&str, String)],
        target: Option<&str>,
    ) -> Result<Self, String> {
        tokio::fs::create_dir_all(base)
            .await
            .map_err(|_| "创建媒体安全配置目录失败")?;
        let mut guard = Self {
            paths: Vec::new(),
            config: PathBuf::new(),
        };
        let mut text = "# coding: utf-8\n".to_owned();
        for (name, value) in options {
            if !name.starts_with("--")
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
                || value.chars().any(char::is_control)
            {
                return Err("媒体配置包含无效控制字符".into());
            }
            text.push_str(name);
            text.push(' ');
            text.push_str(&serde_json::to_string(value).map_err(|_| "媒体安全配置编码失败")?);
            text.push('\n');
        }
        if let Some(target) = target {
            if target.chars().any(char::is_control)
                || !matches!(
                    url::Url::parse(target).ok().as_ref().map(url::Url::scheme),
                    Some("http" | "https")
                )
            {
                return Err("媒体目标地址无效".into());
            }
            let batch = guard.write_private(base, target.as_bytes(), "urls").await?;
            text.push_str("--batch-file ");
            text.push_str(
                &serde_json::to_string(&batch.to_string_lossy())
                    .map_err(|_| "媒体地址文件编码失败")?,
            );
            text.push('\n');
        }
        guard.config = guard.write_private(base, text.as_bytes(), "conf").await?;
        Ok(guard)
    }

    async fn write_private(
        &mut self,
        base: &Path,
        bytes: &[u8],
        extension: &str,
    ) -> Result<PathBuf, String> {
        let path = base.join(format!(
            "maobu-{}.{}",
            uuid::Uuid::new_v4().simple(),
            extension
        ));
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .await
            .map_err(|_| "创建媒体临时配置失败")?;
        self.paths.push(path.clone());
        crate::media_cookies::restrict_to_current_user(&path)?;
        file.write_all(bytes)
            .await
            .map_err(|_| "写入媒体安全配置失败")?;
        file.flush().await.map_err(|_| "刷新媒体安全配置失败")?;
        Ok(path)
    }

    pub fn attach(&self, command: &mut tokio::process::Command) {
        command
            .arg("--ignore-config")
            .arg("--config-locations")
            .arg(&self.config);
    }
}

impl Drop for PrivateConfig {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn secrets_are_private_scoped_files_and_never_argv() {
        let directory = tempfile::tempdir().unwrap();
        let config = PrivateConfig::create(
            directory.path(),
            &[("--proxy", "http://maobu:synthetic@127.0.0.1:23456".into())],
            Some("https://example.test/video?token=synthetic"),
        )
        .await
        .unwrap();
        let mut command = tokio::process::Command::new("yt-dlp");
        config.attach(&mut command);
        assert!(
            !format!("{:?}", command.as_std().get_args().collect::<Vec<_>>()).contains("synthetic")
        );
        let files = config.paths.clone();
        assert_eq!(files.len(), 2);
        drop(config);
        for file in files {
            assert!(!file.exists());
        }
    }
}
