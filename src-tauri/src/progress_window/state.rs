use crate::models::{AppSettings, ColorScheme, DownloadTask, TaskStatus};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::{Mutex as AsyncMutex, Notify};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(crate) struct ProgressTask {
    pub id: String,
    pub file_name: String,
    pub status: TaskStatus,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_bps: u64,
    pub eta_seconds: Option<u64>,
    pub fetching_metadata: bool,
    pub can_pause: bool,
    pub can_resume: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Snapshot {
    pub revision: u64,
    pub tasks: Vec<ProgressTask>,
    pub total_count: usize,
    pub total_speed_bps: u64,
    pub always_on_top: bool,
    pub enabled: bool,
    pub expanded: bool,
    pub revealed: bool,
    pub summary_percent: Option<f64>,
    pub loading: bool,
    pub theme: String,
    pub color_scheme: ColorScheme,
    pub language: String,
    pub accent_color: String,
    pub error: Option<String>,
}

#[derive(Clone)]
pub(super) struct CachedTask {
    pub view: ProgressTask,
    priority: i32,
    position: i64,
    created_at: u64,
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
        let file_name = if fetching_metadata {
            "待获取".into()
        } else {
            crate::manager::redact_sensitive(&task.file_name)
                .chars()
                .filter(|character| !character.is_control())
                .take(512)
                .collect()
        };
        Some(Self {
            view: ProgressTask {
                id: task.id.clone(),
                file_name,
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

pub(super) struct Cache {
    tasks: HashMap<String, CachedTask>,
    pending: HashMap<String, Option<CachedTask>>,
    initialized: bool,
    preferences_seen: bool,
    pub revision: u64,
    pub expanded: bool,
    revealed: bool,
    pub enabled: bool,
    pub pinned: bool,
    pub pin_inflight: bool,
    theme: String,
    color_scheme: ColorScheme,
    language: String,
    accent_color: String,
    pub dismissed: Option<HashSet<String>>,
    pub error: Option<String>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            tasks: HashMap::new(),
            pending: HashMap::new(),
            initialized: false,
            preferences_seen: false,
            revision: 0,
            expanded: false,
            revealed: false,
            enabled: true,
            pinned: true,
            pin_inflight: false,
            theme: "system".into(),
            color_scheme: ColorScheme::System,
            language: "zh-CN".into(),
            accent_color: "blue".into(),
            dismissed: None,
            error: None,
        }
    }
}

impl Cache {
    pub fn initialize(&mut self, tasks: Vec<DownloadTask>, settings: &AppSettings) {
        self.tasks = tasks
            .iter()
            .filter_map(CachedTask::from_task)
            .map(|task| (task.view.id.clone(), task))
            .collect();
        for (id, task) in self.pending.drain() {
            match task {
                Some(task) => {
                    self.tasks.insert(id, task);
                }
                None => {
                    self.tasks.remove(&id);
                }
            }
        }
        if !self.preferences_seen {
            self.preferences(settings);
        }
        self.initialized = true;
        if self.tasks.is_empty() {
            self.revealed = false;
        }
        self.revision = self.revision.saturating_add(1);
    }

    pub fn update(&mut self, task: &DownloadTask) {
        self.update_id(task.id.clone(), CachedTask::from_task(task));
    }

    pub fn update_id(&mut self, id: String, task: Option<CachedTask>) {
        if !self.initialized {
            self.pending.insert(id.clone(), task.clone());
        }
        match task {
            Some(task) => {
                self.tasks.insert(id, task);
            }
            None => {
                self.tasks.remove(&id);
            }
        }
        if self.initialized && self.tasks.is_empty() {
            self.dismissed = None;
            self.revealed = false;
        }
        self.revision = self.revision.saturating_add(1);
    }

    pub fn preferences(&mut self, settings: &AppSettings) {
        if !self.enabled && settings.progress_window_enabled {
            self.dismissed = None;
        }
        self.enabled = settings.progress_window_enabled;
        if !self.enabled {
            self.revealed = false;
        }
        self.pinned = settings.progress_window_always_on_top;
        self.theme = settings.theme.clone();
        self.color_scheme = settings.color_scheme;
        self.language = settings.language.clone();
        self.accent_color = settings.accent_color.clone();
        self.preferences_seen = true;
        self.revision = self.revision.saturating_add(1);
    }

    pub fn dismiss(&mut self) {
        self.dismissed = Some(self.tasks.keys().cloned().collect());
        self.revealed = false;
        self.revision = self.revision.saturating_add(1);
    }

    /// Presentation-only intent; never open an idle, disabled or dismissed batch.
    pub fn set_revealed(&mut self, requested: bool) -> bool {
        let revealed = requested
            && self.initialized
            && self.enabled
            && !self.tasks.is_empty()
            && self.dismissed.is_none();
        if self.revealed == revealed {
            return false;
        }
        self.revealed = revealed;
        self.revision = self.revision.saturating_add(1);
        true
    }

    pub fn should_show(&mut self) -> bool {
        if self.tasks.is_empty() {
            self.dismissed = None;
            self.revealed = false;
            return false;
        }
        if self
            .dismissed
            .as_ref()
            .is_some_and(|ids| self.tasks.keys().any(|id| !ids.contains(id)))
        {
            self.dismissed = None;
        }
        self.initialized && self.enabled && self.dismissed.is_none()
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut tasks: Vec<_> = self.tasks.values().collect();
        tasks.sort_by_key(|task| {
            (
                match task.view.status {
                    TaskStatus::Downloading | TaskStatus::Verifying => 0,
                    TaskStatus::WaitingNetwork => 1,
                    _ => 2,
                },
                task.priority,
                task.position,
                task.created_at,
                task.view.id.clone(),
            )
        });
        Snapshot {
            revision: self.revision,
            total_count: tasks.len(),
            total_speed_bps: tasks
                .iter()
                .fold(0u64, |sum, task| sum.saturating_add(task.view.speed_bps)),
            tasks: tasks
                .into_iter()
                .take(6)
                .map(|task| task.view.clone())
                .collect(),
            always_on_top: self.pinned,
            enabled: self.enabled,
            expanded: self.expanded,
            revealed: self.revealed,
            summary_percent: super::summary::percent(self.tasks.values().map(|task| &task.view)),
            loading: !self.initialized,
            theme: self.theme.clone(),
            color_scheme: self.color_scheme,
            language: self.language.clone(),
            accent_color: self.accent_color.clone(),
            error: self.error.clone(),
        }
    }
}

pub(crate) struct ProgressWindowState {
    pub(super) cache: Mutex<Cache>,
    pub(super) notify: Notify,
    pub(super) operation: AsyncMutex<()>,
}

impl ProgressWindowState {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            cache: Mutex::new(Cache::default()),
            notify: Notify::new(),
            operation: AsyncMutex::new(()),
        })
    }
    pub(super) fn lock(&self) -> Result<MutexGuard<'_, Cache>, String> {
        self.cache
            .lock()
            .map_err(|_| "下载进度状态暂时不可用，请重新打开主窗口".into())
    }
    pub(super) fn snapshot(&self) -> Result<Snapshot, String> {
        Ok(self.lock()?.snapshot())
    }
}
