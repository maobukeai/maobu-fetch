use super::*;
use crate::{manager::tests::test_task, models::CollisionPolicy};

#[test]
fn native_actions_accept_only_fixed_arguments_and_restrict_failure_notices() {
    assert_eq!(Action::parse(None, true).unwrap(), Action::Details);
    for (value, action) in [
        ("file", Action::File),
        ("folder", Action::Folder),
        ("details", Action::Details),
    ] {
        assert_eq!(Action::parse(Some(value), true).unwrap(), action);
    }
    for value in [
        "",
        "FILE",
        " file",
        "file ",
        "../file",
        "C:/private",
        "file?task=1",
        "file' /><action",
        "cmd /c calc",
    ] {
        assert!(Action::parse(Some(value), true).is_err());
    }
    assert!(Action::parse(Some("file"), false).is_err());
    assert!(Action::parse(Some("folder"), false).is_err());
    assert_eq!(
        Action::parse(Some("details"), false).unwrap(),
        Action::Details
    );
}

#[tokio::test]
async fn native_file_actions_recheck_completion_and_resolve_only_task_owned_paths() {
    let directory = tempfile::tempdir().unwrap();
    let mut task = test_task(directory.path(), "wanted.txt", CollisionPolicy::Rename);
    assert!(checked_task(None, Action::Details).is_err());
    assert!(checked_task(None, Action::File).is_err());
    for status in [
        TaskStatus::Queued,
        TaskStatus::Downloading,
        TaskStatus::Paused,
        TaskStatus::Failed,
    ] {
        task.status = status;
        assert!(action_path(&task, Action::File).await.is_err());
        assert!(action_path(&task, Action::Folder).await.is_err());
        assert!(action_path(&task, Action::Details).await.unwrap().is_none());
    }
    task.status = TaskStatus::Completed;
    tokio::fs::write(directory.path().join("other.txt"), b"unrelated")
        .await
        .unwrap();
    assert!(action_path(&task, Action::File).await.is_err());
    tokio::fs::write(directory.path().join("wanted.txt"), b"download")
        .await
        .unwrap();
    assert_eq!(
        action_path(&task, Action::File).await.unwrap().unwrap(),
        directory.path().join("wanted.txt").canonicalize().unwrap()
    );
    assert_eq!(
        action_path(&task, Action::Folder).await.unwrap().unwrap(),
        directory.path().canonicalize().unwrap()
    );
    task.file_name = "../outside.txt".into();
    assert!(action_path(&task, Action::File).await.is_err());
}

/// 使用真实 WinRT XML 与三个按钮，但不读取用户任务、不打开用户文件。
#[test]
#[ignore = "交互式 Windows 通知测试，需要依次点击打开文件、打开文件夹、查看详情"]
fn windows_interactive_completion_buttons() {
    let app_id = std::env::var("MAOBU_NOTIFICATION_TEST_APP_ID")
        .unwrap_or_else(|_| Toast::POWERSHELL_APP_ID.to_string());
    let (sender, receiver) = std::sync::mpsc::channel();
    for (step, expected) in ["file", "folder", "details"].iter().enumerate() {
        let sender = sender.clone();
        let title = format!(
            "猫步下载器通知测试 {}/3：文件 & <XML> \"安全\" '转义'.txt",
            step + 1
        );
        completion_toast(
            &app_id,
            &title,
            "已保存到 C:\\通知测试\\A&B\\<仅测试>；本测试不会打开文件",
        )
        .on_activated(move |argument| {
            let _ = sender.send(argument);
            Ok(())
        })
        .show()
        .expect("原生通知发送失败");
        eprintln!("等待点击系统通知按钮：{expected}");
        let argument = receiver
            .recv_timeout(std::time::Duration::from_secs(120))
            .expect("120 秒内未收到原生通知按钮回调");
        assert_eq!(argument.as_deref(), Some(*expected));
    }
}
