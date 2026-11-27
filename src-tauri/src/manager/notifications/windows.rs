//! Windows 原生通知只接收固定动作名，不接收路径、命令或动态任务参数。
use super::super::{redact_sensitive, SharedManager};
use crate::{
    models::{DownloadTask, TaskStatus},
    task_file_actions,
};
#[cfg(test)]
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};
use tauri_winrt_notification::{Duration as ToastDuration, Toast};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    File,
    Folder,
    Details,
}

impl Action {
    fn parse(argument: Option<&str>, completed: bool) -> Result<Self, String> {
        match argument {
            None | Some("details") => Ok(Self::Details),
            Some("file") if completed => Ok(Self::File),
            Some("folder") if completed => Ok(Self::Folder),
            _ => Err("通知操作无效，请在主窗口查看任务后重试".into()),
        }
    }
}

fn completion_toast(app_id: &str, title: &str, body: &str) -> Toast {
    // 0.7.3 的 title/text1 自动进行 XML 转义；add_button 不转义，故仅使用固定常量。
    Toast::new(app_id)
        .duration(ToastDuration::Long)
        .title(title)
        .text1(body)
        .sound(None)
        .add_button("打开文件", "file")
        .add_button("打开文件夹", "folder")
        .add_button("查看详情", "details")
}

pub(super) fn show(
    app: AppHandle,
    task: &DownloadTask,
    title: &str,
    body: &str,
) -> Result<(), String> {
    // 保留原有 AUMID：开发版借用 PowerShell，正式版由安装器登记应用标识。
    let app_id = if cfg!(debug_assertions) {
        Toast::POWERSHELL_APP_ID
    } else {
        &app.config().identifier
    };
    let completed = task.status == TaskStatus::Completed;
    let toast = if completed {
        completion_toast(app_id, title, body)
    } else {
        Toast::new(app_id)
            .duration(ToastDuration::Long)
            .title(title)
            .text1(body)
            .sound(None)
            .add_button("查看详情", "details")
    };
    let task_id = task.id.clone();
    toast
        .on_activated(move |argument| {
            // 调度器早于 app.manage 启动，发送通知不能依赖 managed state 已注册。
            // 仅用户点击时读取当前 manager，避免保存过期任务或额外持有 manager。
            let manager = app
                .try_state::<SharedManager>()
                .map(|state| state.inner().clone());
            if let Some(manager) = manager {
                let task_id = task_id.clone();
                let action = Action::parse(argument.as_deref(), completed);
                tauri::async_runtime::spawn(async move {
                    let result = match action {
                        Ok(action) => activate(&manager, &task_id, action).await,
                        Err(error) => Err(error),
                    };
                    if let Err(error) = result {
                        report_action_error(&manager.app, &error);
                    }
                });
            } else {
                report_action_error(&app, "任务管理器不可用，请重新启动猫步下载器后重试通知");
            }
            Ok(())
        })
        .show()
        .map_err(|error| error.to_string())
}

fn checked_task(task: Option<&DownloadTask>, action: Action) -> Result<&DownloadTask, String> {
    let task = task.ok_or("该通知对应的任务已被删除，请在主窗口查看现有任务")?;
    if action != Action::Details && task.status != TaskStatus::Completed {
        return Err("该任务已不处于完成状态，请在主窗口查看后重试".into());
    }
    Ok(task)
}

#[cfg(test)]
async fn action_path(task: &DownloadTask, action: Action) -> Result<Option<PathBuf>, String> {
    checked_task(Some(task), action)?;
    match action {
        Action::File => task_file_actions::completed_path(task, false)
            .await
            .map(Some),
        Action::Folder => task_file_actions::completed_path(task, true)
            .await
            .map(Some),
        Action::Details => Ok(None),
    }
}

async fn activate(manager: &SharedManager, task_id: &str, action: Action) -> Result<(), String> {
    let current = manager.store.get_task(task_id).await?;
    checked_task(current.as_ref(), action)?;
    if action == Action::Details {
        show_main(&manager.app)?;
        return manager
            .app
            .emit("notification-focus-task", task_id)
            .map_err(|_| "无法定位任务，请在主窗口任务列表中查找".to_string());
    }
    task_file_actions::open_completed_task(manager, task_id, action == Action::Folder, || Ok(()))
        .await
}

fn show_main(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or("主窗口不可用，请重新打开猫步下载器")?;
    window
        .show()
        .and_then(|_| window.unminimize())
        .and_then(|_| window.set_focus())
        .map_err(|_| "无法显示主窗口，请从任务栏打开猫步下载器".to_string())
}

fn report_action_error(app: &AppHandle, error: &str) {
    let message = redact_sensitive(error);
    tracing::warn!(reason = %message, "系统通知操作失败");
    if let Err(error) = show_main(app) {
        tracing::warn!(reason = %error, "显示通知操作错误失败");
    }
    let _ = app.emit("notification-error", message);
}

#[cfg(test)]
mod tests;
