use super::{
    authorized,
    state::{DesktopCompletionState, Snapshot},
    window,
};
use crate::{
    manager::{redact_sensitive, SharedManager},
    models::TaskStatus,
    task_file_actions,
};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

type Desktop<'a> = State<'a, Arc<DesktopCompletionState>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    File,
    Folder,
    Details,
    Dismiss,
}

impl Action {
    pub(super) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "file" => Ok(Self::File),
            "folder" => Ok(Self::Folder),
            "details" => Ok(Self::Details),
            "dismiss" => Ok(Self::Dismiss),
            _ => Err("不支持此操作，请使用完成提醒中的按钮".into()),
        }
    }
}

#[tauri::command]
pub(crate) fn desktop_completion_snapshot(
    window: WebviewWindow,
    state: Desktop<'_>,
) -> Result<Snapshot, String> {
    authorized(window.label())?;
    state.snapshot()
}

#[tauri::command]
pub(crate) async fn desktop_completion_present(
    window: WebviewWindow,
    app: AppHandle,
    state: Desktop<'_>,
    manager: State<'_, SharedManager>,
    task_id: String,
    notice_id: u64,
    height: f64,
) -> Result<Snapshot, String> {
    authorized(window.label())?;
    let _operation = state.operation.lock().await;
    state.lock()?.check(&task_id, notice_id)?;
    completed_task(manager.inner(), &task_id)
        .await
        .map_err(|error| redact_sensitive(&error))?;
    let result = window::present(&app, state.inner(), task_id.clone(), notice_id, height).await;
    if let Err(error) = &result {
        // 模式切换/队首推进属于正常竞态；真实尺寸、屏幕或 HWND 失败应在可见主窗反馈。
        if state
            .lock()
            .is_ok_and(|cache| cache.check(&task_id, notice_id).is_ok())
        {
            super::runtime::report(&app, error);
        }
    }
    result.map_err(|error| redact_sensitive(&error))
}

struct ActionHold {
    state: Arc<DesktopCompletionState>,
    task_id: String,
    notice_id: u64,
    success: bool,
}
impl Drop for ActionHold {
    fn drop(&mut self) {
        match self.state.lock() {
            Ok(mut cache) => cache.finish_action(&self.task_id, self.notice_id, self.success),
            Err(error) => tracing::warn!(reason=%error, "释放桌面完成提醒操作状态失败"),
        }
        self.state.notify.notify_one();
    }
}

#[tauri::command]
pub(crate) async fn desktop_completion_act(
    window: WebviewWindow,
    app: AppHandle,
    state: Desktop<'_>,
    manager: State<'_, SharedManager>,
    task_id: String,
    notice_id: u64,
    action: String,
) -> Result<Snapshot, String> {
    authorized(window.label())?;
    let action = Action::parse(&action)?;
    state.lock()?.begin_action(&task_id, notice_id)?;
    let mut hold = ActionHold {
        state: state.inner().clone(),
        task_id: task_id.clone(),
        notice_id,
        success: false,
    };
    let _operation = state.operation.lock().await;
    let result = perform(
        &app,
        state.inner(),
        manager.inner(),
        &task_id,
        notice_id,
        action,
    )
    .await;
    hold.success = result.is_ok();
    drop(hold);
    if result.is_ok() {
        // 先隐藏旧队首，再把新快照交给前端测量；文件已成功打开时，窗口刷新
        // 失败只作反馈，不能把成功动作报成失败而诱导用户重复打开。
        if let Err(error) = window::update(&app, state.inner()).await {
            super::runtime::report(&app, &error);
        }
    }
    result.map_err(|error| redact_sensitive(&error))?;
    state.snapshot()
}

async fn completed_task(manager: &SharedManager, id: &str) -> Result<(), String> {
    let task = manager
        .store
        .get_task(id)
        .await?
        .ok_or("这条提醒对应的任务已被删除，请在主窗口查看")?;
    if task.status != TaskStatus::Completed {
        return Err("该任务已不处于完成状态，请在主窗口查看".into());
    }
    Ok(())
}

async fn perform(
    app: &AppHandle,
    state: &Arc<DesktopCompletionState>,
    manager: &SharedManager,
    task_id: &str,
    notice_id: u64,
    action: Action,
) -> Result<(), String> {
    state.lock()?.check(task_id, notice_id)?;
    // 关闭只是移除已经校验过的队首；数据库不可访问时也必须允许用户关闭。
    if action == Action::Dismiss {
        return Ok(());
    }
    completed_task(manager, task_id).await?;
    state.lock()?.check(task_id, notice_id)?;
    match action {
        Action::File | Action::Folder => {
            task_file_actions::open_completed_task(
                manager,
                task_id,
                action == Action::Folder,
                || state.lock()?.check(task_id, notice_id),
            )
            .await
        }
        Action::Details => {
            let main = app
                .get_webview_window("main")
                .ok_or("主窗口尚未准备好，请稍候重试")?;
            crate::bring_window_to_front(&main);
            app.emit_to("main", "notification-focus-task", task_id)
                .map_err(|error| {
                    format!("主窗口已打开，但未能定位任务，请在任务列表中查找：{error}")
                })
        }
        Action::Dismiss => Ok(()),
    }
}
