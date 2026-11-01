//! Bounded, transient completion notices. Time only advances while not interacting.
use super::state::ProgressTask;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub(super) const RETENTION: Duration = Duration::from_secs(180);
const MAX_RECENT: usize = 6;

struct Entry {
    task: ProgressTask,
    remaining: Duration,
}

pub(super) struct Completions {
    entries: VecDeque<Entry>,
    updated: Instant,
    interacting: bool,
    operations: usize,
}

impl Default for Completions {
    fn default() -> Self {
        Self::at(Instant::now())
    }
}

impl Completions {
    pub(super) fn at(now: Instant) -> Self {
        Self {
            entries: VecDeque::new(),
            updated: now,
            interacting: false,
            operations: 0,
        }
    }

    pub(super) fn paused(&self) -> bool {
        self.interacting || self.operations > 0
    }

    pub(super) fn advance(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.updated);
        if !self.paused() {
            for entry in &mut self.entries {
                entry.remaining = entry.remaining.saturating_sub(elapsed);
            }
        }
        self.updated = self.updated.max(now);
        let before = self.entries.len();
        self.entries.retain(|entry| !entry.remaining.is_zero());
        before != self.entries.len()
    }

    pub(super) fn insert(&mut self, task: ProgressTask, now: Instant) {
        self.advance(now);
        self.remove(&task.id);
        self.entries.push_front(Entry {
            task,
            remaining: RETENTION,
        });
        self.entries.truncate(MAX_RECENT);
    }

    pub(super) fn remove(&mut self, id: &str) {
        self.entries.retain(|entry| entry.task.id != id);
    }

    pub(super) fn clear(&mut self, now: Instant) {
        self.entries.clear();
        self.interacting = false;
        self.updated = now;
    }

    pub(super) fn interacting(&mut self, interacting: bool, now: Instant) -> bool {
        let expired = self.advance(now);
        let changed = self.interacting != interacting;
        self.interacting = interacting;
        changed || expired
    }

    pub(super) fn begin_open(&mut self, id: &str, now: Instant) -> Result<(), String> {
        self.advance(now);
        if !self.contains(id) {
            return Err("完成提醒已结束，请在主窗口的已完成列表中打开".into());
        }
        self.operations = self.operations.saturating_add(1);
        Ok(())
    }

    pub(super) fn finish_open(&mut self, id: &str, failed: bool, now: Instant) {
        self.advance(now);
        self.operations = self.operations.saturating_sub(1);
        // A failed launch must leave time to read the error and retry. A manual
        // dismiss/removal during the operation must never resurrect the notice.
        if failed {
            if let Some(entry) = self.entries.iter_mut().find(|entry| entry.task.id == id) {
                entry.remaining = RETENTION;
            }
        }
    }

    pub(super) fn contains(&self, id: &str) -> bool {
        self.entries.iter().any(|entry| entry.task.id == id)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn tasks(&self) -> Vec<ProgressTask> {
        self.entries
            .iter()
            .map(|entry| entry.task.clone())
            .collect()
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        if self.paused() {
            return None;
        }
        self.entries
            .iter()
            .map(|entry| self.updated + entry.remaining)
            .min()
    }
}
