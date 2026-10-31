use super::*;

#[test]
fn valid_boundaries_preserve_year_long_and_unlimited_task_choices() {
    for connection in [1, MAX_CONNECTION_SECONDS] {
        for total in [None, Some(1), Some(365 * 24 * 3600), Some(MAX_TASK_SECONDS)] {
            for retries in [0, MAX_RETRIES] {
                let policy = RetryPolicy {
                    connection_timeout_secs: connection,
                    task_timeout_secs: total,
                    max_retries: retries,
                    initial_backoff_ms: 1,
                    max_backoff_ms: MAX_BACKOFF_MS,
                    ..Default::default()
                };
                validate(&policy).unwrap();
                assert_eq!(for_runtime(&policy), policy);
            }
        }
    }
    let max_backoff = RetryPolicy {
        initial_backoff_ms: MAX_BACKOFF_MS,
        max_backoff_ms: MAX_BACKOFF_MS,
        ..Default::default()
    };
    validate(&max_backoff).unwrap();
    validate(&RetryPolicy::default()).unwrap();
}

#[test]
fn invalid_new_inputs_fail_with_actionable_messages() {
    let cases = [
        (
            RetryPolicy {
                connection_timeout_secs: 0,
                ..Default::default()
            },
            "连接超时",
        ),
        (
            RetryPolicy {
                connection_timeout_secs: 601,
                ..Default::default()
            },
            "连接超时",
        ),
        (
            RetryPolicy {
                connection_timeout_secs: u64::MAX,
                ..Default::default()
            },
            "连接超时",
        ),
        (
            RetryPolicy {
                task_timeout_secs: Some(0),
                ..Default::default()
            },
            "null",
        ),
        (
            RetryPolicy {
                task_timeout_secs: Some(MAX_TASK_SECONDS + 1),
                ..Default::default()
            },
            "366 天",
        ),
        (
            RetryPolicy {
                task_timeout_secs: Some(u64::MAX),
                ..Default::default()
            },
            "366 天",
        ),
        (
            RetryPolicy {
                max_retries: u32::MAX,
                ..Default::default()
            },
            "0–32",
        ),
        (
            RetryPolicy {
                initial_backoff_ms: 0,
                ..Default::default()
            },
            "初始退避",
        ),
        (
            RetryPolicy {
                initial_backoff_ms: u64::MAX,
                ..Default::default()
            },
            "初始退避",
        ),
        (
            RetryPolicy {
                max_backoff_ms: 0,
                ..Default::default()
            },
            "最大退避",
        ),
        (
            RetryPolicy {
                max_backoff_ms: u64::MAX,
                ..Default::default()
            },
            "最大退避",
        ),
    ];
    for (policy, message) in cases {
        assert!(validate(&policy).unwrap_err().contains(message));
    }
}

#[test]
fn legacy_extremes_are_bounded_without_rewriting_original_values() {
    let original = RetryPolicy {
        connection_timeout_secs: u64::MAX,
        task_timeout_secs: Some(u64::MAX),
        max_retries: u32::MAX,
        initial_backoff_ms: u64::MAX,
        max_backoff_ms: 0,
        ..Default::default()
    };
    let persisted = serde_json::to_string(&original).unwrap();
    let safe = for_runtime(&original);
    validate(&safe).unwrap();
    assert_eq!(safe.connection_timeout_secs, MAX_CONNECTION_SECONDS);
    assert_eq!(safe.task_timeout_secs, Some(MAX_TASK_SECONDS));
    assert_eq!(safe.max_retries, MAX_RETRIES);
    assert_eq!(safe.initial_backoff_ms, MAX_BACKOFF_MS);
    assert_eq!(safe.max_backoff_ms, MAX_BACKOFF_MS);
    assert_eq!(serde_json::to_string(&original).unwrap(), persisted);
    let zero = RetryPolicy {
        connection_timeout_secs: 0,
        task_timeout_secs: Some(0),
        initial_backoff_ms: 0,
        max_backoff_ms: 0,
        ..Default::default()
    };
    let safe = for_runtime(&zero);
    assert_eq!(safe.task_timeout_secs, None);
    assert_eq!(safe.initial_backoff_ms, 1);
    validate(&safe).unwrap();
}

#[test]
fn fixed_and_exponential_backoff_cannot_escape_the_runtime_bounds() {
    for backoff in [BackoffStrategy::Fixed, BackoffStrategy::Exponential] {
        let policy = RetryPolicy {
            backoff,
            initial_backoff_ms: u64::MAX,
            max_backoff_ms: u64::MAX,
            ..Default::default()
        };
        for attempt in [0, 1, 32, u32::MAX] {
            assert_eq!(compute_backoff(&policy, attempt), MAX_BACKOFF_MS);
        }
    }
    let policy = RetryPolicy::default();
    assert_eq!(compute_backoff(&policy, 0), 1000);
    assert_eq!(compute_backoff(&policy, 1), 1000);
    assert_eq!(compute_backoff(&policy, 2), 2000);
    assert_eq!(compute_backoff(&policy, u32::MAX), 60000);
}

#[test]
fn partial_policy_json_and_old_settings_receive_safe_defaults() {
    let default = RetryPolicy::default();
    assert_eq!(serde_json::from_str::<RetryPolicy>("{}").unwrap(), default);
    let partial: RetryPolicy =
        serde_json::from_str(r#"{"max_retries":0,"task_timeout_secs":null}"#).unwrap();
    assert_eq!(
        partial.connection_timeout_secs,
        default.connection_timeout_secs
    );
    assert_eq!(partial.initial_backoff_ms, default.initial_backoff_ms);
    assert_eq!(partial.max_retries, 0);
    let mut old = serde_json::to_value(AppSettings::default()).unwrap();
    old.as_object_mut().unwrap().remove("default_retry_policy");
    let settings: AppSettings = serde_json::from_value(old).unwrap();
    assert_eq!(settings.default_retry_policy, default);
}

#[test]
fn global_settings_entry_rejects_extreme_policy_before_persistence() {
    let mut settings = AppSettings::default();
    settings.default_retry_policy.task_timeout_secs = Some(u64::MAX);
    assert!(super::super::validate_settings(&settings)
        .unwrap_err()
        .contains("366 天"));
    settings.default_retry_policy = RetryPolicy::default();
    settings.default_retry_policy.connection_timeout_secs = 601;
    assert!(super::super::validate_settings(&settings)
        .unwrap_err()
        .contains("600"));
}

#[test]
fn explicit_unlimited_task_override_does_not_fall_back_to_global_timeout() {
    let mut task = super::super::tests::test_task(
        std::path::Path::new("C:/isolated"),
        "payload.bin",
        crate::models::CollisionPolicy::Rename,
    );
    let mut settings = AppSettings::default();
    settings.default_retry_policy.task_timeout_secs = Some(30);
    assert_eq!(effective(&task, &settings).task_timeout_secs, Some(30));
    task.retry_policy_override = Some(RetryPolicy {
        task_timeout_secs: None,
        ..Default::default()
    });
    assert_eq!(effective(&task, &settings).task_timeout_secs, None);
    assert_eq!(
        super::super::effective_retry_policy(&task, &settings).task_timeout_secs,
        None
    );
    task.retry_policy_override
        .as_mut()
        .unwrap()
        .task_timeout_secs = Some(u64::MAX);
    assert_eq!(
        super::super::effective_retry_policy(&task, &settings).task_timeout_secs,
        Some(MAX_TASK_SECONDS)
    );
}

#[tokio::test]
async fn legacy_maximum_timeout_remains_cancelable_and_instant_is_representable() {
    use std::time::{Duration, Instant};
    let policy = for_runtime(&RetryPolicy {
        task_timeout_secs: Some(u64::MAX),
        ..Default::default()
    });
    assert!(Instant::now()
        .checked_add(Duration::from_secs(policy.task_timeout_secs.unwrap()))
        .is_some());
    let token = tokio_util::sync::CancellationToken::new();
    let cancellation = token.clone();
    let remaining = super::super::worker_lifecycle::remaining_budget(&policy, Instant::now());
    let waiting = tokio::spawn(async move {
        super::super::worker_lifecycle::wait_with_budget(
            std::future::pending::<()>(),
            &cancellation,
            remaining,
        )
        .await
    });
    tokio::task::yield_now().await;
    token.cancel();
    let result = tokio::time::timeout(Duration::from_millis(100), waiting)
        .await
        .unwrap()
        .unwrap();
    assert!(result.unwrap_err().contains("暂停"));
}
