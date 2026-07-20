use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

const BANDWIDTH_SLICE_BYTES: u64 = 64 * 1024;
const BANDWIDTH_SLICES_PER_SECOND: u64 = 10;
const HIGH_PRIORITY_WEIGHT: u64 = 4;
const NORMAL_PRIORITY_WEIGHT: u64 = 2;
const LOW_PRIORITY_WEIGHT: u64 = 1;

#[derive(Clone)]
pub struct BandwidthScheduler {
    commands: mpsc::UnboundedSender<Command>,
    limit: Arc<AtomicU64>,
    next_request_id: Arc<AtomicU64>,
}

impl BandwidthScheduler {
    pub fn new(limit: u64) -> Self {
        let (commands, receiver) = mpsc::unbounded_channel();
        let shared_limit = Arc::new(AtomicU64::new(limit));
        tokio::spawn(run_scheduler(receiver, shared_limit.clone()));
        Self {
            commands,
            limit: shared_limit,
            next_request_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn set_limit(&self, limit: u64) {
        self.limit.store(limit, Ordering::Relaxed);
        let _ = self.commands.send(Command::Wake);
    }

    pub fn set_priority(&self, task_id: &str, priority: i32) {
        let _ = self.commands.send(Command::SetPriority {
            task_id: task_id.to_string(),
            priority,
        });
    }

    pub async fn acquire(
        &self,
        task_id: &str,
        bytes: u64,
        priority: i32,
        cancel: &CancellationToken,
    ) {
        if bytes == 0 || self.limit.load(Ordering::Relaxed) == 0 {
            return;
        }

        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (granted, receiver) = oneshot::channel();
        if self
            .commands
            .send(Command::Acquire(PermitRequest {
                id: request_id,
                task_id: task_id.to_string(),
                priority,
                remaining: bytes,
                cancel: cancel.clone(),
                granted: Some(granted),
            }))
            .is_err()
        {
            return;
        }
        tokio::select! {
            _ = receiver => {}
            _ = cancel.cancelled() => {
                let _ = self.commands.send(Command::CancelRequest(request_id));
            }
        }
    }
}

enum Command {
    Acquire(PermitRequest),
    CancelRequest(u64),
    SetPriority { task_id: String, priority: i32 },
    Wake,
}

struct PermitRequest {
    id: u64,
    task_id: String,
    priority: i32,
    remaining: u64,
    cancel: CancellationToken,
    granted: Option<oneshot::Sender<()>>,
}

struct TaskQueue {
    priority: i32,
    deficit: u64,
    requests: VecDeque<PermitRequest>,
}

#[derive(Default)]
struct SchedulerState {
    tasks: HashMap<String, TaskQueue>,
    round: VecDeque<String>,
}

impl SchedulerState {
    fn push(&mut self, request: PermitRequest) {
        let task_id = request.task_id.clone();
        if let Some(queue) = self.tasks.get_mut(&task_id) {
            queue.priority = request.priority;
            queue.requests.push_back(request);
            return;
        }
        self.tasks.insert(
            task_id.clone(),
            TaskQueue {
                priority: request.priority,
                deficit: 0,
                requests: VecDeque::from([request]),
            },
        );
        self.round.push_back(task_id);
    }

    fn set_priority(&mut self, task_id: &str, priority: i32) {
        if let Some(queue) = self.tasks.get_mut(task_id) {
            queue.priority = priority;
        }
    }

    fn cancel_request(&mut self, request_id: u64) {
        for queue in self.tasks.values_mut() {
            if let Some(position) = queue
                .requests
                .iter()
                .position(|request| request.id == request_id)
            {
                queue.requests.remove(position);
                break;
            }
        }
        self.remove_empty_tasks();
    }

    fn flush(&mut self) {
        for queue in self.tasks.values_mut() {
            for request in &mut queue.requests {
                if let Some(granted) = request.granted.take() {
                    let _ = granted.send(());
                }
            }
        }
        self.tasks.clear();
        self.round.clear();
    }

    fn grant_next_slice(&mut self, max_slice: u64) -> Option<(String, u64)> {
        loop {
            let task_id = self.round.pop_front()?;
            let Some(queue) = self.tasks.get_mut(&task_id) else {
                continue;
            };

            while queue
                .requests
                .front()
                .is_some_and(|request| request.cancel.is_cancelled())
            {
                queue.requests.pop_front();
            }
            let Some(request) = queue.requests.front_mut() else {
                self.tasks.remove(&task_id);
                continue;
            };

            let slice = request.remaining.min(max_slice);
            if queue.deficit < slice {
                queue.deficit = queue
                    .deficit
                    .saturating_add(max_slice * priority_weight(queue.priority));
            }
            queue.deficit = queue.deficit.saturating_sub(slice);
            request.remaining = request.remaining.saturating_sub(slice);
            let granted_task_id = task_id.clone();

            if request.remaining == 0 {
                if let Some(mut completed) = queue.requests.pop_front() {
                    if let Some(granted) = completed.granted.take() {
                        let _ = granted.send(());
                    }
                }
            }

            if queue.requests.is_empty() {
                self.tasks.remove(&task_id);
            } else if queue
                .requests
                .front()
                .is_some_and(|next| queue.deficit >= next.remaining.min(max_slice))
            {
                self.round.push_front(task_id);
            } else {
                self.round.push_back(task_id);
            }
            return Some((granted_task_id, slice));
        }
    }

    fn remove_empty_tasks(&mut self) {
        self.tasks.retain(|_, queue| !queue.requests.is_empty());
        self.round
            .retain(|task_id| self.tasks.contains_key(task_id));
    }
}

fn priority_weight(priority: i32) -> u64 {
    if priority < 0 {
        HIGH_PRIORITY_WEIGHT
    } else if priority > 0 {
        LOW_PRIORITY_WEIGHT
    } else {
        NORMAL_PRIORITY_WEIGHT
    }
}

async fn run_scheduler(mut commands: mpsc::UnboundedReceiver<Command>, limit: Arc<AtomicU64>) {
    let mut state = SchedulerState::default();
    let mut next_available = Instant::now();

    loop {
        let current_limit = limit.load(Ordering::Relaxed);
        if current_limit == 0 {
            state.flush();
            match commands.recv().await {
                Some(command) => apply_command(command, &mut state),
                None => return,
            }
            continue;
        }

        if state.round.is_empty() {
            match commands.recv().await {
                Some(command) => apply_command(command, &mut state),
                None => return,
            }
            next_available = Instant::now();
            continue;
        }

        let now = Instant::now();
        if next_available > now {
            tokio::select! {
                command = commands.recv() => {
                    match command {
                        Some(command) => apply_command(command, &mut state),
                        None => return,
                    }
                }
                _ = tokio::time::sleep_until(next_available) => {}
            }
            continue;
        }

        let max_slice = bandwidth_slice_for_limit(current_limit);
        if let Some((_task_id, bytes)) = state.grant_next_slice(max_slice) {
            let seconds = bytes as f64 / current_limit as f64;
            next_available = Instant::now() + Duration::from_secs_f64(seconds);
        }
    }
}

fn bandwidth_slice_for_limit(limit: u64) -> u64 {
    BANDWIDTH_SLICE_BYTES.min((limit / BANDWIDTH_SLICES_PER_SECOND).max(1))
}

fn apply_command(command: Command, state: &mut SchedulerState) {
    match command {
        Command::Acquire(request) => state.push(request),
        Command::CancelRequest(request_id) => state.cancel_request(request_id),
        Command::SetPriority { task_id, priority } => state.set_priority(&task_id, priority),
        Command::Wake => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: u64, task_id: &str, slices: u64) -> PermitRequest {
        let (granted, _receiver) = oneshot::channel();
        PermitRequest {
            id,
            task_id: task_id.into(),
            priority: 0,
            remaining: slices * BANDWIDTH_SLICE_BYTES,
            cancel: CancellationToken::new(),
            granted: Some(granted),
        }
    }

    #[test]
    fn priority_bands_map_to_four_two_one_weights() {
        assert_eq!(priority_weight(-1000), 4);
        assert_eq!(priority_weight(-1), 4);
        assert_eq!(priority_weight(0), 2);
        assert_eq!(priority_weight(1), 1);
        assert_eq!(priority_weight(1000), 1);
    }

    #[test]
    fn slice_size_bounds_priority_update_latency() {
        assert_eq!(bandwidth_slice_for_limit(1024 * 1024), 64 * 1024);
        assert_eq!(bandwidth_slice_for_limit(100 * 1024), 10 * 1024);
        assert_eq!(bandwidth_slice_for_limit(5), 1);
    }

    #[test]
    fn weighted_round_robin_grants_four_two_one_slices() {
        let mut state = SchedulerState::default();
        state.push(request(1, "high", 8));
        state.push(request(2, "normal", 8));
        state.push(request(3, "low", 8));
        state.set_priority("high", -1);
        state.set_priority("normal", 0);
        state.set_priority("low", 1);

        let first_round: Vec<_> = (0..7)
            .map(|_| {
                state
                    .grant_next_slice(BANDWIDTH_SLICE_BYTES)
                    .expect("slice should be granted")
                    .0
            })
            .collect();

        assert_eq!(
            first_round,
            ["high", "high", "high", "high", "normal", "normal", "low"]
        );
    }

    #[test]
    fn multiple_connections_share_one_task_weight() {
        let mut state = SchedulerState::default();
        state.push(request(1, "high", 4));
        state.push(request(2, "high", 4));
        state.push(request(3, "normal", 8));
        state.set_priority("high", -1);
        state.set_priority("normal", 0);

        for _ in 0..6 {
            state.grant_next_slice(BANDWIDTH_SLICE_BYTES);
        }

        let high_remaining: u64 = state.tasks["high"]
            .requests
            .iter()
            .map(|request| request.remaining)
            .sum();
        let normal_remaining: u64 = state.tasks["normal"]
            .requests
            .iter()
            .map(|request| request.remaining)
            .sum();
        assert_eq!(high_remaining, 4 * BANDWIDTH_SLICE_BYTES);
        assert_eq!(normal_remaining, 6 * BANDWIDTH_SLICE_BYTES);
    }

    #[test]
    fn live_priority_change_applies_to_pending_requests() {
        let mut state = SchedulerState::default();
        state.push(request(1, "first", 8));
        state.push(request(2, "second", 8));
        state.set_priority("first", -1);
        state.set_priority("second", 0);

        let before_change: Vec<_> = (0..4)
            .map(|_| state.grant_next_slice(BANDWIDTH_SLICE_BYTES).unwrap().0)
            .collect();
        assert_eq!(before_change, ["first", "first", "first", "first"]);

        state.set_priority("first", 1);
        let after_change: Vec<_> = (0..3)
            .map(|_| state.grant_next_slice(BANDWIDTH_SLICE_BYTES).unwrap().0)
            .collect();
        assert_eq!(after_change, ["second", "second", "first"]);
    }

    #[test]
    fn idle_tasks_do_not_reserve_unused_bandwidth() {
        let mut state = SchedulerState::default();
        state.push(request(1, "only-active-task", 3));
        state.set_priority("only-active-task", 1);

        let granted: Vec<_> = (0..3)
            .map(|_| state.grant_next_slice(BANDWIDTH_SLICE_BYTES).unwrap().0)
            .collect();
        assert_eq!(
            granted,
            ["only-active-task", "only-active-task", "only-active-task"]
        );
        assert!(state.tasks.is_empty());
    }

    #[test]
    fn cancelled_request_is_removed_without_affecting_other_tasks() {
        let mut state = SchedulerState::default();
        state.push(request(1, "cancelled", 1));
        state.push(request(2, "active", 1));
        state.cancel_request(1);

        assert_eq!(
            state.grant_next_slice(BANDWIDTH_SLICE_BYTES).unwrap().0,
            "active"
        );
        assert!(state.grant_next_slice(BANDWIDTH_SLICE_BYTES).is_none());
    }

    #[tokio::test]
    async fn unlimited_mode_returns_immediately() {
        let scheduler = BandwidthScheduler::new(0);
        let cancel = CancellationToken::new();
        tokio::time::timeout(
            Duration::from_millis(50),
            scheduler.acquire("task", 1024 * 1024, -1, &cancel),
        )
        .await
        .expect("unlimited scheduler must not delay transfers");
    }

    #[tokio::test]
    async fn cancellation_releases_waiting_caller() {
        let scheduler = BandwidthScheduler::new(1);
        let first_cancel = CancellationToken::new();
        let second_cancel = CancellationToken::new();
        let first = scheduler.acquire("first", BANDWIDTH_SLICE_BYTES * 2, 0, &first_cancel);
        let second = scheduler.acquire("second", BANDWIDTH_SLICE_BYTES, 0, &second_cancel);
        tokio::pin!(first);
        tokio::pin!(second);

        tokio::select! {
            _ = &mut first => {}
            _ = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
        second_cancel.cancel();
        tokio::time::timeout(Duration::from_millis(100), &mut second)
            .await
            .expect("cancelled permit request must wake promptly");
    }
}
