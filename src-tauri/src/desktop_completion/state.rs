use crate::models::{AppSettings, ColorScheme, CompletionNoticeMode};
use serde::Serialize;
use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex, MutexGuard},
};
use tokio::sync::{Mutex as AsyncMutex, Notify};

const MAX_PENDING: usize = 128;
const MAX_RECENT: usize = 1024;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(crate) struct Notice {
    pub task_id: String,
    pub notice_id: u64,
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Snapshot {
    pub notice: Option<Notice>,
    pub pending_count: usize,
    pub duration_seconds: u32,
    pub color_scheme: ColorScheme,
    pub accent_color: String,
    pub language: String,
    pub revision: u64,
}

pub(super) struct Cache {
    queue: VecDeque<Notice>,
    recent: VecDeque<String>,
    seen: HashSet<String>,
    sequence: u64,
    revision: u64,
    enabled: bool,
    preferences_seen: bool,
    duration_seconds: u32,
    color_scheme: ColorScheme,
    accent_color: String,
    language: String,
    pub presented: Option<(u64, f64)>,
    busy: Option<u64>,
}

impl Default for Cache {
    fn default() -> Self {
        let defaults = AppSettings::default();
        Self {
            queue: VecDeque::new(),
            recent: VecDeque::new(),
            seen: HashSet::new(),
            sequence: 0,
            revision: 0,
            enabled: true,
            preferences_seen: false,
            duration_seconds: defaults.completion_notice_seconds,
            color_scheme: defaults.color_scheme,
            accent_color: defaults.accent_color,
            language: defaults.language,
            presented: None,
            busy: None,
        }
    }
}

fn enabled(settings: &AppSettings) -> bool {
    settings.notifications
        && settings.notify_on_complete
        && settings.completion_notice_mode == CompletionNoticeMode::Desktop
}

impl Cache {
    pub fn preferences(&mut self, settings: &AppSettings) {
        self.preferences_seen = true;
        self.enabled = enabled(settings);
        self.duration_seconds = settings.completion_notice_seconds;
        self.color_scheme = settings.color_scheme.clone();
        self.accent_color = settings.accent_color.clone();
        self.language = settings.language.clone();
        if !self.enabled {
            self.queue.clear();
            self.presented = None;
        }
        self.changed();
    }

    pub fn enqueue(
        &mut self,
        task_id: String,
        title: String,
        body: String,
        settings: &AppSettings,
    ) -> Result<bool, String> {
        if !self.preferences_seen {
            self.preferences(settings);
        }
        // settings-changed 是权威状态，晚到的旧完成事件不能重新启用已关闭的提醒。
        if !self.enabled
            || !enabled(settings)
            || self.seen.contains(&task_id)
            || self.queue.iter().any(|notice| notice.task_id == task_id)
        {
            return Ok(false);
        }
        if task_id.is_empty() {
            return Err("完成提醒缺少任务编号，请在主窗口查看下载记录".into());
        }
        let notice_id = self
            .sequence
            .checked_add(1)
            .ok_or("完成提醒队列编号已耗尽，请重启应用")?;
        if self.queue.len() >= MAX_PENDING {
            // 保留正在阅读的队首，合并高并发完成事件时只淘汰最早的等待项。
            self.queue.remove(1);
        }
        self.sequence = notice_id;
        self.seen.insert(task_id.clone());
        self.recent.push_back(task_id.clone());
        if self.recent.len() > MAX_RECENT {
            if let Some(old) = self.recent.pop_front() {
                self.seen.remove(&old);
            }
        }
        self.queue.push_back(Notice {
            task_id,
            notice_id,
            title,
            body,
        });
        self.changed();
        Ok(true)
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            notice: self.queue.front().cloned(),
            pending_count: self.queue.len().saturating_sub(1),
            duration_seconds: self.duration_seconds,
            color_scheme: self.color_scheme.clone(),
            accent_color: self.accent_color.clone(),
            language: self.language.clone(),
            revision: self.revision,
        }
    }

    pub fn check(&self, id: &str, notice_id: u64) -> Result<(), String> {
        if self.enabled
            && self
                .queue
                .front()
                .is_some_and(|head| head.task_id == id && head.notice_id == notice_id)
        {
            Ok(())
        } else {
            Err("这条完成提醒已失效，请查看当前提醒或主窗口任务列表".into())
        }
    }

    pub fn remove(&mut self, id: &str) {
        let previous = self.queue.front().map(|head| head.notice_id);
        let before = self.queue.len();
        self.queue.retain(|notice| notice.task_id != id);
        if self.queue.front().map(|head| head.notice_id) != previous {
            self.presented = None;
        }
        if before != self.queue.len() {
            self.changed();
        }
    }

    pub fn has_recent_tasks(&self) -> bool {
        !self.seen.is_empty() || !self.queue.is_empty()
    }

    pub fn invalidate(&mut self, id: &str) -> bool {
        let before = self.revision;
        self.remove(id);
        if self.seen.remove(id) {
            self.recent.retain(|recent| recent != id);
        }
        before != self.revision
    }

    pub fn begin_action(&mut self, id: &str, notice_id: u64) -> Result<(), String> {
        self.check(id, notice_id)?;
        if self.busy.is_some() {
            return Err("正在处理完成提醒，请稍候再试".into());
        }
        self.busy = Some(notice_id);
        Ok(())
    }

    pub fn finish_action(&mut self, id: &str, notice_id: u64, success: bool) {
        if self.busy == Some(notice_id) {
            self.busy = None;
        }
        if success && self.check(id, notice_id).is_ok() {
            self.remove(id);
        }
    }

    fn changed(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
}

pub(crate) struct DesktopCompletionState {
    cache: Mutex<Cache>,
    pub operation: AsyncMutex<()>,
    pub notify: Notify,
}

impl DesktopCompletionState {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            cache: Mutex::new(Cache::default()),
            operation: AsyncMutex::new(()),
            notify: Notify::new(),
        })
    }
    pub(super) fn lock(&self) -> Result<MutexGuard<'_, Cache>, String> {
        self.cache
            .lock()
            .map_err(|_| "完成提醒状态不可访问，请重启应用后重试".into())
    }
    pub(crate) fn snapshot(&self) -> Result<Snapshot, String> {
        Ok(self.lock()?.snapshot())
    }
}
