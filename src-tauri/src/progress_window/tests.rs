use super::{authorized, layout::placement, state::Cache};
use crate::models::{AppSettings, BtRuntimeStatus, DownloadTask, TaskKind, TaskStatus};

fn task(id: usize, status: TaskStatus) -> DownloadTask {
    let mut task: DownloadTask = serde_json::from_value(serde_json::json!({
        "id":format!("task-{id}"),"url":"https://source.invalid/file?token=synthetic-url-secret",
        "file_name":format!("download-{id}.bin"),"destination":"C:/synthetic-private-path",
        "total_bytes":1000,"downloaded_bytes":250,"speed":128,"status":"queued",
        "created_at":id as u64,"category":"other","queue_position":id as i64,
        "priority":0,"retry_count":0,"max_retries":3,"source":"desktop",
        "headers":{"Cookie":"synthetic-cookie","Authorization":"synthetic-auth"},
        "per_task_speed_limit":0,"collision_policy":"rename","connection_count":1
    }))
    .unwrap();
    task.status = status;
    task.eta_seconds = Some(6);
    task
}

#[test]
fn only_current_download_work_is_shown_and_completed_bt_seeding_is_excluded() {
    let mut cache = Cache::default();
    let mut rows: Vec<_> = [
        TaskStatus::Queued,
        TaskStatus::Downloading,
        TaskStatus::Verifying,
        TaskStatus::WaitingNetwork,
        TaskStatus::Paused,
        TaskStatus::Completed,
        TaskStatus::Failed,
        TaskStatus::Cancelled,
        TaskStatus::Scheduled,
        TaskStatus::Interrupted,
        TaskStatus::RemoteChanged,
        TaskStatus::PausedByLowDisk,
        TaskStatus::PausedByMetered,
    ]
    .into_iter()
    .enumerate()
    .map(|(id, status)| task(id, status))
    .collect();
    rows[5].task_kind = TaskKind::Bt;
    rows[5].bt_runtime = Some(BtRuntimeStatus {
        seeding: true,
        ..Default::default()
    });
    cache.initialize(rows, &AppSettings::default());
    let view = cache.snapshot();
    assert_eq!(view.total_count, 4);
    assert_eq!(view.total_speed_bps, 128);
    assert!(view
        .tasks
        .iter()
        .all(|task| task.status != TaskStatus::Completed));
    assert_eq!(
        view.tasks
            .iter()
            .find(|task| task.status == TaskStatus::Queued)
            .unwrap()
            .speed_bps,
        0
    );
    assert!(
        !view
            .tasks
            .iter()
            .find(|task| task.status == TaskStatus::Verifying)
            .unwrap()
            .can_pause
    );
}

#[test]
fn safe_snapshot_uses_actual_bytes_and_never_serializes_credentials_or_paths() {
    let mut cache = Cache::default();
    cache.initialize(
        vec![task(1, TaskStatus::Downloading)],
        &AppSettings::default(),
    );
    let view = cache.snapshot();
    assert_eq!(view.tasks[0].downloaded_bytes, 250);
    assert_eq!(view.tasks[0].total_bytes, 1000);
    assert_eq!(view.tasks[0].speed_bps, 128);
    assert_eq!(view.tasks[0].eta_seconds, Some(6));
    let serialized = serde_json::to_string(&view).unwrap();
    for secret in [
        "synthetic-url-secret",
        "synthetic-cookie",
        "synthetic-auth",
        "synthetic-private-path",
        "headers",
        "source.invalid",
    ] {
        assert!(
            !serialized.contains(secret),
            "unexpected private data in progress view"
        );
    }
}

#[test]
fn unknown_magnet_metadata_remains_unknown_in_the_restricted_view() {
    let mut row = task(1, TaskStatus::Downloading);
    row.task_kind = TaskKind::Bt;
    row.bt_runtime = Some(BtRuntimeStatus {
        fetching_metadata: true,
        ..Default::default()
    });
    let mut cache = Cache::default();
    cache.initialize(vec![row], &AppSettings::default());
    let view = cache.snapshot();
    assert_eq!(view.tasks[0].file_name, "待获取");
    assert_eq!(view.tasks[0].total_bytes, 0);
    assert_eq!(view.tasks[0].downloaded_bytes, 0);
    assert_eq!(view.tasks[0].eta_seconds, None);
}

#[test]
fn incremental_events_override_inflight_initial_read_without_resurrecting_removed_rows() {
    let mut cache = Cache::default();
    let initial = vec![
        task(1, TaskStatus::Downloading),
        task(2, TaskStatus::Queued),
    ];
    cache.update(&task(1, TaskStatus::Paused));
    cache.update_id("task-2".into(), None);
    cache.update(&task(3, TaskStatus::Downloading));
    cache.initialize(initial, &AppSettings::default());
    let view = cache.snapshot();
    assert_eq!(view.total_count, 1);
    assert_eq!(view.tasks[0].id, "task-3");
    assert!(!view.loading);
}

#[test]
fn dismiss_stays_hidden_for_the_batch_then_new_work_or_restore_can_show_again() {
    let mut cache = Cache::default();
    cache.initialize(
        vec![
            task(1, TaskStatus::Downloading),
            task(2, TaskStatus::Queued),
        ],
        &AppSettings::default(),
    );
    assert!(cache.should_show());
    cache.dismiss();
    cache.update(&task(1, TaskStatus::Downloading));
    assert!(!cache.should_show());
    cache.update(&task(2, TaskStatus::Downloading));
    assert!(!cache.should_show());
    cache.update(&task(3, TaskStatus::Queued));
    assert!(cache.should_show());
    cache.dismiss();
    cache.update_id("task-1".into(), None);
    cache.update_id("task-2".into(), None);
    cache.update_id("task-3".into(), None);
    assert!(!cache.should_show());
    cache.update(&task(1, TaskStatus::Queued));
    assert!(cache.should_show());
}

#[test]
fn large_queue_has_bounded_rows_and_prioritizes_running_tasks_with_true_totals() {
    let mut rows: Vec<_> = (0..500).map(|id| task(id, TaskStatus::Queued)).collect();
    rows[499].status = TaskStatus::Downloading;
    let mut cache = Cache::default();
    cache.initialize(rows, &AppSettings::default());
    let view = cache.snapshot();
    assert_eq!(view.total_count, 500);
    assert_eq!(view.tasks.len(), 6);
    assert_eq!(view.tasks[0].id, "task-499");
    assert_eq!(view.total_speed_bps, 128);
}

#[test]
fn settings_compatibility_defaults_and_user_choices_survive_roundtrip() {
    let mut raw = serde_json::to_value(AppSettings::default()).unwrap();
    raw.as_object_mut()
        .unwrap()
        .remove("progress_window_enabled");
    raw.as_object_mut()
        .unwrap()
        .remove("progress_window_always_on_top");
    let mut old: AppSettings = serde_json::from_value(raw).unwrap();
    assert!(old.progress_window_enabled && old.progress_window_always_on_top);
    old.progress_window_enabled = false;
    old.progress_window_always_on_top = false;
    let restored: AppSettings =
        serde_json::from_str(&serde_json::to_string(&old).unwrap()).unwrap();
    assert!(!restored.progress_window_enabled && !restored.progress_window_always_on_top);
    let mut cache = Cache::default();
    cache.initialize(vec![task(1, TaskStatus::Downloading)], &restored);
    assert!(!cache.should_show());
    cache.preferences(&AppSettings::default());
    assert!(cache.should_show());
}

#[test]
fn work_area_layout_respects_taskbars_dpi_negative_monitors_and_tiny_areas() {
    let normal = placement(0, 0, 1920, 1040, 1.0, 8, false, false);
    assert_eq!(
        (normal.x, normal.y, normal.width, normal.height),
        (1684, 1000, 224, 28)
    );
    let detailed = placement(0, 0, 1920, 1040, 1.0, 8, false, true);
    assert_eq!(detailed.height, 190);
    assert_eq!(
        detailed.y + detailed.height as i32,
        normal.y + normal.height as i32
    );
    let scaled = placement(-1920, 40, 1920, 1040, 1.25, 8, true, true);
    assert_eq!(
        (scaled.x, scaled.y, scaled.width, scaled.height),
        (-295, 692, 280, 373)
    );
    let tiny = placement(-100, -100, 200, 200, 1.25, 6, true, true);
    assert!(tiny.x >= -100 && tiny.y >= -100);
    assert!(i64::from(tiny.x) + i64::from(tiny.width) <= 100);
    assert!(i64::from(tiny.y) + i64::from(tiny.height) <= 100);
    let empty = placement(0, 0, 0, 0, f64::NAN, 0, false, false);
    assert_eq!((empty.width, empty.height), (1, 1));
}

#[test]
fn compact_reveal_is_transient_and_does_not_reopen_an_inactive_batch() {
    let mut cache = Cache::default();
    assert!(!cache.set_revealed(true));
    cache.initialize(
        vec![task(1, TaskStatus::Downloading)],
        &AppSettings::default(),
    );
    assert!(!cache.snapshot().revealed);
    assert!(cache.set_revealed(true));
    assert!(cache.snapshot().revealed);
    let revision = cache.snapshot().revision;
    assert!(!cache.set_revealed(true));
    assert_eq!(cache.snapshot().revision, revision);
    cache.dismiss();
    assert!(!cache.snapshot().revealed);
    assert!(!cache.set_revealed(true));
    assert!(!cache.should_show());
    cache.update_id("task-1".into(), None);
    cache.update(&task(2, TaskStatus::Queued));
    assert!(cache.should_show());
    assert!(!cache.snapshot().revealed);
    cache.set_revealed(true);
    cache.preferences(&AppSettings {
        progress_window_enabled: false,
        ..Default::default()
    });
    assert!(!cache.snapshot().revealed);
    assert!(!cache.set_revealed(true));
}

#[test]
fn compact_summary_is_weighted_over_all_tasks_including_hidden_rows() {
    let mut rows: Vec<_> = (0..8)
        .map(|id| {
            let mut row = task(id, TaskStatus::Downloading);
            row.downloaded_bytes = 500;
            row
        })
        .collect();
    rows[7].total_bytes = 9000;
    rows[7].downloaded_bytes = 9000;
    let mut cache = Cache::default();
    cache.initialize(rows.clone(), &AppSettings::default());
    let view = cache.snapshot();
    assert_eq!(view.tasks.len(), 6);
    assert_eq!(view.summary_percent, Some(78.125));
    rows[7].total_bytes = 0;
    cache.update(&rows[7]);
    assert_eq!(cache.snapshot().summary_percent, None);
    cache.update_id(rows[7].id.clone(), None);
    assert_eq!(cache.snapshot().summary_percent, Some(50.0));
}

#[test]
fn compact_summary_keeps_metadata_unknown_and_avoids_byte_sum_overflow() {
    let mut first = task(1, TaskStatus::Downloading);
    first.total_bytes = u64::MAX;
    first.downloaded_bytes = 0;
    let mut second = task(2, TaskStatus::Downloading);
    second.total_bytes = u64::MAX;
    second.downloaded_bytes = u64::MAX;
    let mut cache = Cache::default();
    assert_eq!(cache.snapshot().summary_percent, None);
    cache.initialize(vec![first, second.clone()], &AppSettings::default());
    assert_eq!(cache.snapshot().summary_percent, Some(50.0));
    second.task_kind = TaskKind::Bt;
    second.bt_runtime = Some(BtRuntimeStatus {
        fetching_metadata: true,
        ..Default::default()
    });
    cache.update(&second);
    assert_eq!(cache.snapshot().summary_percent, None);
    second.bt_runtime = None;
    second.total_bytes = 10;
    second.downloaded_bytes = u64::MAX;
    cache.update_id("task-1".into(), None);
    cache.update(&second);
    assert_eq!(cache.snapshot().summary_percent, Some(100.0));
}

#[test]
fn progress_commands_are_restricted_to_main_and_progress_windows() {
    assert!(authorized("main").is_ok());
    assert!(authorized("download-progress").is_ok());
    for label in ["player", "image", "", "download-progress-evil"] {
        assert!(authorized(label).is_err());
    }
}
