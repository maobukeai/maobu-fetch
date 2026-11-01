use super::{
    completions::RETENTION, layout::placement_with_completions, state::Cache, tests::task,
};
use crate::models::{AppSettings, BtRuntimeStatus, TaskKind, TaskStatus};
use std::time::{Duration, Instant};

fn done(cache: &mut Cache, id: usize, now: Instant) {
    cache.update_at(&task(id, TaskStatus::Downloading), now);
    let mut completed = task(id, TaskStatus::Completed);
    completed.downloaded_bytes = completed.total_bytes;
    cache.update_at(&completed, now);
}

fn initialized() -> Cache {
    let mut cache = Cache::default();
    cache.initialize(Vec::new(), &AppSettings::default());
    cache
}

#[test]
fn completed_history_and_unknown_terminal_events_do_not_create_notices() {
    let mut cache = Cache::default();
    cache.initialize(
        vec![task(1, TaskStatus::Completed)],
        &AppSettings::default(),
    );
    cache.update(&task(1, TaskStatus::Completed));
    cache.update(&task(2, TaskStatus::Completed));
    assert_eq!(cache.snapshot().completed_count, 0);
    assert!(!cache.should_show());
}

#[test]
fn completion_during_initial_read_survives_without_resurrecting_active_history() {
    let mut cache = Cache::default();
    let now = Instant::now();
    done(&mut cache, 1, now);
    cache.initialize(
        vec![
            task(1, TaskStatus::Downloading),
            task(2, TaskStatus::Completed),
        ],
        &AppSettings::default(),
    );
    assert_eq!(cache.snapshot().total_count, 0);
    assert_eq!(cache.snapshot().completed_count, 1);
    assert_eq!(cache.snapshot().completed_tasks[0].id, "task-1");
    assert!(cache.should_show());
}

#[test]
fn completed_only_view_lasts_three_minutes_and_duplicate_updates_do_not_extend_it() {
    let mut cache = initialized();
    let now = Instant::now();
    done(&mut cache, 1, now);
    assert_eq!(cache.completion_deadline(), Some(now + RETENTION));
    assert!(cache.set_revealed(true));
    cache.update_at(
        &task(1, TaskStatus::Completed),
        now + Duration::from_secs(179),
    );
    assert_eq!(cache.completion_deadline(), Some(now + RETENTION));
    assert!(cache.should_show());
    cache.expire_completions(now + RETENTION);
    assert_eq!(cache.snapshot().completed_count, 0);
    assert!(!cache.snapshot().revealed);
    assert!(!cache.should_show());
    cache.update_at(
        &task(1, TaskStatus::Completed),
        now + RETENTION + Duration::from_secs(30),
    );
    assert!(!cache.should_show());
    assert_eq!(cache.completion_deadline(), None);
}

#[test]
fn each_completion_has_its_own_deadline_and_the_list_is_bounded() {
    let mut cache = initialized();
    let now = Instant::now();
    for id in 1..=9 {
        done(&mut cache, id, now + Duration::from_secs(id as u64));
    }
    let view = cache.snapshot();
    assert_eq!(view.completed_count, 6);
    assert_eq!(view.completed_tasks[0].id, "task-9");
    assert_eq!(view.completed_tasks[5].id, "task-4");
    cache.expire_completions(now + RETENTION + Duration::from_secs(4));
    assert_eq!(cache.snapshot().completed_count, 5);
    cache.expire_completions(now + RETENTION + Duration::from_secs(9));
    assert_eq!(cache.snapshot().completed_count, 0);
}

#[test]
fn hover_focus_hold_preserves_remaining_time_and_new_completions_get_a_full_interval() {
    let mut cache = initialized();
    let now = Instant::now();
    done(&mut cache, 1, now);
    cache.set_interacting(true, now + Duration::from_secs(60));
    assert_eq!(cache.completion_deadline(), None);
    done(&mut cache, 2, now + Duration::from_secs(120));
    cache.expire_completions(now + Duration::from_secs(600));
    assert_eq!(cache.snapshot().completed_count, 2);
    cache.set_interacting(false, now + Duration::from_secs(600));
    assert_eq!(
        cache.completion_deadline(),
        Some(now + Duration::from_secs(720))
    );
    cache.expire_completions(now + Duration::from_secs(720));
    assert_eq!(cache.snapshot().completed_count, 1);
    assert_eq!(cache.snapshot().completed_tasks[0].id, "task-2");
    cache.expire_completions(now + Duration::from_secs(780));
    assert_eq!(cache.snapshot().completed_count, 0);
}

#[test]
fn task_retry_delete_and_other_terminal_states_remove_the_retained_item() {
    let mut cache = initialized();
    let now = Instant::now();
    for status in [
        TaskStatus::Queued,
        TaskStatus::Paused,
        TaskStatus::Failed,
        TaskStatus::Cancelled,
        TaskStatus::Interrupted,
    ] {
        done(&mut cache, 1, now);
        cache.update_at(&task(1, status), now);
        assert_eq!(cache.snapshot().completed_count, 0);
    }
    done(&mut cache, 1, now);
    cache.update_id("task-1".into(), None);
    assert_eq!(cache.snapshot().completed_count, 0);
    done(&mut cache, 1, now + Duration::from_secs(30));
    assert_eq!(
        cache.completion_deadline(),
        Some(now + Duration::from_secs(210))
    );
}

#[test]
fn dismiss_does_not_revive_old_completions_but_new_completion_can_show_again() {
    let mut cache = initialized();
    let now = Instant::now();
    done(&mut cache, 1, now);
    cache.update_at(&task(2, TaskStatus::Downloading), now);
    cache.dismiss();
    cache.set_interacting(true, now); // Late pointer event after the close.
    assert!(!cache.should_show());
    cache.update_at(&task(1, TaskStatus::Completed), now);
    assert_eq!(cache.snapshot().completed_count, 0);
    cache.update_at(&task(2, TaskStatus::Completed), now);
    assert!(cache.should_show());
    assert_eq!(cache.snapshot().completed_count, 1);
    assert!(cache.completion_deadline().is_some());
}

#[test]
fn disabled_window_clears_notices_and_does_not_replay_finishes_on_enable() {
    let mut cache = initialized();
    let now = Instant::now();
    done(&mut cache, 1, now);
    cache.update_at(&task(2, TaskStatus::Downloading), now);
    cache.preferences(&AppSettings {
        progress_window_enabled: false,
        ..Default::default()
    });
    cache.update_at(&task(2, TaskStatus::Completed), Instant::now());
    assert_eq!(cache.snapshot().completed_count, 0);
    cache.preferences(&AppSettings::default());
    assert!(!cache.should_show());
}

#[test]
fn late_interaction_on_an_empty_or_disabled_window_cannot_pause_a_future_completion() {
    for disabled in [false, true] {
        let mut cache = initialized();
        if disabled {
            cache.preferences(&AppSettings {
                progress_window_enabled: false,
                ..Default::default()
            });
        }
        cache.set_interacting(true, Instant::now());
        if disabled {
            cache.preferences(&AppSettings::default());
        }
        done(&mut cache, 1, Instant::now());
        assert!(cache.completion_deadline().is_some());
    }
}

#[test]
fn opening_holds_the_timer_and_error_allows_three_minutes_to_retry() {
    let mut cache = initialized();
    let now = Instant::now();
    done(&mut cache, 1, now);
    cache
        .begin_completion_open("task-1", now + Duration::from_secs(179))
        .unwrap();
    cache.expire_completions(now + Duration::from_secs(500));
    assert_eq!(cache.snapshot().completed_count, 1);
    cache.finish_completion_open("task-1", true, now + Duration::from_secs(500));
    assert_eq!(
        cache.completion_deadline(),
        Some(now + Duration::from_secs(680))
    );
    cache
        .begin_completion_open("task-1", now + Duration::from_secs(501))
        .unwrap();
    cache.finish_completion_open("task-1", false, now + Duration::from_secs(600));
    assert_eq!(
        cache.completion_deadline(),
        Some(now + Duration::from_secs(779))
    );
    assert!(cache
        .begin_completion_open("other", now + Duration::from_secs(601))
        .is_err());
}

#[test]
fn operation_finish_never_resurrects_dismissed_removed_or_disabled_rows() {
    let now = Instant::now();
    for mode in ["dismiss", "remove", "disable"] {
        let mut cache = initialized();
        done(&mut cache, 1, now);
        cache.begin_completion_open("task-1", now).unwrap();
        match mode {
            "dismiss" => cache.dismiss(),
            "remove" => cache.update_id("task-1".into(), None),
            _ => cache.preferences(&AppSettings {
                progress_window_enabled: false,
                ..Default::default()
            }),
        }
        cache.finish_completion_open("task-1", true, Instant::now());
        assert_eq!(cache.snapshot().completed_count, 0);
        assert_eq!(cache.completion_deadline(), None);
    }
}

#[test]
fn active_progress_totals_stay_independent_and_completion_snapshot_has_no_secrets() {
    let mut cache = initialized();
    let now = Instant::now();
    done(&mut cache, 1, now);
    cache.update_at(&task(2, TaskStatus::Downloading), now);
    let view = cache.snapshot();
    assert_eq!(view.total_count, 1);
    assert_eq!(view.completed_count, 1);
    assert_eq!(view.summary_percent, Some(25.0));
    assert_eq!(view.total_speed_bps, 128);
    assert_eq!(view.completed_tasks[0].speed_bps, 0);
    assert!(!view.completed_tasks[0].can_pause);
    let json = serde_json::to_string(&view).unwrap();
    for secret in [
        "synthetic-url-secret",
        "synthetic-cookie",
        "synthetic-auth",
        "synthetic-private-path",
        "headers",
        "source.invalid",
    ] {
        assert!(!json.contains(secret));
    }
}

#[test]
fn bt_completed_with_seeding_is_captured_once_after_active_download() {
    let mut cache = initialized();
    let now = Instant::now();
    cache.update_at(&task(1, TaskStatus::Downloading), now);
    let mut row = task(1, TaskStatus::Completed);
    row.task_kind = TaskKind::Bt;
    row.bt_runtime = Some(BtRuntimeStatus {
        seeding: true,
        ..Default::default()
    });
    cache.update_at(&row, now);
    cache.update_at(&row, now + Duration::from_secs(100));
    assert_eq!(cache.snapshot().completed_count, 1);
    assert_eq!(cache.completion_deadline(), Some(now + RETENTION));
}

#[test]
fn mixed_tabs_get_extra_height_while_compact_bottom_and_dpi_bounds_stay_fixed() {
    for scale in [1.0, 1.25] {
        let compact =
            placement_with_completions(0, 0, 1920, 1040, scale, 6, false, false, true, false);
        let base =
            placement_with_completions(0, 0, 1920, 1040, scale, 6, false, true, false, false);
        let mixed =
            placement_with_completions(0, 0, 1920, 1040, scale, 6, false, true, true, false);
        assert_eq!(mixed.height, base.height + (24.0 * scale) as u32);
        assert_eq!(
            mixed.y + mixed.height as i32,
            compact.y + compact.height as i32
        );
    }
}

#[test]
fn completed_only_panel_removes_duplicate_footer_and_keeps_action_bar_anchor() {
    for (scale, width, compact_height, row_heights) in [
        (1.0, 224, 28, [94, 166, 274]),
        (1.25, 280, 35, [118, 208, 343]),
    ] {
        let compact =
            placement_with_completions(-1920, 40, 1920, 1040, scale, 6, true, false, false, true);
        assert_eq!((compact.width, compact.height), (width, compact_height));
        for ((count, expanded), height) in [(1, false), (3, false), (6, true)]
            .into_iter()
            .zip(row_heights)
        {
            let completed = placement_with_completions(
                -1920, 40, 1920, 1040, scale, count, expanded, true, false, true,
            );
            assert_eq!((completed.width, completed.height), (width, height));
            assert_eq!(completed.x, compact.x);
            assert_eq!(
                completed.y + completed.height as i32,
                compact.y + compact.height as i32
            );
        }
    }
}

#[test]
fn completed_only_sizing_preserves_other_modes_and_small_work_area_bounds() {
    let active = placement_with_completions(0, 0, 1920, 1040, 1.0, 6, true, true, false, false);
    let mixed = placement_with_completions(0, 0, 1920, 1040, 1.0, 6, true, true, true, false);
    assert_eq!(active.height, 298);
    assert_eq!(mixed.height, 322);
    // Mixed content must never lose its tab/footer space, even with a stale flag.
    assert_eq!(
        placement_with_completions(0, 0, 1920, 1040, 1.0, 6, true, true, true, true),
        mixed
    );
    let bounded =
        placement_with_completions(-100, -100, 200, 200, 1.25, 6, true, true, false, true);
    assert!(bounded.x >= -100 && bounded.y >= -100);
    assert!(i64::from(bounded.x) + i64::from(bounded.width) <= 100);
    assert!(i64::from(bounded.y) + i64::from(bounded.height) <= 100);
}
