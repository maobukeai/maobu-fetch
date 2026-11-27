//! 独立桌面完成提醒：仅接收真实完成事件，窗口与队列不依赖主窗口渲染。
pub(crate) mod commands;
mod geometry;
mod runtime;
mod state;
#[cfg(test)]
mod tests;
mod window;

pub(crate) use runtime::{enqueue, register};
const LABEL: &str = "desktop-completion";
const UPDATED: &str = "desktop-completion-updated";

#[cfg(all(test, windows))]
mod interactive_test;

fn authorized(label: &str) -> Result<(), String> {
    if label == LABEL {
        Ok(())
    } else {
        Err("当前窗口无权操作桌面完成提醒".into())
    }
}
