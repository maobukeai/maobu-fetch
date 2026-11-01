use super::state::ProgressTask;
use crate::models::{DownloadTask, TaskStatus};

#[derive(Clone)]
pub(super) struct CachedTask {
    pub view: ProgressTask,
    pub priority: i32,
    pub position: i64,
    pub created_at: u64,
}

fn file_name(task: &DownloadTask, fetching_metadata: bool) -> String {
    if fetching_metadata {
        return "待获取".into();
    }
    crate::manager::redact_sensitive(&task.file_name)
        .chars()
        .filter(|character| !character.is_control())
        .take(512)
        .collect()
}

pub(super) fn completed(task: &DownloadTask) -> ProgressTask {
    ProgressTask {
        id: task.id.clone(),
        file_name: file_name(task, false),
        status: TaskStatus::Completed,
        downloaded_bytes: task.downloaded_bytes,
        total_bytes: task.total_bytes,
        speed_bps: 0,
        eta_seconds: None,
        fetching_metadata: false,
        can_pause: false,
        can_resume: false,
    }
}

impl CachedTask {
    pub fn from_task(task: &DownloadTask) -> Option<Self> {
        if !matches!(
            task.status,
            TaskStatus::Queued
                | TaskStatus::Downloading
                | TaskStatus::Verifying
                | TaskStatus::WaitingNetwork
        ) {
            return None;
        }
        let fetching_metadata = task
            .bt_runtime
            .as_ref()
            .is_some_and(|value| value.fetching_metadata);
        let downloading = task.status == TaskStatus::Downloading;
        Some(Self {
            view: ProgressTask {
                id: task.id.clone(),
                file_name: file_name(task, fetching_metadata),
                status: task.status.clone(),
                downloaded_bytes: if fetching_metadata {
                    0
                } else {
                    task.downloaded_bytes
                },
                total_bytes: if fetching_metadata {
                    0
                } else {
                    task.total_bytes
                },
                speed_bps: if downloading { task.speed } else { 0 },
                eta_seconds: if downloading && !fetching_metadata && task.total_bytes > 0 {
                    task.eta_seconds
                } else {
                    None
                },
                fetching_metadata,
                can_pause: matches!(
                    task.status,
                    TaskStatus::Queued | TaskStatus::Downloading | TaskStatus::WaitingNetwork
                ),
                can_resume: false,
            },
            priority: task.priority,
            position: task.queue_position,
            created_at: task.created_at,
        })
    }
}
