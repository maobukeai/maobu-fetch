use super::*;
use std::cell::{Cell, RefCell};
#[derive(Default)]
struct Sink {
    errors: RefCell<Vec<String>>,
    dialogs: RefCell<Vec<String>>,
    fail_dialog: bool,
}
impl FailureSink for Sink {
    fn stderr(&self, message: &str) {
        self.errors.borrow_mut().push(message.into());
    }
    fn dialog(&self, _title: &str, message: &str) -> Result<(), String> {
        self.dialogs.borrow_mut().push(message.into());
        if self.fail_dialog {
            Err("synthetic UI error password=synthetic-private".into())
        } else {
            Ok(())
        }
    }
}

#[test]
fn interactive_build_error_returns_one_without_running_or_panicking() {
    let sink = Sink::default();
    let ran = Cell::new(false);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        finish_build::<(), _>(
            Err("数据库不可读"),
            Presentation::Interactive,
            None,
            &sink,
            |_| ran.set(true),
        )
    }));
    assert_eq!(result.unwrap(), 1);
    assert!(!ran.get());
    assert_eq!(sink.errors.borrow().len(), 1);
    assert_eq!(sink.dialogs.borrow().len(), 1);
    assert!(sink.dialogs.borrow()[0].contains("请勿直接删除 lumaget.db"));
}

#[test]
fn cli_forward_error_uses_stderr_without_a_dialog() {
    let sink = Sink::default();
    assert_eq!(
        finish_build::<(), _>(Err("启动失败"), Presentation::Console, None, &sink, |_| {}),
        1
    );
    assert_eq!(sink.errors.borrow().len(), 1);
    assert!(sink.dialogs.borrow().is_empty());
}

#[test]
fn successful_build_runs_once_and_returns_zero_without_error_output() {
    let sink = Sink::default();
    let value = Cell::new(0);
    assert_eq!(
        finish_build::<_, String>(Ok(42), Presentation::Interactive, None, &sink, |app| value
            .set(app)),
        0
    );
    assert_eq!(value.get(), 42);
    assert!(sink.errors.borrow().is_empty() && sink.dialogs.borrow().is_empty());
}

#[test]
fn error_details_are_redacted_before_either_sink() {
    let sink = Sink::default();
    let error="Cookie: session=synthetic-cookie\nAuthorization: Bearer synthetic-auth\nproxy=https://tester:synthetic-password@proxy.invalid:8080/?token=synthetic-token";
    finish_build::<(), _>(
        Err(error),
        Presentation::Interactive,
        Some(Path::new("C:/synthetic/data")),
        &sink,
        |_| {},
    );
    for output in sink
        .errors
        .borrow()
        .iter()
        .chain(sink.dialogs.borrow().iter())
    {
        for secret in [
            "synthetic-cookie",
            "synthetic-auth",
            "synthetic-password",
            "synthetic-token",
            "tester:",
        ] {
            assert!(!output.contains(secret));
        }
        assert!(output.contains("数据目录：C:/synthetic/data"));
    }
}

#[test]
fn failed_dialog_keeps_failure_exit_and_reports_only_safe_reason() {
    let sink = Sink {
        fail_dialog: true,
        ..Default::default()
    };
    assert_eq!(
        finish_build::<(), _>(
            Err("配置损坏"),
            Presentation::Interactive,
            None,
            &sink,
            |_| {}
        ),
        1
    );
    let errors = sink.errors.borrow();
    assert_eq!(errors.len(), 2);
    assert!(errors[1].contains("无法显示"));
    assert!(!errors[1].contains("synthetic-private"));
}

#[test]
fn long_unicode_and_nul_details_are_bounded_and_safely_terminated() {
    let detail = format!("\0\u{1b}{}", "损坏".repeat(6000));
    let safe = message(&detail, None);
    assert!(safe.chars().count() < 4096);
    assert!(!safe.contains('\0') && !safe.contains('\u{1b}'));
    let wide = super::native::wide(&safe);
    assert_eq!(wide.last(), Some(&0));
    assert_eq!(wide.iter().filter(|character| **character == 0).count(), 1);
}
