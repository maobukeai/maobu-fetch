use super::{
    authorized,
    commands::Action,
    geometry::placement,
    state::{Cache, DesktopCompletionState},
};
use crate::{
    models::{AppSettings, CompletionNoticeMode},
    progress_window::layout::Placement,
};

fn add(cache: &mut Cache, id: &str, settings: &AppSettings) {
    assert!(cache
        .enqueue(
            id.into(),
            format!("下载完成：{id}"),
            "已保存到本地".into(),
            settings
        )
        .unwrap());
}

#[test]
fn queue_deduplicates_and_advances_only_the_exact_current_instance() {
    let mut cache = Cache::default();
    let settings = AppSettings::default();
    add(&mut cache, "first", &settings);
    let first = cache.snapshot().notice.unwrap();
    add(&mut cache, "second", &settings);
    assert_eq!(cache.snapshot().pending_count, 1);
    assert!(!cache
        .enqueue("first".into(), "duplicate".into(), "".into(), &settings)
        .unwrap());
    assert_eq!(cache.snapshot().notice.unwrap().notice_id, first.notice_id);
    cache.begin_action("first", first.notice_id).unwrap();
    assert!(cache.begin_action("first", first.notice_id).is_err());
    cache.finish_action("first", first.notice_id, false);
    assert_eq!(cache.snapshot().notice.unwrap().task_id, "first");
    cache.begin_action("first", first.notice_id).unwrap();
    cache.finish_action("first", first.notice_id, true);
    assert_eq!(cache.snapshot().notice.unwrap().task_id, "second");
    assert!(cache.check("first", first.notice_id).is_err());
    cache.finish_action("first", first.notice_id, true);
    assert_eq!(cache.snapshot().notice.unwrap().task_id, "second");
    cache.invalidate("first");
    add(&mut cache, "first", &settings);
    assert!(cache.check("first", first.notice_id).is_err());
}

#[test]
fn switching_modes_clears_notices_and_late_old_settings_never_reenable_them() {
    let mut cache = Cache::default();
    let desktop = AppSettings::default();
    add(&mut cache, "first", &desktop);
    let first = cache.snapshot().notice.unwrap();
    cache.presented = Some((first.notice_id, 128.0));
    let mut system = desktop.clone();
    system.completion_notice_mode = CompletionNoticeMode::System;
    cache.preferences(&system);
    assert!(cache.snapshot().notice.is_none());
    assert!(cache.presented.is_none());
    assert!(cache.check("first", first.notice_id).is_err());
    assert!(!cache
        .enqueue("late".into(), "late".into(), "".into(), &desktop)
        .unwrap());
    cache.preferences(&desktop);
    assert!(cache.snapshot().notice.is_none());
    add(&mut cache, "after-switch", &desktop);
    let mut disabled = desktop.clone();
    disabled.notifications = false;
    cache.preferences(&disabled);
    assert!(cache.snapshot().notice.is_none());
}

#[test]
fn queue_is_bounded_and_theme_changes_preserve_current_action_identity() {
    let mut cache = Cache::default();
    let mut settings = AppSettings::default();
    add(&mut cache, "head", &settings);
    let head = cache.snapshot().notice.unwrap();
    for number in 0..1500 {
        add(&mut cache, &format!("task-{number}"), &settings);
    }
    assert_eq!(cache.snapshot().pending_count, 127);
    assert_eq!(cache.snapshot().notice.unwrap(), head);
    settings.language = "en".into();
    settings.completion_notice_seconds = 0;
    cache.preferences(&settings);
    assert!(cache.check("head", head.notice_id).is_ok());
    assert_eq!(cache.snapshot().duration_seconds, 0);
    cache.remove("head");
    assert!(cache.presented.is_none());
}

#[test]
fn commands_allow_only_the_completion_window_and_fixed_actions() {
    assert!(authorized("desktop-completion").is_ok());
    for label in ["main", "download-progress", "player", "other"] {
        assert!(authorized(label).is_err());
    }
    for action in ["file", "folder", "details", "dismiss"] {
        assert!(Action::parse(action).is_ok());
    }
    for action in ["", "C:/private", "../file", "FILE", "file;calc", "run"] {
        assert!(Action::parse(action).is_err());
    }
}

#[test]
fn placement_uses_work_area_dpi_and_avoids_the_visible_progress_window() {
    for scale in [1.0, 1.25] {
        let work = Placement {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1040,
        };
        let obstacle = Placement {
            x: -292,
            y: 730,
            width: 280,
            height: 298,
        };
        let result = placement(&work, scale, 160.0, Some(&obstacle));
        assert!(result.x >= work.x);
        assert!(result.y >= work.y);
        assert!(i64::from(result.x) + i64::from(result.width) <= 0);
        assert!(i64::from(result.y) + i64::from(result.height) <= i64::from(obstacle.y));
        assert_eq!(result.width, (352.0 * scale).round() as u32);
    }
    let work = Placement {
        x: 0,
        y: 0,
        width: 1280,
        height: 680,
    };
    let obstacle = Placement {
        x: 1044,
        y: 200,
        width: 224,
        height: 468,
    };
    let result = placement(&work, 1.0, 480.0, Some(&obstacle));
    assert!(result.x + result.width as i32 <= obstacle.x);
    let tiny = Placement {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };
    let result = placement(&tiny, f64::NAN, 128.0, None);
    assert_eq!(
        (result.x, result.y, result.width, result.height),
        (0, 0, 1, 1)
    );
}

#[test]
fn state_starts_empty_and_does_not_load_or_invent_completed_tasks() {
    let state = DesktopCompletionState::new();
    assert!(state.snapshot().unwrap().notice.is_none());
    assert_eq!(state.snapshot().unwrap().pending_count, 0);
}
