use super::{
    authorized,
    state::{ProgressWindowState, Snapshot},
};
use crate::{manager::SharedManager, models::TaskStatus, task_file_actions};
use std::{sync::Arc, time::Instant};
use tauri::{State, WebviewWindow};

type Progress<'a> = State<'a, Arc<ProgressWindowState>>;

#[tauri::command]
pub(crate) fn progress_window_set_interacting(
    window: WebviewWindow,
    state: Progress<'_>,
    interacting: bool,
) -> Result<Snapshot, String> {
    authorized(window.label())?;
    state.lock()?.set_interacting(interacting, Instant::now());
    state.notify.notify_one();
    state.snapshot()
}

/// The hold is released even if the invoke future is cancelled while resolving
/// files or awaiting the OS. Closing the notice during a launch never revives it.
struct OpenHold {
    state: Arc<ProgressWindowState>,
    task_id: String,
    failed: bool,
}

impl Drop for OpenHold {
    fn drop(&mut self) {
        match self.state.lock() {
            Ok(mut cache) => {
                cache.finish_completion_open(&self.task_id, self.failed, Instant::now())
            }
            Err(error) => tracing::warn!(reason=%error, "释放完成提醒操作状态失败"),
        }
        self.state.notify.notify_one();
    }
}

#[tauri::command]
pub(crate) async fn progress_window_open_completed(
    window: WebviewWindow,
    state: Progress<'_>,
    manager: State<'_, SharedManager>,
    task_id: String,
    target: String,
) -> Result<Snapshot, String> {
    authorized(window.label())?;
    validate_target(&target)?;
    let _operation = state.operation.lock().await;
    state
        .lock()?
        .begin_completion_open(&task_id, Instant::now())?;
    let mut hold = OpenHold {
        state: state.inner().clone(),
        task_id: task_id.clone(),
        failed: true,
    };
    state.notify.notify_one();
    let result = open_task(manager.inner(), &task_id, &target).await;
    hold.failed = result.is_err();
    drop(hold);
    result.map_err(|error| crate::manager::redact_sensitive(&error))?;
    state.snapshot()
}

fn validate_target(target: &str) -> Result<(), String> {
    if matches!(target, "file" | "folder") {
        Ok(())
    } else {
        Err("不支持此文件操作，请选择打开文件或打开文件夹".into())
    }
}

async fn open_task(manager: &SharedManager, id: &str, target: &str) -> Result<(), String> {
    let task = task_file_actions::resolved_task(manager, id).await?;
    if task.status != TaskStatus::Completed {
        return Err("该任务已不处于完成状态，请在主窗口查看".into());
    }
    let path = if target == "file" {
        task_file_actions::content_path(&task, None).await?
    } else {
        task_file_actions::folder_path(&task).await?
    };
    tokio::task::spawn_blocking(move || open::that(path))
        .await
        .map_err(|_| "打开文件的后台操作未完成，请重试".to_string())?
        .map_err(|error| format!("无法打开，请确认文件仍在原保存位置并重试：{error}"))
}

#[cfg(test)]
mod tests {
    use super::{validate_target, OpenHold};
    use crate::{
        models::{AppSettings, TaskStatus},
        progress_window::{state::ProgressWindowState, tests::task},
    };
    use std::time::Instant;

    #[test]
    fn open_target_only_accepts_known_operations_and_no_paths() {
        for target in ["file", "folder"] {
            assert!(validate_target(target).is_ok());
        }
        for target in ["", "C:/private", "../file", "run", "FILE"] {
            assert!(validate_target(target).is_err());
        }
    }

    #[test]
    fn dropping_a_cancelled_open_operation_releases_the_timer_hold() {
        let state = ProgressWindowState::new();
        {
            let mut cache = state.lock().unwrap();
            cache.initialize(
                vec![task(1, TaskStatus::Downloading)],
                &AppSettings::default(),
            );
            cache.update(&task(1, TaskStatus::Completed));
            cache
                .begin_completion_open("task-1", Instant::now())
                .unwrap();
            assert!(cache.completion_deadline().is_none());
        }
        let hold = OpenHold {
            state: state.clone(),
            task_id: "task-1".into(),
            failed: true,
        };
        drop(hold);
        assert!(state.lock().unwrap().completion_deadline().is_some());
    }
}
