use super::*;

fn fixture(mode: &str, marker: &std::path::Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "media::process::tests::subprocess_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("MAOBU_PROCESS_FIXTURE", mode)
        .env("MAOBU_PROCESS_MARKER", marker);
    command
}

#[test]
#[ignore = "internal process fixture, invoked only by supervised-process regression tests"]
fn subprocess_fixture() {
    let mode = std::env::var("MAOBU_PROCESS_FIXTURE").expect("internal fixture mode");
    let marker = std::path::PathBuf::from(std::env::var_os("MAOBU_PROCESS_MARKER").unwrap());
    match mode.as_str() {
        "tree" => {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "media::process::tests::subprocess_fixture",
                    "--ignored",
                    "--nocapture",
                ])
                .env("MAOBU_PROCESS_FIXTURE", "grandchild")
                .env("MAOBU_PROCESS_MARKER", &marker);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000);
            }
            let _child = command.spawn().unwrap();
            std::thread::sleep(Duration::from_secs(30));
        }
        "grandchild" => {
            std::fs::write(marker.with_extension("ready"), b"child actually running").unwrap();
            std::thread::sleep(Duration::from_secs(3));
            std::fs::write(marker, b"escaped child").unwrap();
            std::thread::sleep(Duration::from_secs(30));
        }
        "overflow" => {
            use std::io::Write;
            std::io::stdout().write_all(&vec![b'x'; 32 * 1024]).unwrap();
        }
        "closed-output" => {
            std::thread::sleep(Duration::from_secs(30));
        }
        _ => panic!("unknown internal fixture mode"),
    }
}

#[cfg(windows)]
#[tokio::test]
async fn cancel_and_timeout_kill_the_entire_child_tree_before_more_writes() {
    for cancel_instead in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("escaped.txt");
        let token = CancellationToken::new();
        let cancel = token.clone();
        let ready = marker.with_extension("ready");
        let ready_for_watch = ready.clone();
        let cancellation = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(3), async {
                while !ready_for_watch.exists() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("must observe an actual descendant before testing cancellation");
            if cancel_instead {
                cancel.cancel();
            }
        });
        let result = capture(
            &mut fixture("tree", &marker),
            &token,
            Duration::from_secs(2),
            64 * 1024,
        )
        .await;
        cancellation.await.unwrap();
        assert!(ready.exists());
        assert!(result.is_err_and(|error| if cancel_instead {
            error.contains("暂停")
        } else {
            error.contains("超时")
        }));
        tokio::time::sleep(Duration::from_millis(3300)).await;
        assert!(
            !marker.exists(),
            "cancelled descendant must never write after termination"
        );
    }
}

#[tokio::test]
async fn process_output_has_a_real_memory_bound() {
    let directory = tempfile::tempdir().unwrap();
    let result = capture(
        &mut fixture("overflow", &directory.path().join("unused")),
        &CancellationToken::new(),
        Duration::from_secs(5),
        4096,
    )
    .await;
    assert!(result.is_err_and(|error| error.contains("上限")));
}
