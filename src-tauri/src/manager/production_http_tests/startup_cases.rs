//! Exercise the production initializer with private SQLite data and a fake error sink.
use super::super::*;
use crate::startup_failure::{finish_build, initialize_manager, FailureSink, Presentation};
use std::cell::{Cell, RefCell};

#[derive(Default)]
struct Sink {
    errors: RefCell<Vec<String>>,
    dialogs: RefCell<Vec<String>>,
}
impl FailureSink for Sink {
    fn stderr(&self, message: &str) {
        self.errors.borrow_mut().push(message.into());
    }
    fn dialog(&self, _title: &str, message: &str) -> Result<(), String> {
        self.dialogs.borrow_mut().push(message.into());
        Ok(())
    }
}

pub async fn run(app: &AppHandle) {
    eprintln!("native startup: malformed JSON, wrong field type, damaged SQLite, valid startup");
    for raw in ["{", r#"{"min_file_size_mb":"broken"}"#] {
        corrupt_settings(app, raw).await;
    }
    corrupt_database(app).await;
    valid_startup(app).await;
}

async fn open_store(data: PathBuf) -> Store {
    tokio::task::spawn_blocking(move || Store::open(data))
        .await
        .expect("private SQLite fixture thread completes")
        .expect("private SQLite fixture opens")
}

async fn fixture() -> (tempfile::TempDir, PathBuf, DownloadTask) {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("database");
    let output = root.path().join("downloads");
    fs::create_dir_all(&output).await.unwrap();
    let store = open_store(data.clone()).await;
    let settings = AppSettings {
        download_dir: output.to_string_lossy().into_owned(),
        auto_start: false,
        clipboard_monitor: false,
        proxy_mode: "none".into(),
        media_tool_auto_update: false,
        bt_tracker_auto_update: false,
        auto_check_app_update: false,
        notifications: false,
        notify_on_complete: false,
        notify_on_failure: false,
        ..Default::default()
    };
    store.save_settings(&settings).await.unwrap();
    let mut task =
        super::super::tests::test_task(&output, "original-user-file.bin", CollisionPolicy::Rename);
    task.status = TaskStatus::Paused;
    store.upsert_task(&task).await.unwrap();
    drop(store);
    (root, data, task)
}

async fn corrupt_settings(app: &AppHandle, raw: &str) {
    let (_root, data, task) = fixture().await;
    let db = data.join("lumaget.db");
    let inserted = raw.to_owned();
    let write_path = db.clone();
    tokio::task::spawn_blocking(move || {
        let connection = rusqlite::Connection::open(write_path).unwrap();
        connection
            .execute(
                "UPDATE app_state SET value=?1 WHERE key='settings'",
                [&inserted],
            )
            .unwrap();
    })
    .await
    .unwrap();
    let error = match initialize_manager(data.clone(), app.clone()).await {
        Ok(manager) => {
            manager.shutdown_bt().await.unwrap();
            panic!("corrupt settings must fail");
        }
        Err(error) => error,
    };
    assert!(error.contains("未重置"));
    assert_failure(error, &data);
    let retained = tokio::task::spawn_blocking(move || {
        let connection = rusqlite::Connection::open(db).unwrap();
        connection
            .query_row(
                "SELECT value FROM app_state WHERE key='settings'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(
        retained, raw,
        "startup must preserve the original settings record"
    );
    let store = open_store(data.clone()).await;
    let retained_task = store.get_task(&task.id).await.unwrap().unwrap();
    assert_eq!(retained_task.file_name, task.file_name);
    assert_eq!(retained_task.status, TaskStatus::Paused);
    assert_eq!(store.list_tasks().await.unwrap().len(), 1);
    assert!(
        !data.join("session").exists(),
        "settings failure precedes BT initialization"
    );
}

async fn corrupt_database(app: &AppHandle) {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("database");
    fs::create_dir_all(&data).await.unwrap();
    let path = data.join("lumaget.db");
    let bytes = b"damaged SQLite bytes - preserve this user's data";
    fs::write(&path, bytes).await.unwrap();
    let error = match initialize_manager(data.clone(), app.clone()).await {
        Ok(manager) => {
            manager.shutdown_bt().await.unwrap();
            panic!("damaged SQLite must fail");
        }
        Err(error) => error,
    };
    assert_failure(error, &data);
    assert_eq!(fs::read(path).await.unwrap(), bytes);
    assert!(!data.join("session").exists());
}

fn assert_failure(error: String, data: &Path) {
    let sink = Sink::default();
    let ran = Cell::new(false);
    let boxed: Box<dyn std::error::Error> = error.into();
    let result: Result<(), tauri::Error> = Err(tauri::Error::Setup(boxed.into()));
    let exit = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        finish_build(result, Presentation::Interactive, Some(data), &sink, |_| {
            ran.set(true)
        })
    }))
    .expect("recoverable startup failure must not panic");
    assert_eq!(exit, 1);
    assert!(!ran.get());
    assert_eq!(sink.errors.borrow().len(), 1);
    assert_eq!(sink.dialogs.borrow().len(), 1);
    assert!(sink.dialogs.borrow()[0].contains("请勿直接删除 lumaget.db"));
}

async fn valid_startup(app: &AppHandle) {
    let (_root, data, task) = fixture().await;
    let manager = initialize_manager(data.clone(), app.clone())
        .await
        .expect("valid settings initialize through the production async path");
    assert_eq!(manager.settings().await.download_dir, task.destination);
    assert!(!manager.settings().await.auto_start);
    let sink = Sink::default();
    let callbacks = Cell::new(0);
    assert_eq!(
        finish_build::<_, String>(
            Ok(manager.clone()),
            Presentation::Interactive,
            Some(&data),
            &sink,
            |_| callbacks.set(callbacks.get() + 1)
        ),
        0
    );
    assert_eq!(callbacks.get(), 1);
    assert!(sink.errors.borrow().is_empty() && sink.dialogs.borrow().is_empty());
    manager.shutdown_bt().await.unwrap();
    let retained = manager.store.get_task(&task.id).await.unwrap().unwrap();
    assert_eq!(retained.status, TaskStatus::Paused);
    assert_eq!(retained.file_name, task.file_name);
    assert!(manager.controls.lock().await.is_empty());
    assert_eq!(manager.list().await.unwrap().len(), 1);
}
