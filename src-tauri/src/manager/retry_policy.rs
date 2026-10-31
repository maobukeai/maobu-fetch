//! Shared input validation and bounded runtime handling for older persisted retry policies.
use crate::models::{AppSettings, BackoffStrategy, DownloadTask, RetryPolicy};

pub(super) const MAX_CONNECTION_SECONDS: u64 = 600;
pub(super) const MAX_TASK_SECONDS: u64 = 366 * 24 * 60 * 60;
pub(super) const MAX_RETRIES: u32 = 32;
pub(super) const MAX_BACKOFF_MS: u64 = 60 * 60 * 1000;

/// New writes fail clearly rather than silently storing a different user choice.
pub(super) fn validate(policy: &RetryPolicy) -> Result<(), String> {
    if !(1..=MAX_CONNECTION_SECONDS).contains(&policy.connection_timeout_secs) {
        return Err("连接超时必须为 1–600 秒".into());
    }
    if policy
        .task_timeout_secs
        .is_some_and(|seconds| !(1..=MAX_TASK_SECONDS).contains(&seconds))
    {
        return Err("任务总超时必须为 1–31622400 秒（366 天）；不限制请使用 null".into());
    }
    if policy.max_retries > MAX_RETRIES {
        return Err("最大重试次数必须为 0–32".into());
    }
    if !(1..=MAX_BACKOFF_MS).contains(&policy.initial_backoff_ms) {
        return Err("初始退避时长必须为 1–3600000 毫秒（最多 1 小时）".into());
    }
    if !(policy.initial_backoff_ms..=MAX_BACKOFF_MS).contains(&policy.max_backoff_ms) {
        return Err("最大退避时长必须不少于初始退避时长且不超过 1 小时".into());
    }
    Ok(())
}

/// Old or imported records remain readable. Only the runtime copy is bounded; user data is not rewritten.
pub(super) fn for_runtime(policy: &RetryPolicy) -> RetryPolicy {
    let mut policy = policy.clone();
    policy.connection_timeout_secs = policy
        .connection_timeout_secs
        .clamp(1, MAX_CONNECTION_SECONDS);
    policy.task_timeout_secs = policy
        .task_timeout_secs
        .filter(|seconds| *seconds > 0)
        .map(|seconds| seconds.min(MAX_TASK_SECONDS));
    policy.max_retries = policy.max_retries.min(MAX_RETRIES);
    policy.initial_backoff_ms = policy.initial_backoff_ms.clamp(1, MAX_BACKOFF_MS);
    policy.max_backoff_ms = policy
        .max_backoff_ms
        .clamp(policy.initial_backoff_ms, MAX_BACKOFF_MS);
    policy
}

pub(super) fn effective(task: &DownloadTask, settings: &AppSettings) -> RetryPolicy {
    for_runtime(
        task.retry_policy_override
            .as_ref()
            .unwrap_or(&settings.default_retry_policy),
    )
}

pub(super) fn compute_backoff(policy: &RetryPolicy, attempt: u32) -> u64 {
    let policy = for_runtime(policy);
    match policy.backoff {
        BackoffStrategy::Fixed => policy.initial_backoff_ms,
        BackoffStrategy::Exponential => policy
            .initial_backoff_ms
            .saturating_mul(1u64 << attempt.saturating_sub(1).min(63))
            .min(policy.max_backoff_ms),
    }
}

#[cfg(test)]
mod tests;
