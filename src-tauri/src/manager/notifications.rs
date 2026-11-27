//! 完成提醒按用户设置选择一个渠道；事件仍用于提示音与失败重试。
use super::{redact_sensitive, DownloadManager, TaskNotificationPayload};
use crate::models::{AppSettings, CompletionNoticeMode, DownloadTask, TaskStatus};
use tauri::Emitter;

#[cfg(windows)]
mod windows;

impl DownloadManager {
    pub(super) async fn notify_download_completed(&self, task: &DownloadTask) {
        let settings = self.settings().await;
        let Some((mode, (title, body))) = completion_delivery(&settings, task) else {
            return;
        };
        let result = match mode {
            CompletionNoticeMode::Desktop => crate::desktop_completion::enqueue(
                &self.app,
                task.id.clone(),
                title.clone(),
                body.clone(),
                &settings,
            ),
            CompletionNoticeMode::System => self.show_system_notification(task, &title, &body),
        };
        self.report_notification_error(mode, result);
        self.emit_task_notification(task, "completed", title, body);
    }

    pub(super) async fn notify_download_failed(&self, task: &DownloadTask) {
        let settings = self.settings().await;
        if let Some((title, body)) = failure_notification(&settings, task) {
            self.report_notification_error(
                CompletionNoticeMode::System,
                self.show_system_notification(task, &title, &body),
            );
            self.emit_task_notification(task, "failed", title, body);
        }
    }

    fn show_system_notification(
        &self,
        task: &DownloadTask,
        title: &str,
        body: &str,
    ) -> Result<(), String> {
        #[cfg(windows)]
        return windows::show(self.app.clone(), task, title, body);

        #[cfg(not(windows))]
        {
            use tauri_plugin_notification::NotificationExt;
            let _ = task;
            self.app
                .notification()
                .builder()
                .title(title)
                .body(body)
                .show()
                .map_err(|error| error.to_string())
        }
    }

    fn report_notification_error(&self, mode: CompletionNoticeMode, result: Result<(), String>) {
        if let Err(error) = result {
            let hint = match mode {
                CompletionNoticeMode::Desktop => "无法显示桌面完成提醒，可在主窗口查看任务",
                CompletionNoticeMode::System => "系统通知发送失败，请检查 Windows 设置中的通知权限",
            };
            let message = format!("{hint}：{}", redact_sensitive(&error));
            tracing::warn!(%message);
            let _ = self.app.emit_to("main", "notification-error", message);
        }
    }

    fn emit_task_notification(
        &self,
        task: &DownloadTask,
        kind: &'static str,
        title: String,
        body: String,
    ) {
        let _ = self.app.emit(
            "task-notification",
            TaskNotificationPayload {
                task_id: task.id.clone(),
                kind,
                title,
                body,
            },
        );
    }
}

fn completion_delivery(
    settings: &AppSettings,
    task: &DownloadTask,
) -> Option<(CompletionNoticeMode, (String, String))> {
    completion_notification(settings, task)
        .map(|content| (settings.completion_notice_mode, content))
}

/// XML 1.0 禁止部分控制字符；其余字符由原生通知依赖统一转义。
fn notification_text(value: &str, maximum_chars: usize) -> String {
    value.chars()
        .filter(|ch| matches!(*ch, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}'))
        .take(maximum_chars)
        .collect()
}

pub(super) fn completion_notification(
    settings: &AppSettings,
    task: &DownloadTask,
) -> Option<(String, String)> {
    if !settings.notifications
        || !settings.notify_on_complete
        || task.status != TaskStatus::Completed
    {
        return None;
    }
    let file_name = notification_text(&task.file_name, 255);
    let destination = notification_text(&task.destination, 1024);
    Some(if settings.language == "en" {
        (
            format!("Download complete: {file_name}"),
            format!("Saved to {destination}"),
        )
    } else {
        (
            format!("下载完成：{file_name}"),
            format!("已保存到 {destination}"),
        )
    })
}

pub(super) fn failure_notification(
    settings: &AppSettings,
    task: &DownloadTask,
) -> Option<(String, String)> {
    if !settings.notifications || !settings.notify_on_failure || task.status != TaskStatus::Failed {
        return None;
    }
    Some((
        format!("下载失败：{}", notification_text(&task.file_name, 255)),
        notification_text(
            &redact_sensitive(task.error.as_deref().unwrap_or("未知错误")),
            1024,
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{manager::tests::test_task, models::CollisionPolicy};

    #[test]
    fn completion_uses_only_the_selected_channel_when_enabled() {
        let mut task = test_task(
            std::path::Path::new("."),
            "ready.txt",
            CollisionPolicy::Rename,
        );
        task.status = TaskStatus::Completed;
        let mut settings = AppSettings::default();
        assert_eq!(
            completion_delivery(&settings, &task).unwrap().0,
            CompletionNoticeMode::Desktop
        );
        settings.completion_notice_mode = CompletionNoticeMode::System;
        assert_eq!(
            completion_delivery(&settings, &task).unwrap().0,
            CompletionNoticeMode::System
        );
        settings.notify_on_complete = false;
        assert!(completion_delivery(&settings, &task).is_none());
        settings.notify_on_complete = true;
        settings.notifications = false;
        assert!(completion_delivery(&settings, &task).is_none());
    }

    #[test]
    fn text_preserves_unicode_and_xml_characters_for_the_builder_to_escape_once() {
        let text = "猫步 & <文件> \"双引号\" '单引号' 📁\t\r\n";
        assert_eq!(notification_text(text, 255), text);
        assert_eq!(
            notification_text("文件\0\u{1f}\u{fffe}\u{ffff}.zip", 255),
            "文件.zip"
        );
        assert_eq!(notification_text("猫🐱步", 2), "猫🐱");
    }

    #[test]
    fn completion_text_preserves_long_names_and_failure_text_redacts_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let name = format!("{} & '安全'.zip", "文件".repeat(50));
        let mut task = test_task(directory.path(), &name, CollisionPolicy::Rename);
        task.status = TaskStatus::Completed;
        let settings = AppSettings::default();
        assert_eq!(
            completion_notification(&settings, &task).unwrap().0,
            format!("下载完成：{name}")
        );
        task.status = TaskStatus::Failed;
        task.error = Some("连接失败\nCookie: session=private_cookie\nAuthorization: Bearer private_auth\ntoken=private_token".into());
        let body = failure_notification(&settings, &task).unwrap().1;
        for secret in ["private_cookie", "private_auth", "private_token"] {
            assert!(!body.contains(secret));
        }
        assert!(body.contains("连接失败"));
    }
}
