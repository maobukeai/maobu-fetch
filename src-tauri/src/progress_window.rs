//! A small, event-driven progress window with a restricted, credential-free view.
pub(crate) mod commands;
mod creation;
mod layout;
mod native;
mod os;
mod runtime;
mod state;
mod summary;
#[cfg(test)]
mod tests;

pub(crate) use runtime::register;

const LABEL: &str = "download-progress";
const UPDATED: &str = "download-progress-updated";

fn authorized(label: &str) -> Result<(), String> {
    if matches!(label, "main" | LABEL) {
        Ok(())
    } else {
        Err("当前窗口无权操作下载进度悬浮窗".into())
    }
}
