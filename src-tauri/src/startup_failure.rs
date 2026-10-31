//! Report recoverable startup failures without replacing or deleting user data.
use crate::{
    manager::{DownloadManager, SharedManager},
    store::Store,
};
use std::{
    fmt::Display,
    path::{Path, PathBuf},
    sync::Arc,
};

mod native;
pub(crate) use native::NativeSink;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
pub(crate) enum Presentation {
    Interactive,
    Console,
}

pub(crate) trait FailureSink {
    fn stderr(&self, message: &str);
    fn dialog(&self, title: &str, message: &str) -> Result<(), String>;
}

pub(crate) async fn initialize_manager(
    data_dir: PathBuf,
    app: tauri::AppHandle,
) -> Result<SharedManager, String> {
    // Opening SQLite and its legacy migration are blocking operations. In particular,
    // Store::open uses a blocking_lock, so it must not run on an async worker thread.
    let store = tokio::task::spawn_blocking(move || Store::open(data_dir))
        .await
        .map_err(|error| format!("读取本地数据库的后台任务未能完成：{error}"))?
        .map_err(|error| format!("无法打开本地数据库：{error}"))?;
    DownloadManager::new(Arc::new(store), app).await
}

pub(crate) fn finish_build<T, E: Display>(
    built: Result<T, E>,
    presentation: Presentation,
    data_dir: Option<&Path>,
    sink: &impl FailureSink,
    run: impl FnOnce(T),
) -> i32 {
    match built {
        Ok(app) => {
            run(app);
            0
        }
        Err(error) => {
            let message = message(&error.to_string(), data_dir);
            sink.stderr(&message);
            if matches!(presentation, Presentation::Interactive) {
                if let Err(error) = sink.dialog("猫步下载器启动失败", &message) {
                    sink.stderr(&format!(
                        "无法显示启动错误窗口：{}",
                        safe_detail(&error, 512)
                    ));
                }
            }
            1
        }
    }
}

fn safe_detail(detail: &str, limit: usize) -> String {
    crate::manager::redact_sensitive(detail)
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .take(limit)
        .collect()
}

fn message(detail: &str, data_dir: Option<&Path>) -> String {
    let directory = data_dir
        .map(|path| format!("\n数据目录：{}", safe_detail(&path.to_string_lossy(), 512)))
        .unwrap_or_default();
    format!(
        "猫步下载器未能启动。\n原因：{}{directory}\n\n请复制此错误信息，检查数据目录的权限与可用空间；如设置或数据库损坏，请先备份数据目录，再从有效备份恢复。请勿直接删除 lumaget.db 或清空下载记录。",
        safe_detail(detail, 2048)
    )
}
